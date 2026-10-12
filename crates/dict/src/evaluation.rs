//! Measures a dictionary by converting units with Kanaemi: whether each
//! unit's surface is offered at all, and how many come first against the
//! best order that does not look at the text around them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use kanaemi_core::{Converter, Effect};
use kanaemi_engine::{Engine, LineSink, Selections, Slot, TextDictionary};
use kanaemi_functions::LuauFunctions;
use xxhash_rust::xxh3::xxh3_64;

use crate::Unit;
use crate::dictionary::okuri_line;
use crate::kana::hiragana_to_katakana;
use crate::numeral::Number;
use crate::read_number;

/// What a unit is typed as and the candidate that should come out.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    pub reading: String,
    /// The first kana of the okurigana, when the typist marks where it starts.
    pub okurigana: Option<String>,
    pub expected: String,
}

/// A conjugating unit is typed with its okurigana marked, up to the first kana
/// of the okurigana (書い for 書いた, 食べ for 食べられない); the rest is typed
/// after the conversion. Any other unit, and a conjugating one whose surface
/// does not end in kanji and then hiragana, is typed whole.
pub fn query_of(unit: &Unit) -> Query {
    let marked = unit
        .conjugation
        .as_ref()
        .and_then(|_| okuri_line(&unit.reading, &unit.surface))
        .and_then(|(reading, expected)| {
            let (before, first) = reading.rsplit_once('*')?;
            Some(Query {
                reading: format!("{before}{first}"),
                okurigana: Some(first.to_owned()),
                expected,
            })
        });
    marked.unwrap_or_else(|| Query {
        reading: unit.reading.clone(),
        okurigana: None,
        expected: unit.surface.clone(),
    })
}

/// The candidates Kanaemi shows on one page.
pub const PAGE: usize = 9;

/// What a unit is typed as: the reading with the first kana of the okurigana
/// when it is marked. A numeric unit's reading keeps its placeholder
/// (`{}がい`), so every number typed before the same words falls under one
/// key.
pub type Key = (String, Option<String>);

/// The key a unit falls under, and its surface as counted there: a numeric
/// unit's with its notation's placeholder (`{half-num}階`), so 3階 and 5階
/// count as one surface and 三階 as another.
pub fn key_of(unit: &Unit) -> (Key, String) {
    match &unit.numeric {
        Some(n) => ((n.reading.clone(), None), n.surface.clone()),
        None => {
            let query = query_of(unit);
            ((query.reading, query.okurigana), query.expected)
        }
    }
}

/// How the units of one key came out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct KeyTally {
    /// The units offered their surface, by that surface.
    offered: HashMap<String, u64>,
    /// The units not offered their surface, by that surface.
    missing: HashMap<String, u64>,
    first: u64,
    page: u64,
    /// The candidates that came first, as counted under the key.
    tops: HashMap<String, u64>,
}

fn add_counts(into: &mut HashMap<String, u64>, from: HashMap<String, u64>) {
    for (surface, n) in from {
        *into.entry(surface).or_default() += n;
    }
}

/// The surface counted most, the first in byte order of those tied.
fn most(counts: &HashMap<String, u64>) -> Option<(&str, u64)> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(s, n)| (s.as_str(), *n))
}

impl KeyTally {
    fn merge(&mut self, other: KeyTally) {
        add_counts(&mut self.offered, other.offered);
        add_counts(&mut self.missing, other.missing);
        self.first += other.first;
        self.page += other.page;
        add_counts(&mut self.tops, other.tops);
    }

    fn score(&self) -> Score {
        let offered: u64 = self.offered.values().sum();
        let missing: u64 = self.missing.values().sum();
        let best_page = {
            let mut counts: Vec<u64> = self.offered.values().copied().collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            counts.iter().take(PAGE).sum()
        };
        Score {
            units: offered + missing,
            missing,
            first: self.first,
            best_first: most(&self.offered).map_or(0, |(_, n)| n),
            page: self.page,
            best_page,
        }
    }
}

/// How many units were converted, how many were not offered their surface,
/// and how many of the rest came first and on the first page, against the
/// best order without the text around them: the one that puts the surface
/// the units of a key want most first, and the next ones after it. Ordering
/// is measured on the offered units alone, so a unit not offered counts as
/// missing only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    pub units: u64,
    pub missing: u64,
    pub first: u64,
    pub best_first: u64,
    pub page: u64,
    pub best_page: u64,
}

impl Score {
    /// The units the best order without the text gets first and this did
    /// not; below 0 when the text around the units gets more first.
    pub fn loss(&self) -> i64 {
        self.best_first as i64 - self.first as i64
    }

    /// [`Score::loss`] for the first page.
    pub fn page_loss(&self) -> i64 {
        self.best_page as i64 - self.page as i64
    }

    pub fn merge(&mut self, other: Score) {
        self.units += other.units;
        self.missing += other.missing;
        self.first += other.first;
        self.best_first += other.best_first;
        self.page += other.page;
        self.best_page += other.best_page;
    }
}

/// How the units of every key came out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    keys: HashMap<Key, KeyTally>,
}

impl Tally {
    /// Counts `unit`, offered `offered`, its surface at `rank`.
    pub fn add(&mut self, unit: &Unit, offered: &[String], rank: Option<usize>) {
        let (key, surface) = key_of(unit);
        let top = match (offered.first(), &unit.numeric) {
            (None, _) => String::new(),
            (Some(top), Some(n)) => with_notation(top, &n.value).unwrap_or_else(|| top.clone()),
            (Some(top), None) => top.clone(),
        };
        let tally = self.keys.entry(key).or_default();
        *tally.tops.entry(top).or_default() += 1;
        match rank {
            None => *tally.missing.entry(surface).or_default() += 1,
            Some(rank) => {
                *tally.offered.entry(surface).or_default() += 1;
                tally.first += u64::from(rank == 1);
                tally.page += u64::from(rank <= PAGE);
            }
        }
    }

    pub fn merge(&mut self, other: Tally) {
        for (key, tally) in other.keys {
            self.keys.entry(key).or_default().merge(tally);
        }
    }

    /// Every key's score added up; the best order is the best of each key.
    pub fn score(&self) -> Score {
        let mut score = Score::default();
        for tally in self.keys.values() {
            score.merge(tally.score());
        }
        score
    }
}

/// The reading of `key` as a typist marks it: `か*い` for かい typed with
/// its okurigana from い.
fn shown(key: &Key) -> String {
    match &key.1 {
        Some(first) => {
            let before = key.0.strip_suffix(first.as_str()).unwrap_or(&key.0);
            format!("{before}*{first}")
        }
        None => key.0.clone(),
    }
}

/// The kinds of unit scored apart: a word of the analyzer's smallest units,
/// a compound it groups typed whole, a unit that holds a number, and a
/// conjugating word spelled with other okurigana than its normalized form
/// (分らない), which the base dictionary leaves to a dictionary of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Word,
    Compound,
    Numeric,
    Variant,
}

impl Class {
    pub const ALL: [Class; 4] = [Class::Word, Class::Compound, Class::Numeric, Class::Variant];

    pub fn of(unit: &Unit) -> Class {
        if unit.okurigana_variant {
            Class::Variant
        } else if unit.numeric.is_some() {
            Class::Numeric
        } else if unit.compound {
            Class::Compound
        } else {
            Class::Word
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Word => "word",
            Class::Compound => "compound",
            Class::Numeric => "numeric",
            Class::Variant => "variant",
        }
    }

    /// Whether the base dictionary is to offer the class's surfaces: every
    /// class but the spellings it leaves out.
    pub fn in_total(self) -> bool {
        self != Class::Variant
    }
}

/// A tally per class of unit.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tallies {
    by_class: HashMap<Class, Tally>,
}

impl Tallies {
    pub fn add(&mut self, unit: &Unit, offered: &[String], rank: Option<usize>) {
        self.by_class
            .entry(Class::of(unit))
            .or_default()
            .add(unit, offered, rank);
    }

    pub fn merge(&mut self, other: Tallies) {
        for (class, tally) in other.by_class {
            self.by_class.entry(class).or_default().merge(tally);
        }
    }

    pub fn score(&self, class: Class) -> Score {
        self.by_class
            .get(&class)
            .map(Tally::score)
            .unwrap_or_default()
    }

    /// The units of every class the base dictionary is to offer, together.
    /// A reading typed both as a word and as a compound has one order of
    /// candidates, so the classes' tallies are merged before the best order
    /// is taken, not their scores added.
    pub fn total(&self) -> Score {
        let mut all = Tally::default();
        for (class, tally) in &self.by_class {
            if class.in_total() {
                all.merge(tally.clone());
            }
        }
        all.score()
    }

    fn each_key(&self) -> Vec<(Class, &Key, &KeyTally)> {
        let mut rows: Vec<(Class, &Key, &KeyTally)> = self
            .by_class
            .iter()
            .flat_map(|(class, tally)| tally.keys.iter().map(move |(k, t)| (*class, k, t)))
            .collect();
        rows.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        rows
    }

    /// The surfaces not offered, each with its class, the key it was typed
    /// under and its units, in class and key order.
    pub fn missing(&self) -> Vec<(Class, Key, String, u64)> {
        let mut rows: Vec<(Class, Key, String, u64)> = self
            .each_key()
            .into_iter()
            .flat_map(|(class, key, tally)| {
                tally
                    .missing
                    .iter()
                    .map(move |(surface, n)| (class, key.clone(), surface.clone(), *n))
            })
            .collect();
        rows.sort();
        rows
    }

    /// The table of the units not offered their surface: class, reading,
    /// surface, the candidate that came first most often for the key, and
    /// the units, the most first.
    pub fn misses_tsv(&self) -> String {
        let mut rows: Vec<(Class, String, &str, &str, u64)> = Vec::new();
        for (class, key, tally) in self.each_key() {
            let top = most(&tally.tops).map_or("", |(s, _)| s);
            for (surface, n) in &tally.missing {
                rows.push((class, shown(key), surface, top, *n));
            }
        }
        rows.sort_by(|a, b| {
            b.4.cmp(&a.4)
                .then_with(|| (a.0, &a.1, a.2).cmp(&(b.0, &b.1, b.2)))
        });
        let mut tsv = String::from("class\treading\texpected\ttop\tunits\n");
        for (class, reading, surface, top, n) in rows {
            writeln!(tsv, "{}\t{reading}\t{surface}\t{top}\t{n}", class.name())
                .expect("writing to a String never fails");
        }
        tsv
    }

    /// `count` rows of [`Tallies::misses_tsv`] for people to judge whether
    /// the answer is right or the analyzer's error, picked by a hash of each
    /// row, the same rows each time, with an empty column to judge in.
    pub fn missing_sample_tsv(&self, count: usize) -> String {
        let misses = self.misses_tsv();
        let mut lines = misses.lines();
        let header = lines.next().unwrap_or_default();
        let mut rows: Vec<(u64, &str)> = lines.map(|l| (xxh3_64(l.as_bytes()), l)).collect();
        rows.sort_unstable();
        let mut tsv = format!("{header}\tjudgment\n");
        for (_, line) in rows.into_iter().take(count) {
            writeln!(tsv, "{line}\t").expect("writing to a String never fails");
        }
        tsv
    }

    /// The table of the keys whose first candidates differ from the best
    /// order's: class, reading, offered units, the surface the best order
    /// puts first and its units, the units first, the loss, and the
    /// candidate that came first most often; the greatest loss first, gains
    /// last.
    pub fn losses_tsv(&self) -> String {
        let mut rows: Vec<(Class, String, Score, &str, &str)> = Vec::new();
        for (class, key, tally) in self.each_key() {
            let score = tally.score();
            if score.loss() != 0 {
                let best = most(&tally.offered).map_or("", |(s, _)| s);
                let top = most(&tally.tops).map_or("", |(s, _)| s);
                rows.push((class, shown(key), score, best, top));
            }
        }
        rows.sort_by(|a, b| {
            b.2.loss()
                .cmp(&a.2.loss())
                .then_with(|| (a.0, &a.1).cmp(&(b.0, &b.1)))
        });
        let mut tsv = String::from("class\treading\toffered\tbest\tbest_units\tfirst\tloss\ttop\n");
        for (class, reading, score, best, top) in rows {
            writeln!(
                tsv,
                "{}\t{reading}\t{}\t{best}\t{}\t{}\t{}\t{top}",
                class.name(),
                score.units - score.missing,
                score.best_first,
                score.first,
                score.loss(),
            )
            .expect("writing to a String never fails");
        }
        tsv
    }

    /// The table of every key, to compare one evaluation with another:
    /// class, reading, units, missing, first, best first, page, best page,
    /// in class and reading order.
    pub fn keys_tsv(&self) -> String {
        let mut tsv =
            String::from("class\treading\tunits\tmissing\tfirst\tbest_first\tpage\tbest_page\n");
        for (class, key, tally) in self.each_key() {
            let s = tally.score();
            writeln!(
                tsv,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                class.name(),
                shown(key),
                s.units,
                s.missing,
                s.first,
                s.best_first,
                s.page,
                s.best_page,
            )
            .expect("writing to a String never fails");
        }
        tsv
    }
}

/// `surface` with the first run of number characters that writes `value` as
/// the placeholder of its notation (`{kanji}階` for 三階), as a numeric unit's
/// surface is written.
pub(crate) fn with_notation(surface: &str, value: &str) -> Option<String> {
    let (start, end, number) = number_run(surface, value)?;
    Some(format!(
        "{}{}{}",
        &surface[..start],
        number.notation.placeholder(),
        &surface[end..]
    ))
}

/// Where the first run of number characters writing `value` lies in
/// `surface`, and the number it writes.
fn number_run(surface: &str, value: &str) -> Option<(usize, usize, Number)> {
    let is_number_char = |c: char| {
        c.is_ascii_digit()
            || ('０'..='９').contains(&c)
            || c == ','
            || "〇一二三四五六七八九十百千万億兆京壱弐参拾".contains(c)
    };
    let mut start = None;
    for (i, c) in surface.char_indices().chain([(surface.len(), ' ')]) {
        match (start, i < surface.len() && is_number_char(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                if let Some(number) = read_number(&surface[s..i]).filter(|n| n.value == value) {
                    return Some((s, i, number));
                }
                start = None;
            }
            _ => {}
        }
    }
    None
}

/// An input field the evaluation types into: a converter that reads the
/// text typed there.
pub trait Field: Converter {
    /// Forgets the field's text and anything learned, as for a typist who
    /// has never committed.
    fn start_over(&mut self);
    fn type_text(&mut self, text: &str);
}

impl Field for Engine {
    fn start_over(&mut self) {
        self.learn(&Effect::FocusMoved);
        // Moving the focus keeps the picks, which a typist carries from field
        // to field; left in, they would tie a document's ranks to whichever
        // documents the same engine converted before it.
        self.replace_selections(Selections::default());
    }

    fn type_text(&mut self, text: &str) {
        self.learn(&Effect::Typed(text.to_owned()));
    }
}

/// Converts a document's units in order, handing `record` each unit with
/// the surfaces offered and its surface's rank, in one input field that
/// starts the document over and takes the document's `text` before each
/// unit as the text typed so far. Nothing is committed: a commit moves what
/// was picked up the next time, which is Kanaemi learning the typist, not
/// the dictionary. A model reads the text before the unit.
pub fn evaluate_document(
    field: &mut impl Field,
    units: &[Unit],
    text: impl AsRef<str>,
    mut record: impl FnMut(&Unit, &[String], Option<usize>),
) {
    field.start_over();
    let chars: Vec<char> = text.as_ref().chars().collect();
    let mut typed = 0;
    for unit in units {
        let upto = unit.position.min(chars.len());
        if upto > typed {
            field.type_text(&chars[typed..upto].iter().collect::<String>());
            typed = upto;
        }
        let query = query_of(unit);
        let offered = offered(field, &query.reading, query.okurigana.as_deref());
        let rank = rank_in(&offered, &query.expected);
        record(unit, &offered, rank);
    }
}

/// The 1-based rank of the expected candidate, if it is offered at all. As
/// Kanaemi does, the reading's katakana follows the conversion results when
/// they lack it, with okurigana given or not.
pub(crate) fn rank_of(converter: &impl Converter, query: &Query) -> Option<usize> {
    rank_in(
        &offered(converter, &query.reading, query.okurigana.as_deref()),
        &query.expected,
    )
}

fn rank_in(offered: &[String], expected: &str) -> Option<usize> {
    offered.iter().position(|s| s == expected).map(|i| i + 1)
}

/// The surfaces Kanaemi offers for `reading`, best first: the conversion
/// results, then the reading's katakana when they lack it.
pub(crate) fn offered(
    converter: &impl Converter,
    reading: &str,
    okurigana: Option<&str>,
) -> Vec<String> {
    let mut surfaces: Vec<String> = converter
        .convert(reading, okurigana)
        .into_iter()
        .map(|c| c.surface)
        .collect();
    let katakana = hiragana_to_katakana(reading);
    if !surfaces.contains(&katakana) {
        surfaces.push(katakana);
    }
    surfaces
}

/// Registrations go nowhere: the evaluation's user dictionary starts empty
/// and is never kept.
struct Discard;

impl LineSink for Discard {
    fn append(&mut self, _line: &str) -> io::Result<()> {
        Ok(())
    }
}

/// A path that names no folder, so [`LuauFunctions`] reads only Kanaemi's
/// built-in functions, as a typist who has added none has.
const NO_FUNCTIONS_FOLDER: &str = "";

/// A Kanaemi engine with `dictionary` as its only dictionary besides an
/// empty user custom dictionary, filling placeholders with the built-in
/// functions. Engines on several threads share one parsed dictionary; each
/// has functions of its own, which stay on its thread.
pub fn engine(dictionary: Arc<TextDictionary>) -> Engine {
    engine_with(&[dictionary])
}

/// [`engine`] with `dictionaries` in their order of priority, as a typist
/// lists the base dictionary first and the additional ones after it.
pub fn engine_with(dictionaries: &[Arc<TextDictionary>]) -> Engine {
    let mut engine = Engine::new(
        dictionaries
            .iter()
            .map(|d| Slot::Dictionary(Box::new(d.clone()))),
        TextDictionary::default(),
        Discard,
    );
    engine.set_functions(Some(Rc::new(LuauFunctions::open(NO_FUNCTIONS_FOLDER))));
    engine
}

#[cfg(test)]
mod tests {
    use kanaemi_core::Candidate;

    use super::*;
    use crate::Numeric;

    fn unit(reading: &str, surface: &str, stem: Option<(&str, &str)>) -> Unit {
        Unit {
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
            doc_id: "d".into(),
            position: 0,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: stem.map(|s| s.0.into()),
            stem_surface: stem.map(|s| s.1.into()),
            conjugation: stem.map(|_| "五段-カ行".into()),
            unknown_conjugation: None,
            numeric: None,
        }
    }

    fn numeric(reading: &str, surface: &str, placed: (&str, &str), value: &str) -> Unit {
        Unit {
            numeric: Some(Numeric {
                reading: placed.0.into(),
                surface: placed.1.into(),
                value: value.into(),
                kana: String::new(),
            }),
            ..unit(reading, surface, None)
        }
    }

    fn query(reading: &str, okurigana: Option<&str>, expected: &str) -> Query {
        Query {
            reading: reading.into(),
            okurigana: okurigana.map(Into::into),
            expected: expected.into(),
        }
    }

    fn strings(surfaces: &[&str]) -> Vec<String> {
        surfaces.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn conjugating_units_are_typed_up_to_the_first_kana_of_their_okurigana() {
        assert_eq!(
            query_of(&unit("かいた", "書いた", Some(("か", "書")))),
            query("かい", Some("い"), "書い")
        );
        assert_eq!(
            query_of(&unit(
                "たべられない",
                "食べられない",
                Some(("たべ", "食べ"))
            )),
            query("たべ", Some("べ"), "食べ")
        );
    }

    #[test]
    fn other_units_are_typed_whole() {
        assert_eq!(
            query_of(&unit("てがみ", "手紙", None)),
            query("てがみ", None, "手紙")
        );
        assert_eq!(
            query_of(&unit("みなさん", "皆さん", None)),
            query("みなさん", None, "皆さん")
        );
    }

    #[test]
    fn a_conjugating_unit_without_kanji_before_its_okurigana_is_typed_whole() {
        assert_eq!(
            query_of(&unit("ぐぐった", "ググった", Some(("ぐぐ", "ググ")))),
            query("ぐぐった", None, "ググった")
        );
    }

    #[test]
    fn a_unit_falls_under_what_is_typed_and_a_numeric_one_under_its_placeholders() {
        assert_eq!(
            key_of(&unit("かいた", "書いた", Some(("か", "書")))),
            (
                ("かい".to_owned(), Some("い".to_owned())),
                "書い".to_owned()
            )
        );
        assert_eq!(
            key_of(&numeric("3がい", "3階", ("{}がい", "{half-num}階"), "3")),
            (("{}がい".to_owned(), None), "{half-num}階".to_owned())
        );
    }

    #[test]
    fn the_number_of_a_surface_becomes_its_notations_placeholder() {
        assert_eq!(with_notation("3階", "3").as_deref(), Some("{half-num}階"));
        assert_eq!(with_notation("三階", "3").as_deref(), Some("{kanji}階"));
        assert_eq!(
            with_notation("第3回", "3").as_deref(),
            Some("第{half-num}回")
        );
        assert_eq!(
            with_notation("二〇一七年", "2017").as_deref(),
            Some("{kanji-num}年")
        );
        assert_eq!(
            with_notation("1,000円", "1000").as_deref(),
            Some("{grouped-num}円")
        );
        assert_eq!(with_notation("一重", "2"), None, "another number");
        assert_eq!(with_notation("ひとつ", "1"), None);
    }

    fn tally(rows: &[(&str, &[&str], u64)]) -> Tally {
        let mut tally = Tally::default();
        for (surface, offered, n) in rows {
            let offered = strings(offered);
            let rank = rank_in(&offered, surface);
            for _ in 0..*n {
                tally.add(&unit("てがみ", surface, None), &offered, rank);
            }
        }
        tally
    }

    #[test]
    fn the_loss_is_what_the_best_order_gets_first_and_the_dictionary_did_not() {
        let order: &[&str] = &["手神", "手紙"];

        let score = tally(&[("手紙", order, 55), ("手神", order, 45)]).score();

        assert_eq!(
            score,
            Score {
                units: 100,
                missing: 0,
                first: 45,
                best_first: 55,
                page: 100,
                best_page: 100,
            }
        );
        assert_eq!(score.loss(), 10);
    }

    #[test]
    fn a_surface_wanted_far_more_often_loses_much_when_second() {
        let order: &[&str] = &["手神", "手紙"];

        let score = tally(&[("手紙", order, 95), ("手神", order, 5)]).score();

        assert_eq!(score.loss(), 90);
    }

    #[test]
    fn a_unit_not_offered_counts_as_missing_alone() {
        let order: &[&str] = &["手紙"];

        let score = tally(&[("手紙", order, 2), ("手神", order, 3)]).score();

        assert_eq!(score.missing, 3);
        assert_eq!((score.first, score.best_first), (2, 2));
        assert_eq!(score.loss(), 0);
    }

    #[test]
    fn the_best_page_holds_the_nine_surfaces_wanted_most() {
        let surfaces: Vec<String> = (0..10).map(|i| format!("項{i}")).collect();
        let mut order: Vec<&str> = vec!["香"; PAGE];
        order.extend(surfaces.iter().map(String::as_str));
        let rows: Vec<(&str, &[&str], u64)> = surfaces
            .iter()
            .map(|s| (s.as_str(), order.as_slice(), 1))
            .collect();

        let score = tally(&rows).score();

        assert_eq!((score.page, score.best_page), (0, PAGE as u64));
        assert_eq!(score.page_loss(), PAGE as i64);
    }

    #[test]
    fn a_text_that_gets_more_first_than_the_best_order_is_a_gain() {
        let mut tally = Tally::default();
        let letter = unit("てがみ", "手紙", None);
        let god = unit("てがみ", "手神", None);
        tally.add(&letter, &strings(&["手紙", "手神"]), Some(1));
        tally.add(&god, &strings(&["手神", "手紙"]), Some(1));

        assert_eq!(tally.score().loss(), -1);
    }

    #[test]
    fn numbers_written_the_same_way_fall_under_one_key_with_their_first_candidates() {
        let mut tally = Tally::default();
        for (reading, surface, value) in [("3がい", "3階", "3"), ("5がい", "5階", "5")] {
            let floor = numeric(reading, surface, ("{}がい", "{half-num}階"), value);
            let first = format!("{value}回");
            tally.add(&floor, &strings(&[&first, surface]), Some(2));
        }
        let mut tallies = Tallies::default();
        tallies.by_class.insert(Class::Numeric, tally);

        assert_eq!(
            tallies.losses_tsv(),
            "class\treading\toffered\tbest\tbest_units\tfirst\tloss\ttop\n\
             numeric\t{}がい\t2\t{half-num}階\t2\t0\t2\t{half-num}回\n"
        );
    }

    #[test]
    fn units_are_tallied_by_class() {
        let mut tallies = Tallies::default();
        let compound = Unit {
            compound: true,
            ..unit("ろてんぶろ", "露天風呂", None)
        };
        let three = numeric("3ぼん", "3本", ("{}ぼん", "{half-num}本"), "3");

        tallies.add(&unit("てがみ", "手紙", None), &strings(&["手紙"]), Some(1));
        tallies.add(&compound, &strings(&["ロテンブロ"]), None);
        tallies.add(&three, &strings(&["3本"]), Some(1));

        assert_eq!(tallies.score(Class::Word).first, 1);
        assert_eq!(tallies.score(Class::Compound).missing, 1);
        assert_eq!(tallies.score(Class::Numeric).first, 1);
        assert_eq!(tallies.total().units, 3);
    }

    #[test]
    fn a_spelling_with_other_okurigana_is_tallied_apart_and_left_out_of_the_total() {
        let mut tallies = Tallies::default();
        let variant = Unit {
            okurigana_variant: true,
            parts: Vec::new(),
            ..unit("かわる", "変る", Some(("か", "変")))
        };
        tallies.add(&variant, &strings(&["変わる"]), None);
        tallies.add(&unit("てがみ", "手紙", None), &strings(&["手紙"]), Some(1));

        assert_eq!(Class::of(&variant), Class::Variant);
        assert_eq!(tallies.score(Class::Variant).missing, 1);
        assert_eq!((tallies.total().units, tallies.total().missing), (1, 0));
    }

    #[test]
    fn a_reading_typed_as_a_word_and_as_a_compound_has_one_best_order_in_the_total() {
        let mut tallies = Tallies::default();
        let order = strings(&["W杯", "ワールドカップ"]);
        for _ in 0..2 {
            tallies.add(&unit("わーるどかっぷ", "W杯", None), &order, Some(1));
        }
        let compound = Unit {
            compound: true,
            ..unit("わーるどかっぷ", "ワールドカップ", None)
        };
        tallies.add(&compound, &order, Some(2));

        let total = tallies.total();

        assert_eq!((total.first, total.best_first), (2, 2));
        assert_eq!(total.loss(), 0);
    }

    #[test]
    fn the_tables_list_what_was_not_offered_what_lost_and_every_key() {
        let mut tallies = Tallies::default();
        let order = strings(&["手神", "手紙"]);
        for _ in 0..2 {
            tallies.add(&unit("てがみ", "手紙", None), &order, Some(2));
        }
        tallies.add(&unit("てがみ", "手神", None), &order, Some(1));
        tallies.add(
            &unit("かいた", "書いた", Some(("か", "書"))),
            &strings(&["買い"]),
            None,
        );

        assert_eq!(
            tallies.misses_tsv(),
            "class\treading\texpected\ttop\tunits\n\
             word\tか*い\t書い\t買い\t1\n"
        );
        assert_eq!(
            tallies.losses_tsv(),
            "class\treading\toffered\tbest\tbest_units\tfirst\tloss\ttop\n\
             word\tてがみ\t3\t手紙\t2\t1\t1\t手神\n"
        );
        assert_eq!(
            tallies.keys_tsv(),
            "class\treading\tunits\tmissing\tfirst\tbest_first\tpage\tbest_page\n\
             word\tか*い\t1\t1\t0\t0\t0\t0\n\
             word\tてがみ\t3\t0\t1\t2\t3\t3\n"
        );
    }

    #[test]
    fn the_sample_of_what_was_not_offered_is_the_same_each_time_with_a_column_to_judge() {
        let mut tallies = Tallies::default();
        for surface in ["獏", "貘", "漠", "幕"] {
            tallies.add(&unit("ばく", surface, None), &strings(&["バク"]), None);
        }

        let sample = tallies.missing_sample_tsv(2);

        assert_eq!(sample, tallies.missing_sample_tsv(2));
        let lines: Vec<&str> = sample.lines().collect();
        assert_eq!(lines[0], "class\treading\texpected\ttop\tunits\tjudgment");
        assert_eq!(lines.len(), 3);
        assert!(lines[1..].iter().all(|l| l.ends_with("\t1\t")));
    }

    /// Offers `candidates` for every reading, and records the text typed.
    #[derive(Default)]
    struct Fake {
        candidates: Vec<&'static str>,
        texts: Vec<String>,
    }

    impl Converter for Fake {
        fn convert(&self, _reading: &str, _okurigana: Option<&str>) -> Vec<Candidate> {
            self.candidates
                .iter()
                .map(|c| Candidate {
                    surface: (*c).into(),
                    source: None,
                    preview: None,
                })
                .collect()
        }
    }

    impl Field for Fake {
        fn start_over(&mut self) {
            self.texts.clear();
        }

        fn type_text(&mut self, text: &str) {
            self.texts.push(text.into());
        }
    }

    #[test]
    fn the_readings_katakana_follows_the_conversion_results_as_in_kanaemi() {
        let fake = |candidates| Fake {
            candidates,
            ..Fake::default()
        };
        let page = query("ぺーじ", None, "ページ");

        assert_eq!(rank_of(&fake(vec!["頁"]), &page), Some(2));
        assert_eq!(rank_of(&fake(vec![]), &page), Some(1));
        assert_eq!(rank_of(&fake(vec!["ページ", "頁"]), &page), Some(1));
        assert_eq!(
            rank_of(&fake(vec!["書い"]), &query("かい", Some("い"), "カイ")),
            Some(2),
            "with okurigana too"
        );
    }

    #[test]
    fn the_text_before_each_unit_reaches_the_field() {
        let mut field = Fake {
            texts: vec!["前の文書".into()],
            ..Fake::default()
        };
        let mut units = [unit("かん", "漢", None), unit("じ", "字", None)];
        units[0].position = 1;
        units[1].position = 5;

        evaluate_document(&mut field, &units, "あ漢い\nう字。", |_, _, _| {});

        assert_eq!(field.texts, ["あ", "漢い\nう"]);
    }

    fn dictionary(text: &str) -> Arc<TextDictionary> {
        let (dictionary, invalid) = TextDictionary::parse(text);
        assert_eq!(invalid, []);
        Arc::new(dictionary)
    }

    #[test]
    fn kanaemi_converts_the_queries_with_the_dictionary() {
        let dictionary = dictionary("か\t書\t五段-カ行\nかい\t会\nてがみ\t手紙\n");
        let units = [
            unit("かいた", "書いた", Some(("か", "書"))),
            unit("てがみ", "手紙", None),
        ];
        let mut tallies = Tallies::default();

        evaluate_document(&mut engine(dictionary), &units, "", |u, o, r| {
            tallies.add(u, o, r)
        });

        let score = tallies.score(Class::Word);
        assert_eq!((score.units, score.missing, score.first), (2, 0, 2));
    }

    #[test]
    fn kanaemi_writes_numbers_with_its_built_in_notations() {
        let dictionary = dictionary("{}ほん\t{kanji}本\n{}ほん\t{half-num}本\n");
        let surfaces = |reading: &str| -> Vec<String> {
            let mut surfaces: Vec<String> = engine(dictionary.clone())
                .convert(reading, None)
                .into_iter()
                .map(|c| c.surface)
                .collect();
            surfaces.sort();
            surfaces
        };

        assert_eq!(surfaces("3ほん"), ["3本", "三本"]);
        assert_eq!(
            surfaces("３ほん"),
            ["3本", "三本"],
            "half-num writes half-width"
        );
    }

    #[test]
    fn a_surface_typed_before_in_the_document_does_not_move_up() {
        let dictionary = dictionary("てがみ\t手紙\t\t100\nてがみ\t手神\t\t200\n");
        let units = [unit("てがみ", "手神", None), unit("てがみ", "手神", None)];
        let mut tallies = Tallies::default();

        evaluate_document(&mut engine(dictionary), &units, "", |u, o, r| {
            tallies.add(u, o, r)
        });

        assert_eq!(tallies.score(Class::Word).first, 0);
    }
}

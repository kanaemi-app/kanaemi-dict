//! Dictionary entries from conversion units and UniDic words, written as
//! Kanaemi's text dictionary.

use std::borrow::Borrow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use kanaemi_engine::{InvalidLine, ItemLine, TextDictionary, mark_placeholders, may_follow_stem};

use crate::kana::{HA_ROW, PLAIN, SEMI_VOICED, VOICED, is_hiragana, is_kanji};
use crate::{UnidicWord, Unit};

/// The label of the base dictionary in its description line.
pub const BASE_LABEL: &str = "基本";

/// An item the dictionary carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub reading: String,
    pub surface: String,
    /// The conjugation type of a stem; `None` for a word written whole.
    pub conjugation: Option<String>,
    pub cost: u32,
}

/// Okurigana of a conjugation type that neither are nor begin a reading
/// Kanaemi allows after a stem of the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisallowedOkurigana {
    pub conjugation: String,
    pub okurigana: String,
    /// Units with these okurigana.
    pub count: usize,
    /// The surface of the first such unit.
    pub example: String,
}

/// What the build reports for a person to look at.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Conjugation types Kanaemi's table does not know, with unit counts.
    pub unknown_conjugations: Vec<(String, usize)>,
    pub disallowed_okurigana: Vec<DisallowedOkurigana>,
}

#[derive(Debug, Default)]
pub struct Dictionary {
    pub entries: Vec<Entry>,
    /// Lines for typing with okurigana, such as 書く under `か*く`, whose
    /// reading marks the okurigana with its last `*`.
    pub okuri: Vec<Entry>,
    /// Numeric items, whose reading and surface hold the number as a
    /// placeholder (`{}ほん` for `{}本`).
    pub numeric: Vec<Entry>,
    pub report: Report,
}

/// What a line of the dictionary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A word written whole, or the stem of a conjugating word.
    Word,
    /// A word typed with its okurigana marked (`か*く` for 書く).
    Okurigana,
    /// A numeric item, with the number as a placeholder.
    Numeric,
}

/// A dictionary text with lines Kanaemi would not read.
#[derive(Debug, thiserror::Error)]
#[error("Kanaemi rejects {} line(s) of the dictionary, the first being {:?}", .0.len(), .0.first())]
pub struct RejectedLines(pub Vec<InvalidLine>);

/// Stems and attested forms need this many occurrences to enter.
pub(crate) const MIN_COUNT: usize = 2;
/// Added to a UniDic word's cost when its surface occurs in the units.
const UNIDIC_EXTRA: u32 = 1000;
/// The cost of a UniDic word whose surface no unit shows.
const UNIDIC_UNSEEN: u32 = 3000;

type Key = (String, String, Option<String>);

impl Dictionary {
    /// Builds the dictionary from every unit given and the UniDic words.
    pub fn build<U: Borrow<Unit>>(
        units: impl IntoIterator<Item = U>,
        unidic: &[UnidicWord],
    ) -> Self {
        let mut total = 0usize;
        // A whole form and a plain word with the same reading and surface are
        // one item, so they share one count.
        let mut counts: HashMap<Key, usize> = HashMap::new();
        let mut numeric_counts: HashMap<(String, String), usize> = HashMap::new();
        let mut surfaces: HashMap<String, usize> = HashMap::new();
        let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
        let mut disallowed: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
        for unit in units {
            let unit = unit.borrow();
            total += 1;
            *surfaces.entry(unit.surface.clone()).or_default() += 1;
            if let Some(n) = &unit.numeric {
                *numeric_counts
                    .entry((n.reading.clone(), n.surface.clone()))
                    .or_default() += 1;
                continue;
            }
            if let Some(t) = &unit.unknown_conjugation {
                *unknown.entry(t.clone()).or_default() += 1;
                continue;
            }
            let (Some(stem_reading), Some(stem_surface), Some(conjugation)) =
                (&unit.stem_reading, &unit.stem_surface, &unit.conjugation)
            else {
                *counts
                    .entry((unit.reading.clone(), unit.surface.clone(), None))
                    .or_default() += 1;
                continue;
            };
            let stem = (
                stem_reading.clone(),
                stem_surface.clone(),
                Some(conjugation.clone()),
            );
            *counts.entry(stem).or_default() += 1;
            *counts
                .entry((unit.reading.clone(), unit.surface.clone(), None))
                .or_default() += 1;
            let okuri = unit
                .surface
                .strip_prefix(stem_surface.as_str())
                .unwrap_or_default();
            // A unit may stop partway through an allowed reading, before a
            // word it does not take in (食べなけれ before ば).
            if !may_follow_stem(conjugation, okuri) {
                disallowed
                    .entry((conjugation.clone(), okuri.to_owned()))
                    .or_insert_with(|| (0, unit.surface.clone()))
                    .0 += 1;
            }
        }

        let mut costs: HashMap<Key, u32> = HashMap::new();
        for (key, count) in counts {
            if count >= MIN_COUNT {
                let cost = costs.entry(key).or_insert(u32::MAX);
                *cost = (*cost).min(cost_of(count, total));
            }
        }
        for word in unidic {
            let cost = surfaces
                .get(word.surface.as_str())
                .map_or(UNIDIC_UNSEEN, |&n| cost_of(n, total) + UNIDIC_EXTRA);
            let key = (word.reading.clone(), word.surface.clone(), None);
            let kept = costs.entry(key).or_insert(u32::MAX);
            *kept = (*kept).min(cost);
        }
        drop(surfaces);
        let entries: Vec<Entry> = costs
            .into_iter()
            .filter(|((_, surface, _), _)| !is_glossed(surface))
            .map(|((reading, surface, conjugation), cost)| Entry {
                reading,
                surface,
                conjugation,
                cost,
            })
            .collect();

        Self {
            okuri: okuri_lines(&entries),
            numeric: numeric_entries(numeric_counts, total),
            entries,
            report: Report {
                unknown_conjugations: unknown.into_iter().collect(),
                disallowed_okurigana: disallowed
                    .into_iter()
                    .map(
                        |((conjugation, okurigana), (count, example))| DisallowedOkurigana {
                            conjugation,
                            okurigana,
                            count,
                            example,
                        },
                    )
                    .collect(),
            },
        }
    }

    /// Adds `words` that no unit made, keeping the smaller cost of a word
    /// already there, with their okurigana lines.
    pub fn add_words(&mut self, words: impl IntoIterator<Item = Entry>) {
        let mut at: HashMap<Key, usize> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                (
                    (e.reading.clone(), e.surface.clone(), e.conjugation.clone()),
                    i,
                )
            })
            .collect();
        for word in words {
            let key = (
                word.reading.clone(),
                word.surface.clone(),
                word.conjugation.clone(),
            );
            match at.get(&key) {
                Some(&i) => self.entries[i].cost = self.entries[i].cost.min(word.cost),
                None => {
                    at.insert(key, self.entries.len());
                    self.entries.push(word);
                }
            }
        }
        self.okuri = okuri_lines(&self.entries);
    }

    /// Removes the words whose surface `drops` tells, with their okurigana
    /// lines.
    pub fn drop_words(&mut self, drops: impl Fn(&str) -> bool) {
        self.entries.retain(|e| !drops(&e.surface));
        self.okuri.retain(|e| !drops(&e.surface));
    }

    /// The dictionary as Kanaemi's text dictionary, described as the official
    /// dictionary of `label`, every line with its cost: by reading, and within
    /// a reading from the largest cost to the smallest, so a reader that
    /// ignores the costs still ranks the cheapest first.
    pub fn to_text(&self, label: impl AsRef<str>) -> String {
        self.to_text_where(label, |_, _| true)
    }

    /// [`Dictionary::to_text`] with only the lines `keep` keeps.
    pub fn to_text_where(
        &self,
        label: impl AsRef<str>,
        keep: impl Fn(&ItemLine, LineKind) -> bool,
    ) -> String {
        let numeric: Vec<Entry> = self
            .numeric
            .iter()
            .filter_map(|e| {
                Some(Entry {
                    reading: mark_placeholders(&e.reading)?,
                    surface: mark_placeholders(&e.surface)?,
                    ..e.clone()
                })
            })
            .collect();
        let mut lines: Vec<(&Entry, LineKind)> = self
            .entries
            .iter()
            .map(|e| (e, LineKind::Word))
            .chain(numeric.iter().map(|e| (e, LineKind::Numeric)))
            .chain(self.okuri.iter().map(|e| (e, LineKind::Okurigana)))
            .collect();
        lines.sort_by(|(a, _), (b, _)| {
            a.reading
                .as_bytes()
                .cmp(b.reading.as_bytes())
                .then(b.cost.cmp(&a.cost))
                .then(a.surface.as_bytes().cmp(b.surface.as_bytes()))
                .then_with(|| compare_conjugations(&a.conjugation, &b.conjugation))
        });
        let mut out = format!("# Kanaemi 公式辞書・{}（kanaemi-dict）\n", label.as_ref());
        for (entry, kind) in lines {
            let (reading, okurigana) = match entry.reading.rsplit_once('*') {
                Some((stem, kana)) if kind == LineKind::Okurigana => (stem, Some(kana)),
                _ => (entry.reading.as_str(), None),
            };
            let line = ItemLine {
                reading,
                okurigana,
                surface: &entry.surface,
                conjugation: entry.conjugation.as_deref(),
                cost: Some(entry.cost),
            };
            if !keep(&line, kind) {
                continue;
            }
            out.push_str(&line.to_string());
            out.push('\n');
        }
        out
    }

    /// [`Dictionary::to_text`], unless Kanaemi would not read some line of it.
    pub fn to_checked_text(&self, label: impl AsRef<str>) -> Result<String, RejectedLines> {
        let text = self.to_text(label);
        let (_, invalid) = TextDictionary::parse(&text);
        if invalid.is_empty() {
            Ok(text)
        } else {
            Err(RejectedLines(invalid))
        }
    }
}

impl Report {
    /// The report as TSV with a header, the most frequent first within each
    /// kind.
    pub fn to_tsv(&self) -> String {
        let mut unknown: Vec<_> = self.unknown_conjugations.iter().collect();
        unknown.sort_by_key(|(_, n)| Reverse(*n));
        let mut disallowed: Vec<_> = self.disallowed_okurigana.iter().collect();
        disallowed.sort_by_key(|d| Reverse(d.count));
        let mut tsv = String::from("kind\tconjugation\tokurigana\tcount\texample\n");
        for (conjugation, n) in unknown {
            tsv.push_str(&format!("unknown-conjugation\t{conjugation}\t\t{n}\t\n"));
        }
        for d in disallowed {
            tsv.push_str(&format!(
                "disallowed-okurigana\t{}\t{}\t{}\t{}\n",
                d.conjugation, d.okurigana, d.count, d.example
            ));
        }
        tsv
    }
}

/// A surface with its reading added in brackets (明日（あした）), which is not
/// a word.
fn is_glossed(surface: &str) -> bool {
    surface.contains(['（', '）', '(', ')'])
}

/// The okurigana lines of `entries`, each at the cheapest cost of the
/// entries it follows from.
pub(crate) fn okuri_lines(entries: &[Entry]) -> Vec<Entry> {
    let mut okuri: BTreeMap<(String, String), u32> = BTreeMap::new();
    for entry in entries {
        if let Some(key) = okuri_line(&entry.reading, &entry.surface) {
            okuri
                .entry(key)
                .and_modify(|c| *c = (*c).min(entry.cost))
                .or_insert(entry.cost);
        }
    }
    okuri
        .into_iter()
        .map(|((reading, surface), cost)| Entry {
            reading,
            surface,
            cost,
            conjugation: None,
        })
        .collect()
}

/// Numeric entries from their counts, each with the readings whose first kana
/// after the number is another of its plain, voiced and semi-voiced forms
/// (`{}ほん` and `{}ぼん` from `{}ぽん`), as the reading changes with the
/// number before it. A reading met more than once keeps its smallest cost.
fn numeric_entries(counts: HashMap<(String, String), usize>, total: usize) -> Vec<Entry> {
    let mut costs: BTreeMap<(String, String), u32> = BTreeMap::new();
    for ((reading, surface), count) in counts {
        if count < MIN_COUNT {
            continue;
        }
        let cost = cost_of(count, total);
        for reading in voicings(&reading) {
            costs
                .entry((reading, surface.clone()))
                .and_modify(|c| *c = (*c).min(cost))
                .or_insert(cost);
        }
    }
    costs
        .into_iter()
        .map(|((reading, surface), cost)| Entry {
            reading,
            surface,
            conjugation: None,
            cost,
        })
        .collect()
}

/// `reading` with the first kana after its first `{}` in each of its plain,
/// voiced and semi-voiced forms; `reading` alone when that kana has no other
/// form.
fn voicings(reading: &str) -> Vec<String> {
    let family = reading.split_once("{}").and_then(|(before, after)| {
        let mut chars = after.chars();
        let first = chars.next()?;
        let i = [PLAIN, VOICED, SEMI_VOICED]
            .iter()
            .enumerate()
            .find_map(|(row, kana)| {
                let at = kana.chars().position(|k| k == first)?;
                Some(if row == 2 { at + HA_ROW } else { at })
            })?;
        let semi_voiced = i
            .checked_sub(HA_ROW)
            .and_then(|j| SEMI_VOICED.chars().nth(j));
        let rest = chars.as_str();
        Some(
            PLAIN
                .chars()
                .nth(i)
                .into_iter()
                .chain(VOICED.chars().nth(i))
                .chain(semi_voiced)
                .map(|kana| format!("{before}{{}}{kana}{rest}"))
                .collect(),
        )
    });
    family.unwrap_or_else(|| vec![reading.to_owned()])
}

/// `-ln(count / total) × 100`, rounded.
pub(crate) fn cost_of(count: usize, total: usize) -> u32 {
    (-(count as f64 / total as f64).ln() * 100.0).round() as u32
}

/// The okurigana line of a word whose surface ends in kanji and hiragana:
/// `(か*く, 書く)` for 書く.
pub(crate) fn okuri_line(reading: &str, surface: &str) -> Option<(String, String)> {
    let okuri_len = surface
        .chars()
        .rev()
        .take_while(|c| is_hiragana(*c))
        .count();
    let kanji_part: String = surface
        .chars()
        .take(surface.chars().count() - okuri_len)
        .collect();
    if okuri_len == 0 || !kanji_part.chars().last().is_some_and(is_kanji) {
        return None;
    }
    let okuri: String = surface.chars().skip(kanji_part.chars().count()).collect();
    let before = reading.strip_suffix(okuri.as_str())?;
    let first = okuri.chars().next()?;
    Some((format!("{before}*{first}"), format!("{kanji_part}{first}")))
}

/// No conjugation type first, then types by their UTF-8 bytes.
fn compare_conjugations(a: &Option<String>, b: &Option<String>) -> std::cmp::Ordering {
    a.as_ref()
        .map(|s| s.as_bytes())
        .cmp(&b.as_ref().map(|s| s.as_bytes()))
}

#[cfg(test)]
mod tests {
    use kanaemi_engine::InvalidReason;

    use super::*;
    use crate::Numeric;

    fn word(reading: &str, surface: &str) -> Unit {
        Unit {
            doc_id: "d".into(),
            position: 0,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        }
    }

    fn conj(reading: &str, surface: &str, stem: (&str, &str), conjugation: &str) -> Unit {
        Unit {
            stem_reading: Some(stem.0.into()),
            stem_surface: Some(stem.1.into()),
            conjugation: Some(conjugation.into()),
            ..word(reading, surface)
        }
    }

    fn unidic(reading: &str, surface: &str) -> UnidicWord {
        UnidicWord {
            reading: reading.into(),
            surface: surface.into(),
        }
    }

    fn entry(reading: &str, surface: &str, conjugation: Option<&str>, cost: u32) -> Entry {
        Entry {
            reading: reading.into(),
            surface: surface.into(),
            conjugation: conjugation.map(str::to_owned),
            cost,
        }
    }

    fn repeat(unit: Unit, n: usize) -> Vec<Unit> {
        vec![unit; n]
    }

    fn cost(count: usize, total: usize) -> u32 {
        (-(count as f64 / total as f64).ln() * 100.0).round() as u32
    }

    fn find<'a>(entries: &'a [Entry], reading: &str, surface: &str) -> Option<&'a Entry> {
        entries
            .iter()
            .find(|e| e.reading == reading && e.surface == surface)
    }

    fn counted(reading: &str, surface: &str, item: (&str, &str, &str)) -> Unit {
        Unit {
            numeric: Some(Numeric {
                reading: item.0.into(),
                surface: item.1.into(),
                value: item.2.into(),
            }),
            ..word(reading, surface)
        }
    }

    #[test]
    fn added_words_join_the_entries_with_their_okurigana_lines_and_the_smaller_cost() {
        let mut dict = Dictionary::build(repeat(word("ひとつ", "一つ"), 2), &[]);
        let kept = find(&dict.entries, "ひとつ", "一つ").unwrap().cost;

        dict.add_words([
            entry("ひとつ", "一つ", None, kept + 100),
            entry("いっぴき", "一匹", None, 900),
        ]);

        assert_eq!(find(&dict.entries, "ひとつ", "一つ").unwrap().cost, kept);
        assert_eq!(find(&dict.entries, "いっぴき", "一匹").unwrap().cost, 900);
        assert_eq!(dict.entries.len(), 2);
        assert!(
            find(&dict.okuri, "ひと*つ", "一つ").is_some(),
            "{:?}",
            dict.okuri
        );
    }

    #[test]
    fn dropped_words_leave_with_their_okurigana_lines() {
        let mut dict = Dictionary::build(
            [
                repeat(word("とうつ", "十つ"), 2),
                repeat(word("ひとつ", "一つ"), 2),
            ]
            .concat(),
            &[],
        );

        dict.drop_words(|surface| surface == "十つ");

        assert!(find(&dict.entries, "とうつ", "十つ").is_none());
        assert!(find(&dict.okuri, "とう*つ", "十つ").is_none());
        assert!(find(&dict.entries, "ひとつ", "一つ").is_some());
        assert!(find(&dict.okuri, "ひと*つ", "一つ").is_some());
    }

    #[test]
    fn numeric_units_are_counted_across_their_values_into_numeric_entries() {
        let units = [
            counted("3まい", "3枚", ("{}まい", "{}枚", "3")),
            counted("5まい", "5枚", ("{}まい", "{}枚", "5")),
            counted("3まい", "三枚", ("{}まい", "{kanji}枚", "3")),
            word("てがみ", "手紙"),
        ];

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(dict.numeric, [entry("{}まい", "{}枚", None, cost(2, 4))]);
        assert!(dict.entries.is_empty(), "{:?}", dict.entries);
    }

    #[test]
    fn a_numeric_entry_takes_every_voicing_of_the_first_kana_after_the_number() {
        let units: Vec<Unit> = [
            repeat(counted("3ぽん", "3本", ("{}ぽん", "{}本", "3")), 2),
            repeat(
                counted("だい3かい", "第3回", ("だい{}かい", "第{}回", "3")),
                2,
            ),
            repeat(counted("3がつ", "3月", ("{}がつ", "{}月", "3")), 2),
            repeat(counted("3にん", "3人", ("{}にん", "{}人", "3")), 2),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let mut items: Vec<(&str, &str, u32)> = dict
            .numeric
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str(), e.cost))
            .collect();
        items.sort();
        let c = cost(2, 8);
        assert_eq!(
            items,
            [
                ("{}かつ", "{}月", c),
                ("{}がつ", "{}月", c),
                ("{}にん", "{}人", c),
                ("{}ほん", "{}本", c),
                ("{}ぼん", "{}本", c),
                ("{}ぽん", "{}本", c),
                ("だい{}かい", "第{}回", c),
                ("だい{}がい", "第{}回", c),
            ]
        );
    }

    #[test]
    fn a_variant_meeting_a_counted_entry_keeps_the_smaller_cost() {
        let units: Vec<Unit> = [
            repeat(counted("3ほん", "3本", ("{}ほん", "{}本", "3")), 5),
            repeat(counted("3ぼん", "3本", ("{}ぼん", "{}本", "3")), 2),
            repeat(counted("3こ", "3個", ("{}こ", "{}個", "3")), 2),
            repeat(counted("5ご", "5個", ("{}ご", "{}個", "5")), 5),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let cost_of = |reading, surface| find(&dict.numeric, reading, surface).unwrap().cost;
        assert_eq!(cost_of("{}ぼん", "{}本"), cost(5, 14));
        assert_eq!(cost_of("{}こ", "{}個"), cost(5, 14));
        assert_eq!(cost_of("{}ご", "{}個"), cost(5, 14));
    }

    #[test]
    fn numeric_entries_make_no_okurigana_lines() {
        let units = repeat(
            counted("3にちぶり", "3日ぶり", ("{}にちぶり", "{}日ぶり", "3")),
            2,
        );

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(dict.numeric.len(), 1);
        assert_eq!(dict.okuri, []);
    }

    #[test]
    fn numeric_entries_are_written_with_their_placeholders_among_the_other_lines() {
        let dict = Dictionary {
            entries: vec![entry("ほん", "本", None, 50)],
            numeric: vec![
                entry("{}ほん", "{}本", None, 300),
                entry("だい{}かい", "第{kanji}回", None, 200),
                entry("{}ほん", "{wide-num}本", None, 100),
            ],
            ..Default::default()
        };

        let text = dict.to_checked_text(BASE_LABEL).unwrap();

        assert_eq!(
            text,
            "# Kanaemi 公式辞書・基本（kanaemi-dict）\n\
             だい{}かい\t第{kanji}回\t\t200\n\
             ほん\t本\t\t50\n\
             {}ほん\t{}本\t\t300\n\
             {}ほん\t{wide-num}本\t\t100\n"
        );
    }

    #[test]
    fn words_ending_in_kanji_and_hiragana_get_their_cheapest_okurigana_line() {
        let entries = [
            entry("だんちょうのおもい", "断腸の思い", None, 900),
            entry("だんちょうのおもい", "断腸の思い", None, 700),
            entry("こっかじょうほうきょく", "国家情報局", None, 800),
        ];

        assert_eq!(
            okuri_lines(&entries),
            [entry("だんちょうのおも*い", "断腸の思い", None, 700)]
        );
    }

    #[test]
    fn a_surface_ending_in_the_long_vowel_mark_has_no_okurigana_line() {
        assert_eq!(okuri_line("けーき", "ケーキ"), None);
        assert_eq!(okuri_line("かー", "化ー"), None);
    }

    #[test]
    fn stems_and_attested_forms_seen_twice_become_entries() {
        let units: Vec<Unit> = [
            repeat(conj("かいた", "書いた", ("か", "書"), "五段-カ行"), 2),
            repeat(conj("かく", "書く", ("か", "書"), "五段-カ行"), 1),
            repeat(word("てがみ", "手紙"), 2),
            repeat(word("きしゃ", "記者"), 1),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let stem = find(&dict.entries, "か", "書").unwrap();
        assert_eq!(stem.conjugation.as_deref(), Some("五段-カ行"));
        assert_eq!(stem.cost, cost(3, 6));
        let kaita = find(&dict.entries, "かいた", "書いた").unwrap();
        assert_eq!(
            (kaita.cost, kaita.conjugation.as_deref()),
            (cost(2, 6), None)
        );
        assert_eq!(
            find(&dict.entries, "てがみ", "手紙").unwrap().cost,
            cost(2, 6)
        );
        assert!(find(&dict.entries, "かく", "書く").is_none());
        assert!(find(&dict.entries, "きしゃ", "記者").is_none());
    }

    #[test]
    fn a_word_seen_once_plain_and_once_as_a_whole_form_counts_twice() {
        let units = [
            word("たか", "高"),
            conj("たか", "高", ("たか", "高"), "形容詞"),
        ];

        let dict = Dictionary::build(&units, &[]);

        let plain = dict
            .entries
            .iter()
            .find(|e| e.reading == "たか" && e.surface == "高" && e.conjugation.is_none())
            .unwrap();
        assert_eq!(plain.cost, cost(2, 2));
    }

    #[test]
    fn unidic_words_cost_more_than_the_units_and_most_when_unseen() {
        let units: Vec<Unit> = [
            repeat(word("わたくし", "私"), 3),
            repeat(word("てがみ", "手紙"), 1),
        ]
        .concat();
        let unidic = [
            unidic("わたし", "私"),
            unidic("わたくし", "私"),
            unidic("じょうけい", "情景"),
        ];

        let dict = Dictionary::build(&units, &unidic);

        assert_eq!(
            find(&dict.entries, "わたし", "私").unwrap().cost,
            cost(3, 4) + 1000
        );
        assert_eq!(
            find(&dict.entries, "わたくし", "私").unwrap().cost,
            cost(3, 4)
        );
        assert_eq!(
            find(&dict.entries, "じょうけい", "情景").unwrap().cost,
            3000
        );
    }

    #[test]
    fn glossed_surfaces_and_unknown_types_stay_out_and_unknown_types_are_reported() {
        let units: Vec<Unit> = [
            repeat(word("あした", "明日（あした）"), 2),
            repeat(word("あした", "明日(あした)"), 2),
            repeat(
                Unit {
                    unknown_conjugation: Some("文語四段-ハ行".into()),
                    ..word("そうろう", "候")
                },
                2,
            ),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        assert!(dict.entries.is_empty());
        assert_eq!(
            dict.report.unknown_conjugations,
            [("文語四段-ハ行".to_owned(), 2)]
        );
    }

    #[test]
    fn okurigana_the_table_rejects_are_reported_with_an_example() {
        let units = repeat(conj("かくっぺ", "書くっぺ", ("か", "書"), "五段-カ行"), 3);

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            dict.report.disallowed_okurigana,
            [DisallowedOkurigana {
                conjugation: "五段-カ行".into(),
                okurigana: "くっぺ".into(),
                count: 3,
                example: "書くっぺ".into(),
            }]
        );
    }

    #[test]
    fn okurigana_that_stop_partway_through_an_allowed_reading_are_not_reported() {
        let units = [
            conj("たべなけれ", "食べなけれ", ("たべ", "食べ"), "下一段-バ行"),
            conj("かけ", "書け", ("か", "書"), "五段-カ行"),
        ];

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(dict.report.disallowed_okurigana, []);
    }

    #[test]
    fn words_ending_in_kanji_and_hiragana_get_okurigana_lines() {
        let units: Vec<Unit> = [
            repeat(conj("かいた", "書いた", ("か", "書"), "五段-カ行"), 2),
            repeat(word("みなさん", "皆さん"), 2),
            repeat(conj("たべる", "食べる", ("たべ", "食べ"), "下一段-バ行"), 2),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let lines: Vec<(&str, &str)> = dict
            .okuri
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str()))
            .collect();
        assert!(lines.contains(&("か*い", "書い")), "{lines:?}");
        assert!(lines.contains(&("みな*さ", "皆さ")), "{lines:?}");
        assert!(lines.contains(&("た*べ", "食べ")), "{lines:?}");
        assert_eq!(find(&dict.okuri, "か*い", "書い").unwrap().cost, cost(2, 6));
    }

    #[test]
    fn lines_of_a_reading_go_from_the_largest_cost_to_the_smallest() {
        let dict = Dictionary {
            entries: vec![
                entry("か", "蚊", None, 500),
                entry("か", "書", Some("五段-カ行"), 300),
                entry("か", "下", None, 900),
                entry("か", "化", None, 300),
                entry("あ", "亜", None, 1),
            ],
            okuri: vec![entry("か*く", "書く", None, 10)],
            ..Default::default()
        };

        assert_eq!(
            dict.to_text("説明"),
            "# Kanaemi 公式辞書・説明（kanaemi-dict）\n\
             あ\t亜\t\t1\n\
             か\t下\t\t900\n\
             か\t蚊\t\t500\n\
             か\t化\t\t300\n\
             か\t書\t五段-カ行\t300\n\
             か*く\t書く\t\t10\n"
        );
    }

    #[test]
    fn special_characters_are_escaped() {
        let dict = Dictionary {
            entries: vec![
                entry("#あ", "a\\b\tc", None, 1),
                entry("!い", "x", None, 1),
                entry("う*", "y", None, 1),
            ],
            ..Default::default()
        };

        assert_eq!(
            dict.to_text(BASE_LABEL),
            "# Kanaemi 公式辞書・基本（kanaemi-dict）\n\
             \\!い\tx\t\t1\n\
             \\#あ\ta\\\\b\\tc\t\t1\n\
             う\\*\ty\t\t1\n"
        );
    }

    #[test]
    fn kanaemi_reads_every_line_written_with_its_cost() {
        let units: Vec<Unit> = [
            repeat(conj("かいた", "書いた", ("か", "書"), "五段-カ行"), 2),
            repeat(word("みなさん", "皆さん"), 2),
            repeat(conj("たべる", "食べる", ("たべ", "食べ"), "下一段-バ行"), 2),
        ]
        .concat();
        let dict = Dictionary::build(&units, &[unidic("わたし", "私")]);

        let text = dict.to_checked_text(BASE_LABEL).unwrap();

        let minasan = find(&dict.entries, "みなさん", "皆さん").unwrap();
        assert!(
            text.lines()
                .any(|l| l == format!("みなさん\t皆さん\t\t{}", minasan.cost)),
            "{text}"
        );
    }

    #[test]
    fn a_dictionary_with_a_line_kanaemi_rejects_gives_no_text() {
        let dict = Dictionary {
            entries: vec![
                entry("か", "書", Some("五段-カ行"), 1),
                entry("か", "書", Some("存在しない活用"), 1),
            ],
            ..Default::default()
        };

        let RejectedLines(invalid) = dict.to_checked_text(BASE_LABEL).unwrap_err();

        assert_eq!(
            invalid,
            [InvalidLine {
                line: 3,
                reason: InvalidReason::ConjugationType
            }]
        );
    }

    #[test]
    fn the_report_lists_the_most_frequent_first_within_each_kind() {
        let report = Report {
            unknown_conjugations: vec![("A".into(), 1), ("B".into(), 5)],
            disallowed_okurigana: vec![
                DisallowedOkurigana {
                    conjugation: "五段-カ行".into(),
                    okurigana: "くっぺ".into(),
                    count: 2,
                    example: "書くっぺ".into(),
                },
                DisallowedOkurigana {
                    conjugation: "五段-カ行".into(),
                    okurigana: "かね".into(),
                    count: 3,
                    example: "書かね".into(),
                },
            ],
        };

        assert_eq!(
            report.to_tsv(),
            "kind\tconjugation\tokurigana\tcount\texample\n\
             unknown-conjugation\tB\t\t5\t\n\
             unknown-conjugation\tA\t\t1\t\n\
             disallowed-okurigana\t五段-カ行\tかね\t3\t書かね\n\
             disallowed-okurigana\t五段-カ行\tくっぺ\t2\t書くっぺ\n"
        );
    }
}

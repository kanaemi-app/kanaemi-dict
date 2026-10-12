//! Dictionary entries from conversion units and UniDic words, written as
//! Kanaemi's text dictionary.

use std::borrow::Borrow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use kanaemi_engine::{InvalidLine, ItemLine, TextDictionary, mark_placeholders, may_follow_stem};

use crate::kana::{HA_ROW, PLAIN, SEMI_VOICED, VOICED, is_hiragana, is_kanji, unvoiced};
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
    /// The cost of each surface of the numeric units as written (一発, 3本),
    /// from the units met at least [`MIN_COUNT`] times.
    pub numeric_written: HashMap<String, u32>,
    /// The cost from which a word was met at most [`MIN_COUNT`] times in the
    /// units the dictionary counts, or comes from UniDic alone; `None` for a
    /// dictionary not counted from units.
    pub rare_cost: Option<u32>,
    /// The cost of the line of each word written whole that conjugated forms
    /// also count (書く under かく), in place of its entry's. The
    /// documents type a conjugated form with its okurigana marked (か*く), so
    /// the line costs as the units written whole alone, or, when those are
    /// too few to enter, behind every word met at least [`MIN_COUNT`] times.
    /// The entry keeps the cost of both, which its okurigana line takes.
    pub whole_forms: HashMap<(String, String), u32>,
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
pub(crate) const UNIDIC_UNSEEN: u32 = 3000;

/// A surface of words written whole with at least this share (numerator,
/// denominator) of its places, in any reading, inside compound units is a
/// fragment of them (一髪 of 危機一髪), and its words count only their places
/// outside them.
const FRAGMENT_SHARE: (usize, usize) = (3, 4);

type Key = (String, String, Option<String>);

/// How many places of each word written whole lie inside a compound unit
/// (一髪 in 危機一髪). Units come document by document, as
/// `build/units.jsonl` holds them; a compound comes after the unit that
/// starts where it does, so a document is looked at once it ends.
#[derive(Debug, Default)]
struct CompoundParts {
    inside: HashMap<Key, usize>,
    /// Places of each surface of words written whole, in any reading.
    places: HashMap<String, usize>,
    doc_id: String,
    /// Places of words written whole: start, end, and the word.
    words: Vec<(usize, usize, Key)>,
    /// Places of compounds: start and end.
    compounds: Vec<(usize, usize)>,
}

impl CompoundParts {
    fn observe(&mut self, unit: &Unit) {
        if unit.doc_id != self.doc_id {
            self.flush();
            self.doc_id.clone_from(&unit.doc_id);
        }
        if unit.numeric.is_some()
            || unit.stem_surface.is_some()
            || unit.unknown_conjugation.is_some()
        {
            return;
        }
        let span = (unit.position, unit.position + unit.surface.chars().count());
        if unit.compound {
            self.compounds.push(span);
        } else {
            *self.places.entry(unit.surface.clone()).or_default() += 1;
            self.words.push((
                span.0,
                span.1,
                (unit.reading.clone(), unit.surface.clone(), None),
            ));
        }
    }

    /// The places inside compounds of each word, and the surfaces that are
    /// fragments. The analyzer may read a surface one way alone and another
    /// way inside compounds (日本 as にっぽん and にほん), so whether a surface
    /// is a fragment is judged across its readings.
    fn finish(mut self) -> (HashMap<Key, usize>, HashSet<String>) {
        self.flush();
        let mut inside_of: HashMap<&str, usize> = HashMap::new();
        for ((_, surface, _), &n) in &self.inside {
            *inside_of.entry(surface.as_str()).or_default() += n;
        }
        let fragments = inside_of
            .into_iter()
            .filter(|&(surface, inside)| {
                inside * FRAGMENT_SHARE.1 >= self.places[surface] * FRAGMENT_SHARE.0
            })
            .map(|(surface, _)| surface.to_owned())
            .collect();
        (self.inside, fragments)
    }

    fn flush(&mut self) {
        let compounds = Spans::new(std::mem::take(&mut self.compounds));
        for (start, end, key) in std::mem::take(&mut self.words) {
            if compounds.covers(start, end) {
                *self.inside.entry(key).or_default() += 1;
            }
        }
    }
}

/// Spans of a document, start and end, that may overlap: a joined word
/// (朽ち果てた) and a compound (朽ち果て) can start at one place.
#[derive(Debug, Clone, Default)]
pub struct Spans {
    starts: Vec<usize>,
    /// The farthest end among the spans up to each, in start order.
    reach: Vec<usize>,
}

impl Spans {
    pub fn new(spans: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let mut spans: Vec<(usize, usize)> = spans.into_iter().collect();
        spans.sort_unstable();
        let mut reach = Vec::with_capacity(spans.len());
        let mut farthest = 0;
        for &(_, end) in &spans {
            farthest = farthest.max(end);
            reach.push(farthest);
        }
        Self {
            starts: spans.into_iter().map(|(start, _)| start).collect(),
            reach,
        }
    }

    /// Whether `start..end` lies inside one of the spans.
    pub fn covers(&self, start: usize, end: usize) -> bool {
        let before = self.starts.partition_point(|&s| s <= start);
        before > 0 && end <= self.reach[before - 1]
    }
}

/// The surfaces that are fragments of compounds among `units` (see
/// [`FRAGMENT_SHARE`]), as the dictionary built from them counts them.
pub fn fragment_surfaces<U: Borrow<Unit>>(units: impl IntoIterator<Item = U>) -> HashSet<String> {
    let mut parts = CompoundParts::default();
    for unit in units {
        let unit = unit.borrow();
        if !unit.okurigana_variant {
            parts.observe(unit);
        }
    }
    parts.finish().1
}

impl Dictionary {
    /// Builds the dictionary from every unit given and the UniDic words.
    pub fn build<U: Borrow<Unit>>(
        units: impl IntoIterator<Item = U>,
        unidic: &[UnidicWord],
    ) -> Self {
        Self::build_with_lexicon(units, unidic, &[])
    }

    /// [`Dictionary::build`] with the words of the whole SudachiDict
    /// lexicon, which enter as UniDic words do but only where units show
    /// their surface as often as a word must occur to enter: the lexicon
    /// reads a surface the text has in ways the analyzer never chooses
    /// (大文字 as おおもじ), and the rest of it would bring in words no text
    /// uses.
    pub fn build_with_lexicon<U: Borrow<Unit>>(
        units: impl IntoIterator<Item = U>,
        unidic: &[UnidicWord],
        lexicon: &[UnidicWord],
    ) -> Self {
        let mut total = 0usize;
        // A whole form and a plain word with the same reading and surface are
        // one item, so they share one count.
        let mut counts: HashMap<Key, usize> = HashMap::new();
        let mut form_counts: HashMap<(String, String), usize> = HashMap::new();
        let mut numeric_counts: HashMap<(String, String), usize> = HashMap::new();
        let mut numeric_written: HashMap<String, usize> = HashMap::new();
        let mut surfaces: HashMap<String, usize> = HashMap::new();
        let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
        let mut disallowed: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
        let mut parts = CompoundParts::default();
        for unit in units {
            let unit = unit.borrow();
            total += 1;
            if unit.okurigana_variant {
                continue;
            }
            parts.observe(unit);
            *surfaces.entry(unit.surface.clone()).or_default() += 1;
            if let Some(n) = &unit.numeric {
                if !is_glossed(&n.surface) {
                    *numeric_counts
                        .entry((n.reading.clone(), n.surface.clone()))
                        .or_default() += 1;
                    *numeric_written.entry(unit.surface.clone()).or_default() += 1;
                }
                continue;
            }
            if let Some(t) = &unit.unknown_conjugation {
                *unknown.entry(t.clone()).or_default() += 1;
                // Kanaemi cannot conjugate the type, so the form is typed
                // whole, as a word outside the table is (如く, 若き). One in
                // kanji alone is mostly misread (深 as こし of 深掘り).
                if okuri_line(&unit.reading, &unit.surface).is_some() {
                    *counts
                        .entry((unit.reading.clone(), unit.surface.clone(), None))
                        .or_default() += 1;
                }
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
            // A form cut short at a small つ (ハマっ) is no word; its stem is.
            if !unit.reading.ends_with('っ') {
                *counts
                    .entry((unit.reading.clone(), unit.surface.clone(), None))
                    .or_default() += 1;
                *form_counts
                    .entry((unit.reading.clone(), unit.surface.clone()))
                    .or_default() += 1;
            }
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

        // A fragment's surface counts only where it stands outside the
        // compounds, for the lexicon's words of it as for the units.
        let mut outside = surfaces.clone();
        {
            let (inside, fragments) = parts.finish();
            for (key, n) in inside {
                if !fragments.contains(&key.1) {
                    continue;
                }
                if let Some(count) = counts.get_mut(&key) {
                    *count -= n;
                }
                if let Some(count) = outside.get_mut(&key.1) {
                    *count -= n;
                }
            }
        }

        let rare_cost = (total > 0).then(|| cost_of(MIN_COUNT, total));
        // UniDic and the lexicon read a conjugated form as a word too (無い as
        // ない), so their costs, which can come ahead of rare counted words,
        // are not taken for these lines.
        let whole_forms: HashMap<(String, String), u32> = form_counts
            .into_iter()
            .map(|((reading, surface), forms)| {
                let count = counts[&(reading.clone(), surface.clone(), None)];
                let whole = count.saturating_sub(forms);
                let cost = if whole >= MIN_COUNT {
                    cost_of(whole, total)
                } else {
                    cost_of(count, total) + rare_cost.unwrap_or_default()
                };
                ((reading, surface), cost)
            })
            .collect();
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
        for word in lexicon {
            let Some(&n) = outside
                .get(word.surface.as_str())
                .filter(|&&n| n >= MIN_COUNT)
            else {
                continue;
            };
            let key = (word.reading.clone(), word.surface.clone(), None);
            let kept = costs.entry(key).or_insert(u32::MAX);
            *kept = (*kept).min(cost_of(n, total) + UNIDIC_EXTRA);
        }
        drop(surfaces);
        drop(outside);
        let entries: Vec<Entry> = costs
            .into_iter()
            .filter(|((reading, surface, conjugation), _)| {
                !is_glossed(surface) && (conjugation.is_some() || !is_cut_short(reading, surface))
            })
            .map(|((reading, surface, conjugation), cost)| Entry {
                reading,
                surface,
                conjugation,
                cost,
            })
            .collect();

        Self {
            okuri: okuri_lines(&entries, rare_cost, typed_marked(&whole_forms)),
            numeric: numeric_entries(numeric_counts, total),
            numeric_written: numeric_written
                .into_iter()
                .filter(|&(_, count)| count >= MIN_COUNT)
                .map(|(surface, count)| (surface, cost_of(count, total)))
                .collect(),
            entries,
            rare_cost,
            whole_forms,
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
    /// Adds `entries` to the numeric items; a line already there keeps the
    /// smaller cost.
    pub fn add_numeric(&mut self, entries: impl IntoIterator<Item = Entry>) {
        let mut at: HashMap<(String, String), usize> = self
            .numeric
            .iter()
            .enumerate()
            .map(|(i, e)| ((e.reading.clone(), e.surface.clone()), i))
            .collect();
        for entry in entries {
            let key = (entry.reading.clone(), entry.surface.clone());
            match at.get(&key) {
                Some(&i) => self.numeric[i].cost = self.numeric[i].cost.min(entry.cost),
                None => {
                    at.insert(key, self.numeric.len());
                    self.numeric.push(entry);
                }
            }
        }
    }

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
            if word.conjugation.is_none()
                && let Some(whole) = self
                    .whole_forms
                    .get_mut(&(word.reading.clone(), word.surface.clone()))
            {
                *whole = (*whole).min(word.cost);
            }
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
        self.okuri = okuri_lines(
            &self.entries,
            self.rare_cost,
            typed_marked(&self.whole_forms),
        );
    }

    /// Removes the words `drops` tells by their reading and surface, with
    /// their okurigana lines.
    pub fn drop_words(&mut self, drops: impl Fn(&str, &str) -> bool) {
        self.entries.retain(|e| !drops(&e.reading, &e.surface));
        self.okuri.retain(|e| !drops(&e.reading, &e.surface));
        self.whole_forms
            .retain(|(reading, surface), _| !drops(reading, surface));
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
        let words: Vec<Entry> = self
            .entries
            .iter()
            .map(|e| {
                let whole = e
                    .conjugation
                    .is_none()
                    .then(|| {
                        self.whole_forms
                            .get(&(e.reading.clone(), e.surface.clone()))
                    })
                    .flatten();
                Entry {
                    cost: whole.copied().unwrap_or(e.cost),
                    ..e.clone()
                }
            })
            .collect();
        let mut lines: Vec<(&Entry, LineKind)> = words
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

/// The dictionary of the words `units` spell with other okurigana than their
/// normalized form (小い, 行なう), which every other dictionary leaves out,
/// costing against those units alone.
pub fn okurigana_dictionary(units: impl IntoIterator<Item = Unit>) -> Dictionary {
    Dictionary::build(
        units
            .into_iter()
            .filter(|u| u.okurigana_variant)
            .map(|u| Unit {
                okurigana_variant: false,
                parts: Vec::new(),
                ..u
            }),
        &[],
    )
}

/// Whether `reading` and `surface` can be a word, whichever way it came in: a
/// piece cut from inside a word, read from a small kana, a moraic nasal or a
/// long vowel (ッて, ンな, っ放し), is not; nor is text broken by a space, a
/// loose voicing mark or a comma (美 少年, ヘ゛ヒ゛ー, 一、二塁間), or left
/// open by a dash or a wave dash at its end (インター-, カーポーカー〜).
pub fn is_word(reading: impl AsRef<str>, surface: impl AsRef<str>) -> bool {
    let surface = surface.as_ref();
    let starts_a_word = reading
        .as_ref()
        .chars()
        .next()
        .is_some_and(|c| !"ぁぃぅぇぉゃゅょゎゕゖっんー".contains(c));
    starts_a_word
        && !surface
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '゛' | '゜' | '、'))
        && !surface.ends_with(['-', '－', '〜', '～'])
}

/// A form with kanji cut short at a small つ (知らなかっ), where a unit
/// stopped taking in what follows; its word comes from its stem.
fn is_cut_short(reading: &str, surface: &str) -> bool {
    reading.ends_with('っ') && surface.chars().any(is_kanji)
}

/// The にほん readings of the words of `entries` that read their 日本 only as
/// にっぽん, at the cost of that reading. The analyzer reads 日本 inside a
/// longer word as にっぽん most of the time (日本時間 にっぽんじかん), though
/// many such words are typed with にほん; names that are にっぽん by right
/// (日本銀行) keep that reading too.
pub fn nihon_readings(entries: &[Entry]) -> Vec<Entry> {
    let nihon: HashSet<&str> = entries
        .iter()
        .filter(|e| e.conjugation.is_none() && e.reading.contains("にほん"))
        .map(|e| e.surface.as_str())
        .collect();
    entries
        .iter()
        .filter(|e| {
            e.conjugation.is_none()
                && !nihon.contains(e.surface.as_str())
                && e.reading.contains("にっぽん")
                && e.reading.matches("にっぽん").count() <= e.surface.matches("日本").count()
        })
        .map(|e| Entry {
            reading: e.reading.replace("にっぽん", "にほん"),
            ..e.clone()
        })
        .collect()
}

/// Whether an entry is typed with its okurigana marked: a stem, or a word
/// written whole that conjugated forms count (see
/// [`Dictionary::whole_forms`]).
fn typed_marked(whole_forms: &HashMap<(String, String), u32>) -> impl Fn(&Entry) -> bool + '_ {
    |e| e.conjugation.is_some() || whole_forms.contains_key(&(e.reading.clone(), e.surface.clone()))
}

/// The okurigana lines of `entries`, each at the cheapest cost of the
/// entries it follows from, but for a line of one kanji costing `rare_cost` or
/// more, which is mostly kana written for a kanji (祈とう, 気っぷ) rather than
/// okurigana. A line that follows only from words the documents type whole,
/// as `typed_marked` tells them from the forms typed with their okurigana
/// marked (長さ, 来て), costs `rare_cost` more, behind every word met at
/// least [`MIN_COUNT`] times that is typed so (流さ, 着て).
pub(crate) fn okuri_lines(
    entries: &[Entry],
    rare_cost: Option<u32>,
    typed_marked: impl Fn(&Entry) -> bool,
) -> Vec<Entry> {
    // The cheapest cost of each line, and of the entries typed marked.
    let mut okuri: BTreeMap<(String, String), (u32, Option<u32>)> = BTreeMap::new();
    for entry in entries {
        if let Some(key) = okuri_line(&entry.reading, &entry.surface) {
            let marked = typed_marked(entry).then_some(entry.cost);
            okuri
                .entry(key)
                .and_modify(|(c, m)| {
                    *c = (*c).min(entry.cost);
                    *m = match (*m, marked) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                })
                .or_insert((entry.cost, marked));
        }
    }
    okuri
        .into_iter()
        .filter(|((_, surface), (cost, _))| {
            surface.chars().count() > 2 || rare_cost.is_none_or(|rare| *cost < rare)
        })
        .map(|((reading, surface), (cost, marked))| Entry {
            reading,
            surface,
            cost: marked.unwrap_or(cost + rare_cost.unwrap_or_default()),
            conjugation: None,
        })
        .collect()
}

/// Numeric entries from their counts. A counter of the は row also takes the
/// readings whose first kana after the number is another of its plain, voiced
/// and semi-voiced forms (`{}ほん` and `{}ぼん` from `{}ぽん`), as it changes
/// with the number before it: the analyzer reads 本 as ぽん after any number.
/// No other row voices after a number by rule (回 stays かい); one that does
/// after some numbers (三階 as さんがい) takes its voiced lines from the small
/// counted words (see [`crate::counted_voicings`]). A reading met more than
/// once keeps its smallest cost.
fn numeric_entries(counts: HashMap<(String, String), usize>, total: usize) -> Vec<Entry> {
    let mut costs: BTreeMap<(String, String), u32> = BTreeMap::new();
    for ((reading, surface), count) in counts {
        if count < MIN_COUNT {
            continue;
        }
        let cost = cost_of(count, total);
        let readings = if voices_freely(&reading) {
            voicings(&reading)
        } else {
            vec![reading]
        };
        for reading in readings {
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

/// Whether the first kana after the first `{}` of `reading` is of the は row,
/// voiced or not.
fn voices_freely(reading: &str) -> bool {
    reading
        .split_once("{}")
        .and_then(|(_, after)| after.chars().next())
        .is_some_and(|first| "はひふへほ".contains(unvoiced(first)))
}

/// `reading` with the first kana after its first `{}` in each of its plain,
/// voiced and semi-voiced forms; `reading` alone when that kana has no other
/// form.
pub(crate) fn voicings(reading: &str) -> Vec<String> {
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

    #[test]
    fn a_word_of_nippon_alone_is_also_read_with_nihon_at_its_cost() {
        let entry = |reading, surface, cost| entry(reading, surface, None, cost);
        let entries = [
            entry("にっぽんじかん", "日本時間", 1078),
            entry("にっぽんぎんこう", "日本銀行", 1217),
            entry("にっぽんし", "日本史", 1230),
            entry("にほんし", "日本史", 1500),
            entry("にっぽん", "日本", 626),
            entry("にほん", "日本", 600),
            entry("にっぽんにっぽん", "日本人", 3000),
            entry("にっぽん", "ニッポン", 900),
        ];

        let nihon = nihon_readings(&entries);

        let lines: Vec<(&str, &str, u32)> = nihon
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str(), e.cost))
            .collect();
        assert_eq!(
            lines,
            [
                ("にほんじかん", "日本時間", 1078),
                ("にほんぎんこう", "日本銀行", 1217),
            ],
            "日本史 and 日本 read にほん already, ニッポン has no 日本, \
             and にっぽんにっぽん 日本人 reads にっぽん more often than it holds 日本"
        );
    }

    fn word(reading: &str, surface: &str) -> Unit {
        Unit {
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
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
                kana: String::new(),
            }),
            ..word(reading, surface)
        }
    }

    #[test]
    fn added_words_join_the_entries_with_their_okurigana_lines_and_the_smaller_cost() {
        let mut dict = Dictionary::build(
            [
                repeat(word("ひとつ", "一つ"), 3),
                repeat(word("ほん", "本"), 2),
            ]
            .concat(),
            &[],
        );
        let kept = find(&dict.entries, "ひとつ", "一つ").unwrap().cost;

        dict.add_words([
            entry("ひとつ", "一つ", None, kept + 100),
            entry("いっぴき", "一匹", None, 900),
        ]);

        assert_eq!(find(&dict.entries, "ひとつ", "一つ").unwrap().cost, kept);
        assert_eq!(find(&dict.entries, "いっぴき", "一匹").unwrap().cost, 900);
        assert_eq!(dict.entries.len(), 3);
        assert!(
            find(&dict.okuri, "ひと*つ", "一つ").is_some(),
            "{:?}",
            dict.okuri
        );
    }

    #[test]
    fn a_word_spelled_with_variant_okurigana_makes_no_entry_but_counts_toward_the_total() {
        let variant = Unit {
            okurigana_variant: true,
            parts: Vec::new(),
            ..conj("ちいさい", "小い", ("ちいさ", "小"), "形容詞")
        };
        let mut units = repeat(variant, 2);
        units.extend(repeat(word("てがみ", "手紙"), 2));

        let dict = Dictionary::build(units, &[]);

        assert_eq!(dict.entries, [entry("てがみ", "手紙", None, cost(2, 4))]);
    }

    #[test]
    fn the_okurigana_dictionary_holds_the_variant_spellings_alone() {
        let variant = Unit {
            okurigana_variant: true,
            parts: Vec::new(),
            ..conj("ちいさい", "小い", ("ちいさ", "小"), "形容詞")
        };
        let mut units = repeat(variant, 2);
        units.extend(repeat(word("てがみ", "手紙"), 2));

        let mut entries = okurigana_dictionary(units).entries;
        entries.sort_by(|a, b| a.reading.cmp(&b.reading));

        assert_eq!(
            entries,
            [
                entry("ちいさ", "小", Some("形容詞"), cost(2, 2)),
                entry("ちいさい", "小い", None, cost(2, 2)),
            ]
        );
    }

    #[test]
    fn a_form_cut_short_at_a_small_tsu_is_no_word_but_its_stem_is() {
        let units = [
            repeat(
                conj("しらなかっ", "知らなかっ", ("し", "知"), "五段-ラ行"),
                2,
            ),
            repeat(conj("はまっ", "ハマっ", ("はま", "ハマ"), "五段-ラ行"), 2),
            repeat(word("ぱっ", "パッ"), 2),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        assert!(find(&dict.entries, "しらなかっ", "知らなかっ").is_none());
        assert!(find(&dict.entries, "し", "知").is_some());
        assert!(find(&dict.entries, "はまっ", "ハマっ").is_none());
        assert!(find(&dict.entries, "ぱっ", "パッ").is_some());
    }

    #[test]
    fn a_surface_broken_by_spaces_marks_or_commas_or_left_open_is_no_word() {
        for broken in [
            "美 少年",
            "ヘ゛ヒ゛ー",
            "一、二塁間",
            "インター-",
            "カーポーカー〜",
            "ハ～",
        ] {
            assert!(!is_word("よみ", broken), "{broken}");
        }
        for word in ["手紙", "セブン-イレブン", "まどか☆マギカ", "Wi-Fi"] {
            assert!(is_word("よみ", word), "{word}");
        }
    }

    #[test]
    fn a_reading_from_a_small_kana_a_moraic_nasal_or_a_long_vowel_is_no_word() {
        for (reading, surface) in [
            ("っぱなし", "放し"),
            ("って", "ッて"),
            ("んな", "ンな"),
            ("ぁ", "ァ"),
        ] {
            assert!(!is_word(reading, surface), "{surface}");
        }
        assert!(is_word("ぱっ", "パッ"));
    }

    #[test]
    fn dropped_words_leave_with_their_okurigana_lines() {
        let mut dict = Dictionary::build(
            [
                repeat(word("とうつ", "十つ"), 3),
                repeat(word("ひとつ", "一つ"), 3),
            ]
            .concat(),
            &[],
        );

        dict.drop_words(|_, surface| surface == "十つ");

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
    fn a_numeric_unit_with_a_gloss_in_brackets_makes_no_numeric_entry() {
        let units = [
            counted(
                "5か",
                "5日（アスキー）",
                ("{}か", "{half-num}日（アスキー）", "5"),
            ),
            counted(
                "3か",
                "3日（アスキー）",
                ("{}か", "{half-num}日（アスキー）", "3"),
            ),
        ];

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(dict.numeric, []);
    }

    #[test]
    fn a_numeric_entry_of_the_ha_row_takes_every_voicing_and_no_other_does() {
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
                ("{}がつ", "{}月", c),
                ("{}にん", "{}人", c),
                ("{}ほん", "{}本", c),
                ("{}ぼん", "{}本", c),
                ("{}ぽん", "{}本", c),
                ("だい{}かい", "第{}回", c),
            ]
        );
    }

    #[test]
    fn added_numeric_entries_keep_the_smaller_cost_of_a_line_met_twice() {
        let mut dict = Dictionary::build(
            repeat(counted("3かい", "3階", ("{}かい", "{}階", "3")), 2),
            &[],
        );
        let kept = dict.numeric[0].cost;

        dict.add_numeric([
            entry("{}がい", "{}階", None, 900),
            entry("{}かい", "{}階", None, kept + 1),
        ]);

        assert_eq!(find(&dict.numeric, "{}がい", "{}階").unwrap().cost, 900);
        assert_eq!(find(&dict.numeric, "{}かい", "{}階").unwrap().cost, kept);
        assert_eq!(dict.numeric.len(), 2);
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
        assert_eq!(
            cost_of("{}こ", "{}個"),
            cost(2, 14),
            "a counter of the か row is no voicing of another reading"
        );
        assert_eq!(cost_of("{}ご", "{}個"), cost(5, 14));
    }

    #[test]
    fn numeric_units_are_also_costed_by_their_surface_as_written() {
        let units: Vec<Unit> = [
            repeat(counted("1はつ", "一発", ("{}はつ", "{kanji}発", "1")), 5),
            repeat(counted("2はつ", "2発", ("{}はつ", "{}発", "2")), 3),
            repeat(counted("3ばつ", "三発", ("{}ばつ", "{kanji}発", "3")), 1),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let mut written: Vec<(&str, u32)> = dict
            .numeric_written
            .iter()
            .map(|(surface, &cost)| (surface.as_str(), cost))
            .collect();
        written.sort();
        assert_eq!(written, [("2発", cost(3, 9)), ("一発", cost(5, 9))]);
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
            okuri_lines(&entries, None, |_| true),
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

    /// `unit` at `position` of document `doc_id`.
    fn at(unit: Unit, doc_id: &str, position: usize) -> Unit {
        Unit {
            doc_id: doc_id.into(),
            position,
            ..unit
        }
    }

    fn compound(reading: &str, surface: &str) -> Unit {
        Unit {
            compound: true,
            ..word(reading, surface)
        }
    }

    /// 危機一髪 as the analyzer cuts it in document `doc_id`: the parts, and
    /// the compound beside them in position order.
    fn kiki_ippatsu(doc_id: &str) -> Vec<Unit> {
        vec![
            at(word("きき", "危機"), doc_id, 0),
            at(compound("ききいっぱつ", "危機一髪"), doc_id, 0),
            at(word("いっぱつ", "一髪"), doc_id, 2),
        ]
    }

    #[test]
    fn a_word_met_mostly_inside_compounds_counts_only_where_it_stands_outside_them() {
        let units: Vec<Unit> = [
            (0..6)
                .flat_map(|i| kiki_ippatsu(&format!("k{i}")))
                .collect(),
            vec![at(word("いっぱつ", "一髪"), "a", 0)],
            vec![at(word("いっぱつ", "一髪"), "b", 0)],
            repeat(word("きき", "危機"), 2),
        ]
        .concat();
        let total = units.len();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            find(&dict.entries, "いっぱつ", "一髪").unwrap().cost,
            cost(2, total),
            "6 of the 8 places of 一髪 are inside 危機一髪"
        );
        assert_eq!(
            find(&dict.entries, "ききいっぱつ", "危機一髪")
                .unwrap()
                .cost,
            cost(6, total)
        );
    }

    #[test]
    fn lexicon_words_count_a_fragment_s_surface_only_outside_the_compounds() {
        let units: Vec<Unit> = [
            (0..6)
                .flat_map(|i| kiki_ippatsu(&format!("k{i}")))
                .collect(),
            vec![at(word("いっぱつ", "一髪"), "a", 0)],
        ]
        .concat();

        let dict = Dictionary::build_with_lexicon(&units, &[], &[unidic("ひとはつ", "一髪")]);

        assert!(
            find(&dict.entries, "ひとはつ", "一髪").is_none(),
            "6 of the 7 places of 一髪 are inside 危機一髪, so it stands alone once"
        );
    }

    #[test]
    fn a_word_met_only_inside_compounds_makes_no_entry() {
        let units: Vec<Unit> = (0..3)
            .flat_map(|i| {
                let doc_id = format!("d{i}");
                [
                    at(word("ぜんぽう", "前方"), &doc_id, 0),
                    at(compound("ぜんぽうこうえんふん", "前方後円墳"), &doc_id, 0),
                    at(word("こうえんふん", "後円墳"), &doc_id, 2),
                ]
            })
            .collect();

        let dict = Dictionary::build(&units, &[]);

        assert!(find(&dict.entries, "こうえんふん", "後円墳").is_none());
        assert!(find(&dict.entries, "ぜんぽう", "前方").is_none());
        assert!(find(&dict.entries, "ぜんぽうこうえんふん", "前方後円墳").is_some());
    }

    #[test]
    fn a_word_met_often_enough_outside_compounds_counts_every_place() {
        let units: Vec<Unit> = [
            (0..2)
                .flat_map(|i| {
                    let doc_id = format!("t{i}");
                    [
                        at(compound("とうきょうとちょう", "東京都庁"), &doc_id, 0),
                        at(word("とうきょう", "東京"), &doc_id, 0),
                        at(word("とちょう", "都庁"), &doc_id, 2),
                    ]
                })
                .collect(),
            repeat(word("とうきょう", "東京"), 2),
        ]
        .concat();
        let total = units.len();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            find(&dict.entries, "とうきょう", "東京").unwrap().cost,
            cost(4, total),
            "half the places of 東京 are outside 東京都庁"
        );
    }

    #[test]
    fn a_reading_met_mostly_inside_compounds_counts_every_place_when_its_surface_mostly_stands_alone()
     {
        let units: Vec<Unit> = [
            (0..3)
                .flat_map(|i| {
                    let doc_id = format!("n{i}");
                    [
                        at(compound("にほんてれび", "日本テレビ"), &doc_id, 0),
                        at(word("にほん", "日本"), &doc_id, 0),
                        at(word("てれび", "テレビ"), &doc_id, 2),
                    ]
                })
                .collect(),
            repeat(word("にっぽん", "日本"), 6),
        ]
        .concat();
        let total = units.len();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            find(&dict.entries, "にほん", "日本").unwrap().cost,
            cost(3, total),
            "the analyzer reads 日本 alone にっぽん and inside compounds にほん"
        );
    }

    #[test]
    fn spans_that_overlap_still_cover_what_lies_inside_any_of_them() {
        // 朽ち果てた joined and 朽ち果て grouped start at one place.
        let spans = Spans::new([(6, 8), (0, 5), (0, 4)]);

        assert!(spans.covers(2, 5));
        assert!(spans.covers(0, 4));
        assert!(spans.covers(6, 8));
        assert!(!spans.covers(4, 6));
        assert!(!spans.covers(5, 6));
    }

    #[test]
    fn a_word_that_only_touches_a_compound_is_not_inside_it() {
        let units: Vec<Unit> = (0..2)
            .flat_map(|i| {
                let doc_id = format!("d{i}");
                [
                    at(word("きのう", "昨日"), &doc_id, 0),
                    at(compound("ききいっぱつ", "危機一髪"), &doc_id, 2),
                    at(word("きき", "危機"), &doc_id, 2),
                    at(word("いっぱつ", "一髪"), &doc_id, 4),
                    at(word("あした", "明日"), &doc_id, 6),
                ]
            })
            .collect();
        let total = units.len();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            find(&dict.entries, "きのう", "昨日").unwrap().cost,
            cost(2, total)
        );
        assert_eq!(
            find(&dict.entries, "あした", "明日").unwrap().cost,
            cost(2, total)
        );
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
    fn lexicon_words_enter_as_unidic_words_only_where_units_show_the_surface_enough() {
        let units: Vec<Unit> = [
            repeat(word("だいもんじ", "大文字"), 3),
            repeat(word("てがみ", "手紙"), 1),
        ]
        .concat();
        let lexicon = [
            unidic("おおもじ", "大文字"),
            unidic("だいもんじ", "大文字"),
            unidic("おおそうじ", "大掃除"),
            unidic("しょかん", "手紙"),
        ];

        let dict = Dictionary::build_with_lexicon(&units, &[], &lexicon);

        assert_eq!(
            find(&dict.entries, "おおもじ", "大文字").unwrap().cost,
            cost(3, 4) + 1000
        );
        assert_eq!(
            find(&dict.entries, "だいもんじ", "大文字").unwrap().cost,
            cost(3, 4)
        );
        assert!(find(&dict.entries, "おおそうじ", "大掃除").is_none());
        assert!(find(&dict.entries, "しょかん", "手紙").is_none());
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
    fn a_form_of_a_type_the_table_does_not_know_is_a_word_written_whole_when_it_shows_its_okurigana()
     {
        let unknown = |reading: &str, surface: &str, conjugation: &str| Unit {
            unknown_conjugation: Some(conjugation.into()),
            ..word(reading, surface)
        };
        let units = [
            repeat(unknown("わかき", "若き", "文語形容詞-ク"), 2),
            repeat(unknown("ごとく", "如く", "文語助動詞-ゴトシ"), 3),
            repeat(unknown("そうろう", "候", "文語四段-ハ行"), 2),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        let mut words: Vec<(&str, &str, u32)> = dict
            .entries
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str(), e.cost))
            .collect();
        words.sort();
        assert_eq!(
            words,
            [
                ("ごとく", "如く", cost(3, 7)),
                ("わかき", "若き", cost(2, 7))
            ]
        );
        assert_eq!(dict.report.unknown_conjugations.len(), 3);
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
            repeat(conj("かいた", "書いた", ("か", "書"), "五段-カ行"), 3),
            repeat(word("みなさん", "皆さん"), 3),
            repeat(conj("たべる", "食べる", ("たべ", "食べ"), "下一段-バ行"), 3),
            repeat(word("こころぐるしい", "心苦しい"), 2),
            repeat(word("きとう", "祈とう"), 2),
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
        assert!(lines.contains(&("こころぐる*し", "心苦し")), "{lines:?}");
        assert!(
            !lines.contains(&("き*と", "祈と")),
            "one kanji met twice: {lines:?}"
        );
        assert_eq!(
            find(&dict.okuri, "か*い", "書い").unwrap().cost,
            cost(3, 13)
        );
        assert_eq!(dict.rare_cost, Some(cost(2, 13)));
    }

    /// The cost of the line `reading surface` with no conjugation type in
    /// `dict`'s text.
    fn line_cost(dict: &Dictionary, reading: &str, surface: &str) -> Option<u32> {
        dict.to_text("t").lines().find_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields[..] {
                [r, s, "", c] if r == reading && s == surface => c.parse().ok(),
                _ => None,
            }
        })
    }

    #[test]
    fn a_conjugated_form_typed_whole_comes_behind_every_counted_word_of_its_reading() {
        let units: Vec<Unit> = [
            repeat(conj("かく", "書く", ("か", "書"), "五段-カ行"), 5),
            repeat(word("かく", "各"), 2),
            repeat(conj("かく", "描く", ("か", "描"), "五段-カ行"), 1),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[unidic("かく", "書く"), unidic("かく", "描く")]);

        assert_eq!(line_cost(&dict, "かく", "各"), Some(cost(2, 8)));
        assert_eq!(
            line_cost(&dict, "かく", "書く"),
            Some(cost(5, 8) + cost(2, 8)),
        );
        assert_eq!(
            line_cost(&dict, "かく", "描く"),
            Some(cost(1, 8) + cost(2, 8)),
            "a form met once, which UniDic brings in, stays behind the forms met more",
        );
        assert_eq!(
            find(&dict.okuri, "か*く", "書く").unwrap().cost,
            cost(5, 8),
            "typed with its okurigana, the form keeps its count",
        );
    }

    #[test]
    fn a_word_written_whole_as_often_as_a_word_enters_costs_by_those_units_alone() {
        let units: Vec<Unit> = [
            repeat(conj("わらい", "笑い", ("わら", "笑"), "五段-ワア行"), 5),
            repeat(word("わらい", "笑い"), 2),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(line_cost(&dict, "わらい", "笑い"), Some(cost(2, 7)));
        assert_eq!(
            find(&dict.okuri, "わら*い", "笑い").unwrap().cost,
            cost(7, 7)
        );
    }

    #[test]
    fn an_okurigana_line_of_words_typed_whole_alone_comes_behind_the_forms_typed_marked() {
        let units: Vec<Unit> = [
            repeat(word("ながさ", "長さ"), 5),
            repeat(conj("ながさ", "流さ", ("なが", "流"), "五段-サ行"), 3),
        ]
        .concat();

        let dict = Dictionary::build(&units, &[]);

        assert_eq!(
            find(&dict.okuri, "なが*さ", "流さ").unwrap().cost,
            cost(3, 8)
        );
        assert_eq!(
            find(&dict.okuri, "なが*さ", "長さ").unwrap().cost,
            cost(5, 8) + cost(2, 8),
            "長さ is typed ながさ, not with its okurigana marked"
        );
    }

    #[test]
    fn one_kanji_lines_of_rare_words_are_left_out() {
        let entries = [
            entry("きとう", "祈とう", None, 1631),
            entry("きっぷ", "気っぷ", None, 3000),
            entry("みなさん", "皆さん", None, 1500),
            entry("やまおり", "山折り", None, 1700),
        ];

        let lines = |rare| -> Vec<(String, String)> {
            okuri_lines(&entries, rare, |_| false)
                .into_iter()
                .map(|e| (e.reading, e.surface))
                .collect()
        };
        let line = |r: &str, s: &str| (r.to_owned(), s.to_owned());
        assert_eq!(
            lines(Some(1631)),
            [line("みな*さ", "皆さ"), line("やまお*り", "山折り")]
        );
        assert_eq!(lines(None).len(), 4);
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

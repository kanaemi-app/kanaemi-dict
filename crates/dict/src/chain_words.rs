//! The runs of units the evaluation types as one word because another IME
//! converts them whole (起動時 of 起動＋時): what a typist pays when the
//! dictionary lacks a word the analyzer splits.

use std::collections::HashMap;
use std::io::{self, BufRead};

use crate::Unit;
use crate::eval_documents::span_of;
use crate::kana::{is_kanji, is_katakana, unvoiced};

/// The longest run of units looked at as one word.
const MAX_UNITS: usize = 6;

/// The words another IME's word list converts whole, by surface: their
/// readings. Only surfaces of kanji and katakana with a kanji are kept, the
/// ones a run of units can write and a typist needs to convert.
#[derive(Debug, Default, Clone)]
pub struct ChainWords {
    readings: HashMap<String, Vec<String>>,
}

/// A run of units another IME converts whole, typed as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSpan {
    /// The run as one unit, read as the word list reads it.
    pub unit: Unit,
    /// How many units the run is.
    pub parts: usize,
}

/// Reads the word list of Mozc's OSS dictionary: per line, the reading, two
/// ids, a cost and the surface, tab-separated.
pub fn read_chain_words(lines: impl BufRead, words: &mut ChainWords) -> io::Result<()> {
    for line in lines.lines() {
        let line = line?;
        let fields: Vec<&str> = line.split('\t').collect();
        let (Some(&reading), Some(&surface)) = (fields.first(), fields.get(4)) else {
            continue;
        };
        if surface.chars().nth(1).is_none()
            || !surface.chars().all(is_run_char)
            || !surface.chars().any(is_kanji)
        {
            continue;
        }
        let readings = words.readings.entry(surface.to_owned()).or_default();
        if !readings.iter().any(|r| r == reading) {
            readings.push(reading.to_owned());
        }
    }
    Ok(())
}

impl ChainWords {
    pub fn len(&self) -> usize {
        self.readings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.readings.is_empty()
    }

    /// The runs of `typed`, the units of a document as the evaluation types
    /// them, that the list has as a word: touching units of kanji and
    /// katakana, two or more, not ending in a numeric unit (一月一日 is typed
    /// as numeric items), the longest from the first unit on, then on from
    /// its end. A run is read as the list reads it, the reading its units
    /// join to or one that differs from it only in voicing and small tsu
    /// where they join (かぶしきがいしゃ for 株式＋会社); a run the list reads
    /// otherwise is left out, as what a typist types for it is unclear.
    pub fn spans(&self, typed: &[Unit]) -> Vec<ChainSpan> {
        let mut spans = Vec::new();
        let mut start = 0;
        while start < typed.len() {
            let mut end = start;
            while end < typed.len()
                && is_run_unit(&typed[end])
                && (end == start || span_of(&typed[end - 1]).1 == typed[end].position)
            {
                end += 1;
            }
            let run = &typed[start..end];
            let mut i = 0;
            while i < run.len() {
                match self.longest_from(&run[i..]) {
                    Some(span) => {
                        i += span.parts;
                        spans.push(span);
                    }
                    None => i += 1,
                }
            }
            start = end.max(start + 1);
        }
        spans
    }

    fn longest_from(&self, run: &[Unit]) -> Option<ChainSpan> {
        (2..=run.len().min(MAX_UNITS)).rev().find_map(|n| {
            let units = &run[..n];
            if units[n - 1].numeric.is_some() {
                return None;
            }
            let surface: String = units.iter().map(|u| u.surface.as_str()).collect();
            let readings = self.readings.get(&surface)?;
            let joined: String = units.iter().map(reading_of).collect();
            let reading = readings
                .iter()
                .find(|r| **r == joined)
                .or_else(|| readings.iter().find(|r| same_but_joins(r, &joined)))?;
            Some(ChainSpan {
                unit: Unit {
                    compound: true,
                    parts: Vec::new(),
                    particle: None,
                    suru: None,
                    copula: None,
                    numeric: None,
                    stem_reading: None,
                    stem_surface: None,
                    conjugation: None,
                    unknown_conjugation: None,
                    okurigana_variant: false,
                    doc_id: units[0].doc_id.clone(),
                    position: units[0].position,
                    reading: reading.clone(),
                    surface,
                },
                parts: n,
            })
        })
    }
}

/// A numeric unit reads its number in kana for a run.
fn reading_of(unit: &Unit) -> &str {
    unit.numeric
        .as_ref()
        .map_or(unit.reading.as_str(), |n| n.kana.as_str())
}

fn is_run_char(c: char) -> bool {
    is_kanji(c) || is_katakana(c) || c == 'ー'
}

fn is_run_unit(unit: &Unit) -> bool {
    unit.stem_surface.is_none()
        && !unit.okurigana_variant
        && !unit.surface.is_empty()
        && unit.surface.chars().all(is_run_char)
        && !reading_of(unit).is_empty()
}

/// Whether `a` and `b` read alike but for voicing and small tsu.
fn same_but_joins(a: &str, b: &str) -> bool {
    let plain = |s: &str| -> String {
        s.chars()
            .map(|c| match unvoiced(c) {
                'っ' => 'つ',
                c => c,
            })
            .collect()
    };
    a.chars().count() == b.chars().count() && plain(a) == plain(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Numeric;

    fn unit(position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            particle: None,
            suru: None,
            copula: None,
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
            doc_id: "d".into(),
            position,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        }
    }

    /// `parts` (surface, reading) as units that touch, starting at `position`.
    fn touching(position: usize, parts: &[(&str, &str)]) -> Vec<Unit> {
        let mut at = position;
        parts
            .iter()
            .map(|&(s, r)| {
                let u = unit(at, r, s);
                at += s.chars().count();
                u
            })
            .collect()
    }

    fn words(lines: &[(&str, &str)]) -> ChainWords {
        let text: String = lines
            .iter()
            .map(|(r, s)| format!("{r}\t1847\t1847\t5000\t{s}\n"))
            .collect();
        let mut words = ChainWords::default();
        read_chain_words(text.as_bytes(), &mut words).unwrap();
        words
    }

    fn found(words: &ChainWords, typed: &[Unit]) -> Vec<(usize, String, String, usize)> {
        words
            .spans(typed)
            .into_iter()
            .map(|s| (s.unit.position, s.unit.reading, s.unit.surface, s.parts))
            .collect()
    }

    #[test]
    fn the_list_keeps_surfaces_of_kanji_and_katakana_longer_than_one_character() {
        let words = words(&[
            ("きどうじ", "起動時"),
            ("あう", "会う"),
            ("き", "木"),
            ("てすと", "テスト"),
            ("にほんけいざい", "日本経済"),
            ("にっぽんけいざい", "日本経済"),
        ]);

        assert_eq!(words.len(), 2);
        assert_eq!(
            words.readings["日本経済"],
            ["にほんけいざい", "にっぽんけいざい"]
        );
    }

    #[test]
    fn touching_units_the_list_has_as_a_word_are_one_span() {
        let words = words(&[("きどうじ", "起動時")]);
        let mut typed = touching(0, &[("起動", "きどう"), ("時", "じ")]);
        typed.push(unit(3, "の", "の"));

        assert_eq!(
            found(&words, &typed),
            [(0, "きどうじ".into(), "起動時".into(), 2)]
        );
    }

    #[test]
    fn the_longest_from_the_first_unit_wins_and_the_next_starts_after_it() {
        let words = words(&[
            ("じこてんけん", "自己点検"),
            ("てんけんこうもく", "点検項目"),
            ("こうもくすう", "項目数"),
        ]);
        let typed = touching(
            0,
            &[
                ("自己", "じこ"),
                ("点検", "てんけん"),
                ("項目", "こうもく"),
                ("数", "すう"),
            ],
        );

        assert_eq!(
            found(&words, &typed),
            [
                (0, "じこてんけん".into(), "自己点検".into(), 2),
                (4, "こうもくすう".into(), "項目数".into(), 2),
            ]
        );
    }

    #[test]
    fn kana_or_a_gap_breaks_a_run() {
        let words = words(&[("じこてんけん", "自己点検")]);
        let typed = [
            unit(0, "じこ", "自己"),
            unit(2, "の", "の"),
            unit(3, "てんけん", "点検"),
            unit(10, "じこ", "自己"),
            unit(13, "てんけん", "点検"),
        ];

        assert!(found(&words, &typed).is_empty());
    }

    #[test]
    fn a_single_unit_is_no_span() {
        let words = words(&[("きどうじ", "起動時")]);

        assert!(found(&words, &[unit(0, "きどうじ", "起動時")]).is_empty());
    }

    fn numeric(position: usize, surface: &str, reading: &str, kana: &str) -> Unit {
        Unit {
            numeric: Some(Numeric {
                reading: reading.into(),
                surface: String::new(),
                value: String::new(),
                kana: kana.into(),
            }),
            ..unit(position, reading, surface)
        }
    }

    #[test]
    fn a_numeric_unit_reads_its_kana_and_may_not_end_a_span() {
        let words = words(&[("にじかい", "二次会"), ("いちがつついたち", "一月一日")]);
        let typed = [
            numeric(0, "二次", "{}じ", "にじ"),
            unit(2, "かい", "会"),
            unit(3, "、", "、"),
            numeric(4, "一月", "{}がつ", "いちがつ"),
            numeric(6, "一日", "{}にち", "ついたち"),
        ];

        assert_eq!(
            found(&words, &typed),
            [(0, "にじかい".into(), "二次会".into(), 2)]
        );
    }

    #[test]
    fn a_reading_that_differs_only_where_units_join_reads_the_span() {
        let words = words(&[
            ("かぶしきがいしゃ", "株式会社"),
            ("にっぽんけいざい", "日本経済"),
            ("にほんけいざい", "日本経済"),
            ("じゅうしょ", "住所録"),
        ]);
        let typed = [
            touching(0, &[("株式", "かぶしき"), ("会社", "かいしゃ")]),
            touching(5, &[("日本", "にほん"), ("経済", "けいざい")]),
            touching(10, &[("住所", "じゅうしょ"), ("録", "ろく")]),
        ]
        .concat();

        assert_eq!(
            found(&words, &typed),
            [
                (0, "かぶしきがいしゃ".into(), "株式会社".into(), 2),
                (5, "にほんけいざい".into(), "日本経済".into(), 2),
            ],
            "住所録 is read otherwise",
        );
    }
}

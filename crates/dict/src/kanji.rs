//! Single kanji readings (すで 既), which no text makes: a kanji alone is
//! seldom a word. They come from Mozc's table of single kanji, with Unihan's
//! Sino-Japanese readings for the kanji the table lacks, and sort after every
//! word by how often the base dictionary's words read each kanji so.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::Entry;
use crate::kana::{is_hiragana, is_kanji, is_katakana};
use crate::katakana_to_hiragana;

/// A kanji and a reading of it.
pub type KanjiReading = (String, char);

/// Where the cost of a reading some word backs starts: past every word's
/// cost, the ranking model's band 12.
const BACKED: u32 = 4096;
/// Where the cost of a reading no word backs starts, the model's band 13.
const UNBACKED: u32 = 8192;
/// How many ways of reading one word's kanji are followed at most.
const MAX_ALIGNMENTS: usize = 64;

/// The readings of Mozc's table of single kanji, one line a reading and the
/// kanji it reads.
pub fn mozc_readings(text: &str) -> Vec<KanjiReading> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .flat_map(|(reading, kanji)| kanji.chars().map(move |k| (reading.to_owned(), k)))
        .collect()
}

/// The Sino-Japanese readings, in hiragana, of each kanji Unihan's
/// `kJapanese` field reads in katakana; the readings in hiragana are
/// Japanese ones, with no okurigana marked and many of them old.
pub fn unihan_readings(text: &str) -> HashMap<char, Vec<String>> {
    let mut readings: HashMap<char, Vec<String>> = HashMap::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let mut fields = line.split('\t');
        let (Some(code), Some("kJapanese"), Some(values)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some(kanji) = code
            .strip_prefix("U+")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
        else {
            continue;
        };
        let sino: Vec<String> = values
            .split_whitespace()
            .filter(|v| v.chars().all(|c| is_katakana(c) || c == 'ー'))
            .map(katakana_to_hiragana)
            .collect();
        if !sino.is_empty() {
            readings.insert(kanji, sino);
        }
    }
    readings
}

/// The pairs `kanji/excluded.tsv` keeps out, one `reading<TAB>kanji` a line;
/// a line starting with `#` and a blank line are skipped.
pub fn excluded_readings(text: &str) -> Result<HashSet<KanjiReading>, KanjiError> {
    let mut excluded = HashSet::new();
    for (i, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let pair = line.split_once('\t').and_then(|(reading, kanji)| {
            let mut chars = kanji.chars();
            match (chars.next(), chars.next()) {
                (Some(k), None) if !reading.is_empty() => Some((reading.to_owned(), k)),
                _ => None,
            }
        });
        excluded.insert(pair.ok_or(KanjiError { line: i + 1 })?);
    }
    Ok(excluded)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("line {line}: expected a reading, a tab and one kanji")]
pub struct KanjiError {
    pub line: usize,
}

/// The single kanji entries the base dictionary's `words` lack, and whether
/// any reading came from Unihan. The readings are Mozc's, and Unihan's for
/// the kanji of the words Mozc lacks, less the `excluded` pairs. Each word's
/// weight, from its cost, is shared among the ways its reading splits into
/// the readings of its kanji, and a reading so backed costs by its share,
/// past every word; one never backed costs by how much its kanji is used,
/// past those.
pub fn kanji_entries(
    words: &[Entry],
    mozc: &[KanjiReading],
    unihan: &HashMap<char, Vec<String>>,
    excluded: &HashSet<KanjiReading>,
) -> (Vec<Entry>, bool) {
    let whole: Vec<&Entry> = words.iter().filter(|w| w.conjugation.is_none()).collect();
    let have: HashSet<(&str, &str)> = whole
        .iter()
        .map(|w| (w.reading.as_str(), w.surface.as_str()))
        .collect();
    let readable: Vec<(&Entry, f64)> = whole
        .iter()
        .filter(|w| {
            w.surface.chars().any(is_kanji) && w.surface.chars().all(|c| is_kanji(c) || is_kana(c))
        })
        .map(|w| (*w, (-f64::from(w.cost) / 100.0).exp()))
        .collect();
    let mut kanji_weight: BTreeMap<char, f64> = BTreeMap::new();
    for (w, weight) in &readable {
        for c in w.surface.chars().filter(|c| is_kanji(*c)) {
            *kanji_weight.entry(c).or_default() += weight;
        }
    }
    let mut readings: BTreeMap<char, BTreeSet<String>> = BTreeMap::new();
    for (reading, kanji) in mozc {
        if !excluded.contains(&(reading.clone(), *kanji)) {
            readings.entry(*kanji).or_default().insert(reading.clone());
        }
    }
    let in_mozc: HashSet<char> = mozc.iter().map(|(_, k)| *k).collect();
    let mut from_unihan = false;
    for kanji in kanji_weight.keys().filter(|k| !in_mozc.contains(k)) {
        for reading in unihan.get(kanji).into_iter().flatten() {
            if !excluded.contains(&(reading.clone(), *kanji)) {
                readings.entry(*kanji).or_default().insert(reading.clone());
                from_unihan = true;
            }
        }
    }
    let mut backing: BTreeMap<(String, char), f64> = BTreeMap::new();
    for (w, weight) in &readable {
        let chars: Vec<char> = w.surface.chars().collect();
        let ways = alignments(&chars, &w.reading, &readings);
        let share = weight / ways.len().max(1) as f64;
        for way in ways {
            for pair in way {
                *backing.entry(pair).or_default() += share;
            }
        }
    }
    let backed_total: f64 = backing.values().sum();
    let kanji_total: f64 = kanji_weight.values().sum();
    let mut entries = Vec::new();
    for (kanji, kanji_readings) in &readings {
        let surface = kanji.to_string();
        for reading in kanji_readings {
            if have.contains(&(reading.as_str(), surface.as_str())) {
                continue;
            }
            let cost = match backing.get(&(reading.clone(), *kanji)) {
                Some(&b) => (BACKED + cost_of(b, backed_total)).min(UNBACKED - 1),
                None => (UNBACKED
                    + kanji_weight
                        .get(kanji)
                        .map_or(UNBACKED - 1, |&w| cost_of(w, kanji_total)))
                .min(2 * UNBACKED - 1),
            };
            entries.push(Entry {
                reading: reading.clone(),
                surface: surface.clone(),
                conjugation: None,
                cost,
            });
        }
    }
    entries.sort_by(|a, b| (&a.reading, &a.surface).cmp(&(&b.reading, &b.surface)));
    (entries, from_unihan)
}

fn is_kana(c: char) -> bool {
    is_hiragana(c) || is_katakana(c) || c == 'ー'
}

fn cost_of(share: f64, total: f64) -> u32 {
    (-(share / total).ln() * 100.0).round() as u32
}

/// The ways `reading` splits over `chars`, each kanji read as one of its
/// `readings` and each kana as itself, as the kanji and readings taken; at
/// most [`MAX_ALIGNMENTS`] of them.
fn alignments(
    chars: &[char],
    reading: &str,
    readings: &BTreeMap<char, BTreeSet<String>>,
) -> Vec<Vec<KanjiReading>> {
    let Some((&c, rest)) = chars.split_first() else {
        return if reading.is_empty() {
            vec![Vec::new()]
        } else {
            Vec::new()
        };
    };
    let mut ways = Vec::new();
    if is_kanji(c) {
        for r in readings.get(&c).into_iter().flatten() {
            let Some(after) = reading.strip_prefix(r.as_str()) else {
                continue;
            };
            for mut way in alignments(rest, after, readings) {
                way.insert(0, (r.clone(), c));
                ways.push(way);
                if ways.len() == MAX_ALIGNMENTS {
                    return ways;
                }
            }
        }
    } else if let Some(after) = reading.strip_prefix(katakana_to_hiragana(c.to_string()).as_str()) {
        ways = alignments(rest, after, readings);
    }
    ways
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(reading: &str, surface: &str, cost: u32) -> Entry {
        Entry {
            reading: reading.into(),
            surface: surface.into(),
            conjugation: None,
            cost,
        }
    }

    fn pair(reading: &str, kanji: char) -> KanjiReading {
        (reading.into(), kanji)
    }

    fn cost_of(entries: &[Entry], reading: &str, surface: &str) -> Option<u32> {
        entries
            .iter()
            .find(|e| e.reading == reading && e.surface == surface)
            .map(|e| e.cost)
    }

    #[test]
    fn each_kanji_of_a_mozc_line_is_read_as_the_line_reads() {
        assert_eq!(
            mozc_readings("すで\t既旣\nき\t木\n"),
            [pair("すで", '既'), pair("すで", '旣'), pair("き", '木')]
        );
    }

    #[test]
    fn unihan_gives_the_katakana_readings_in_hiragana_and_drops_the_rest() {
        let text = "# comment\nU+9AD9\tkJapanese\tコウ たかい\nU+9AD9\tkJapaneseOn\tKOU\nU+541E\tkJapanese\tドン トン のむ\n";

        let readings = unihan_readings(text);

        assert_eq!(readings[&'髙'], ["こう"]);
        assert_eq!(readings[&'吞'], ["どん", "とん"]);
    }

    #[test]
    fn excluded_pairs_are_read_one_a_line_skipping_comments_and_blanks() {
        let excluded =
            excluded_readings("# 読み\t字\nおれえるさま\t忙\n\nうしゃおし\t孝\n").unwrap();

        assert_eq!(
            excluded,
            HashSet::from([pair("おれえるさま", '忙'), pair("うしゃおし", '孝')])
        );
        assert_eq!(
            excluded_readings("すで\t既旣\n"),
            Err(KanjiError { line: 1 })
        );
    }

    #[test]
    fn a_single_kanji_sorts_after_every_word() {
        let words = [word("すでに", "既に", 900), word("みき", "幹", 3000)];

        let (entries, _) = kanji_entries(
            &words,
            &[pair("すで", '既')],
            &HashMap::new(),
            &HashSet::new(),
        );

        assert!(cost_of(&entries, "すで", "既").unwrap() > 3000);
    }

    #[test]
    fn a_reading_the_words_back_sorts_before_one_they_do_not() {
        let words = [word("すでに", "既に", 900), word("きせい", "既成", 1200)];
        let mozc = [
            pair("すで", '既'),
            pair("すで", '旣'),
            pair("き", '既'),
            pair("せい", '成'),
        ];

        let (entries, _) = kanji_entries(&words, &mozc, &HashMap::new(), &HashSet::new());

        let backed = cost_of(&entries, "すで", "既").unwrap();
        let unbacked = cost_of(&entries, "すで", "旣").unwrap();
        assert!((4096..8192).contains(&backed), "{backed}");
        assert!((8192..16384).contains(&unbacked), "{unbacked}");
    }

    #[test]
    fn a_reading_backed_more_often_sorts_first() {
        let words = [word("きせい", "既成", 900), word("すでに", "既に", 1500)];
        let mozc = [pair("すで", '既'), pair("き", '既'), pair("せい", '成')];

        let (entries, _) = kanji_entries(&words, &mozc, &HashMap::new(), &HashSet::new());

        assert!(cost_of(&entries, "き", "既") < cost_of(&entries, "すで", "既"));
    }

    #[test]
    fn a_pair_the_words_have_already_makes_no_entry() {
        let words = [word("き", "木", 1000)];

        let (entries, _) = kanji_entries(
            &words,
            &[pair("き", '木')],
            &HashMap::new(),
            &HashSet::new(),
        );

        assert_eq!(entries, []);
    }

    #[test]
    fn an_excluded_pair_makes_no_entry() {
        let excluded = HashSet::from([pair("おれえるさま", '忙')]);

        let (entries, _) = kanji_entries(
            &[],
            &[pair("おれえるさま", '忙'), pair("ぼう", '忙')],
            &HashMap::new(),
            &excluded,
        );

        assert_eq!(
            entries
                .iter()
                .map(|e| e.reading.as_str())
                .collect::<Vec<_>>(),
            ["ぼう"]
        );
    }

    #[test]
    fn unihan_reads_only_the_kanji_of_the_words_mozc_lacks() {
        let words = [word("たかはし", "髙橋", 1500), word("き", "木", 1000)];
        let unihan = HashMap::from([
            ('髙', vec!["こう".to_owned()]),
            ('木', vec!["ぼく".to_owned()]),
            ('吞', vec!["どん".to_owned()]),
        ]);

        let (entries, from_unihan) =
            kanji_entries(&words, &[pair("き", '木')], &unihan, &HashSet::new());

        assert_eq!(
            entries
                .iter()
                .map(|e| (e.reading.as_str(), e.surface.as_str()))
                .collect::<Vec<_>>(),
            [("こう", "髙")]
        );
        assert!(from_unihan);
    }
}

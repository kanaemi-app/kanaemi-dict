//! How often the documents spell the readings of a surface in kana, which
//! tells the readings the analyzer never gives the surface apart by use
//! (明日, always read あす, is spelled あした more often than あす).

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};

use rayon::prelude::*;

use crate::documents::{DocumentsError, each_document};
use crate::kana::unvoiced;
use crate::{AnalyzerError, Split, split_of};

/// The counts of each reading spelled in kana, by the surface the analyzer
/// normalizes the spelling to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KanaReadings(HashMap<String, BTreeMap<String, usize>>);

/// A reading spelled fewer times than this says nothing.
const MIN_SPELLED: usize = 2;

impl KanaReadings {
    /// Counts from (surface, reading, count).
    pub fn new(counts: impl IntoIterator<Item = (String, String, usize)>) -> Self {
        let mut out = Self::default();
        for (surface, reading, n) in counts {
            *out.0
                .entry(surface)
                .or_default()
                .entry(reading)
                .or_default() += n;
        }
        out
    }

    /// How many of the `n` places of `surface` are estimated to read
    /// `reading`, where the analyzer reads the surface `main`: its share of
    /// the spellings in kana. Where `main` is not spelled in kana the shares
    /// say nothing of it, and the spellings of `reading` themselves are the
    /// estimate. None without spellings enough, and for a reading that voices
    /// the head of another (木 as ぎ), which only stands inside words,
    /// nor for a reading of one kana, which the analyzer mostly takes from a
    /// particle or a piece of a word.
    pub fn estimate(&self, surface: &str, reading: &str, main: &str, n: usize) -> Option<f64> {
        let spelled = self.0.get(surface)?;
        let k = *spelled.get(reading).filter(|&&k| k >= MIN_SPELLED)?;
        let mut kana = reading.chars();
        let first = kana.next()?;
        if kana.as_str().is_empty() {
            return None;
        }
        let plain = format!("{}{}", unvoiced(first), kana.as_str());
        if plain != reading && (plain == main || spelled.contains_key(&plain)) {
            return None;
        }
        let share = if spelled.get(main).is_some_and(|&m| m >= MIN_SPELLED) {
            k as f64 / spelled.values().sum::<usize>() as f64 * n as f64
        } else {
            k.min(n) as f64
        };
        Some(share)
    }
}

/// The counts of the train documents and of every document.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    train: usize,
    all: usize,
}

/// Counts the kana spellings of `docs` that `words` gives as (surface,
/// reading), and writes a surface, a reading, the count in the train
/// documents and the count in every document per line.
pub fn write_kana_readings(
    docs: impl BufRead,
    words: impl Fn(&str) -> Result<Vec<(String, String)>, AnalyzerError> + Sync,
    mut out: impl Write,
) -> Result<usize, KanaReadingsError> {
    let mut total: BTreeMap<(String, String), Counts> = BTreeMap::new();
    let mut batch = Vec::new();
    let mut flush = |batch: &mut Vec<(String, String)>| -> Result<(), KanaReadingsError> {
        let found: Vec<(bool, Vec<(String, String)>)> = batch
            .par_iter()
            .map(|(doc_id, text)| {
                let mut found = Vec::new();
                for line in text.lines() {
                    found.extend(words(line)?);
                }
                Ok((split_of(doc_id) == Split::Train, found))
            })
            .collect::<Result<_, AnalyzerError>>()?;
        for (train, words) in found {
            for word in words {
                let counts = total.entry(word).or_default();
                counts.all += 1;
                counts.train += usize::from(train);
            }
        }
        batch.clear();
        Ok(())
    };
    let mut bytes = 0;
    each_document(docs, |doc| {
        bytes += doc.text.len();
        batch.push((doc.doc_id, doc.text));
        if bytes >= BATCH_BYTES {
            flush(&mut batch)?;
            bytes = 0;
        }
        Ok::<_, KanaReadingsError>(())
    })?;
    flush(&mut batch)?;
    for ((surface, reading), counts) in &total {
        writeln!(
            out,
            "{surface}\t{reading}\t{}\t{}",
            counts.train, counts.all
        )?;
    }
    Ok(total.len())
}

/// Text held at once while it is counted.
const BATCH_BYTES: usize = 16_000_000;

#[derive(Debug, thiserror::Error)]
pub enum KanaReadingsError {
    #[error("reading the documents: {0}")]
    Documents(#[from] DocumentsError),
    #[error(transparent)]
    Analyzer(#[from] AnalyzerError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// What [`write_kana_readings`] wrote, from the train documents alone with
/// `train_only`.
pub fn parse_kana_readings(text: &str, train_only: bool) -> Option<KanaReadings> {
    let counts = text
        .lines()
        .map(|line| {
            let [surface, reading, train, all] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            let n = if train_only { train } else { all }.parse().ok()?;
            Some((surface.to_owned(), reading.to_owned(), n))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(KanaReadings::new(
        counts.into_iter().filter(|&(_, _, n)| n > 0),
    ))
}

/// A reading the analyzer does not give a surface, with the share of the
/// surface's places estimated to read so.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherReading {
    pub surface: String,
    pub reading: String,
    pub share: f64,
}

/// The other readings of each surface, which the ranking model also learns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OtherReadings(HashMap<String, Vec<(String, f64)>>);

impl OtherReadings {
    pub fn new(readings: impl IntoIterator<Item = OtherReading>) -> Self {
        let mut out = Self::default();
        for r in readings {
            out.0
                .entry(r.surface)
                .or_default()
                .push((r.reading, r.share));
        }
        out
    }

    /// The readings the word `surface` at `position` of `doc_id` is also
    /// typed by: each in about its share of the places, the same ones in
    /// every run.
    pub fn at<'a>(
        &'a self,
        surface: &str,
        doc_id: &'a str,
        position: usize,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.0
            .get(surface)
            .into_iter()
            .flatten()
            .filter(move |(reading, share)| {
                let key = format!("{doc_id}:{position}:{reading}");
                let place = xxhash_rust::xxh3::xxh3_64(key.as_bytes()) as f64 / u64::MAX as f64;
                place < *share
            })
            .map(|(reading, _)| reading.as_str())
    }
}

/// A surface, a reading and its share per line.
pub fn other_readings_tsv(readings: &[OtherReading]) -> String {
    let mut lines: Vec<String> = readings
        .iter()
        .map(|r| format!("{}\t{}\t{:.6}\n", r.surface, r.reading, r.share))
        .collect();
    lines.sort();
    lines.concat()
}

/// What [`other_readings_tsv`] wrote.
pub fn parse_other_readings(text: &str) -> Option<OtherReadings> {
    text.lines()
        .map(|line| {
            let [surface, reading, share] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(OtherReading {
                surface: surface.to_owned(),
                reading: reading.to_owned(),
                share: share.parse().ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .map(OtherReadings::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(counts: &[(&str, &str, usize)]) -> KanaReadings {
        KanaReadings::new(
            counts
                .iter()
                .map(|&(s, r, n)| (s.to_owned(), r.to_owned(), n)),
        )
    }

    #[test]
    fn a_reading_takes_its_share_of_the_spellings() {
        let kana = readings(&[("明日", "あした", 300), ("明日", "あす", 100)]);

        assert_eq!(kana.estimate("明日", "あした", "あす", 1000), Some(750.0));
        assert_eq!(kana.estimate("明日", "みょうにち", "あす", 1000), None);
        assert_eq!(kana.estimate("昨日", "きのう", "さくじつ", 9), None);
    }

    #[test]
    fn a_reading_spelled_once_says_nothing() {
        let kana = readings(&[("明日", "あした", 1), ("明日", "あす", 100)]);

        assert_eq!(kana.estimate("明日", "あした", "あす", 1000), None);
    }

    #[test]
    fn without_spellings_of_the_main_reading_the_spellings_are_the_estimate() {
        let kana = readings(&[("目", "もく", 5)]);

        assert_eq!(kana.estimate("目", "もく", "め", 1000), Some(5.0));
        assert_eq!(kana.estimate("目", "もく", "め", 3), Some(3.0));
    }

    #[test]
    fn a_reading_of_one_kana_is_not_estimated() {
        let kana = readings(&[("彼", "か", 50), ("彼", "かれ", 50)]);

        assert_eq!(kana.estimate("彼", "か", "かれ", 1000), None);
    }

    #[test]
    fn a_reading_that_voices_another_is_not_estimated() {
        let kana = readings(&[("本", "ぼん", 5), ("本", "ほん", 9)]);

        assert_eq!(kana.estimate("本", "ぼん", "ほん", 1000), None);
        assert_eq!(kana.estimate("顔", "がお", "かお", 1000), None);
    }

    #[test]
    fn counts_are_kept_apart_by_split_and_read_back() {
        let docs = (0..40)
            .map(|i| {
                format!("{{\"doc_id\":\"a:{i:03}\",\"source_id\":\"s\",\"text\":\"あした\"}}\n")
            })
            .collect::<String>();
        let train = (0..40)
            .filter(|i| split_of(format!("a:{i:03}")) == Split::Train)
            .count();
        let mut out = Vec::new();

        write_kana_readings(
            docs.as_bytes(),
            |_| Ok(vec![("明日".to_owned(), "あした".to_owned())]),
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(train < 40);
        assert_eq!(
            parse_kana_readings(&text, true),
            Some(readings(&[("明日", "あした", train)]))
        );
        assert_eq!(
            parse_kana_readings(&text, false),
            Some(readings(&[("明日", "あした", 40)]))
        );
    }

    #[test]
    fn a_word_is_typed_by_another_reading_in_about_its_share_of_the_places() {
        let others = parse_other_readings(&other_readings_tsv(&[
            OtherReading {
                surface: "明日".into(),
                reading: "あした".into(),
                share: 0.6,
            },
            OtherReading {
                surface: "何時".into(),
                reading: "なんじ".into(),
                share: 1.0,
            },
        ]))
        .unwrap();
        let typed = |surface| (0..1000).flat_map(|i| others.at(surface, "d", i)).count();

        assert!((550..650).contains(&typed("明日")));
        assert_eq!(typed("何時"), 1000);
        assert_eq!(typed("昨日"), 0);
    }
}

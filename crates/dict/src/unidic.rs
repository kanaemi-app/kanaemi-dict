//! UniDic words from SudachiDict small's lexicon file.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;

use crate::kana::has_kanji;
use crate::katakana_to_hiragana;

/// A word with kanji from UniDic, as its lexicon gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnidicWord {
    pub reading: String,
    pub surface: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UnidicError {
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error("the lexicon has no {0} column")]
    MissingColumn(&'static str),
}

/// The non-conjugating words with kanji in the lexicon, without duplicates,
/// in reading and surface order.
pub fn plain_words(lexicon: impl Read) -> Result<Vec<UnidicWord>, UnidicError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(lexicon);
    let headers = reader.headers()?.clone();
    let column = |name: &'static str| {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or(UnidicError::MissingColumn(name))
    };
    let (index, head, pos1, pos5, reading) = (
        column("IndexForm")?,
        column("Headword")?,
        column("POS1")?,
        column("POS5")?,
        column("ReadingForm")?,
    );
    let mut words = BTreeSet::new();
    for record in reader.records() {
        let record = record?;
        let field = |i: usize| unescape(record.get(i).unwrap_or_default());
        if field(pos5) != "*" || matches!(field(pos1).as_str(), "補助記号" | "空白") {
            continue;
        }
        let surface = match field(head) {
            head if head.is_empty() => field(index),
            head => head,
        };
        let reading = field(reading);
        if surface.is_empty() || reading.is_empty() || !has_kanji(&surface) {
            continue;
        }
        words.insert((katakana_to_hiragana(&reading), surface));
    }
    Ok(words
        .into_iter()
        .map(|(reading, surface)| UnidicWord { reading, surface })
        .collect())
}

/// The readings UniDic gives each surface of its words, to check a reading
/// against.
#[derive(Debug, Clone, Default)]
pub struct UnidicReadings(HashMap<String, Vec<String>>);

impl UnidicReadings {
    pub fn new(words: impl IntoIterator<Item = UnidicWord>) -> Self {
        let mut readings: HashMap<String, Vec<String>> = HashMap::new();
        for word in words {
            readings.entry(word.surface).or_default().push(word.reading);
        }
        Self(readings)
    }

    /// UniDic's readings of `surface`; none when UniDic does not have it.
    pub fn of(&self, surface: impl AsRef<str>) -> &[String] {
        self.0.get(surface.as_ref()).map_or(&[], Vec::as_slice)
    }
}

/// Decodes the lexicon's character escapes, `\uXXXX` and `\u{X…}`.
fn unescape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(at) = rest.find("\\u") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let (hex, len) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => ("", 0),
            },
            None => (after.get(..4).unwrap_or_default(), 4),
        };
        match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
            Some(c) => {
                out.push(c);
                rest = &after[len..];
            }
            None => {
                out.push_str("\\u");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "IndexForm,LeftId,RightId,Cost,Headword,POS1,POS2,POS3,POS4,POS5,POS6,ReadingForm,NormalizedForm,DictionaryForm,SplitA,SplitB,WordStructure,SynonymGroups,ReferenceId\n";

    fn row(index: &str, head: &str, pos1: &str, pos5: &str, reading: &str) -> String {
        format!("{index},1,1,100,{head},{pos1},*,*,*,{pos5},*,{reading},,,,,,,\n")
    }

    fn word(reading: &str, surface: &str) -> UnidicWord {
        UnidicWord {
            reading: reading.into(),
            surface: surface.into(),
        }
    }

    #[test]
    fn non_conjugating_words_with_kanji_are_taken_with_hiragana_readings() {
        let csv = [
            HEADER.to_owned(),
            row("私", "私", "代名詞", "*", "ワタシ"),
            row("書く", "書く", "動詞", "五段-カ行", "カク"),
            row("。", "。", "補助記号", "*", "。"),
            row("あの", "あの", "連体詞", "*", "アノ"),
            row("ﾃｽﾄ", "", "名詞", "*", "テスト"),
            row("東京", "", "名詞", "*", "トウキョウ"),
            row("私", "私", "代名詞", "*", "ワタシ"),
        ]
        .concat();

        let words = plain_words(csv.as_bytes()).unwrap();

        assert_eq!(words, [word("とうきょう", "東京"), word("わたし", "私")]);
    }

    #[test]
    fn escaped_characters_are_decoded() {
        let csv = [
            HEADER.to_owned(),
            row("1\\u002c2塁", "1\\u002C2塁", "名詞", "*", "イチニルイ"),
            row(
                "km\\u002f時",
                "km\\u002F時",
                "名詞",
                "*",
                "キロメートルマイジ",
            ),
            row("漢", "\\u{6F22}", "名詞", "*", "カン"),
        ]
        .concat();

        let words = plain_words(csv.as_bytes()).unwrap();

        let surfaces: Vec<_> = words.iter().map(|w| w.surface.as_str()).collect();
        assert_eq!(surfaces, ["1,2塁", "漢", "km/時"]);
    }

    #[test]
    fn an_escape_that_is_not_a_character_stays_as_written() {
        assert_eq!(unescape("a\\uZZZZb\\u{110000}"), "a\\uZZZZb\\u{110000}");
    }

    #[test]
    fn readings_are_looked_up_by_surface() {
        let readings = UnidicReadings::new([
            word("よなご", "米子"),
            word("よねこ", "米子"),
            word("ながの", "長野"),
        ]);

        assert_eq!(readings.of("米子"), ["よなご", "よねこ"]);
        assert_eq!(readings.of("長野"), ["ながの"]);
        assert!(readings.of("深掘り").is_empty());
    }

    #[test]
    fn a_lexicon_without_the_needed_columns_is_an_error() {
        let err = plain_words("A,B\n1,2\n".as_bytes()).unwrap_err();

        assert!(matches!(err, UnidicError::MissingColumn(_)), "{err}");
    }
}

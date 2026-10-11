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
    words_where(lexicon, |row| {
        !matches!(row.pos[0].as_str(), "補助記号" | "空白")
    })
}

/// The non-conjugating words with kanji in the lexicon that are not proper
/// nouns, without duplicates, in reading and surface order.
pub fn common_words(lexicon: impl Read) -> Result<Vec<UnidicWord>, UnidicError> {
    words_where(lexicon, |row| {
        !matches!(row.pos[0].as_str(), "補助記号" | "空白") && row.pos[1] != "固有名詞"
    })
}

/// The non-conjugating words with kanji in the lexicon that are not person
/// names, without duplicates, in reading and surface order. A person name
/// reads its surface in ways a word does not (一日 as かずてる).
pub fn unnamed_words(lexicon: impl Read) -> Result<Vec<UnidicWord>, UnidicError> {
    words_where(lexicon, |row| {
        !matches!(row.pos[0].as_str(), "補助記号" | "空白") && row.pos[2] != "人名"
    })
}

/// The words with kanji the lexicon gives as counters (a suffix, or a noun
/// that can count), without duplicates, in reading and surface order. Rows of
/// cost 0, which the lexicon keeps for symbols, are left out.
pub fn counter_words(lexicon: impl Read) -> Result<Vec<UnidicWord>, UnidicError> {
    words_where(lexicon, |row| {
        row.cost > 0
            && (row.pos[0] == "接尾辞" || row.pos[1] == "助数詞" || row.pos[2] == "助数詞可能")
    })
}

/// What [`words_where`] decides a row by: its cost and its first four
/// part-of-speech fields.
struct Row {
    cost: i64,
    pos: [String; 4],
}

/// The non-conjugating words with kanji of the rows `keep` keeps.
fn words_where(
    lexicon: impl Read,
    keep: impl Fn(&Row) -> bool,
) -> Result<Vec<UnidicWord>, UnidicError> {
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
    let (index, head, cost, pos5, reading) = (
        column("IndexForm")?,
        column("Headword")?,
        column("Cost")?,
        column("POS5")?,
        column("ReadingForm")?,
    );
    let pos = [
        column("POS1")?,
        column("POS2")?,
        column("POS3")?,
        column("POS4")?,
    ];
    let mut words = BTreeSet::new();
    for record in reader.records() {
        let record = record?;
        let field = |i: usize| unescape(record.get(i).unwrap_or_default());
        let row = Row {
            cost: field(cost).parse().unwrap_or_default(),
            pos: pos.map(field),
        };
        if field(pos5) != "*" || !keep(&row) {
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
/// against, and of its counters, which a counter's sound changes start from;
/// and the readings the whole SudachiDict lexicon adds to them, which a word
/// the build joins is read by.
#[derive(Debug, Clone, Default)]
pub struct UnidicReadings {
    words: HashMap<String, Vec<String>>,
    counters: HashMap<String, Vec<String>>,
    /// UniDic's readings and the lexicon's, of the surfaces the lexicon has.
    lexicon: HashMap<String, Vec<String>>,
}

impl UnidicReadings {
    pub fn new(words: impl IntoIterator<Item = UnidicWord>) -> Self {
        Self {
            words: by_surface(words),
            ..Self::default()
        }
    }

    /// These readings with `lexicon`, the words of the whole SudachiDict
    /// lexicon (see [`unnamed_words`]).
    pub fn with_lexicon(mut self, lexicon: impl IntoIterator<Item = UnidicWord>) -> Self {
        for word in lexicon {
            let readings = self
                .lexicon
                .entry(word.surface)
                .or_insert_with_key(|surface| self.words.get(surface).cloned().unwrap_or_default());
            if !readings.contains(&word.reading) {
                readings.push(word.reading);
            }
        }
        self
    }

    /// UniDic's readings of `surface` and the lexicon's; none when neither
    /// has it.
    pub fn in_lexicon(&self, surface: impl AsRef<str>) -> &[String] {
        let surface = surface.as_ref();
        self.lexicon
            .get(surface)
            .or_else(|| self.words.get(surface))
            .map_or(&[], Vec::as_slice)
    }

    /// These readings with `counters`, UniDic's words read as counters (see
    /// [`counter_words`]).
    pub fn with_counters(self, counters: impl IntoIterator<Item = UnidicWord>) -> Self {
        Self {
            counters: by_surface(counters),
            ..self
        }
    }

    /// UniDic's readings of `surface`; none when UniDic does not have it.
    pub fn of(&self, surface: impl AsRef<str>) -> &[String] {
        self.words.get(surface.as_ref()).map_or(&[], Vec::as_slice)
    }

    /// UniDic's readings of `surface` as a counter; none when UniDic does not
    /// read it so, or these readings were made without counters.
    pub fn as_counter(&self, surface: impl AsRef<str>) -> &[String] {
        self.counters
            .get(surface.as_ref())
            .map_or(&[], Vec::as_slice)
    }
}

fn by_surface(words: impl IntoIterator<Item = UnidicWord>) -> HashMap<String, Vec<String>> {
    let mut readings: HashMap<String, Vec<String>> = HashMap::new();
    for word in words {
        readings.entry(word.surface).or_default().push(word.reading);
    }
    readings
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
    fn counters_are_the_suffixes_and_the_nouns_that_can_count_with_a_cost() {
        let csv = [
            HEADER.to_owned(),
            "杯,1,1,6894,,名詞,普通名詞,助数詞可能,*,*,*,ハイ,,,,,,,\n".to_owned(),
            "匹,1,1,10509,,接尾辞,名詞的,助数詞,*,*,*,ヒキ,,,,,,,\n".to_owned(),
            "台,1,1,11250,,名詞,普通名詞,一般,*,*,*,タイ,,,,,,,\n".to_owned(),
            "段,1,1,0,,名詞,普通名詞,助数詞可能,*,*,*,タン,,,,,,,\n".to_owned(),
        ]
        .concat();

        let words = counter_words(csv.as_bytes()).unwrap();

        assert_eq!(words, [word("はい", "杯"), word("ひき", "匹")]);
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
    fn common_words_leave_out_proper_nouns() {
        let csv = [
            HEADER.to_owned(),
            "大文字,1,1,8118,,名詞,普通名詞,一般,*,*,*,オオモジ,,,,,,,\n".to_owned(),
            "宇宙人,1,1,12481,,名詞,固有名詞,一般,*,*,*,ウチュウジン,,,,,,,\n".to_owned(),
            "深田,1,1,10000,,名詞,固有名詞,地名,一般,*,*,フケダ,,,,,,,\n".to_owned(),
        ]
        .concat();

        let words = common_words(csv.as_bytes()).unwrap();

        assert_eq!(words, [word("おおもじ", "大文字")]);
    }

    #[test]
    fn unnamed_words_leave_out_person_names() {
        let csv = [
            HEADER.to_owned(),
            "大文字,1,1,8118,,名詞,普通名詞,一般,*,*,*,オオモジ,,,,,,,\n".to_owned(),
            "大文字,-1,-1,0,,名詞,固有名詞,人名,姓,*,*,ダイモンジ,,,,,,,\n".to_owned(),
            "宇宙人,1,1,12481,,名詞,固有名詞,一般,*,*,*,ウチュウジン,,,,,,,\n".to_owned(),
            "日本海,1,1,10000,,名詞,固有名詞,地名,一般,*,*,ニホンカイ,,,,,,,\n".to_owned(),
            "書く,1,1,100,,動詞,一般,*,*,五段-カ行,*,カク,,,,,,,\n".to_owned(),
        ]
        .concat();

        let words = unnamed_words(csv.as_bytes()).unwrap();

        assert_eq!(
            words,
            [
                word("うちゅうじん", "宇宙人"),
                word("おおもじ", "大文字"),
                word("にほんかい", "日本海"),
            ]
        );
    }

    #[test]
    fn the_lexicon_gives_unidic_s_readings_and_its_own() {
        let readings = UnidicReadings::new([word("だいもんじ", "大文字"), word("ながの", "長野")])
            .with_lexicon([
                word("おおもじ", "大文字"),
                word("だいもんじ", "大文字"),
                word("おおそうじ", "大掃除"),
            ]);

        assert_eq!(readings.of("大文字"), ["だいもんじ"]);
        assert_eq!(readings.in_lexicon("大文字"), ["だいもんじ", "おおもじ"]);
        assert_eq!(readings.in_lexicon("長野"), ["ながの"]);
        assert_eq!(readings.in_lexicon("大掃除"), ["おおそうじ"]);
        assert!(readings.of("大掃除").is_empty());
    }

    #[test]
    fn without_a_lexicon_unidic_s_readings_are_the_lexicon_s() {
        let readings = UnidicReadings::new([word("ながの", "長野")]);

        assert_eq!(readings.in_lexicon("長野"), ["ながの"]);
    }

    #[test]
    fn a_lexicon_without_the_needed_columns_is_an_error() {
        let err = plain_words("A,B\n1,2\n".as_bytes()).unwrap_err();

        assert!(matches!(err, UnidicError::MissingColumn(_)), "{err}");
    }
}

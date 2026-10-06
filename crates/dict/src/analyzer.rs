//! Reading text with Sudachi: the analyzer the units are cut with.

use std::path::Path;

use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::analysis::{Mode, Tokenize};
use sudachi::config::Config;
use sudachi::dic::dictionary::JapaneseDictionary;
use sudachi::error::SudachiError;

use crate::{Corrections, UnidicReadings, katakana_to_hiragana};

/// One morpheme as the unit cutting needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub surface: String,
    /// Sudachi's reading in hiragana.
    pub reading: String,
    /// The six part-of-speech fields; the fifth is the conjugation type.
    pub pos: Vec<String>,
    pub dictionary_form: String,
    /// Characters from the start of the analyzed text.
    pub begin: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyzerError {
    #[error(transparent)]
    Config(#[from] sudachi::config::ConfigError),
    #[error(transparent)]
    Sudachi(#[from] SudachiError),
}

/// SudachiDict full, which reads the text, and SudachiDict small, which
/// checks its readings against UniDic, both loaded once and read in B mode
/// with the configuration and resources sudachi.rs embeds; and the
/// corrections applied to what they read.
pub struct Analyzer {
    dict: JapaneseDictionary,
    checker: JapaneseDictionary,
    readings: UnidicReadings,
    corrections: Corrections,
}

impl Analyzer {
    pub fn open(
        system_dictionary: impl AsRef<Path>,
        checker_dictionary: impl AsRef<Path>,
        readings: UnidicReadings,
        corrections: Corrections,
    ) -> Result<Self, AnalyzerError> {
        let open = |path: &Path| -> Result<JapaneseDictionary, AnalyzerError> {
            let config = Config::new(None, None, Some(path.to_path_buf()))?;
            Ok(JapaneseDictionary::from_cfg(&config)?)
        };
        Ok(Self {
            dict: open(system_dictionary.as_ref())?,
            checker: open(checker_dictionary.as_ref())?,
            readings,
            corrections,
        })
    }

    /// UniDic's readings the analyzer checks with.
    pub fn readings(&self) -> &UnidicReadings {
        &self.readings
    }

    /// The tokens of `text`. A non-conjugating word read otherwise than UniDic
    /// reads it takes the reading SudachiDict small gives the same word at the
    /// same place, when UniDic has that one; then the corrections apply.
    pub fn tokens(&self, text: impl AsRef<str>) -> Result<Vec<Token>, AnalyzerError> {
        let text = text.as_ref();
        let mut tokens = read(&self.dict, text)?;
        if tokens.iter().any(|t| self.misread(t)) {
            let checked = read(&self.checker, text)?;
            for token in tokens.iter_mut() {
                if !self.misread(token) {
                    continue;
                }
                let known = self.readings.of(&token.surface);
                if let Some(c) = checked.iter().find(|c| {
                    c.begin == token.begin
                        && c.surface == token.surface
                        && known.contains(&c.reading)
                }) {
                    token.reading = c.reading.clone();
                }
            }
        }
        Ok(self.corrections.apply(tokens))
    }

    fn misread(&self, token: &Token) -> bool {
        let known = self.readings.of(&token.surface);
        token.pos.get(4).is_some_and(|t| t == "*")
            && !known.is_empty()
            && !known.contains(&token.reading)
    }
}

/// Text that grows past Sudachi's input limit in its normalization
/// (㍿ becomes 株式会社) is read in halves.
fn read(dict: &JapaneseDictionary, text: &str) -> Result<Vec<Token>, AnalyzerError> {
    let tokenizer = StatelessTokenizer::new(dict);
    let morphemes = match tokenizer.tokenize(text, Mode::B, false) {
        Ok(morphemes) => morphemes,
        Err(SudachiError::InputTooLong(..)) if text.chars().nth(1).is_some() => {
            let (head, tail) = text.split_at(middle_cut(text));
            let offset = head.chars().count();
            let mut tokens = read(dict, head)?;
            tokens.extend(read(dict, tail)?.into_iter().map(|mut t| {
                t.begin += offset;
                t
            }));
            return Ok(tokens);
        }
        Err(e) => return Err(e.into()),
    };
    Ok(morphemes
        .iter()
        .map(|m| Token {
            surface: m.surface().to_string(),
            reading: katakana_to_hiragana(m.reading_form()),
            pos: m.part_of_speech().to_vec(),
            dictionary_form: m.dictionary_form().to_string(),
            begin: m.begin_c(),
        })
        .collect())
}

/// Where text may be cut without splitting a word.
pub(crate) fn is_break(c: char) -> bool {
    matches!(
        c,
        '。' | '、' | '．' | '，' | '！' | '？' | '」' | '）' | ' ' | '　' | '\t'
    )
}

/// A byte offset near the middle of `text` to cut it in two: just after the
/// break closest to the middle within its central half, else the middle.
fn middle_cut(text: &str) -> usize {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let middle = chars.len() / 2;
    let reach = chars.len() / 4;
    let after = |k: usize| chars.get(k + 1).map_or(text.len(), |(b, _)| *b);
    (0..=reach)
        .flat_map(|d| [middle.checked_sub(d), Some(middle + d)])
        .flatten()
        .find(|&k| k < chars.len() && is_break(chars[k].1))
        .map_or(chars[middle].0, after)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn text_is_cut_just_after_the_break_closest_to_the_middle() {
        let text = "あいう、えおかきくけこ";

        let (head, tail) = text.split_at(middle_cut(text));

        assert_eq!((head, tail), ("あいう、", "えおかきくけこ"));
    }

    #[test]
    fn text_without_a_break_near_the_middle_is_cut_at_the_middle() {
        let text = "、あいうえおかきくけこさし";

        let (head, tail) = text.split_at(middle_cut(text));

        assert_eq!((head, tail), ("、あいうえお", "かきくけこさし"));
    }

    fn workspace() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.join("Cargo.lock").is_file())
            .expect("the workspace root holds Cargo.lock")
    }

    /// Where the ignored tests find SudachiDict: `KANAEMI_DICT_SUDACHI_DIR`
    /// when set, else where `just sudachi` writes it under the workspace root.
    fn sudachi_dir() -> PathBuf {
        std::env::var_os("KANAEMI_DICT_SUDACHI_DIR")
            .map_or_else(|| workspace().join("build/sudachi"), PathBuf::from)
    }

    /// The analyzer as the build opens it, with the repository's corrections.
    fn analyzer() -> &'static Analyzer {
        static ANALYZER: std::sync::OnceLock<Analyzer> = std::sync::OnceLock::new();
        ANALYZER.get_or_init(|| {
            let dir = sudachi_dir();
            let lexicon = std::fs::File::open(dir.join("small_lex.csv")).unwrap();
            let readings = UnidicReadings::new(crate::plain_words(lexicon).unwrap());
            let corrections = crate::parse_corrections(
                std::fs::read_to_string(workspace().join("analyzer/corrections.tsv")).unwrap(),
            )
            .unwrap();
            Analyzer::open(
                dir.join("system_full.dic"),
                dir.join("system_small.dic"),
                readings,
                crate::Corrections::new(corrections),
            )
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        })
    }

    fn view(tokens: &[Token]) -> Vec<(&str, &str, &str, &str)> {
        tokens
            .iter()
            .map(|t| {
                (
                    t.surface.as_str(),
                    t.reading.as_str(),
                    t.pos[0].as_str(),
                    t.dictionary_form.as_str(),
                )
            })
            .collect()
    }

    #[test]
    #[ignore = "needs SudachiDict full"]
    fn text_that_grows_too_long_in_normalization_is_read_in_halves() {
        let text = "㍿".repeat(10_000);

        let tokens = analyzer().tokens(&text).unwrap();

        let surface: String = tokens.iter().map(|t| t.surface.as_str()).collect();
        assert_eq!(surface, text);
        let last = tokens.last().unwrap();
        assert_eq!(last.begin + last.surface.chars().count(), 10_000);
    }

    #[test]
    #[ignore = "needs SudachiDict full"]
    fn halves_are_cut_at_a_break_near_the_middle() {
        let text = format!("{}東京都{}", "㍿、".repeat(3000), "、㍿".repeat(3000));

        let tokens = analyzer().tokens(&text).unwrap();

        assert!(tokens.iter().any(|t| t.surface == "東京都"));
    }

    #[test]
    #[ignore = "needs SudachiDict full"]
    fn text_is_read_in_b_mode_with_hiragana_readings() {
        let tokens = analyzer().tokens("書いた手紙を国土交通省に送った").unwrap();

        assert_eq!(
            view(&tokens),
            [
                ("書い", "かい", "動詞", "書く"),
                ("た", "た", "助動詞", "た"),
                ("手紙", "てがみ", "名詞", "手紙"),
                ("を", "を", "助詞", "を"),
                ("国土", "こくど", "名詞", "国土"),
                ("交通省", "こうつうしょう", "名詞", "交通省"),
                ("に", "に", "助詞", "に"),
                ("送っ", "おくっ", "動詞", "送る"),
                ("た", "た", "助動詞", "た"),
            ]
        );
        assert_eq!(tokens[0].pos[4], "五段-カ行");
    }

    fn readings_of<'a>(tokens: &'a [Token], surface: &str) -> Vec<&'a str> {
        tokens
            .iter()
            .filter(|t| t.surface == surface)
            .map(|t| t.reading.as_str())
            .collect()
    }

    #[test]
    #[ignore = "needs SudachiDict full and small"]
    fn a_word_the_corrections_add_is_read_as_one_word() {
        let tokens = analyzer()
            .tokens("王騎の魅力を深掘りしています。深掘りする。深掘が進行した")
            .unwrap();

        assert_eq!(readings_of(&tokens, "深掘り"), ["ふかぼり", "ふかぼり"]);
        assert_eq!(readings_of(&tokens, "深掘"), ["ふかぼり"]);
    }

    #[test]
    #[ignore = "needs SudachiDict full and small"]
    fn every_correction_is_read_as_one_word_in_a_sentence() {
        let corrections = crate::parse_corrections(
            std::fs::read_to_string(workspace().join("analyzer/corrections.tsv")).unwrap(),
        )
        .unwrap();

        let misread: Vec<_> = corrections
            .iter()
            .filter_map(|c| {
                let reading = c.reading.as_deref()?;
                let tokens = analyzer()
                    .tokens(format!("その{}を見た", c.surface))
                    .unwrap();
                (readings_of(&tokens, &c.surface) != [reading]).then(|| {
                    let read: Vec<_> = view(&tokens).into_iter().map(|t| (t.0, t.1)).collect();
                    format!("{}: {read:?}", c.surface)
                })
            })
            .collect();
        assert!(misread.is_empty(), "{misread:#?}");
    }

    #[test]
    #[ignore = "needs SudachiDict full and small"]
    fn a_reading_unidic_does_not_have_takes_the_one_sudachidict_small_gives() {
        let tokens = analyzer().tokens("米子市から長野に行く").unwrap();

        assert_eq!(readings_of(&tokens, "米子"), ["よなご"]);
        assert_eq!(readings_of(&tokens, "長野"), ["ながの"]);
    }
}

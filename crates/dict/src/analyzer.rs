//! Reading text with Sudachi: the analyzer the units are cut with.

use std::path::Path;

use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::analysis::{Mode, Tokenize};
use sudachi::config::Config;
use sudachi::dic::dictionary::JapaneseDictionary;
use sudachi::error::SudachiError;

use crate::katakana_to_hiragana;

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

/// SudachiDict's system dictionary loaded once, read in B mode with the
/// configuration and resources sudachi.rs embeds.
pub struct Analyzer {
    dict: JapaneseDictionary,
}

impl Analyzer {
    pub fn open(system_dictionary: impl AsRef<Path>) -> Result<Self, AnalyzerError> {
        let config = Config::new(None, None, Some(system_dictionary.as_ref().to_path_buf()))?;
        Ok(Self {
            dict: JapaneseDictionary::from_cfg(&config)?,
        })
    }

    /// Text that grows past Sudachi's input limit in its normalization
    /// (㍿ becomes 株式会社) is read in halves.
    pub fn tokens(&self, text: impl AsRef<str>) -> Result<Vec<Token>, AnalyzerError> {
        let text = text.as_ref();
        let tokenizer = StatelessTokenizer::new(&self.dict);
        let morphemes = match tokenizer.tokenize(text, Mode::B, false) {
            Ok(morphemes) => morphemes,
            Err(SudachiError::InputTooLong(..)) if text.chars().nth(1).is_some() => {
                let (head, tail) = text.split_at(middle_cut(text));
                let offset = head.chars().count();
                let mut tokens = self.tokens(head)?;
                tokens.extend(self.tokens(tail)?.into_iter().map(|mut t| {
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

    /// The system dictionary the ignored tests read: `KANAEMI_DICT_SUDACHI_DIC`
    /// when set, else what `just sudachi` writes under the workspace root.
    fn system_dictionary() -> PathBuf {
        if let Some(path) = std::env::var_os("KANAEMI_DICT_SUDACHI_DIC") {
            return path.into();
        }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.join("Cargo.lock").is_file())
            .expect("the workspace root holds Cargo.lock");
        workspace.join("build/sudachi/system_full.dic")
    }

    fn analyzer() -> Analyzer {
        let dic = system_dictionary();
        Analyzer::open(&dic).unwrap_or_else(|e| panic!("{}: {e}", dic.display()))
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
}

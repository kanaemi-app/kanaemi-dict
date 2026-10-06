//! The words that correct what the analyzer reads wrong, as
//! `analyzer/corrections.tsv` writes them, applied to the analyzer's tokens,
//! and the surfaces dropped from the dictionaries.

use std::collections::{HashMap, HashSet};

use crate::Token;
use crate::kana::is_reading_kana;

/// The part of speech of tokens a correction joins into one word.
const JOINED_POS: [&str; 6] = ["名詞", "普通名詞", "一般", "*", "*", "*"];
/// The reading that marks a surface to drop.
const DROPPED: &str = "-";

/// A word and the reading the analyzer is to give it, or a surface that is
/// no word and is dropped from the dictionaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    pub surface: String,
    /// In hiragana; None for a surface to drop.
    pub reading: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct CorrectionsError {
    pub line: usize,
    pub reason: String,
}

/// The corrections of `text`, a surface and a reading, or `-` to drop the
/// surface, per tab-separated line; lines starting with `#` and blank lines
/// are skipped.
pub fn parse_corrections(text: impl AsRef<str>) -> Result<Vec<Correction>, CorrectionsError> {
    text.as_ref()
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(i, line)| {
            parse_line(line).map_err(|reason| CorrectionsError {
                line: i + 1,
                reason,
            })
        })
        .collect()
}

fn parse_line(line: &str) -> Result<Correction, String> {
    let [surface, reading] = line.split('\t').collect::<Vec<_>>()[..] else {
        return Err(format!("{line:?} does not have two tab-separated fields"));
    };
    if surface.is_empty() {
        return Err("the surface is empty".into());
    }
    let reading = match reading {
        DROPPED => None,
        _ if !reading.is_empty() && reading.chars().all(is_reading_kana) => {
            Some(reading.to_owned())
        }
        _ => {
            return Err(format!(
                "the reading {reading:?} is neither hiragana nor {DROPPED:?}"
            ));
        }
    };
    Ok(Correction {
        surface: surface.to_owned(),
        reading,
    })
}

/// The corrections, looked up by surface.
#[derive(Debug, Clone, Default)]
pub struct Corrections {
    readings: HashMap<String, String>,
    dropped: HashSet<String>,
    /// The longest surface in characters, past which no run of tokens can match.
    longest: usize,
}

impl Corrections {
    pub fn new(corrections: impl IntoIterator<Item = Correction>) -> Self {
        let mut readings = HashMap::new();
        let mut dropped = HashSet::new();
        for c in corrections {
            match c.reading {
                Some(reading) => {
                    readings.insert(c.surface, reading);
                }
                None => {
                    dropped.insert(c.surface);
                }
            }
        }
        let longest = readings
            .keys()
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0);
        Self {
            readings,
            dropped,
            longest,
        }
    }

    /// The reading a correction gives `surface`, if one does.
    pub fn reading_of(&self, surface: impl AsRef<str>) -> Option<&str> {
        self.readings.get(surface.as_ref()).map(String::as_str)
    }

    /// Whether `surface` is no word and stays out of the dictionaries.
    pub fn drops(&self, surface: impl AsRef<str>) -> bool {
        self.dropped.contains(surface.as_ref())
    }

    /// `tokens` with every run whose surfaces join into a correction's surface
    /// made one token read as the correction says: the longest run first, from
    /// the front. A run of one keeps its part of speech and dictionary form,
    /// so a conjugated form still finds its stem; a longer one becomes a
    /// common noun. No run starts inside a number the analyzer cut in two
    /// (三 of 二十＋三＋分 stays apart, and the number stays whole).
    pub fn apply(&self, tokens: Vec<Token>) -> Vec<Token> {
        if self.readings.is_empty() {
            return tokens;
        }
        let is_numeral = |t: &Token| t.pos.get(1).is_some_and(|p| p == "数詞");
        let mut out = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let inside_number = i > 0 && is_numeral(&tokens[i - 1]) && is_numeral(&tokens[i]);
            let matched = (!inside_number)
                .then(|| self.longest_match(&tokens[i..]))
                .flatten();
            let Some((end, reading)) = matched else {
                out.push(tokens[i].clone());
                i += 1;
                continue;
            };
            let token = match &tokens[i..i + end] {
                [one] => Token {
                    reading: reading.to_owned(),
                    ..one.clone()
                },
                run => {
                    let surface: String = run.iter().map(|t| t.surface.as_str()).collect();
                    Token {
                        dictionary_form: surface.clone(),
                        surface,
                        reading: reading.to_owned(),
                        pos: JOINED_POS.map(str::to_owned).to_vec(),
                        begin: run[0].begin,
                    }
                }
            };
            out.push(token);
            i += end;
        }
        out
    }

    /// How many tokens from the head of `tokens` the longest matching
    /// correction takes, and its reading.
    fn longest_match(&self, tokens: &[Token]) -> Option<(usize, &str)> {
        let mut surface = String::new();
        let mut chars = 0;
        let mut found = None;
        for (n, token) in tokens.iter().enumerate() {
            surface.push_str(&token.surface);
            chars += token.surface.chars().count();
            if chars > self.longest {
                break;
            }
            if let Some(reading) = self.readings.get(&surface) {
                found = Some((n + 1, reading.as_str()));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_is_a_word_and_comments_and_blank_lines_are_skipped() {
        let text = "# 表記\t読み\n\n深掘り\tふかぼり\n";

        assert_eq!(
            parse_corrections(text).unwrap(),
            [Correction {
                surface: "深掘り".into(),
                reading: Some("ふかぼり".into()),
            }]
        );
    }

    #[test]
    fn a_line_read_as_a_dash_drops_its_surface_and_corrects_nothing() {
        let parsed = parse_corrections("十つ\t-\n深掘り\tふかぼり\n").unwrap();
        assert_eq!(parsed[0].reading, None);

        let corrections = Corrections::new(parsed);

        assert!(corrections.drops("十つ"));
        assert!(!corrections.drops("深掘り"));
        let tokens = vec![
            tok("十", "じゅう", "名詞,数詞,*,*,*,*", 0),
            tok("つ", "つ", "接尾辞,名詞的,助数詞,*,*,*", 1),
        ];
        assert_eq!(corrections.apply(tokens.clone()), tokens);
    }

    #[test]
    fn a_bad_line_is_an_error_naming_its_line() {
        let cases = [
            "深掘り",
            "深掘り\tフカボリ",
            "深掘り\t",
            "深掘り\tふかぼり\t名詞",
            "\tふかぼり",
        ];

        for case in cases {
            let err = parse_corrections(format!("# head\n{case}\n")).unwrap_err();
            assert_eq!(err.line, 2, "{case}: {err}");
        }
    }

    fn tok(surface: &str, reading: &str, pos: &str, begin: usize) -> Token {
        Token {
            surface: surface.into(),
            reading: reading.into(),
            pos: pos.split(',').map(str::to_owned).collect(),
            dictionary_form: surface.into(),
            begin,
        }
    }

    fn corrections(pairs: &[(&str, &str)]) -> Corrections {
        Corrections::new(pairs.iter().map(|(surface, reading)| Correction {
            surface: (*surface).into(),
            reading: Some((*reading).into()),
        }))
    }

    fn view(tokens: &[Token]) -> Vec<(&str, &str, &str, usize)> {
        tokens
            .iter()
            .map(|t| {
                (
                    t.surface.as_str(),
                    t.reading.as_str(),
                    t.pos[0].as_str(),
                    t.begin,
                )
            })
            .collect()
    }

    const PARTICLE: &str = "助詞,格助詞,*,*,*,*";

    #[test]
    fn tokens_whose_surfaces_join_into_a_correction_become_one_noun() {
        let tokens = vec![
            tok("深", "こし", "形容詞,一般,*,*,文語形容詞-ク,連体形-一般", 0),
            tok("掘り", "ほり", "動詞,一般,*,*,五段-ラ行,連用形-一般", 1),
            tok("を", "を", PARTICLE, 3),
        ];

        let tokens = corrections(&[("深掘り", "ふかぼり")]).apply(tokens);

        assert_eq!(
            view(&tokens),
            [("深掘り", "ふかぼり", "名詞", 0), ("を", "を", "助詞", 3)]
        );
        assert_eq!(tokens[0].pos, JOINED_POS);
    }

    #[test]
    fn a_token_whose_surface_is_a_correction_takes_its_reading_and_keeps_its_part_of_speech() {
        let tokens = vec![tok("一回", "いちかい", "名詞,普通名詞,副詞可能,*,*,*", 0)];

        let tokens = corrections(&[("一回", "いっかい")]).apply(tokens);

        assert_eq!(view(&tokens), [("一回", "いっかい", "名詞", 0)]);
        assert_eq!(tokens[0].pos[2], "副詞可能");
    }

    #[test]
    fn a_conjugated_token_a_correction_reads_keeps_its_dictionary_form() {
        let mut token = tok(
            "大きく",
            "だいきく",
            "形容詞,一般,*,*,形容詞,連用形-一般",
            0,
        );
        token.dictionary_form = "大きい".into();

        let tokens = corrections(&[("大きく", "おおきく")]).apply(vec![token]);

        assert_eq!(tokens[0].reading, "おおきく");
        assert_eq!(tokens[0].dictionary_form, "大きい");
        assert_eq!(tokens[0].pos[4], "形容詞");
    }

    #[test]
    fn the_longest_correction_wins_from_the_front() {
        let tokens = vec![
            tok("九", "きゅう", "名詞,数詞,*,*,*,*", 0),
            tok("時間", "じかん", "名詞,普通名詞,一般,*,*,*", 1),
        ];

        let tokens = corrections(&[("九時", "くじ"), ("九時間", "くじかん")]).apply(tokens);

        assert_eq!(view(&tokens), [("九時間", "くじかん", "名詞", 0)]);
    }

    #[test]
    fn a_correction_does_not_start_inside_a_number() {
        let numeral = "名詞,数詞,*,*,*,*";
        let tokens = vec![
            tok("二十", "にじゅう", numeral, 0),
            tok("三", "さん", numeral, 2),
            tok("分", "ふん", "接尾辞,名詞的,助数詞,*,*,*", 3),
        ];

        let applied = corrections(&[("三分", "さんぷん")]).apply(tokens.clone());

        assert_eq!(applied, tokens);
    }

    #[test]
    fn a_correction_ending_inside_a_token_does_not_match() {
        let tokens = vec![
            tok("九", "きゅう", "名詞,数詞,*,*,*,*", 0),
            tok("時間", "じかん", "名詞,普通名詞,一般,*,*,*", 1),
        ];

        let tokens = corrections(&[("九時", "くじ")]).apply(tokens);

        assert_eq!(
            view(&tokens),
            [("九", "きゅう", "名詞", 0), ("時間", "じかん", "名詞", 1)]
        );
    }
}

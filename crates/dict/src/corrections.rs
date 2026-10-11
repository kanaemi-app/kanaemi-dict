//! The words that correct what the analyzer reads wrong, as
//! `analyzer/corrections.tsv` writes them, applied to the analyzer's tokens,
//! and the surfaces dropped from the dictionaries.

use std::collections::{HashMap, HashSet};

use aho_corasick::AhoCorasick;

use crate::kana::is_reading_kana;
use crate::{Token, Unit};

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

    /// Whether a correction reads `surface` other than as `reading`.
    pub fn contradicts(&self, reading: impl AsRef<str>, surface: impl AsRef<str>) -> bool {
        self.reading_of(surface)
            .is_some_and(|corrected| corrected != reading.as_ref())
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
                        normalized_form: surface.clone(),
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

/// What a correction does to the text: the places where the text holds its
/// surface (for a surface to drop, where the units or a list hold it), and of
/// those the places it changes, where the units over the surface read with
/// every correction but this one are not the ones read with all of them. A
/// correction changes the units around its word too: 数百 joined into one
/// word keeps 数百人 from being cut as a number. Leaving out one at a time
/// keeps a correction a longer one overlaps (十日 in 二十日) from taking the
/// longer one's change.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionUse {
    pub places: usize,
    pub changed: usize,
}

impl CorrectionUse {
    /// Whether taking the correction out would change what the build makes.
    pub fn is_needed(&self) -> bool {
        self.changed > 0
    }
}

/// What each correction does to its surface read alone, as the small counted
/// words are read, and to every line of `texts` holding a correction's
/// surface. `cut(text, left_out)` cuts a text into units with every
/// correction, or with all but the one at index `left_out`. A surface to drop
/// is also needed where `listed` names it: the dictionaries take titles and
/// names whole.
pub fn correction_uses<E>(
    corrections: &[Correction],
    texts: impl IntoIterator<Item = impl AsRef<str>>,
    listed: impl Fn(&str) -> bool,
    mut cut: impl FnMut(&str, Option<usize>) -> Result<Vec<Unit>, E>,
) -> Result<Vec<CorrectionUse>, E> {
    let mut uses = vec![CorrectionUse::default(); corrections.len()];
    for (u, c) in uses.iter_mut().zip(corrections) {
        if c.reading.is_none() && listed(&c.surface) {
            u.places += 1;
            u.changed += 1;
        }
    }
    let finder = AhoCorasick::new(corrections.iter().map(|c| &c.surface))
        .expect("a few hundred short surfaces fit the matcher's limits");
    let mut tally_text = |text: &str, uses: &mut [CorrectionUse]| -> Result<(), E> {
        let with = cut(text, None)?;
        let mut without: HashMap<usize, Vec<Unit>> = HashMap::new();
        for found in finder.find_overlapping_iter(text) {
            let i = found.pattern().as_usize();
            if corrections[i].reading.is_none() {
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = without.entry(i) {
                e.insert(cut(text, Some(i))?);
            }
            let start = text[..found.start()].chars().count();
            let end = start + corrections[i].surface.chars().count();
            let over = |units: &[Unit]| -> Vec<Unit> {
                units
                    .iter()
                    .filter(|x| x.position < end && x.position + x.surface.chars().count() > start)
                    .cloned()
                    .collect()
            };
            uses[i].places += 1;
            if over(&with) != over(&without[&i]) {
                uses[i].changed += 1;
            }
        }
        // A surface to drop never changes the units, so the units read with
        // every correction hold it where it would enter.
        for (u, c) in uses.iter_mut().zip(corrections) {
            if c.reading.is_none() {
                let n = with.iter().filter(|x| x.surface == c.surface).count();
                u.places += n;
                u.changed += n;
            }
        }
        Ok(())
    };
    for c in corrections {
        tally_text(&c.surface, &mut uses)?;
    }
    for text in texts {
        for line in text.as_ref().split('\n') {
            if finder.is_match(line) {
                tally_text(line, &mut uses)?;
            }
        }
    }
    Ok(uses)
}

/// The uses of `corrections`, one line each in their order, as TSV.
pub fn correction_uses_tsv(corrections: &[Correction], uses: &[CorrectionUse]) -> String {
    let mut out = String::from("surface\treading\tplaces\tchanged\tneeded\n");
    for (c, u) in corrections.iter().zip(uses) {
        let reading = c.reading.as_deref().unwrap_or(DROPPED);
        let needed = if u.is_needed() { "yes" } else { "no" };
        out.push_str(&format!(
            "{}\t{reading}\t{}\t{}\t{needed}\n",
            c.surface, u.places, u.changed
        ));
    }
    out
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
    fn a_reading_other_than_the_correction_s_is_contradicted() {
        let corrections = Corrections::new(parse_corrections("一回\tいっかい\n").unwrap());

        assert!(corrections.contradicts("いちかい", "一回"));
        assert!(!corrections.contradicts("いっかい", "一回"));
        assert!(!corrections.contradicts("にかい", "二回"));
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

    fn unit(position: usize, surface: &str, reading: &str) -> Unit {
        Unit {
            doc_id: String::new(),
            position,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
            compound: false,
            okurigana_variant: false,
        }
    }

    /// Cuts a text as `table` says, with the corrections or without them.
    fn cut_by<'a>(
        table: &'a [(&str, Vec<Unit>, Vec<Unit>)],
        calls: &'a std::cell::RefCell<Vec<String>>,
    ) -> impl FnMut(&str, Option<usize>) -> Result<Vec<Unit>, ()> + 'a {
        move |text, left_out| {
            calls.borrow_mut().push(text.to_owned());
            let (_, with, without) = table
                .iter()
                .find(|(t, _, _)| *t == text)
                .unwrap_or_else(|| panic!("{text:?} is not in the table"));
            Ok(if left_out.is_none() { with } else { without }.clone())
        }
    }

    #[test]
    fn a_correction_a_longer_one_overlaps_is_judged_by_leaving_it_out_alone() {
        let all = vec![unit(0, "二十日", "はつか")];
        let cut = |text: &str, left_out: Option<usize>| -> Result<Vec<Unit>, ()> {
            Ok(match (text, left_out) {
                ("二十日", Some(0)) => vec![unit(0, "二", "に"), unit(1, "十日", "とおか")],
                ("二十日", _) => all.clone(),
                ("十日", _) => vec![unit(0, "十日", "とおか")],
                _ => panic!("{text:?} is not cut"),
            })
        };

        let uses = correction_uses(
            &parsed("二十日\tはつか\n十日\tとおか\n"),
            Vec::<String>::new(),
            |_| false,
            cut,
        )
        .unwrap();

        assert!(uses[0].is_needed());
        assert!(!uses[1].is_needed(), "{:?}", uses[1]);
    }

    fn parsed(text: &str) -> Vec<Correction> {
        parse_corrections(text).unwrap()
    }

    #[test]
    fn a_correction_that_changes_the_units_somewhere_is_needed() {
        let table = [
            (
                "深掘り",
                vec![unit(0, "深掘り", "ふかぼり")],
                vec![unit(0, "深掘り", "ふかぼり")],
            ),
            (
                "問題を深掘りした",
                vec![unit(0, "問題", "もんだい"), unit(3, "深掘り", "ふかぼり")],
                vec![unit(0, "問題", "もんだい"), unit(3, "深掘り", "しんくつり")],
            ),
        ];
        let calls = std::cell::RefCell::new(Vec::new());

        let uses = correction_uses(
            &parsed("深掘り\tふかぼり\n"),
            ["問題を深掘りした"],
            |_| false,
            cut_by(&table, &calls),
        )
        .unwrap();

        assert_eq!(
            uses,
            [CorrectionUse {
                places: 2,
                changed: 1
            }]
        );
        assert!(uses[0].is_needed());
    }

    #[test]
    fn a_correction_that_changes_the_units_over_its_surface_is_needed_without_its_own_word() {
        let table = [
            ("数百", vec![], vec![]),
            (
                "数百人",
                vec![unit(0, "数百人", "すうひゃくにん")],
                vec![unit(0, "数百", "すうひゃく"), unit(2, "人", "にん")],
            ),
            (
                "人数百",
                vec![unit(0, "人数", "にんずう"), unit(2, "百", "ひゃく")],
                vec![unit(0, "人数", "にんずう"), unit(2, "百", "ひゃく")],
            ),
        ];
        let calls = std::cell::RefCell::new(Vec::new());

        let uses = correction_uses(
            &parsed("数百\tすうひゃく\n"),
            ["数百人", "人数百"],
            |_| false,
            cut_by(&table, &calls),
        )
        .unwrap();

        assert_eq!(
            uses,
            [CorrectionUse {
                places: 3,
                changed: 1
            }]
        );
    }

    #[test]
    fn a_correction_the_analyzer_already_reads_so_everywhere_is_not_needed() {
        let same = vec![unit(0, "一回", "いっかい")];
        let table = [
            ("一回", same.clone(), same.clone()),
            (
                "もう一回",
                vec![unit(2, "一回", "いっかい")],
                vec![unit(2, "一回", "いっかい")],
            ),
        ];
        let calls = std::cell::RefCell::new(Vec::new());

        let uses = correction_uses(
            &parsed("一回\tいっかい\n"),
            ["もう一回"],
            |_| false,
            cut_by(&table, &calls),
        )
        .unwrap();

        assert_eq!(
            uses,
            [CorrectionUse {
                places: 2,
                changed: 0
            }]
        );
        assert!(!uses[0].is_needed());
    }

    #[test]
    fn only_the_lines_that_hold_a_correction_s_surface_are_cut() {
        let table = [("一回", vec![], vec![]), ("もう一回", vec![], vec![])];
        let calls = std::cell::RefCell::new(Vec::new());

        correction_uses(
            &parsed("一回\tいっかい\n"),
            ["今日は晴れ\nもう一回"],
            |_| false,
            cut_by(&table, &calls),
        )
        .unwrap();

        assert_eq!(calls.into_inner(), ["一回", "一回", "もう一回", "もう一回"]);
    }

    #[test]
    fn a_dropped_surface_is_needed_where_a_unit_or_a_listed_name_holds_it() {
        let table = [
            ("研究所大", vec![], vec![]),
            ("十つ", vec![], vec![]),
            ("二形", vec![], vec![]),
            (
                "研究所大の前",
                vec![unit(0, "研究所大", "けんきゅうじょだい")],
                vec![unit(0, "研究所大", "けんきゅうじょだい")],
            ),
        ];
        let calls = std::cell::RefCell::new(Vec::new());

        let uses = correction_uses(
            &parsed("研究所大\t-\n十つ\t-\n二形\t-\n"),
            ["研究所大の前"],
            |surface| surface == "十つ",
            cut_by(&table, &calls),
        )
        .unwrap();

        assert_eq!(
            uses.iter()
                .map(CorrectionUse::is_needed)
                .collect::<Vec<_>>(),
            [true, true, false]
        );
    }

    #[test]
    fn the_uses_are_written_with_the_corrections_in_their_order() {
        let tsv = correction_uses_tsv(
            &parsed("深掘り\tふかぼり\n十つ\t-\n"),
            &[
                CorrectionUse {
                    places: 2,
                    changed: 1,
                },
                CorrectionUse::default(),
            ],
        );

        assert_eq!(
            tsv,
            "surface\treading\tplaces\tchanged\tneeded\n\
             深掘り\tふかぼり\t2\t1\tyes\n\
             十つ\t-\t0\t0\tno\n"
        );
    }

    fn tok(surface: &str, reading: &str, pos: &str, begin: usize) -> Token {
        Token {
            surface: surface.into(),
            reading: reading.into(),
            pos: pos.split(',').map(str::to_owned).collect(),
            dictionary_form: surface.into(),
            normalized_form: surface.into(),
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

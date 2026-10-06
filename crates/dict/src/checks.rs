//! Checks of the built dictionaries whose answers come from outside the
//! analyzer: a word set people wrote, a sample of items for people to judge,
//! and the readings of another analyzer.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::BufRead;
use std::sync::Arc;

use kanaemi_engine::{RankingModel, TextDictionary, terminal_ending};
use xxhash_rust::xxh3::xxh3_64;

use crate::dictionary::UNIDIC_UNSEEN;
use crate::evaluation::{Field, Score, engine, offered};
use crate::kana::{hiragana_to_katakana, is_kanji};

/// One word of the word set: what is typed, after which text, and the
/// surfaces any of which is right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordCase {
    pub category: String,
    pub context: String,
    pub reading: String,
    /// The first kana of the okurigana, when the word is typed with it marked.
    pub okurigana: Option<String>,
    pub accepted: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WordCasesError {
    #[error("line {0}: not four tab-separated columns")]
    Columns(usize),
    #[error("line {0}: no category, reading or surface")]
    Empty(usize),
    #[error("line {0}: nothing after the `*` that marks the okurigana")]
    Okurigana(usize),
}

/// The word set of `evaluation/words.tsv`: category, the text before the
/// word, the reading with a `*` before the okurigana's first kana, and the
/// right surfaces separated by `|`. `#` lines and empty lines are skipped.
pub fn parse_word_cases(text: impl AsRef<str>) -> Result<Vec<WordCase>, WordCasesError> {
    let mut cases = Vec::new();
    for (i, line) in text.as_ref().lines().enumerate() {
        let number = i + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let [category, context, reading, accepted] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            return Err(WordCasesError::Columns(number));
        };
        let accepted: Vec<String> = accepted
            .split('|')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if category.is_empty() || reading.is_empty() || accepted.is_empty() {
            return Err(WordCasesError::Empty(number));
        }
        let (reading, okurigana) = match reading.split_once('*') {
            Some((_, "")) => return Err(WordCasesError::Okurigana(number)),
            Some((before, after)) => {
                let first: String = after.chars().take(1).collect();
                (format!("{before}{after}"), Some(first))
            }
            None => (reading.to_owned(), None),
        };
        cases.push(WordCase {
            category: category.into(),
            context: context.into(),
            reading,
            okurigana,
            accepted,
        });
    }
    Ok(cases)
}

/// How one word of the word set came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordResult {
    /// The 1-based rank of the first right surface offered, if any is.
    pub rank: Option<usize>,
    /// The surface offered first.
    pub top: String,
}

/// Converts `case` in a field that has learned nothing but its context.
pub fn check_word(field: &mut impl Field, case: &WordCase) -> WordResult {
    field.start_over();
    if !case.context.is_empty() {
        field.type_text(&case.context);
    }
    let surfaces = offered(field, &case.reading, case.okurigana.as_deref());
    WordResult {
        rank: surfaces
            .iter()
            .position(|s| case.accepted.contains(s))
            .map(|i| i + 1),
        top: surfaces.into_iter().next().unwrap_or_default(),
    }
}

/// Converts every case with an engine reading `dictionary` alone, ranking
/// with `model` when given.
pub fn check_words(
    cases: &[WordCase],
    dictionary: &Arc<TextDictionary>,
    model: Option<&Arc<RankingModel>>,
) -> Vec<WordResult> {
    let mut engine = engine(dictionary.clone());
    engine.set_model(model.cloned());
    cases.iter().map(|c| check_word(&mut engine, c)).collect()
}

/// The table of `build/check-words.tsv`, one row per category and model
/// (`off`, `on`), categories in the order of their UTF-8 bytes and `all`
/// last; `runs` pairs each model label with the results of every case.
pub fn word_scores_tsv(cases: &[WordCase], runs: &[(&str, &[WordResult])]) -> String {
    let mut tsv = String::from("category\tmodel\twords\tcovered\tfirst\tmean_rank\n");
    for (label, results) in runs {
        let mut by_category: BTreeMap<&str, Score> = BTreeMap::new();
        let mut all = Score::default();
        for (case, result) in cases.iter().zip(results.iter()) {
            by_category
                .entry(&case.category)
                .or_default()
                .add(result.rank);
            all.add(result.rank);
        }
        for (category, score) in by_category.iter().chain([(&"all", &all)]) {
            writeln!(
                tsv,
                "{category}\t{label}\t{}\t{}\t{}\t{:.3}",
                score.units,
                score.covered,
                score.first,
                score.mean_rank()
            )
            .expect("writing to a String never fails");
        }
    }
    tsv
}

/// The table of `build/check-words-misses.tsv`: every case whose right
/// surface did not come first, with the surface that did and the right one's
/// rank (empty when not offered).
pub fn word_misses_tsv(cases: &[WordCase], runs: &[(&str, &[WordResult])]) -> String {
    let mut tsv = String::from("category\tmodel\tcontext\treading\taccepted\ttop\trank\n");
    for (label, results) in runs {
        for (case, result) in cases.iter().zip(results.iter()) {
            if result.rank == Some(1) {
                continue;
            }
            writeln!(
                tsv,
                "{}\t{label}\t{}\t{}\t{}\t{}\t{}",
                case.category,
                case.context,
                case.reading,
                case.accepted.join("|"),
                result.top,
                result.rank.map(|r| r.to_string()).unwrap_or_default(),
            )
            .expect("writing to a String never fails");
        }
    }
    tsv
}

/// An item line of a built dictionary, its fields as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryLine<'a> {
    pub reading: &'a str,
    pub surface: &'a str,
    pub conjugation: &'a str,
    pub cost: Option<u32>,
}

/// The item lines of a built dictionary, skipping `#` lines and empty ones.
pub fn dictionary_lines(text: &str) -> impl Iterator<Item = (&str, DictionaryLine<'_>)> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let reading = fields.next()?;
            let surface = fields.next()?;
            let conjugation = fields.next().unwrap_or_default();
            let cost = fields.next().and_then(|c| c.parse().ok());
            Some((
                line,
                DictionaryLine {
                    reading,
                    surface,
                    conjugation,
                    cost,
                },
            ))
        })
}

/// The base dictionary's items from text whose cost is below this are the
/// common ones; the rest, up to the cost of the UniDic words text never
/// shows, are the rare ones.
const COMMON_COST: u32 = 1500;

/// The stratum `line` of the dictionary `name` is sampled from. Every item of
/// an additional dictionary is in the stratum of its dictionary.
pub fn stratum_of(name: &str, line: &DictionaryLine) -> String {
    if name != "base" {
        return name.to_owned();
    }
    let stratum = if !line.conjugation.is_empty() {
        "conjugating"
    } else if line.reading.contains('{') {
        "numeric"
    } else if line.cost.is_some_and(|c| c >= UNIDIC_UNSEEN) {
        "unidic-only"
    } else if line.cost.is_some_and(|c| c < COMMON_COST) {
        "common"
    } else {
        "rare"
    };
    format!("base:{stratum}")
}

/// Up to `per_base` items of each stratum of the base dictionary and
/// `per_additional` of each additional dictionary, as the table of
/// `build/check-sample.tsv` with the columns people fill left empty. The
/// items are taken in the order of their lines' hashes, so the same
/// dictionaries give the same sample.
pub fn sample_tsv(dictionaries: &[(&str, &str)], per_base: usize, per_additional: usize) -> String {
    let mut strata: BTreeMap<String, Vec<(u64, &str, DictionaryLine)>> = BTreeMap::new();
    for (name, text) in dictionaries {
        for (raw, line) in dictionary_lines(text) {
            strata.entry(stratum_of(name, &line)).or_default().push((
                xxh3_64(raw.as_bytes()),
                name,
                line,
            ));
        }
    }
    let mut tsv =
        String::from("stratum\tdictionary\treading\tsurface\tconjugation\tcost\tverdict\tnote\n");
    for (stratum, mut items) in strata {
        items.sort_by_key(|(hash, ..)| *hash);
        let take = if stratum.starts_with("base:") {
            per_base
        } else {
            per_additional
        };
        for (_, name, line) in items.into_iter().take(take) {
            writeln!(
                tsv,
                "{stratum}\t{name}\t{}\t{}\t{}\t{}\t\t",
                line.reading,
                line.surface,
                line.conjugation,
                line.cost.map(|c| c.to_string()).unwrap_or_default(),
            )
            .expect("writing to a String never fails");
        }
    }
    tsv
}

/// The surface and reading of the form another analyzer reads for `line`:
/// a conjugating word's terminal form, any other word as written. `None`
/// for an item without kanji, a numeric item, an okurigana line (it repeats
/// the item it was made from, cut where the okurigana starts), a surface
/// with digits (the analyzer reads them one by one, 2人 as にじん), a surface
/// with whitespace, and a conjugation the table does not know.
pub fn reading_form(line: &DictionaryLine) -> Option<(String, String)> {
    let surface = line.surface;
    if !surface.chars().any(is_kanji)
        || line.reading.contains(['{', '*'])
        || surface
            .chars()
            .any(|c| c.is_whitespace() || c.is_ascii_digit() || ('０'..='９').contains(&c))
    {
        return None;
    }
    let reading = line.reading.to_owned();
    if line.conjugation.is_empty() {
        return Some((surface.to_owned(), reading));
    }
    let ending = terminal_ending(line.conjugation)?;
    Some((format!("{surface}{ending}"), format!("{reading}{ending}")))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadingsError {
    #[error("the analyzer's output does not follow the input at {0:?}")]
    Unmatched(String),
    #[error("reading the analyzer's output: {0}")]
    Read(String),
}

/// The readings another analyzer gives each of `inputs`, read from its
/// n-best output: lines of `surface<TAB>reading` per word, `EOS` after each
/// path, and a path's surfaces joined being its input. A path holds `None`
/// when one of its words has no reading. `inputs` must not repeat a line
/// next to itself, as the paths of two such inputs cannot be told apart.
pub fn paths_of(
    inputs: &[String],
    output: impl BufRead,
) -> Result<Vec<Vec<Option<String>>>, ReadingsError> {
    let mut paths: Vec<Vec<Option<String>>> = vec![Vec::new(); inputs.len()];
    let mut at = 0;
    let mut surface = String::new();
    let mut reading = Some(String::new());
    for line in output.lines() {
        let line = line.map_err(|e| ReadingsError::Read(e.to_string()))?;
        if line != "EOS" {
            let (word, word_reading) = line.split_once('\t').unwrap_or((&line, ""));
            surface.push_str(word);
            reading = match (reading, word_reading) {
                (_, "" | "*") => None,
                (Some(r), w) => Some(r + w),
                (None, _) => None,
            };
            continue;
        }
        while at < inputs.len() && inputs[at] != surface {
            if paths[at].is_empty() {
                return Err(ReadingsError::Unmatched(inputs[at].clone()));
            }
            at += 1;
        }
        let Some(path) = paths.get_mut(at) else {
            return Err(ReadingsError::Unmatched(surface));
        };
        path.push(reading.take());
        surface.clear();
        reading = Some(String::new());
    }
    Ok(paths)
}

/// How another analyzer's paths read an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    /// One of the paths reads it as the dictionary does.
    Agrees,
    /// No path does, and the best path has a reading.
    Disagrees,
    /// The best path has a word without a reading, and no path agrees.
    Unread,
}

/// Compares the dictionary's `reading` with the paths' readings in katakana.
pub fn agreement(reading: &str, paths: &[Option<String>]) -> Agreement {
    let katakana = hiragana_to_katakana(reading);
    if paths.iter().flatten().any(|p| *p == katakana) {
        Agreement::Agrees
    } else if paths.first().is_some_and(Option::is_some) {
        Agreement::Disagrees
    } else {
        Agreement::Unread
    }
}

/// The readings UniDic gives the surface (`known`, in hiragana) that some
/// path of the other analyzer reads too: what a disagreeing item is likely
/// read as.
pub fn shared_readings(known: &[String], paths: &[Option<String>]) -> Vec<String> {
    known
        .iter()
        .filter(|r| {
            let katakana = hiragana_to_katakana(r);
            paths.iter().flatten().any(|p| *p == katakana)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use kanaemi_core::{Candidate, Converter};

    use super::*;
    use crate::evaluation::Query;

    fn case(
        category: &str,
        context: &str,
        reading: &str,
        okurigana: Option<&str>,
        accepted: &[&str],
    ) -> WordCase {
        WordCase {
            category: category.into(),
            context: context.into(),
            reading: reading.into(),
            okurigana: okurigana.map(Into::into),
            accepted: accepted.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn a_word_line_gives_its_category_context_reading_and_surfaces() {
        let text =
            "# 分類\t前の文\t読み\t正解\n\n熟字訓\t\tあす\t明日\n同音\t会議で\tいぎ\t異議|意義\n";

        assert_eq!(
            parse_word_cases(text),
            Ok(vec![
                case("熟字訓", "", "あす", None, &["明日"]),
                case("同音", "会議で", "いぎ", None, &["異議", "意義"]),
            ])
        );
    }

    #[test]
    fn a_star_marks_the_first_kana_of_the_okurigana() {
        assert_eq!(
            parse_word_cases("送り\t\tか*い\t書い\n"),
            Ok(vec![case("送り", "", "かい", Some("い"), &["書い"])])
        );
    }

    #[test]
    fn a_broken_word_line_is_an_error_naming_its_line() {
        assert_eq!(
            parse_word_cases("a\tb\tc\n"),
            Err(WordCasesError::Columns(1))
        );
        assert_eq!(
            parse_word_cases("# x\na\t\t\t書\n"),
            Err(WordCasesError::Empty(2))
        );
        assert_eq!(
            parse_word_cases("a\t\tか\t|\n"),
            Err(WordCasesError::Empty(1))
        );
        assert_eq!(
            parse_word_cases("a\t\tか*\t書\n"),
            Err(WordCasesError::Okurigana(1))
        );
    }

    /// Offers `candidates`, or `after_context` once that text was typed.
    #[derive(Default)]
    struct Fake {
        candidates: Vec<&'static str>,
        after_context: Option<(&'static str, Vec<&'static str>)>,
        typed: String,
    }

    impl Converter for Fake {
        fn convert(&self, _reading: &str, _okurigana: Option<&str>) -> Vec<Candidate> {
            let surfaces = match &self.after_context {
                Some((context, surfaces)) if self.typed == *context => surfaces,
                _ => &self.candidates,
            };
            surfaces
                .iter()
                .map(|s| Candidate {
                    surface: (*s).into(),
                })
                .collect()
        }
    }

    impl Field for Fake {
        fn start_over(&mut self) {
            self.typed.clear();
        }

        fn type_text(&mut self, text: &str) {
            self.typed.push_str(text);
        }

        fn commit(&mut self, _query: &Query) {}
    }

    #[test]
    fn a_word_ranks_where_any_right_surface_first_comes() {
        let mut fake = Fake {
            candidates: vec!["意義", "異議", "威儀"],
            ..Fake::default()
        };

        assert_eq!(
            check_word(
                &mut fake,
                &case("同音", "", "いぎ", None, &["威儀", "異議"])
            ),
            WordResult {
                rank: Some(2),
                top: "意義".into()
            }
        );
        assert_eq!(
            check_word(&mut fake, &case("同音", "", "いぎ", None, &["イギ"])).rank,
            Some(4),
            "the reading's katakana follows the results"
        );
        assert_eq!(
            check_word(&mut fake, &case("同音", "", "いぎ", None, &["遺技"])).rank,
            None
        );
    }

    #[test]
    fn each_word_is_converted_after_its_own_context_alone() {
        let mut fake = Fake {
            candidates: vec!["意義"],
            after_context: Some(("会議で", vec!["異議"])),
            ..Fake::default()
        };
        let objection = case("同音", "会議で", "いぎ", None, &["異議"]);
        let meaning = case("同音", "", "いぎ", None, &["意義"]);

        assert_eq!(check_word(&mut fake, &objection).rank, Some(1));
        assert_eq!(check_word(&mut fake, &meaning).rank, Some(1));
    }

    #[test]
    fn kanaemi_converts_the_words_with_the_dictionary() {
        let (dictionary, invalid) = TextDictionary::parse("か\t書\t五段-カ行\nあす\t明日\n");
        assert_eq!(invalid, []);
        let cases = [
            case("送り", "", "かい", Some("い"), &["書い"]),
            case("熟字訓", "", "あす", None, &["明日"]),
            case("熟字訓", "", "いなか", None, &["田舎"]),
        ];

        let results = check_words(&cases, &Arc::new(dictionary), None);

        let ranks: Vec<_> = results.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, [Some(1), Some(1), None]);
    }

    #[test]
    fn the_word_tables_count_per_category_and_list_what_missed() {
        let cases = [
            case("b", "", "あ", None, &["亜"]),
            case("a", "x", "い", Some("い"), &["胃", "イ"]),
        ];
        let off = [
            WordResult {
                rank: Some(1),
                top: "亜".into(),
            },
            WordResult {
                rank: None,
                top: "井".into(),
            },
        ];

        assert_eq!(
            word_scores_tsv(&cases, &[("off", &off)]),
            "category\tmodel\twords\tcovered\tfirst\tmean_rank\n\
             a\toff\t1\t0\t0\t0.000\n\
             b\toff\t1\t1\t1\t1.000\n\
             all\toff\t2\t1\t1\t1.000\n"
        );
        assert_eq!(
            word_misses_tsv(&cases, &[("off", &off)]),
            "category\tmodel\tcontext\treading\taccepted\ttop\trank\n\
             a\toff\tx\tい\t胃|イ\t井\t\n"
        );
    }

    fn line<'a>(
        reading: &'a str,
        surface: &'a str,
        conjugation: &'a str,
        cost: Option<u32>,
    ) -> DictionaryLine<'a> {
        DictionaryLine {
            reading,
            surface,
            conjugation,
            cost,
        }
    }

    #[test]
    fn dictionary_lines_skip_the_header_and_read_the_columns() {
        let text = "# 辞書\nか\t書\t五段-カ行\t1626\nてがみ\t手紙\nあ\t亜\t\t3000\n";

        let lines: Vec<_> = dictionary_lines(text).map(|(_, l)| l).collect();

        assert_eq!(
            lines,
            [
                line("か", "書", "五段-カ行", Some(1626)),
                line("てがみ", "手紙", "", None),
                line("あ", "亜", "", Some(3000)),
            ]
        );
    }

    #[test]
    fn base_items_fall_into_strata_by_kind_and_cost() {
        let strata = [
            ("base", line("か", "書", "五段-カ行", Some(100))),
            ("base", line("{}ほん", "{}本", "", Some(100))),
            ("base", line("あ", "亜", "", Some(UNIDIC_UNSEEN))),
            ("base", line("て", "手", "", Some(COMMON_COST - 1))),
            ("base", line("て", "手", "", Some(COMMON_COST))),
            ("railway", line("か", "書", "五段-カ行", Some(100))),
        ]
        .map(|(name, line)| stratum_of(name, &line));

        assert_eq!(
            strata,
            [
                "base:conjugating",
                "base:numeric",
                "base:unidic-only",
                "base:common",
                "base:rare",
                "railway",
            ]
        );
    }

    #[test]
    fn the_sample_takes_up_to_the_count_of_each_stratum_the_same_each_time() {
        let base = "# 基本\nあ\t亜\t\t3000\nい\t胃\t\t3000\nう\t宇\t\t3000\nて\t手\t\t100\n";
        let railway = "# 鉄道\nえき\t駅\t\t100\nせん\t線\t\t100\n";
        let dictionaries = [("base", base), ("railway", railway)];

        let sample = sample_tsv(&dictionaries, 2, 1);

        let rows: Vec<&str> = sample.lines().skip(1).collect();
        let strata: Vec<&str> = rows.iter().map(|r| r.split('\t').next().unwrap()).collect();
        assert_eq!(
            strata,
            [
                "base:common",
                "base:unidic-only",
                "base:unidic-only",
                "railway"
            ]
        );
        assert!(rows[0].ends_with("\tて\t手\t\t100\t\t"));
        assert_eq!(sample, sample_tsv(&dictionaries, 2, 1));
    }

    #[test]
    fn the_form_read_is_the_terminal_form_of_a_conjugating_word() {
        assert_eq!(
            reading_form(&line("か", "書", "五段-カ行", None)),
            Some(("書く".into(), "かく".into()))
        );
        assert_eq!(
            reading_form(&line("てがみ", "手紙", "", None)),
            Some(("手紙".into(), "てがみ".into()))
        );
    }

    #[test]
    fn items_another_analyzer_cannot_read_as_written_are_not_read() {
        for item in [
            line("ぺーじ", "ページ", "", None),
            line("{}ほん", "{}本", "", None),
            line("す*ご", "過ご", "", None),
            line("ふたり", "2人", "", None),
            line("ふたり", "２人", "", None),
            line("あ い", "亜 胃", "", None),
            line("か", "書", "知らない型", None),
        ] {
            assert_eq!(reading_form(&item), None, "{item:?}");
        }
    }

    #[test]
    fn the_paths_of_each_input_follow_one_another() {
        let inputs = ["深掘り".to_owned(), "米子".to_owned()];
        let output = "深\tフカ\n掘り\tホリ\nEOS\n深掘り\tフカボリ\nEOS\n米子\tヨナゴ\nEOS\n米\tコメ\n子\t\nEOS\n";

        assert_eq!(
            paths_of(&inputs, output.as_bytes()),
            Ok(vec![
                vec![Some("フカホリ".into()), Some("フカボリ".into())],
                vec![Some("ヨナゴ".into()), None],
            ])
        );
    }

    #[test]
    fn output_that_does_not_follow_the_inputs_is_an_error() {
        let inputs = ["深掘り".to_owned(), "米子".to_owned()];

        assert_eq!(
            paths_of(&inputs, "米子\tヨナゴ\nEOS\n".as_bytes()),
            Err(ReadingsError::Unmatched("深掘り".into()))
        );
        assert_eq!(
            paths_of(
                &inputs,
                "深掘り\tフカボリ\nEOS\n米子\tヨナゴ\nEOS\n亜\tア\nEOS\n".as_bytes()
            ),
            Err(ReadingsError::Unmatched("亜".into()))
        );
    }

    #[test]
    fn the_readings_unidic_and_a_path_share_are_the_likely_ones() {
        let known = [
            "ざう".to_owned(),
            "ざゆう".to_owned(),
            "すわりみぎ".to_owned(),
        ];
        let paths = [Some("ザユウ".to_owned()), None, Some("ザミギ".to_owned())];

        assert_eq!(shared_readings(&known, &paths), ["ざゆう"]);
        assert_eq!(shared_readings(&[], &paths), Vec::<String>::new());
    }

    #[test]
    fn an_item_agrees_when_any_path_reads_it_as_the_dictionary_does() {
        let paths = [Some("フカホリ".to_owned()), Some("フカボリ".to_owned())];
        assert_eq!(agreement("ふかぼり", &paths), Agreement::Agrees);
        assert_eq!(agreement("しんくつ", &paths), Agreement::Disagrees);
        assert_eq!(
            agreement("しんくつ", &[None, Some("フカボリ".to_owned())]),
            Agreement::Unread
        );
        assert_eq!(
            agreement("ふかぼり", &[None, Some("フカボリ".to_owned())]),
            Agreement::Agrees
        );
    }
}

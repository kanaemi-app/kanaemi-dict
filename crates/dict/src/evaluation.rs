//! Measures a dictionary by converting units with Kanaemi and finding where
//! each unit's surface ranks among the candidates.

use std::io;
use std::sync::Arc;

use kanaemi_core::{Converter, Effect};
use kanaemi_engine::{Engine, LineSink, Selections, Slot, TextDictionary};

use crate::Unit;
use crate::dictionary::okuri_line;
use crate::kana::hiragana_to_katakana;

/// What a unit is typed as and the candidate that should come out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub reading: String,
    /// The first kana of the okurigana, when the typist marks where it starts.
    pub okurigana: Option<String>,
    pub expected: String,
}

/// A conjugating unit is typed with its okurigana marked, up to the first kana
/// of the okurigana (書い for 書いた, 食べ for 食べられない); the rest is typed
/// after the conversion. Any other unit, and a conjugating one whose surface
/// does not end in kanji and then hiragana, is typed whole.
pub fn query_of(unit: &Unit) -> Query {
    let marked = unit
        .conjugation
        .as_ref()
        .and_then(|_| okuri_line(&unit.reading, &unit.surface))
        .and_then(|(reading, expected)| {
            let (before, first) = reading.rsplit_once('*')?;
            Some(Query {
                reading: format!("{before}{first}"),
                okurigana: Some(first.to_owned()),
                expected,
            })
        });
    marked.unwrap_or_else(|| Query {
        reading: unit.reading.clone(),
        okurigana: None,
        expected: unit.surface.clone(),
    })
}

impl Query {
    /// What the core reports when this query's expected candidate is
    /// committed: the reading as converted, with the okurigana it was
    /// converted with.
    fn committed(&self) -> Effect {
        Effect::Committed {
            reading: self.reading.clone(),
            okurigana: self.okurigana.clone(),
            surface: self.expected.clone(),
        }
    }
}

/// Units converted and where their expected candidates ranked.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    pub units: usize,
    /// Units whose expected candidate was among the candidates.
    pub covered: usize,
    /// Units whose expected candidate came first.
    pub first: usize,
    /// The 1-based ranks of the covered units, summed.
    pub rank_sum: usize,
}

impl Score {
    pub(crate) fn add(&mut self, rank: Option<usize>) {
        self.units += 1;
        if let Some(rank) = rank {
            self.covered += 1;
            self.first += usize::from(rank == 1);
            self.rank_sum += rank;
        }
    }

    pub fn merge(&mut self, other: Score) {
        self.units += other.units;
        self.covered += other.covered;
        self.first += other.first;
        self.rank_sum += other.rank_sum;
    }

    /// The mean rank of the covered units; 0 when none is covered.
    pub fn mean_rank(&self) -> f64 {
        if self.covered == 0 {
            return 0.0;
        }
        self.rank_sum as f64 / self.covered as f64
    }
}

/// Scores without and with the commit history.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scores {
    pub fresh: Score,
    pub with_history: Score,
}

impl Scores {
    pub fn merge(&mut self, other: Scores) {
        self.fresh.merge(other.fresh);
        self.with_history.merge(other.with_history);
    }
}

/// Scores of the numeric units and of the other units apart.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClassScores {
    pub numeric: Scores,
    pub other: Scores,
}

impl ClassScores {
    pub fn merge(&mut self, other: ClassScores) {
        self.numeric.merge(other.numeric);
        self.other.merge(other.other);
    }

    /// Every unit's scores together.
    pub fn all(&self) -> Scores {
        let mut all = self.numeric;
        all.merge(self.other);
        all
    }

    /// The table's classes in their order: `numeric`, `other`, `all`.
    pub fn classes(&self) -> [(&'static str, Scores); 3] {
        [
            ("numeric", self.numeric),
            ("other", self.other),
            ("all", self.all()),
        ]
    }

    fn of(&mut self, unit: &Unit) -> &mut Scores {
        if unit.numeric.is_some() {
            &mut self.numeric
        } else {
            &mut self.other
        }
    }
}

/// An input field the evaluation types into: a converter that learns what is
/// committed and typed there.
pub trait Field: Converter {
    /// Forgets everything learned, as for a typist who has never committed.
    fn start_over(&mut self);
    fn type_text(&mut self, text: &str);
    fn commit(&mut self, query: &Query);
}

impl Field for Engine {
    fn start_over(&mut self) {
        self.learn(&Effect::FocusMoved);
        // Moving the focus keeps the picks, which a typist carries from field
        // to field; left in, they would tie a document's ranks to whichever
        // documents the same engine converted before it.
        self.replace_selections(Selections::default());
    }

    fn type_text(&mut self, text: &str) {
        self.learn(&Effect::Typed(text.to_owned()));
    }

    fn commit(&mut self, query: &Query) {
        self.learn(&query.committed());
    }
}

/// Converts a document's units in order. `fresh` never sees a commit or
/// text; `with_history` starts the document over, takes the document's
/// `text` before each unit as the text typed so far, and commits each
/// expected candidate, as one input field the typist stays in.
pub fn evaluate_document(
    fresh: &impl Converter,
    with_history: &mut impl Field,
    units: &[Unit],
    text: impl AsRef<str>,
) -> ClassScores {
    let mut scores = ClassScores::default();
    with_history.start_over();
    let chars: Vec<char> = text.as_ref().chars().collect();
    let mut typed = 0;
    for unit in units {
        let upto = unit.position.min(chars.len());
        if upto > typed {
            with_history.type_text(&chars[typed..upto].iter().collect::<String>());
            typed = upto;
        }
        let query = query_of(unit);
        let class = scores.of(unit);
        class.fresh.add(rank_of(fresh, &query));
        class.with_history.add(rank_of(with_history, &query));
        with_history.commit(&query);
    }
    scores
}

/// The 1-based rank of the expected candidate, if it is offered at all. As
/// Kanaemi does, the reading's katakana follows the conversion results when
/// they lack it, with okurigana given or not.
pub(crate) fn rank_of(converter: &impl Converter, query: &Query) -> Option<usize> {
    offered(converter, &query.reading, query.okurigana.as_deref())
        .iter()
        .position(|s| *s == query.expected)
        .map(|i| i + 1)
}

/// The surfaces Kanaemi offers for `reading`, best first: the conversion
/// results, then the reading's katakana when they lack it.
pub(crate) fn offered(
    converter: &impl Converter,
    reading: &str,
    okurigana: Option<&str>,
) -> Vec<String> {
    let mut surfaces: Vec<String> = converter
        .convert(reading, okurigana)
        .into_iter()
        .map(|c| c.surface)
        .collect();
    let katakana = hiragana_to_katakana(reading);
    if !surfaces.contains(&katakana) {
        surfaces.push(katakana);
    }
    surfaces
}

/// Registrations go nowhere: the evaluation's user dictionary starts empty
/// and is never kept.
struct Discard;

impl LineSink for Discard {
    fn append(&mut self, _line: &str) -> io::Result<()> {
        Ok(())
    }
}

/// A Kanaemi engine with `dictionary` as its only dictionary besides an
/// empty user custom dictionary. Engines on several threads share one parsed
/// dictionary.
pub fn engine(dictionary: Arc<TextDictionary>) -> Engine {
    Engine::new(
        [Slot::Dictionary(Box::new(dictionary))],
        TextDictionary::default(),
        Discard,
    )
}

#[cfg(test)]
mod tests {
    use kanaemi_core::Candidate;

    use super::*;
    use crate::Numeric;

    fn unit(reading: &str, surface: &str, stem: Option<(&str, &str)>) -> Unit {
        Unit {
            doc_id: "d".into(),
            position: 0,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: stem.map(|s| s.0.into()),
            stem_surface: stem.map(|s| s.1.into()),
            conjugation: stem.map(|_| "五段-カ行".into()),
            unknown_conjugation: None,
            numeric: None,
        }
    }

    fn query(reading: &str, okurigana: Option<&str>, expected: &str) -> Query {
        Query {
            reading: reading.into(),
            okurigana: okurigana.map(Into::into),
            expected: expected.into(),
        }
    }

    #[test]
    fn conjugating_units_are_typed_up_to_the_first_kana_of_their_okurigana() {
        assert_eq!(
            query_of(&unit("かいた", "書いた", Some(("か", "書")))),
            query("かい", Some("い"), "書い")
        );
        assert_eq!(
            query_of(&unit(
                "たべられない",
                "食べられない",
                Some(("たべ", "食べ"))
            )),
            query("たべ", Some("べ"), "食べ")
        );
    }

    #[test]
    fn other_units_are_typed_whole() {
        assert_eq!(
            query_of(&unit("てがみ", "手紙", None)),
            query("てがみ", None, "手紙")
        );
        assert_eq!(
            query_of(&unit("みなさん", "皆さん", None)),
            query("みなさん", None, "皆さん")
        );
    }

    #[test]
    fn a_conjugating_unit_without_kanji_before_its_okurigana_is_typed_whole() {
        assert_eq!(
            query_of(&unit("ぐぐった", "ググった", Some(("ぐぐ", "ググ")))),
            query("ぐぐった", None, "ググった")
        );
    }

    #[test]
    fn a_commit_reports_the_reading_and_okurigana_it_was_converted_with() {
        assert_eq!(
            query("かい", Some("い"), "書い").committed(),
            Effect::Committed {
                reading: "かい".into(),
                okurigana: Some("い".into()),
                surface: "書い".into(),
            }
        );
        assert_eq!(
            query("てがみ", None, "手紙").committed(),
            Effect::Committed {
                reading: "てがみ".into(),
                okurigana: None,
                surface: "手紙".into(),
            }
        );
    }

    #[test]
    fn mean_rank_is_over_the_covered_units_only() {
        let mut score = Score::default();
        assert_eq!(score.mean_rank(), 0.0);

        for rank in [Some(1), Some(3), None] {
            score.add(rank);
        }

        assert_eq!(
            score,
            Score {
                units: 3,
                covered: 2,
                first: 1,
                rank_sum: 4,
            }
        );
        assert_eq!(score.mean_rank(), 2.0);
    }

    /// Offers `candidates` for every reading, the surfaces committed since it
    /// started over first, latest first.
    #[derive(Default)]
    struct Fake {
        candidates: Vec<&'static str>,
        committed: Vec<String>,
        texts: Vec<String>,
    }

    impl Converter for Fake {
        fn convert(&self, _reading: &str, _okurigana: Option<&str>) -> Vec<Candidate> {
            let mut surfaces: Vec<String> = self.committed.iter().rev().cloned().collect();
            for c in &self.candidates {
                if !surfaces.iter().any(|s| s == c) {
                    surfaces.push((*c).into());
                }
            }
            surfaces
                .into_iter()
                .map(|surface| Candidate { surface })
                .collect()
        }
    }

    impl Field for Fake {
        fn start_over(&mut self) {
            self.committed.clear();
        }

        fn type_text(&mut self, text: &str) {
            self.texts.push(text.into());
        }

        fn commit(&mut self, query: &Query) {
            self.committed.push(query.expected.clone());
        }
    }

    #[test]
    fn the_readings_katakana_follows_the_conversion_results_as_in_kanaemi() {
        let fake = |candidates| Fake {
            candidates,
            ..Fake::default()
        };
        let page = query("ぺーじ", None, "ページ");

        assert_eq!(rank_of(&fake(vec!["頁"]), &page), Some(2));
        assert_eq!(rank_of(&fake(vec![]), &page), Some(1));
        assert_eq!(rank_of(&fake(vec!["ページ", "頁"]), &page), Some(1));
        assert_eq!(
            rank_of(&fake(vec!["書い"]), &query("かい", Some("い"), "カイ")),
            Some(2),
            "with okurigana too"
        );
    }

    #[test]
    fn the_text_before_each_unit_reaches_the_field_with_history_only() {
        let fresh = Fake::default();
        let mut with_history = Fake::default();
        let mut units = [unit("かん", "漢", None), unit("じ", "字", None)];
        units[0].position = 1;
        units[1].position = 5;

        evaluate_document(&fresh, &mut with_history, &units, "あ漢い\nう字。");

        assert_eq!(fresh.texts, Vec::<String>::new());
        assert_eq!(with_history.texts, ["あ", "漢い\nう"]);
    }

    #[test]
    fn ranks_are_counted_without_and_with_the_commit_history() {
        let fake = || Fake {
            candidates: vec!["手神", "手紙"],
            committed: vec!["前の文書".into()],
            ..Fake::default()
        };
        let units = [
            unit("てがみ", "手紙", None),
            unit("てがみ", "手紙", None),
            unit("ばく", "獏", None),
        ];

        let scores = evaluate_document(&fake(), &mut fake(), &units, "");

        assert_eq!(
            scores.other,
            Scores {
                fresh: Score {
                    units: 3,
                    covered: 2,
                    first: 0,
                    rank_sum: 6,
                },
                with_history: Score {
                    units: 3,
                    covered: 2,
                    first: 1,
                    rank_sum: 3,
                },
            }
        );
    }

    #[test]
    fn numeric_units_are_scored_apart_from_the_others() {
        let fake = || Fake {
            candidates: vec!["3本", "手紙"],
            ..Fake::default()
        };
        let mut three = unit("3ぽん", "三本", None);
        three.numeric = Some(Numeric {
            reading: "{}ぽん".into(),
            surface: "{kanji}本".into(),
            value: "3".into(),
        });
        let units = [three, unit("てがみ", "手紙", None)];

        let scores = evaluate_document(&fake(), &mut fake(), &units, "");

        let missed = Score {
            units: 1,
            ..Score::default()
        };
        let second = Score {
            units: 1,
            covered: 1,
            first: 0,
            rank_sum: 2,
        };
        assert_eq!(scores.numeric.fresh, missed);
        assert_eq!(scores.other.fresh, second);
        assert_eq!(scores.all().fresh, Score { units: 2, ..second });
    }

    fn dictionary(text: &str) -> Arc<TextDictionary> {
        let (dictionary, invalid) = TextDictionary::parse(text);
        assert_eq!(invalid, []);
        Arc::new(dictionary)
    }

    #[test]
    fn kanaemi_converts_the_queries_with_the_dictionary() {
        let dictionary = dictionary("か\t書\t五段-カ行\nかい\t会\nてがみ\t手紙\n");
        let units = [
            unit("かいた", "書いた", Some(("か", "書"))),
            unit("てがみ", "手紙", None),
        ];

        let scores = evaluate_document(
            &engine(dictionary.clone()),
            &mut engine(dictionary),
            &units,
            "",
        );

        let all_first = Score {
            units: 2,
            covered: 2,
            first: 2,
            rank_sum: 2,
        };
        assert_eq!(
            scores.other,
            Scores {
                fresh: all_first,
                with_history: all_first,
            }
        );
    }

    #[test]
    fn each_document_starts_with_nothing_learned_from_the_one_before() {
        let dictionary = dictionary("てがみ\t手紙\t\t100\nてがみ\t手神\t\t200\n");
        let fresh = engine(dictionary.clone());
        let mut with_history = engine(dictionary);
        let picked_often = [
            unit("てがみ", "手神", None),
            unit("てがみ", "手神", None),
            unit("てがみ", "手神", None),
        ];
        evaluate_document(&fresh, &mut with_history, &picked_often, "");

        let scores = evaluate_document(
            &fresh,
            &mut with_history,
            &[unit("てがみ", "手紙", None)],
            "",
        );

        assert_eq!(scores.other.with_history.first, 1);
    }
}

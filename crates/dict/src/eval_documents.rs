//! Evaluating with the documents of the eval split: reading them with their
//! units, converting them in parallel, and counting the scores per kind of
//! source.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::BufRead;
use std::sync::Arc;

use kanaemi_engine::{RankingModel, TextDictionary};
use rayon::prelude::*;

use crate::documents::each_document;
use crate::{
    ClassScores, DocumentsError, Split, Unit, UnitsError, engine, evaluate_document, read_units,
    split_of,
};

/// A document of the eval split with its units in position order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDocument {
    pub doc_id: String,
    pub source_id: String,
    pub units: Vec<Unit>,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EvalDocumentsError {
    #[error("units: {0}")]
    Units(#[from] UnitsError),
    #[error(
        "units: line {line}: {doc_id} at {position} comes after {previous_doc_id} at {previous_position}; units must be sorted by doc_id and position"
    )]
    UnsortedUnits {
        line: usize,
        doc_id: String,
        position: usize,
        previous_doc_id: String,
        previous_position: usize,
    },
    #[error("documents: {0}")]
    Documents(#[from] DocumentsError),
    #[error("documents: no text for {0}, whose units are evaluated")]
    MissingText(String),
}

/// The documents of the eval split in doc ID order, from `units`, JSON Lines
/// sorted by doc ID and position as `kanaemi-dict units` writes them, and
/// their texts from `docs`.
pub fn eval_documents(
    units: impl BufRead,
    docs: impl BufRead,
) -> Result<Vec<EvalDocument>, EvalDocumentsError> {
    let mut eval = Vec::new();
    each_document_with_units(
        units,
        docs,
        |doc_id| split_of(doc_id) == Split::Eval,
        |doc| {
            eval.push(doc);
            Ok::<_, EvalDocumentsError>(())
        },
    )?;
    Ok(eval)
}

/// Calls `f` on each document `wanted` takes by its doc ID, in doc ID order,
/// with its units from `units` and its text from `docs`, both sorted as
/// `kanaemi-dict units` and the documents are; one document is held at a
/// time. Compounds are left out, as they overlap the units typed. A document
/// without units is no document here.
pub fn each_document_with_units<E: From<EvalDocumentsError>>(
    units: impl BufRead,
    docs: impl BufRead,
    wanted: impl Fn(&str) -> bool,
    mut f: impl FnMut(EvalDocument) -> Result<(), E>,
) -> Result<(), E> {
    let mut units = read_units(units).enumerate();
    let mut previous: Option<(String, usize)> = None;
    let mut pending: Option<Unit> = None;
    // The units of the next wanted document, all of them.
    let mut next_group = || -> Result<Option<Vec<Unit>>, EvalDocumentsError> {
        let mut group: Vec<Unit> = pending.take().into_iter().collect();
        for (i, unit) in units.by_ref() {
            let unit = unit?;
            if let Some((previous_doc_id, previous_position)) =
                previous.take_if(|(doc_id, position)| {
                    (doc_id.as_str(), *position) > (unit.doc_id.as_str(), unit.position)
                })
            {
                return Err(EvalDocumentsError::UnsortedUnits {
                    line: i + 1,
                    doc_id: unit.doc_id,
                    position: unit.position,
                    previous_doc_id,
                    previous_position,
                });
            }
            previous = Some((unit.doc_id.clone(), unit.position));
            if unit.compound || !wanted(&unit.doc_id) {
                continue;
            }
            if group
                .first()
                .is_some_and(|first| first.doc_id != unit.doc_id)
            {
                pending = Some(unit);
                return Ok(Some(group));
            }
            group.push(unit);
        }
        Ok((!group.is_empty()).then_some(group))
    };
    // Reading fails with the reader's errors, `f` with the caller's.
    enum Failed<E> {
        Read(EvalDocumentsError),
        Caller(E),
    }
    impl<E> From<DocumentsError> for Failed<E> {
        fn from(e: DocumentsError) -> Self {
            Failed::Read(e.into())
        }
    }
    let mut next = next_group()?;
    let read = each_document(docs, |doc| {
        if let Some(group) = next.take_if(|g| g[0].doc_id < doc.doc_id) {
            return Err(Failed::Read(EvalDocumentsError::MissingText(
                group[0].doc_id.clone(),
            )));
        }
        if next.as_ref().is_some_and(|g| g[0].doc_id == doc.doc_id) {
            let units = next.take().expect("checked");
            next = next_group().map_err(Failed::Read)?;
            f(EvalDocument {
                doc_id: doc.doc_id,
                source_id: doc.source_id,
                units,
                text: doc.text,
            })
            .map_err(Failed::Caller)?;
        }
        Ok(())
    });
    match read {
        Err(Failed::Read(e)) => return Err(e.into()),
        Err(Failed::Caller(e)) => return Err(e),
        Ok(()) => {}
    }
    match next {
        Some(group) => Err(EvalDocumentsError::MissingText(group[0].doc_id.clone()).into()),
        None => Ok(()),
    }
}

/// Scores per kind of source, the part of a doc ID before its first `:`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub by_kind: BTreeMap<String, ClassScores>,
}

impl Evaluation {
    /// Every kind's scores together.
    pub fn all(&self) -> ClassScores {
        let mut all = ClassScores::default();
        for scores in self.by_kind.values() {
            all.merge(*scores);
        }
        all
    }

    /// The table of `build/evaluation.tsv`: a header, then a row per kind,
    /// history and class, kinds in the order of their UTF-8 bytes and `all`
    /// last.
    pub fn to_tsv(&self) -> String {
        let all = self.all();
        let mut tsv = String::from("kind\thistory\tclass\tunits\tcovered\tfirst\tmean_rank\n");
        for (kind, scores) in self.by_kind.iter().chain([(&"all".to_owned(), &all)]) {
            for history in ["off", "on"] {
                for (class, class_scores) in scores.classes() {
                    let score = match history {
                        "off" => class_scores.fresh,
                        _ => class_scores.with_history,
                    };
                    writeln!(
                        tsv,
                        "{kind}\t{history}\t{class}\t{}\t{}\t{}\t{:.3}",
                        score.units,
                        score.covered,
                        score.first,
                        score.mean_rank(),
                    )
                    .expect("writing to a String never fails");
                }
            }
        }
        tsv
    }
}

/// Converts every document with engines reading `dictionary` alone, ranking
/// with `model` when given, in parallel by chunks of documents, a few chunks
/// per thread. Each chunk has its own pair of engines, all sharing the one
/// parsed dictionary and model.
pub fn evaluate_documents(
    docs: &[EvalDocument],
    dictionary: &Arc<TextDictionary>,
    model: Option<&Arc<RankingModel>>,
) -> Evaluation {
    let engine = || {
        let mut engine = engine(dictionary.clone());
        engine.set_model(model.cloned());
        engine
    };
    let chunk = docs.len().div_ceil(rayon::current_num_threads() * 4).max(1);
    let per_doc: Vec<ClassScores> = docs
        .par_chunks(chunk)
        .flat_map_iter(|chunk| {
            let fresh = engine();
            let mut with_history = engine();
            chunk
                .iter()
                .map(|doc| evaluate_document(&fresh, &mut with_history, &doc.units, &doc.text))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut evaluation = Evaluation::default();
    for (doc, scores) in docs.iter().zip(per_doc) {
        let kind = doc.doc_id.split(':').next().unwrap_or_default();
        evaluation
            .by_kind
            .entry(kind.to_owned())
            .or_default()
            .merge(scores);
    }
    evaluation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Score, Scores};

    // Splits computed apart from this code: aozora:000001 is train,
    // aozora:000013 dev, and aozora:000004 and aozora:000015 eval.
    fn unit(doc_id: &str, position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            compound: false,
            doc_id: doc_id.into(),
            position,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        }
    }

    fn units_jsonl(units: &[Unit]) -> String {
        units
            .iter()
            .map(|u| format!("{}\n", serde_json::to_string(u).unwrap()))
            .collect()
    }

    fn docs_jsonl(docs: &[(&str, &str)]) -> String {
        docs.iter()
            .map(|(doc_id, text)| {
                format!(
                    "{}\n",
                    serde_json::json!({ "doc_id": doc_id, "source_id": "aozora-text", "text": text })
                )
            })
            .collect()
    }

    #[test]
    fn only_eval_documents_are_read_in_doc_id_order_with_their_texts() {
        let units = [
            unit("aozora:000001", 0, "てがみ", "手紙"),
            unit("aozora:000004", 0, "かん", "漢"),
            unit("aozora:000004", 2, "じ", "字"),
            unit("aozora:000013", 0, "ほん", "本"),
            unit("aozora:000015", 1, "しょ", "書"),
        ];
        let docs = docs_jsonl(&[
            ("aozora:000001", "手紙"),
            ("aozora:000004", "漢い字"),
            ("aozora:000013", "本"),
            ("aozora:000014", "外"),
            ("aozora:000015", "あ書"),
        ]);

        let read = eval_documents(units_jsonl(&units).as_bytes(), docs.as_bytes()).unwrap();

        assert_eq!(
            read,
            [
                EvalDocument {
                    doc_id: "aozora:000004".into(),
                    source_id: "aozora-text".into(),
                    units: units[1..3].to_vec(),
                    text: "漢い字".into(),
                },
                EvalDocument {
                    doc_id: "aozora:000015".into(),
                    source_id: "aozora-text".into(),
                    units: units[4..].to_vec(),
                    text: "あ書".into(),
                },
            ]
        );
    }

    #[test]
    fn compounds_are_left_out_of_the_documents_units() {
        let parts = [
            unit("aozora:000004", 0, "ろてん", "露天"),
            unit("aozora:000004", 2, "ふろ", "風呂"),
        ];
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "ろてんぶろ", "露天風呂")
        };
        let units = [parts[0].clone(), compound, parts[1].clone()];
        let docs = docs_jsonl(&[("aozora:000004", "露天風呂")]);

        let read = eval_documents(units_jsonl(&units).as_bytes(), docs.as_bytes()).unwrap();

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].units, parts);
    }

    #[test]
    fn wanted_documents_stream_one_at_a_time_with_their_units_and_texts() {
        let units = [
            unit("aozora:000001", 0, "てがみ", "手紙"),
            unit("aozora:000004", 0, "かん", "漢"),
            unit("aozora:000004", 2, "じ", "字"),
            unit("aozora:000013", 0, "ほん", "本"),
        ];
        let docs = docs_jsonl(&[
            ("aozora:000001", "手紙"),
            ("aozora:000004", "漢い字"),
            ("aozora:000009", "単位のない文書"),
            ("aozora:000013", "本"),
        ]);
        let mut seen = Vec::new();

        each_document_with_units(
            units_jsonl(&units).as_bytes(),
            docs.as_bytes(),
            |doc_id| doc_id != "aozora:000013",
            |doc| {
                seen.push(doc);
                Ok::<_, EvalDocumentsError>(())
            },
        )
        .unwrap();

        assert_eq!(
            seen,
            [
                EvalDocument {
                    doc_id: "aozora:000001".into(),
                    source_id: "aozora-text".into(),
                    units: units[..1].to_vec(),
                    text: "手紙".into(),
                },
                EvalDocument {
                    doc_id: "aozora:000004".into(),
                    source_id: "aozora-text".into(),
                    units: units[1..3].to_vec(),
                    text: "漢い字".into(),
                },
            ]
        );
    }

    #[test]
    fn units_out_of_doc_id_order_are_an_error() {
        let units = [
            unit("aozora:000015", 0, "しょ", "書"),
            unit("aozora:000004", 0, "かん", "漢"),
        ];

        let err = eval_documents(units_jsonl(&units).as_bytes(), "".as_bytes()).unwrap_err();

        assert!(
            matches!(err, EvalDocumentsError::UnsortedUnits { line: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn units_out_of_position_order_within_a_document_are_an_error() {
        let units = [
            unit("aozora:000004", 2, "じ", "字"),
            unit("aozora:000004", 0, "かん", "漢"),
        ];

        let err = eval_documents(units_jsonl(&units).as_bytes(), "".as_bytes()).unwrap_err();

        assert!(
            matches!(err, EvalDocumentsError::UnsortedUnits { line: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn an_eval_document_without_its_text_is_an_error() {
        let units = [unit("aozora:000004", 0, "かん", "漢")];
        let docs = docs_jsonl(&[("aozora:000001", "手紙")]);

        let err = eval_documents(units_jsonl(&units).as_bytes(), docs.as_bytes()).unwrap_err();

        assert!(
            matches!(&err, EvalDocumentsError::MissingText(doc_id) if doc_id == "aozora:000004"),
            "{err}"
        );
    }

    #[test]
    fn a_line_that_is_not_a_unit_is_an_error() {
        let err = eval_documents("{}\n".as_bytes(), "".as_bytes()).unwrap_err();

        assert!(
            matches!(err, EvalDocumentsError::Units(UnitsError { line: 1, .. })),
            "{err}"
        );
    }

    fn score(units: usize, covered: usize, first: usize, rank_sum: usize) -> Score {
        Score {
            units,
            covered,
            first,
            rank_sum,
        }
    }

    fn scores(fresh: Score, with_history: Score) -> Scores {
        Scores {
            fresh,
            with_history,
        }
    }

    #[test]
    fn the_table_has_a_row_per_kind_history_and_class_in_byte_order_with_all_last() {
        let evaluation = Evaluation {
            by_kind: BTreeMap::from([
                (
                    "law".to_owned(),
                    ClassScores {
                        numeric: scores(score(1, 1, 0, 2), score(1, 1, 1, 1)),
                        other: scores(score(3, 2, 2, 3), score(3, 3, 2, 4)),
                    },
                ),
                (
                    "aozora".to_owned(),
                    ClassScores {
                        numeric: Scores::default(),
                        other: scores(score(2, 0, 0, 0), score(2, 1, 1, 1)),
                    },
                ),
            ]),
        };

        assert_eq!(
            evaluation.to_tsv(),
            "kind\thistory\tclass\tunits\tcovered\tfirst\tmean_rank\n\
             aozora\toff\tnumeric\t0\t0\t0\t0.000\n\
             aozora\toff\tother\t2\t0\t0\t0.000\n\
             aozora\toff\tall\t2\t0\t0\t0.000\n\
             aozora\ton\tnumeric\t0\t0\t0\t0.000\n\
             aozora\ton\tother\t2\t1\t1\t1.000\n\
             aozora\ton\tall\t2\t1\t1\t1.000\n\
             law\toff\tnumeric\t1\t1\t0\t2.000\n\
             law\toff\tother\t3\t2\t2\t1.500\n\
             law\toff\tall\t4\t3\t2\t1.667\n\
             law\ton\tnumeric\t1\t1\t1\t1.000\n\
             law\ton\tother\t3\t3\t2\t1.333\n\
             law\ton\tall\t4\t4\t3\t1.250\n\
             all\toff\tnumeric\t1\t1\t0\t2.000\n\
             all\toff\tother\t5\t2\t2\t1.500\n\
             all\toff\tall\t6\t3\t2\t1.667\n\
             all\ton\tnumeric\t1\t1\t1\t1.000\n\
             all\ton\tother\t5\t4\t3\t1.250\n\
             all\ton\tall\t6\t5\t4\t1.200\n"
        );
    }

    fn eval_document(doc_id: &str, units: &[(&str, &str)]) -> EvalDocument {
        EvalDocument {
            doc_id: doc_id.into(),
            source_id: "aozora-text".into(),
            units: units
                .iter()
                .map(|(reading, surface)| unit(doc_id, 0, reading, surface))
                .collect(),
            text: String::new(),
        }
    }

    #[test]
    fn documents_are_scored_per_kind_the_same_as_each_one_alone() {
        let dictionary = {
            let (dictionary, invalid) =
                TextDictionary::parse("てがみ\t手紙\t\t100\nてがみ\t手神\t\t200\n");
            assert_eq!(invalid, []);
            Arc::new(dictionary)
        };
        let picked = [("てがみ", "手神"); 3];
        let docs: Vec<EvalDocument> = (0..40)
            .map(|i| {
                let kind = if i % 2 == 0 { "aozora" } else { "law" };
                let units: &[(&str, &str)] = if i % 3 == 0 {
                    &picked
                } else {
                    &[("てがみ", "手紙")]
                };
                eval_document(&format!("{kind}:{i:02}"), units)
            })
            .collect();

        // One thread, so each chunk holds several documents.
        let evaluation = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| evaluate_documents(&docs, &dictionary, None));

        let mut alone: BTreeMap<String, ClassScores> = BTreeMap::new();
        for doc in &docs {
            let scores = evaluate_document(
                &engine(dictionary.clone()),
                &mut engine(dictionary.clone()),
                &doc.units,
                &doc.text,
            );
            let kind = doc.doc_id.split(':').next().unwrap();
            alone.entry(kind.to_owned()).or_default().merge(scores);
        }
        assert_eq!(evaluation.by_kind, alone);
        assert_eq!(
            evaluation.all().all().fresh.units,
            docs.iter().map(|d| d.units.len()).sum::<usize>()
        );
    }
}

#[cfg(test)]
mod model_tests {
    use kanaemi_engine::RankingModel;

    use super::*;
    use crate::model_file;

    #[test]
    fn with_a_model_the_candidates_go_by_its_scores() {
        let (dictionary, _) = TextDictionary::parse("てがみ\t手紙\t\t100\nてがみ\t手神\t\t200\n");
        let dictionary = Arc::new(dictionary);
        let doc = EvalDocument {
            doc_id: "aozora:1".into(),
            source_id: "aozora-text".into(),
            units: vec![Unit {
                compound: false,
                doc_id: "aozora:1".into(),
                position: 0,
                reading: "てがみ".into(),
                surface: "手神".into(),
                stem_reading: None,
                stem_surface: None,
                conjugation: None,
                unknown_conjugation: None,
                numeric: None,
            }],
            text: "手神".into(),
        };
        let model = {
            let bits = 10;
            let mut weights = vec![0.0f32; 1 << bits];
            let at = xxhash_rust::xxh3::xxh3_64("s\u{1f}手神".as_bytes()) & ((1 << bits) - 1);
            weights[at as usize] = 1.0;
            let path = std::env::temp_dir()
                .join(format!("kanaemi-dict-eval-model-{}", std::process::id()));
            std::fs::write(&path, model_file(bits, &weights)).unwrap();
            let model = RankingModel::open(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            Arc::new(model)
        };

        let without = evaluate_documents(std::slice::from_ref(&doc), &dictionary, None);
        let with = evaluate_documents(&[doc], &dictionary, Some(&model));

        assert_eq!(without.all().all().fresh.first, 0);
        assert_eq!(with.all().all().fresh.first, 1);
    }
}

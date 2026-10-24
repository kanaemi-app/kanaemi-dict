//! Evaluating with the documents of the eval split: reading them with their
//! units, converting them in parallel, and counting the scores per kind of
//! source.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::BufRead;
use std::sync::Arc;

use kanaemi_engine::{RankingModel, TextDictionary};
use rayon::prelude::*;

use crate::documents::each_document;
use crate::{
    Class, DocumentsError, Established, Score, Split, Tallies, Unit, UnitsError, as_typed,
    engine_with, evaluate_document_with_parts, read_units, split_of,
};

/// A document of the eval split with its units in position order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDocument {
    pub doc_id: String,
    pub source_id: String,
    pub units: Vec<Unit>,
    pub text: String,
    /// The compound units left out of `units`, in position order.
    pub compounds: Vec<Unit>,
    /// The units inside each compound `units` types whole, by where it
    /// starts (see [`compound_parts`]).
    pub parts: HashMap<usize, Vec<Unit>>,
}

/// Where `unit` lies in its document: start and end.
pub fn span_of(unit: &Unit) -> (usize, usize) {
    (unit.position, unit.position + unit.surface.chars().count())
}

/// The units of a document the evaluation types, decided by the analyzer
/// alone so every dictionary is asked the same: each compound typed whole
/// (危機一髪) in place of the units inside it, and every other unit. Of
/// compounds that overlap, the one starting first, then the longest, is
/// typed (朽ち果てた over 朽ち果て); a compound a unit runs out of (社会主義
/// and 主義的) is not, as the typist cannot type both, and its units are
/// typed instead. A word joined with its affixes that no lexicon has and that
/// is not `established` (山田さん) is typed as its parts, as a typist types it,
/// and its parts are what a compound may run out of (社会主義 is typed whole
/// before the 的 of a rare 主義的).
pub fn typed_by_analyzer(
    units: &[Unit],
    compounds: &[Unit],
    established: &Established,
) -> Vec<Unit> {
    let units: Vec<Unit> = units
        .iter()
        .flat_map(|u| as_typed(u, established))
        .collect();
    let mut candidates: Vec<&Unit> = compounds.iter().collect();
    candidates.sort_by_key(|c| {
        let (start, end) = span_of(c);
        (start, std::cmp::Reverse(end))
    });
    let mut whole: Vec<&Unit> = Vec::new();
    let mut taken_to = 0;
    for compound in candidates {
        let (start, end) = span_of(compound);
        // Units do not overlap one another, so their ends rise with their
        // starts.
        let first = units.partition_point(|u| span_of(u).1 <= start);
        let runs_out = units[first..]
            .iter()
            .map(span_of)
            .take_while(|&(s, _)| s < end)
            .any(|(s, e)| s < start || end < e);
        if start >= taken_to && !runs_out {
            taken_to = end;
            whole.push(compound);
        }
    }
    let spans = crate::Spans::new(whole.iter().map(|c| span_of(c)));
    let mut typed: Vec<Unit> = units
        .iter()
        .filter(|u| {
            let (start, end) = span_of(u);
            !spans.covers(start, end)
        })
        .chain(whole)
        .cloned()
        .collect();
    typed.sort_by_key(|u| u.position);
    typed
}

/// The units inside each compound of `typed` by where it starts, as
/// [`typed_by_analyzer`] would type them without it: what a typist types for
/// a compound no dictionary offers (see [`crate::evaluate_document_with_parts`]).
pub fn compound_parts(
    units: &[Unit],
    typed: &[Unit],
    established: &Established,
) -> HashMap<usize, Vec<Unit>> {
    let units: Vec<Unit> = units
        .iter()
        .flat_map(|u| as_typed(u, established))
        .collect();
    typed
        .iter()
        .filter(|u| u.compound)
        .map(|compound| {
            let (start, end) = span_of(compound);
            let first = units.partition_point(|u| u.position < start);
            let inside = units[first..]
                .iter()
                .take_while(|u| span_of(u).1 <= end)
                .cloned()
                .collect();
            (start, inside)
        })
        .collect()
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
/// time. Compounds are left out of the units, as they overlap them, and kept
/// apart. A document without units is no document here.
pub fn each_document_with_units<E: From<EvalDocumentsError>>(
    units: impl BufRead,
    docs: impl BufRead,
    wanted: impl Fn(&str) -> bool,
    mut f: impl FnMut(EvalDocument) -> Result<(), E>,
) -> Result<(), E> {
    let mut units = read_units(units).enumerate();
    let mut previous: Option<(String, usize)> = None;
    let mut pending: Option<Unit> = None;
    // The units of the next wanted document, all of them, and its compounds.
    type Group = (Vec<Unit>, Vec<Unit>);
    let mut next_group = || -> Result<Option<Group>, EvalDocumentsError> {
        // A document's first unit may be a compound: no unit starts it when
        // it opens with kana alone (むっつり右門).
        let mut doc_id: Option<String> = None;
        let mut group: Vec<Unit> = Vec::new();
        let mut compounds: Vec<Unit> = Vec::new();
        let take = |unit: Unit, group: &mut Vec<Unit>, compounds: &mut Vec<Unit>| {
            if unit.compound {
                compounds.push(unit);
            } else {
                group.push(unit);
            }
        };
        if let Some(unit) = pending.take() {
            doc_id = Some(unit.doc_id.clone());
            take(unit, &mut group, &mut compounds);
        }
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
            if !wanted(&unit.doc_id) {
                continue;
            }
            if doc_id.as_ref().is_some_and(|d| *d != unit.doc_id) {
                if !group.is_empty() {
                    pending = Some(unit);
                    return Ok(Some((group, compounds)));
                }
                // The document before had compounds alone.
                compounds.clear();
            }
            doc_id = Some(unit.doc_id.clone());
            take(unit, &mut group, &mut compounds);
        }
        Ok((!group.is_empty()).then_some((group, compounds)))
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
        if let Some((group, _)) = next.take_if(|(g, _)| g[0].doc_id < doc.doc_id) {
            return Err(Failed::Read(EvalDocumentsError::MissingText(
                group[0].doc_id.clone(),
            )));
        }
        if next
            .as_ref()
            .is_some_and(|(g, _)| g[0].doc_id == doc.doc_id)
        {
            let (units, compounds) = next.take().expect("checked");
            next = next_group().map_err(Failed::Read)?;
            f(EvalDocument {
                doc_id: doc.doc_id,
                source_id: doc.source_id,
                units,
                text: doc.text,
                compounds,
                parts: HashMap::new(),
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
        Some((group, _)) => Err(EvalDocumentsError::MissingText(group[0].doc_id.clone()).into()),
        None => Ok(()),
    }
}

/// The tallies per kind of source, the part of a doc ID before its first
/// `:`, and the tally of the units whose reading another analyzer agrees
/// with, of every kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub by_kind: BTreeMap<String, Tallies>,
    pub agreed: Tallies,
    /// The compounds not offered, typed as their parts instead.
    pub split: u64,
}

impl Evaluation {
    /// Every kind's tallies together; the best order is the one for all the
    /// documents.
    pub fn all(&self) -> Tallies {
        let mut all = Tallies::default();
        for tallies in self.by_kind.values() {
            all.merge(tallies.clone());
        }
        all
    }

    /// The table of `build/evaluation.tsv`: a header, then a row per kind
    /// and class, kinds in the order of their UTF-8 bytes, then `all`, then
    /// `agreed` for the units another analyzer reads as the answer does;
    /// classes with `all` last.
    pub fn to_tsv(&self) -> String {
        let all = self.all();
        let (all_name, agreed_name) = ("all".to_owned(), "agreed".to_owned());
        let kinds = self
            .by_kind
            .iter()
            .chain([(&all_name, &all), (&agreed_name, &self.agreed)]);
        let mut tsv = String::from(
            "kind\tclass\tunits\tmissing\tfirst\tbest_first\tloss\tpage\tbest_page\tpage_loss\n",
        );
        for (kind, tallies) in kinds {
            let rows = Class::ALL
                .map(|class| (class.name(), tallies.score(class)))
                .into_iter()
                .chain([("all", tallies.total())]);
            for (class, s) in rows {
                writeln!(
                    tsv,
                    "{kind}\t{class}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    s.units,
                    s.missing,
                    s.first,
                    s.best_first,
                    s.loss(),
                    s.page,
                    s.best_page,
                    s.page_loss(),
                )
                .expect("writing to a String never fails");
            }
        }
        tsv
    }
}

/// Converts every document with engines reading `dictionaries` alone, in
/// their order of priority, ranking with `model` when given, in parallel by
/// chunks of documents, a few chunks per thread. Each chunk has its own
/// engine, all sharing the parsed dictionaries and model. The units `agrees` takes are tallied apart too.
pub fn evaluate_documents(
    docs: &[EvalDocument],
    dictionaries: &[Arc<TextDictionary>],
    model: Option<&Arc<RankingModel>>,
    agrees: &(dyn Fn(&Unit) -> bool + Sync),
) -> Evaluation {
    let chunk = docs.len().div_ceil(rayon::current_num_threads() * 4).max(1);
    let parts: Vec<Evaluation> = docs
        .par_chunks(chunk)
        .map(|chunk| {
            let mut field = engine_with(dictionaries);
            field.set_model(model.cloned());
            let mut part = Evaluation::default();
            for doc in chunk {
                let kind = doc.doc_id.split(':').next().unwrap_or_default();
                let tallies = part.by_kind.entry(kind.to_owned()).or_default();
                let agreed = &mut part.agreed;
                part.split += evaluate_document_with_parts(
                    &mut field,
                    &doc.units,
                    &doc.parts,
                    &doc.text,
                    |unit, offered, rank| {
                        tallies.add(unit, offered, rank);
                        if agrees(unit) {
                            agreed.add(unit, offered, rank);
                        }
                    },
                );
            }
            part
        })
        .collect();
    let mut evaluation = Evaluation::default();
    for part in parts {
        for (kind, tallies) in part.by_kind {
            evaluation.by_kind.entry(kind).or_default().merge(tallies);
        }
        evaluation.agreed.merge(part.agreed);
        evaluation.split += part.split;
    }
    evaluation
}

/// One key's line of [`Tallies::keys_tsv`].
struct KeyLine {
    units: u64,
    missing: u64,
    loss: i64,
}

fn key_lines(tsv: &str) -> BTreeMap<(String, String), KeyLine> {
    tsv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let n = |i: usize| f.get(i)?.parse::<u64>().ok();
            let score = Score {
                units: n(2)?,
                missing: n(3)?,
                first: n(4)?,
                best_first: n(5)?,
                ..Score::default()
            };
            Some((
                (f[0].to_owned(), f[1].to_owned()),
                KeyLine {
                    units: score.units,
                    missing: score.missing,
                    loss: score.loss(),
                },
            ))
        })
        .collect()
}

/// What changed from `old` to `new`, both tables of every key: the keys
/// whose units not offered or loss changed, with both, the greatest change
/// first. A key in one table only counts as nothing in the other.
pub fn keys_diff_tsv(old: &str, new: &str) -> String {
    let (old, new) = (key_lines(old), key_lines(new));
    let none = KeyLine {
        units: 0,
        missing: 0,
        loss: 0,
    };
    let mut rows: Vec<(&(String, String), &KeyLine, &KeyLine)> = old
        .keys()
        .chain(new.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|k| (k, old.get(k).unwrap_or(&none), new.get(k).unwrap_or(&none)))
        .filter(|(_, o, n)| o.missing != n.missing || o.loss != n.loss)
        .collect();
    let change = |o: &KeyLine, n: &KeyLine| o.missing.abs_diff(n.missing) + o.loss.abs_diff(n.loss);
    rows.sort_by(|a, b| {
        change(b.1, b.2)
            .cmp(&change(a.1, a.2))
            .then_with(|| a.0.cmp(b.0))
    });
    let mut tsv = String::from(
        "class\treading\tunits\tmissing_before\tmissing_after\tloss_before\tloss_after\n",
    );
    for ((class, reading), o, n) in rows {
        writeln!(
            tsv,
            "{class}\t{reading}\t{}\t{}\t{}\t{}\t{}",
            n.units.max(o.units),
            o.missing,
            n.missing,
            o.loss,
            n.loss,
        )
        .expect("writing to a String never fails");
    }
    tsv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{engine, evaluate_document};

    // Splits computed apart from this code: aozora:000001 is train,
    // aozora:000013 dev, and aozora:000004 and aozora:000015 eval.
    fn unit(doc_id: &str, position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            particle: None,
            suru: None,
            copula: None,
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
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
                    compounds: Vec::new(),
                    parts: HashMap::new(),
                },
                EvalDocument {
                    doc_id: "aozora:000015".into(),
                    source_id: "aozora-text".into(),
                    units: units[4..].to_vec(),
                    text: "あ書".into(),
                    compounds: Vec::new(),
                    parts: HashMap::new(),
                },
            ]
        );
    }

    #[test]
    fn compounds_are_left_out_of_the_documents_units_but_kept_apart() {
        let parts = [
            unit("aozora:000004", 0, "ろてん", "露天"),
            unit("aozora:000004", 2, "ふろ", "風呂"),
        ];
        let compound = Unit {
            compound: true,
            okurigana_variant: false,
            parts: Vec::new(),
            ..unit("aozora:000004", 0, "ろてんぶろ", "露天風呂")
        };
        let units = [parts[0].clone(), compound, parts[1].clone()];
        let docs = docs_jsonl(&[("aozora:000004", "露天風呂")]);

        let read = eval_documents(units_jsonl(&units).as_bytes(), docs.as_bytes()).unwrap();

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].units, parts);
        assert_eq!(read[0].compounds, [units[1].clone()]);
    }

    #[test]
    fn every_compound_is_typed_whole_in_place_of_its_parts() {
        let compound = |position, reading: &str, surface: &str| Unit {
            compound: true,
            ..unit("aozora:000004", position, reading, surface)
        };
        let crisis = compound(0, "ききいっぱつ", "危機一髪");
        let bath = compound(5, "ろてんぶろ", "露天風呂");
        let units = [
            unit("aozora:000004", 0, "きき", "危機"),
            unit("aozora:000004", 2, "いっぱつ", "一髪"),
            unit("aozora:000004", 4, "と", "と"),
            unit("aozora:000004", 5, "ろてん", "露天"),
            unit("aozora:000004", 7, "ふろ", "風呂"),
        ];

        let typed = typed_by_analyzer(
            &units,
            &[crisis.clone(), bath.clone()],
            &Established::default(),
        );

        assert_eq!(typed, [crisis, units[2].clone(), bath]);
    }

    #[test]
    fn a_word_joined_with_affixes_no_lexicon_has_is_typed_as_its_parts() {
        let joined = Unit {
            parts: vec![crate::Part {
                offset: 0,
                reading: "やまだ".into(),
                surface: "山田".into(),
                ..Default::default()
            }],
            ..unit("aozora:000004", 3, "やまださん", "山田さん")
        };
        let units = [unit("aozora:000004", 0, "おおずもう", "大相撲"), joined];

        let typed = typed_by_analyzer(&units, &[], &Established::default());

        assert_eq!(
            typed,
            [units[0].clone(), unit("aozora:000004", 3, "やまだ", "山田"),]
        );
    }

    #[test]
    fn a_compound_is_typed_whole_when_only_a_rare_joined_word_it_splits_runs_out_of_it() {
        let joined = Unit {
            parts: vec![
                crate::Part {
                    offset: 0,
                    reading: "しゅぎ".into(),
                    surface: "主義".into(),
                    ..Default::default()
                },
                crate::Part {
                    offset: 2,
                    reading: "てき".into(),
                    surface: "的".into(),
                    ..Default::default()
                },
            ],
            ..unit("aozora:000004", 2, "しゅぎてき", "主義的")
        };
        let units = [unit("aozora:000004", 0, "しゃかい", "社会"), joined];
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "しゃかいしゅぎ", "社会主義")
        };

        let typed = typed_by_analyzer(
            &units,
            std::slice::from_ref(&compound),
            &Established::default(),
        );

        assert_eq!(typed, [compound, unit("aozora:000004", 4, "てき", "的")]);
    }

    #[test]
    fn the_parts_of_a_compound_typed_whole_are_the_units_inside_it() {
        let units = [
            unit("aozora:000004", 0, "しゃかい", "社会"),
            unit("aozora:000004", 2, "しゅぎ", "主義"),
            unit("aozora:000004", 4, "てき", "的"),
        ];
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "しゃかいしゅぎ", "社会主義")
        };
        let typed = typed_by_analyzer(
            &units,
            std::slice::from_ref(&compound),
            &Established::default(),
        );

        let parts = compound_parts(&units, &typed, &Established::default());

        assert_eq!(parts, HashMap::from([(0, units[..2].to_vec())]));
    }

    #[test]
    fn of_compounds_that_overlap_the_longest_first_is_typed_whole() {
        let compound = |position, reading: &str, surface: &str| Unit {
            compound: true,
            ..unit("aozora:000004", position, reading, surface)
        };
        let joined = compound(0, "くちはてた", "朽ち果てた");
        let units = [
            unit("aozora:000004", 0, "くち", "朽ち"),
            unit("aozora:000004", 2, "はてた", "果てた"),
        ];

        let typed = typed_by_analyzer(
            &units,
            &[joined.clone(), compound(0, "くちはて", "朽ち果て")],
            &Established::default(),
        );

        assert_eq!(typed, [joined]);
    }

    #[test]
    fn a_compound_a_unit_runs_out_of_is_typed_unit_by_unit() {
        let units = [
            unit("aozora:000004", 0, "しゃかい", "社会"),
            unit("aozora:000004", 2, "しゅぎてき", "主義的"),
        ];
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "しゃかいしゅぎ", "社会主義")
        };

        let typed = typed_by_analyzer(&units, &[compound], &Established::default());

        assert_eq!(typed, units);
    }

    #[test]
    fn a_compound_that_opens_a_document_keeps_its_place() {
        // むっつり is kana alone, so no unit starts the compound before it.
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "むっつりうもん", "むっつり右門")
        };
        let units = [
            unit("aozora:000001", 0, "てがみ", "手紙"),
            compound,
            unit("aozora:000004", 4, "うもん", "右門"),
        ];
        let docs = docs_jsonl(&[("aozora:000001", "手紙"), ("aozora:000004", "むっつり右門")]);
        let mut seen = Vec::new();

        each_document_with_units(
            units_jsonl(&units).as_bytes(),
            docs.as_bytes(),
            |_| true,
            |doc| {
                seen.push(doc);
                Ok::<_, EvalDocumentsError>(())
            },
        )
        .unwrap();

        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].compounds, []);
        assert_eq!(seen[1].units, units[2..]);
        assert_eq!(seen[1].compounds, [units[1].clone()]);
    }

    #[test]
    fn a_document_of_compounds_alone_is_no_document() {
        let compound = Unit {
            compound: true,
            ..unit("aozora:000004", 0, "むっつりうもん", "むっつり右門")
        };
        let units = [compound, unit("aozora:000015", 0, "しょ", "書")];
        let docs = docs_jsonl(&[("aozora:000004", "むっつり右門"), ("aozora:000015", "書")]);
        let mut seen = Vec::new();

        each_document_with_units(
            units_jsonl(&units).as_bytes(),
            docs.as_bytes(),
            |_| true,
            |doc| {
                seen.push(doc);
                Ok::<_, EvalDocumentsError>(())
            },
        )
        .unwrap();

        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].doc_id, "aozora:000015");
        assert_eq!(seen[0].compounds, []);
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
                    compounds: Vec::new(),
                    parts: HashMap::new(),
                },
                EvalDocument {
                    doc_id: "aozora:000004".into(),
                    source_id: "aozora-text".into(),
                    units: units[1..3].to_vec(),
                    text: "漢い字".into(),
                    compounds: Vec::new(),
                    parts: HashMap::new(),
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

    fn eval_document(doc_id: &str, units: &[(&str, &str)]) -> EvalDocument {
        EvalDocument {
            doc_id: doc_id.into(),
            source_id: "aozora-text".into(),
            units: units
                .iter()
                .map(|(reading, surface)| unit(doc_id, 0, reading, surface))
                .collect(),
            text: String::new(),
            compounds: Vec::new(),
            parts: HashMap::new(),
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
            .install(|| {
                evaluate_documents(&docs, std::slice::from_ref(&dictionary), None, &|_| false)
            });

        let mut alone: BTreeMap<String, Tallies> = BTreeMap::new();
        for doc in &docs {
            let kind = doc.doc_id.split(':').next().unwrap();
            let tallies = alone.entry(kind.to_owned()).or_default();
            evaluate_document(
                &mut engine(dictionary.clone()),
                &doc.units,
                &doc.text,
                |u, o, r| tallies.add(u, o, r),
            );
        }
        assert_eq!(evaluation.by_kind, alone);
        assert_eq!(
            evaluation.all().total().units,
            docs.iter().map(|d| d.units.len() as u64).sum::<u64>()
        );
    }

    #[test]
    fn the_table_has_a_row_per_kind_and_class_with_all_last() {
        let order =
            |surfaces: &[&str]| -> Vec<String> { surfaces.iter().map(|s| (*s).into()).collect() };
        let mut law = Tallies::default();
        let letters = order(&["手神", "手紙"]);
        for surface in ["手紙", "手紙", "手神"] {
            let rank = letters.iter().position(|s| s == surface).map(|i| i + 1);
            law.add(&unit("law:1", 0, "てがみ", surface), &letters, rank);
        }
        let three = Unit {
            numeric: Some(crate::Numeric {
                reading: "{}ぼん".into(),
                surface: "{half-num}本".into(),
                value: "3".into(),
                kana: String::new(),
            }),
            ..unit("law:1", 0, "3ぼん", "3本")
        };
        law.add(&three, &order(&["三本"]), None);
        let mut aozora = Tallies::default();
        aozora.add(&unit("aozora:1", 0, "ばく", "獏"), &order(&["バク"]), None);
        let evaluation = Evaluation {
            by_kind: BTreeMap::from([("law".to_owned(), law), ("aozora".to_owned(), aozora)]),
            agreed: Tallies::default(),
            split: 0,
        };

        assert_eq!(
            evaluation.to_tsv(),
            "kind\tclass\tunits\tmissing\tfirst\tbest_first\tloss\tpage\tbest_page\tpage_loss\n\
             aozora\tword\t1\t1\t0\t0\t0\t0\t0\t0\n\
             aozora\tcompound\t0\t0\t0\t0\t0\t0\t0\t0\n\
             aozora\tnumeric\t0\t0\t0\t0\t0\t0\t0\t0\n\
             aozora\tvariant\t0\t0\t0\t0\t0\t0\t0\t0\n\
             aozora\tall\t1\t1\t0\t0\t0\t0\t0\t0\n\
             law\tword\t3\t0\t1\t2\t1\t3\t3\t0\n\
             law\tcompound\t0\t0\t0\t0\t0\t0\t0\t0\n\
             law\tnumeric\t1\t1\t0\t0\t0\t0\t0\t0\n\
             law\tvariant\t0\t0\t0\t0\t0\t0\t0\t0\n\
             law\tall\t4\t1\t1\t2\t1\t3\t3\t0\n\
             all\tword\t4\t1\t1\t2\t1\t3\t3\t0\n\
             all\tcompound\t0\t0\t0\t0\t0\t0\t0\t0\n\
             all\tnumeric\t1\t1\t0\t0\t0\t0\t0\t0\n\
             all\tvariant\t0\t0\t0\t0\t0\t0\t0\t0\n\
             all\tall\t5\t2\t1\t2\t1\t3\t3\t0\n\
             agreed\tword\t0\t0\t0\t0\t0\t0\t0\t0\n\
             agreed\tcompound\t0\t0\t0\t0\t0\t0\t0\t0\n\
             agreed\tnumeric\t0\t0\t0\t0\t0\t0\t0\t0\n\
             agreed\tvariant\t0\t0\t0\t0\t0\t0\t0\t0\n\
             agreed\tall\t0\t0\t0\t0\t0\t0\t0\t0\n"
        );
    }

    #[test]
    fn the_diff_lists_the_keys_whose_missing_or_loss_changed_the_greatest_first() {
        let header = "class\treading\tunits\tmissing\tfirst\tbest_first\tpage\tbest_page\n";
        let old = format!(
            "{header}word\tてがみ\t10\t0\t5\t8\t10\t10\n\
             word\tばく\t3\t3\t0\t0\t0\t0\n\
             word\tほん\t4\t0\t4\t4\t4\t4\n"
        );
        let new = format!(
            "{header}word\tてがみ\t10\t0\t8\t8\t10\t10\n\
             word\tばく\t3\t1\t2\t2\t2\t2\n\
             word\tほん\t4\t0\t4\t4\t4\t4\n\
             compound\tろてんぶろ\t2\t2\t0\t0\t0\t0\n"
        );

        assert_eq!(
            keys_diff_tsv(&old, &new),
            "class\treading\tunits\tmissing_before\tmissing_after\tloss_before\tloss_after\n\
             word\tてがみ\t10\t0\t0\t3\t0\n\
             compound\tろてんぶろ\t2\t0\t2\t0\t0\n\
             word\tばく\t3\t3\t1\t0\t0\n"
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
                particle: None,
                suru: None,
                copula: None,
                compound: false,
                okurigana_variant: false,
                parts: Vec::new(),
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
            compounds: Vec::new(),
            parts: HashMap::new(),
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

        let without = evaluate_documents(
            std::slice::from_ref(&doc),
            std::slice::from_ref(&dictionary),
            None,
            &|_| false,
        );
        let with = evaluate_documents(
            &[doc],
            std::slice::from_ref(&dictionary),
            Some(&model),
            &|_| false,
        );

        assert_eq!(without.all().total().first, 0);
        assert_eq!(with.all().total().first, 1);
    }
}

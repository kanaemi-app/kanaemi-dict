//! Cutting every document of `build/docs.jsonl` into units.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Write};

use rayon::prelude::*;

use crate::units::each_unit;
use crate::{AnalyzerError, UnidicReadings, Words};

/// One line of `build/docs.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct Document {
    pub(crate) doc_id: String,
    pub(crate) source_id: String,
    pub(crate) text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentsError {
    #[error("line {line}: {source}")]
    Read { line: usize, source: io::Error },
    #[error("line {line}: {source}")]
    Parse {
        line: usize,
        source: serde_json::Error,
    },
    /// Documents are batched in the order they come, so the order of the
    /// units written follows the order of the documents read.
    #[error(
        "line {line}: {doc_id} does not come after {previous}; documents must be sorted by doc_id"
    )]
    Unsorted {
        line: usize,
        doc_id: String,
        previous: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CutError {
    #[error("reading the documents: {0}")]
    Documents(#[from] DocumentsError),
    #[error("analyzing {doc_id}: {source}")]
    Analyzer {
        doc_id: String,
        source: AnalyzerError,
    },
    #[error("writing the units: {0}")]
    Write(#[from] io::Error),
}

/// What [`cut_documents`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CutSummary {
    pub documents: usize,
    /// Units by the kind of document, the part of its doc ID before the
    /// first `:`.
    pub units_by_kind: BTreeMap<String, usize>,
}

impl CutSummary {
    pub fn units(&self) -> usize {
        self.units_by_kind.values().sum()
    }

    fn count(&mut self, doc_id: &str, units: usize) {
        let kind = doc_id.split(':').next().unwrap_or_default();
        *self.units_by_kind.entry(kind.to_owned()).or_default() += units;
    }
}

/// Documents are cut a batch at a time; a batch closes once its text reaches
/// this many bytes, and a document this long is cut on its own, so the units
/// held at once stay bounded however long the documents are.
const BATCH_BYTES: usize = 4_000_000;

/// Cuts every document of `docs`, JSON Lines sorted by doc ID, into units read
/// with `tokenize`, the words joined there checked against `readings`, and
/// writes them to `out` as JSON Lines in document order.
pub fn cut_documents(
    docs: impl BufRead,
    tokenize: impl Fn(&str) -> Result<Words, AnalyzerError> + Sync,
    readings: &UnidicReadings,
    out: impl Write,
) -> Result<CutSummary, CutError> {
    cut_in_batches(docs, &tokenize, readings, out, BATCH_BYTES)
}

fn cut_in_batches(
    docs: impl BufRead,
    tokenize: &(impl Fn(&str) -> Result<Words, AnalyzerError> + Sync),
    readings: &UnidicReadings,
    mut out: impl Write,
    batch_bytes: usize,
) -> Result<CutSummary, CutError> {
    let mut summary = CutSummary::default();
    let mut batch = Vec::new();
    let mut held = 0usize;
    each_document(docs, |d| {
        summary.documents += 1;
        if d.text.len() >= batch_bytes {
            cut_batch(&batch, tokenize, readings, &mut out, &mut summary)?;
            batch.clear();
            held = 0;
            return cut_alone(&d, tokenize, readings, &mut out, &mut summary);
        }
        held += d.text.len();
        batch.push(d);
        if held >= batch_bytes {
            cut_batch(&batch, tokenize, readings, &mut out, &mut summary)?;
            batch.clear();
            held = 0;
        }
        Ok(())
    })?;
    cut_batch(&batch, tokenize, readings, &mut out, &mut summary)?;
    out.flush()?;
    Ok(summary)
}

/// Hands each document of `docs`, JSON Lines sorted by doc ID, to `f` in order.
pub(crate) fn each_document<E: From<DocumentsError>>(
    docs: impl BufRead,
    mut f: impl FnMut(Document) -> Result<(), E>,
) -> Result<(), E> {
    let mut previous: Option<String> = None;
    for (i, line) in docs.lines().enumerate() {
        let line_no = i + 1;
        let line = line.map_err(|source| DocumentsError::Read {
            line: line_no,
            source,
        })?;
        let doc: Document =
            serde_json::from_str(&line).map_err(|source| DocumentsError::Parse {
                line: line_no,
                source,
            })?;
        if let Some(previous) = previous.take_if(|previous| *previous >= doc.doc_id) {
            return Err(DocumentsError::Unsorted {
                line: line_no,
                doc_id: doc.doc_id,
                previous,
            }
            .into());
        }
        previous = Some(doc.doc_id.clone());
        f(doc)?;
    }
    Ok(())
}

/// Cuts a batch of documents in parallel and writes their units in order.
/// Each document's units are held only as their JSON lines.
fn cut_batch(
    batch: &[Document],
    tokenize: &(impl Fn(&str) -> Result<Words, AnalyzerError> + Sync),
    readings: &UnidicReadings,
    out: &mut impl Write,
    summary: &mut CutSummary,
) -> Result<(), CutError> {
    let per_doc: Vec<(Vec<u8>, usize)> = batch
        .par_iter()
        .map(|d| {
            let mut lines = Vec::new();
            let mut n = 0;
            each_unit(&d.doc_id, &d.text, tokenize, readings, |unit| {
                serde_json::to_writer(&mut lines, &unit).expect("a unit serializes into memory");
                lines.push(b'\n');
                n += 1;
            })
            .map_err(|source| CutError::Analyzer {
                doc_id: d.doc_id.clone(),
                source,
            })?;
            Ok((lines, n))
        })
        .collect::<Result<_, CutError>>()?;
    for (doc, (lines, n)) in batch.iter().zip(per_doc) {
        out.write_all(&lines)?;
        summary.count(&doc.doc_id, n);
    }
    Ok(())
}

/// Cuts a document too long to batch on its own, writing each unit as it is
/// cut instead of holding them.
fn cut_alone(
    doc: &Document,
    tokenize: &impl Fn(&str) -> Result<Words, AnalyzerError>,
    readings: &UnidicReadings,
    out: &mut impl Write,
    summary: &mut CutSummary,
) -> Result<(), CutError> {
    let mut failure = None;
    let mut n = 0;
    each_unit(&doc.doc_id, &doc.text, tokenize, readings, |unit| {
        if failure.is_none() {
            failure = serde_json::to_writer(&mut *out, &unit)
                .map_err(io::Error::from)
                .and_then(|()| out.write_all(b"\n"))
                .err();
        }
        n += 1;
    })
    .map_err(|source| CutError::Analyzer {
        doc_id: doc.doc_id.clone(),
        source,
    })?;
    if let Some(e) = failure {
        return Err(e.into());
    }
    summary.count(&doc.doc_id, n);
    Ok(())
}

/// The source IDs of the documents of `docs`, JSON Lines sorted by doc ID,
/// whose doc ID is `wanted`, each once.
pub fn document_sources(
    docs: impl BufRead,
    wanted: impl Fn(&str) -> bool,
) -> Result<BTreeSet<String>, DocumentsError> {
    let mut sources = BTreeSet::new();
    each_document(docs, |d| {
        if wanted(&d.doc_id) {
            sources.insert(d.source_id);
        }
        Ok::<_, DocumentsError>(())
    })?;
    Ok(sources)
}

/// The texts of the documents of `docs`, JSON Lines sorted by doc ID, whose
/// doc ID is `wanted`, in order.
pub fn document_texts(
    docs: impl BufRead,
    wanted: impl Fn(&str) -> bool,
) -> Result<Vec<String>, DocumentsError> {
    let mut texts = Vec::new();
    each_document(docs, |d| {
        if wanted(&d.doc_id) {
            texts.push(d.text);
        }
        Ok::<_, DocumentsError>(())
    })?;
    Ok(texts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Token, Unit};

    /// One noun per kanji, each read as "よみ".
    fn tokenize(text: &str) -> Result<Words, AnalyzerError> {
        Ok(Words::from(
            text.chars()
                .enumerate()
                .filter(|(_, c)| crate::kana::is_kanji(*c))
                .map(|(begin, c)| Token {
                    surface: c.to_string(),
                    reading: "よみ".into(),
                    pos: ["名詞", "普通名詞", "一般", "*", "*", "*"]
                        .map(str::to_owned)
                        .to_vec(),
                    dictionary_form: c.to_string(),
                    normalized_form: c.to_string(),
                    begin,
                })
                .collect::<Vec<_>>(),
        ))
    }

    fn jsonl(docs: &[(&str, &str)]) -> String {
        docs.iter()
            .map(|(doc_id, text)| {
                let doc = serde_json::json!({ "doc_id": doc_id, "source_id": "s", "text": text });
                format!("{doc}\n")
            })
            .collect()
    }

    fn cut(docs: &str, batch_bytes: usize) -> Result<(Vec<Unit>, CutSummary), CutError> {
        let mut out = Vec::new();
        let summary = cut_in_batches(
            docs.as_bytes(),
            &tokenize,
            &UnidicReadings::default(),
            &mut out,
            batch_bytes,
        )?;
        let units = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        Ok((units, summary))
    }

    fn view(units: &[Unit]) -> Vec<(&str, usize, &str)> {
        units
            .iter()
            .map(|u| (u.doc_id.as_str(), u.position, u.surface.as_str()))
            .collect()
    }

    #[test]
    fn every_document_s_units_are_written_in_document_order_and_counted_by_kind() {
        let docs = jsonl(&[("aozora:1", "あ漢"), ("aozora:2", "字"), ("law:1", "法")]);

        let (units, summary) = cut(&docs, BATCH_BYTES).unwrap();

        assert_eq!(
            view(&units),
            [
                ("aozora:1", 1, "漢"),
                ("aozora:2", 0, "字"),
                ("law:1", 0, "法")
            ]
        );
        assert_eq!(summary.documents, 3);
        assert_eq!(
            summary.units_by_kind,
            BTreeMap::from([("aozora".to_owned(), 2), ("law".to_owned(), 1)])
        );
        assert_eq!(summary.units(), 3);
    }

    #[test]
    fn small_batches_and_documents_cut_alone_keep_the_document_order() {
        let long = format!("{}長", "あ".repeat(10));
        let docs = jsonl(&[("a:1", "一"), ("a:2", "二"), ("a:3", &long), ("a:4", "四")]);

        let (units, summary) = cut(&docs, 6).unwrap();

        assert_eq!(
            view(&units),
            [
                ("a:1", 0, "一"),
                ("a:2", 0, "二"),
                ("a:3", 10, "長"),
                ("a:4", 0, "四")
            ]
        );
        assert_eq!(summary.units(), 4);
    }

    #[test]
    fn documents_out_of_doc_id_order_are_an_error() {
        let docs = jsonl(&[("b:1", "一"), ("a:1", "二")]);

        let err = cut(&docs, BATCH_BYTES).unwrap_err();

        assert!(
            matches!(
                &err,
                CutError::Documents(DocumentsError::Unsorted { line: 2, doc_id, previous })
                    if doc_id == "a:1" && previous == "b:1"
            ),
            "{err}"
        );
    }

    #[test]
    fn a_repeated_doc_id_is_an_error() {
        let docs = jsonl(&[("a:1", "一"), ("a:1", "二")]);

        let err = cut(&docs, BATCH_BYTES).unwrap_err();

        assert!(
            matches!(err, CutError::Documents(DocumentsError::Unsorted { .. })),
            "{err}"
        );
    }

    #[test]
    fn a_line_that_is_not_a_document_is_an_error_with_its_line_number() {
        let docs = format!("{}{{\"doc_id\":\"b\"}}\n", jsonl(&[("a", "一")]));

        let err = cut(&docs, BATCH_BYTES).unwrap_err();

        assert!(
            matches!(
                err,
                CutError::Documents(DocumentsError::Parse { line: 2, .. })
            ),
            "{err}"
        );
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn the_texts_of_the_wanted_documents_come_in_order() {
        let docs = concat!(
            r#"{"doc_id":"a:1","source_id":"s","text":"一"}"#,
            "\n",
            r#"{"doc_id":"b:2","source_id":"s","text":"二"}"#,
            "\n",
            r#"{"doc_id":"b:3","source_id":"s","text":"三"}"#,
            "\n",
        );

        let texts = document_texts(docs.as_bytes(), |id| id.starts_with("b:")).unwrap();

        assert_eq!(texts, ["二", "三"]);
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn the_sources_of_the_wanted_documents_come_once_each() {
        let docs = concat!(
            r#"{"doc_id":"a:1","source_id":"aozora-text","text":"一"}"#,
            "\n",
            r#"{"doc_id":"b:2","source_id":"hatena-hotentry:20260101","text":"二"}"#,
            "\n",
            r#"{"doc_id":"b:3","source_id":"hatena-hotentry:20260101","text":"三"}"#,
            "\n",
        );

        let sources = document_sources(docs.as_bytes(), |id| id.starts_with("b:")).unwrap();

        assert_eq!(
            sources,
            ["hatena-hotentry:20260101"].map(String::from).into()
        );
    }
}

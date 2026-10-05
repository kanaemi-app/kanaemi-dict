//! `kanaemi-dict`: cuts the collected documents into units, builds the base
//! dictionary from them, and evaluates it, reading and writing under the
//! current directory.

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use kanaemi_dict::{
    Analyzer, AnalyzerError, BASE_LABEL, CutError, Dictionary, EvalDocumentsError, RejectedLines,
    Split, UnidicError, UnitsError, WriteError, cut_documents, eval_documents, evaluate_documents,
    plain_words, read_units, split_of, write_atomically,
};
use kanaemi_engine::TextDictionary;

const SYSTEM_DICTIONARY: &str = "build/sudachi/system_full.dic";
const LEXICON: &str = "build/sudachi/small_lex.csv";
const DOCS: &str = "build/docs.jsonl";
const UNITS: &str = "build/units.jsonl";
const EVALUATION: &str = "build/evaluation.tsv";

/// Where a base dictionary build writes.
struct Outputs {
    dictionary: &'static str,
    report: &'static str,
}

const BASE: Outputs = Outputs {
    dictionary: "build/dictionaries/base.tsv",
    report: "build/dictionaries/base-report.tsv",
};
const BASE_TRAIN: Outputs = Outputs {
    dictionary: "build/dictionaries/base-train.tsv",
    report: "build/dictionaries/base-train-report.tsv",
};

const USAGE: &str = "\
usage: kanaemi-dict units
         cut every document of build/docs.jsonl into build/units.jsonl,
         reading them with build/sudachi/system_full.dic
       kanaemi-dict dictionary [--train-only]
         build build/dictionaries/base.tsv from build/units.jsonl and the
         UniDic lexicon build/sudachi/small_lex.csv, reporting to
         build/dictionaries/base-report.tsv; with --train-only, from the
         units of the train documents alone into
         build/dictionaries/base-train.tsv and base-train-report.tsv
       kanaemi-dict evaluate
         convert the units of the eval documents with
         build/dictionaries/base-train.tsv and write where their surfaces
         rank to build/evaluation.tsv";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Analyzer {
        path: PathBuf,
        source: AnalyzerError,
    },
    #[error("{}: {source}", path.display())]
    Cut { path: PathBuf, source: CutError },
    #[error("{}: {source}", path.display())]
    Unidic { path: PathBuf, source: UnidicError },
    #[error("{}: {source}", path.display())]
    Units { path: PathBuf, source: UnitsError },
    #[error("reading the eval documents from {UNITS} and {DOCS}: {0}")]
    EvalDocuments(#[from] EvalDocumentsError),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error("{} left as it was: {source}", path.display())]
    Rejected {
        path: PathBuf,
        source: RejectedLines,
    },
    #[error("{}: {source}", path.display())]
    Unreadable {
        path: PathBuf,
        source: RejectedLines,
    },
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["units"] => cut_units(),
        ["dictionary"] => build_dictionary(false),
        ["dictionary", "--train-only"] => build_dictionary(true),
        ["evaluate"] => evaluate(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn open(path: &str) -> Result<BufReader<File>, Error> {
    File::open(path)
        .map(BufReader::new)
        .map_err(|source| Error::Read {
            path: path.into(),
            source,
        })
}

/// Cuts every document into units and prints the units per kind of document.
fn cut_units() -> Result<(), Error> {
    let analyzer = Analyzer::open(SYSTEM_DICTIONARY).map_err(|source| Error::Analyzer {
        path: SYSTEM_DICTIONARY.into(),
        source,
    })?;
    let docs = open(DOCS)?;
    let summary = write_atomically(UNITS, |out| {
        cut_documents(docs, |text| analyzer.tokens(text), out).map_err(|source| Error::Cut {
            path: DOCS.into(),
            source,
        })
    })?;
    for (kind, n) in &summary.units_by_kind {
        println!("{kind}\t{n}");
    }
    println!(
        "documents: {}, units: {}, out: {UNITS}",
        summary.documents,
        summary.units()
    );
    Ok(())
}

/// Builds the base dictionary, from the units of every document or with
/// `train_only` of the train documents alone, and writes it and its report
/// only when Kanaemi reads every line.
fn build_dictionary(train_only: bool) -> Result<(), Error> {
    let outputs = if train_only { BASE_TRAIN } else { BASE };
    let unidic = plain_words(open(LEXICON)?).map_err(|source| Error::Unidic {
        path: LEXICON.into(),
        source,
    })?;
    let dictionary = {
        let mut failure = None;
        let units = read_units(open(UNITS)?)
            .map_while(|unit| {
                unit.map_err(|source| {
                    failure = Some(Error::Units {
                        path: UNITS.into(),
                        source,
                    })
                })
                .ok()
            })
            .filter(|unit| !train_only || split_of(&unit.doc_id) == Split::Train);
        let dictionary = Dictionary::build(units, &unidic);
        if let Some(e) = failure {
            return Err(e);
        }
        dictionary
    };
    let text = dictionary
        .to_checked_text(BASE_LABEL)
        .map_err(|source| Error::Rejected {
            path: outputs.dictionary.into(),
            source,
        })?;
    write_atomically(outputs.dictionary, |w| {
        w.write_all(text.as_bytes())
            .map_err(write_error(outputs.dictionary))
    })?;
    write_atomically(outputs.report, |w| {
        w.write_all(dictionary.report.to_tsv().as_bytes())
            .map_err(write_error(outputs.report))
    })?;
    println!(
        "entries: {}, okurigana lines: {}, numeric lines: {}, unknown types: {}, disallowed okurigana: {}, out: {}, report: {}",
        dictionary.entries.len(),
        dictionary.okuri.len(),
        dictionary.numeric.len(),
        dictionary.report.unknown_conjugations.len(),
        dictionary.report.disallowed_okurigana.len(),
        outputs.dictionary,
        outputs.report,
    );
    Ok(())
}

/// Converts the units of the eval documents with the dictionary built from
/// the train documents, and writes the scores per kind of source.
fn evaluate() -> Result<(), Error> {
    let path = BASE_TRAIN.dictionary;
    let dictionary = {
        let text = std::fs::read(path).map_err(|source| Error::Read {
            path: path.into(),
            source,
        })?;
        let (dictionary, invalid) = TextDictionary::parse(text);
        if !invalid.is_empty() {
            return Err(Error::Unreadable {
                path: path.into(),
                source: RejectedLines(invalid),
            });
        }
        Arc::new(dictionary)
    };
    let docs = eval_documents(open(UNITS)?, open(DOCS)?)?;
    let evaluation = evaluate_documents(&docs, &dictionary);
    write_atomically(EVALUATION, |w| {
        w.write_all(evaluation.to_tsv().as_bytes())
            .map_err(write_error(EVALUATION))
    })?;
    for (class, scores) in evaluation.all().classes() {
        for (history, score) in [("off", scores.fresh), ("on", scores.with_history)] {
            println!(
                "{class} history {history}: units {}, covered {:.2}%, first {:.2}%, mean rank {:.3}",
                score.units,
                percent(score.covered, score.units),
                percent(score.first, score.units),
                score.mean_rank(),
            );
        }
    }
    println!("documents: {}, out: {EVALUATION}", docs.len());
    Ok(())
}

fn percent(n: usize, of: usize) -> f64 {
    n as f64 * 100.0 / of.max(1) as f64
}

fn write_error(path: &str) -> impl FnOnce(io::Error) -> Error {
    move |source| {
        Error::Write(WriteError {
            path: path.into(),
            source,
        })
    }
}

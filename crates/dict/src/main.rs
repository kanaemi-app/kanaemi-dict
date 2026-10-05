//! `kanaemi-dict`: cuts the collected documents into units, builds the base
//! dictionary from them and evaluates it, and builds the additional
//! dictionaries, reading and writing under the current directory.

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use kanaemi_dict::{
    Analyzer, AnalyzerError, BASE_LABEL, Base, CutError, Dictionary, DocumentsError,
    EvalDocumentsError, RejectedLines, Split, TitlesError, UnidicError, Unit, UnitsError,
    WriteError, cut_documents, document_texts, eval_documents, evaluate_documents,
    field_dictionary, place_dictionary, place_names, plain_words, read_titles, read_units,
    split_of, write_atomically, year_dictionary,
};
use kanaemi_engine::TextDictionary;

const SYSTEM_DICTIONARY: &str = "build/sudachi/system_full.dic";
const LEXICON: &str = "build/sudachi/small_lex.csv";
const DOCS: &str = "build/docs.jsonl";
const UNITS: &str = "build/units.jsonl";
const EVALUATION: &str = "build/evaluation.tsv";
const DICTIONARIES: &str = "build/dictionaries";
/// Where each additional dictionary is set up, one directory per dictionary.
const ADDITIONAL: &str = "additional";
/// Where `scripts/additional.ts` leaves what each additional dictionary is built from.
const ADDITIONAL_BUILD: &str = "build/additional";
/// The additional dictionary built from the postal code data, not from documents.
const PLACE: &str = "place";

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
         rank to build/evaluation.tsv
       kanaemi-dict additional [NAME...]
         build the additional dictionaries NAME, or every one under
         additional/, from build/additional/NAME/ into
         build/dictionaries/NAME.tsv and NAME-report.tsv, without what
         build/dictionaries/base.tsv already gives";

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
    #[error("{}: {source}", path.display())]
    Documents {
        path: PathBuf,
        source: DocumentsError,
    },
    #[error("{}: {source}", path.display())]
    Titles { path: PathBuf, source: TitlesError },
    #[error("{}: not a page ID", path.display())]
    Since { path: PathBuf },
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
        ["additional", ref names @ ..] => build_additional(names),
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

fn open_analyzer() -> Result<Analyzer, Error> {
    Analyzer::open(SYSTEM_DICTIONARY).map_err(|source| Error::Analyzer {
        path: SYSTEM_DICTIONARY.into(),
        source,
    })
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
    let analyzer = open_analyzer()?;
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

/// Builds the additional dictionaries `names`, or every one set up under
/// additional/, each without what the base dictionary already gives.
fn build_additional(names: &[&str]) -> Result<(), Error> {
    let names: Vec<String> = if names.is_empty() {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(ADDITIONAL).map_err(read_error(ADDITIONAL))? {
            let entry = entry.map_err(read_error(ADDITIONAL))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // A dot directory is a year's setup left half-written, as
            // scripts/additional.ts skips it too.
            if entry.path().is_dir() && !name.starts_with('.') {
                found.push(name);
            }
        }
        found.sort();
        found
    } else {
        names.iter().map(|&n| n.to_owned()).collect()
    };
    let base = {
        let text = read_to_string(BASE.dictionary)?;
        Base::parse(text).map_err(|source| Error::Unreadable {
            path: BASE.dictionary.into(),
            source,
        })?
    };
    let mut analyzer = None;
    for name in names {
        let label = read_to_string(format!("{ADDITIONAL}/{name}/label.txt"))?;
        let dictionary = if name == PLACE {
            place_dictionary_of(&format!("{ADDITIONAL_BUILD}/{name}/ken_all.csv"))?
        } else {
            if analyzer.is_none() {
                analyzer = Some(open_analyzer()?);
            }
            sourced_dictionary(&name, &base, analyzer.as_ref().unwrap())?
        };
        let dictionary_path = format!("{DICTIONARIES}/{name}.tsv");
        let report_path = format!("{DICTIONARIES}/{name}-report.tsv");
        let text = dictionary.to_text_where(label.trim(), |line, kind| base.keeps(line, kind));
        let (_, invalid) = TextDictionary::parse(&text);
        if !invalid.is_empty() {
            return Err(Error::Rejected {
                path: dictionary_path.into(),
                source: RejectedLines(invalid),
            });
        }
        write_atomically(&dictionary_path, |w| {
            w.write_all(text.as_bytes())
                .map_err(write_error(&dictionary_path))
        })?;
        write_atomically(&report_path, |w| {
            w.write_all(dictionary.report.to_tsv().as_bytes())
                .map_err(write_error(&report_path))
        })?;
        println!(
            "{name}\tlines: {}\tout: {dictionary_path}",
            text.lines().count() - 1
        );
    }
    Ok(())
}

/// The place name dictionary from the postal code data's CSV.
fn place_dictionary_of(path: &str) -> Result<Dictionary, Error> {
    let csv = read_to_string(path)?;
    Ok(place_dictionary(&place_names(&csv), csv.lines().count()))
}

/// The dictionary `name` built from its documents: a year's new words when it
/// names the first article of its year, or else a field's words.
fn sourced_dictionary(name: &str, base: &Base, analyzer: &Analyzer) -> Result<Dictionary, Error> {
    let dir = format!("{ADDITIONAL_BUILD}/{name}");
    let (docs, units_path) = (format!("{dir}/docs.jsonl"), format!("{dir}/units.jsonl"));
    write_atomically(&units_path, |out| {
        cut_documents(open(&docs)?, |text| analyzer.tokens(text), out).map_err(|source| {
            Error::Cut {
                path: docs.clone().into(),
                source,
            }
        })
    })?;
    let units = read_all_units(&units_path)?;
    let texts = document_texts(open(&docs)?, |_| true).map_err(|source| Error::Documents {
        path: docs.clone().into(),
        source,
    })?;
    let titles = {
        let path = format!("{dir}/titles.tsv");
        match File::open(&path) {
            Ok(file) => read_titles(BufReader::new(file)).map_err(|source| Error::Titles {
                path: path.into(),
                source,
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(source) => {
                return Err(Error::Read {
                    path: path.into(),
                    source,
                });
            }
        }
    };
    let since_path = format!("{ADDITIONAL}/{name}/wikipedia-since.txt");
    let since = match std::fs::read_to_string(&since_path) {
        Ok(text) => Some(text.trim().parse::<u64>().map_err(|_| Error::Since {
            path: since_path.into(),
        })?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::Read {
                path: since_path.into(),
                source,
            });
        }
    };
    let Some(since) = since else {
        return Ok(field_dictionary(&units, &titles, &texts));
    };
    // Text from before the year: the works and laws, and the articles
    // created before it.
    let older = document_texts(open(DOCS)?, |doc_id| {
        doc_id.starts_with("aozora:")
            || doc_id.starts_with("law:")
            || doc_id
                .strip_prefix("wikipedia:")
                .and_then(|id| id.parse::<u64>().ok())
                .is_some_and(|id| id < since)
    })
    .map_err(|source| Error::Documents {
        path: DOCS.into(),
        source,
    })?;
    Ok(year_dictionary(&titles, base, &texts, &older, units.len()))
}

fn read_all_units(path: &str) -> Result<Vec<Unit>, Error> {
    read_units(open(path)?)
        .collect::<Result<_, _>>()
        .map_err(|source| Error::Units {
            path: path.into(),
            source,
        })
}

fn read_to_string(path: impl AsRef<str>) -> Result<String, Error> {
    let path = path.as_ref();
    std::fs::read_to_string(path).map_err(read_error(path))
}

fn read_error(path: &str) -> impl FnOnce(io::Error) -> Error {
    move |source| Error::Read {
        path: path.into(),
        source,
    }
}

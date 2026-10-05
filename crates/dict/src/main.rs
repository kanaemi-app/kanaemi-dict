//! `kanaemi-dict`: cuts the collected documents into units, builds the base
//! dictionary from them and evaluates it, and builds the additional
//! dictionaries, reading and writing under the current directory.

use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use kanaemi_dict::{
    Analyzer, AnalyzerError, BASE_LABEL, Base, CutError, Dictionary, DistError, DocumentsError,
    EvalDocument, EvalDocumentsError, Evaluation, ExampleFile, Examples, RejectedLines, Split,
    TitlesError, UnidicError, Unit, UnitsError, WriteError, cut_documents, document_sources,
    document_texts, each_document_with_units, engine, eval_documents, evaluate_documents,
    examples_of_document, field_dictionary, gather, model_file, place_dictionary, place_names,
    plain_words, read_titles, read_units, sources_file, split_of, take, train, write_atomically,
    year_dictionary,
};
use kanaemi_engine::{RankingModel, TextDictionary};
use rayon::prelude::*;

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
/// The sources every dictionary read with the analyzer has: the analyzer's
/// dictionary, and for the base the UniDic lexicon too.
const ANALYZER_SOURCE: &str = "sudachidict-full";
const LEXICON_SOURCE: &str = "sudachidict-small-lex";
/// The source of the article titles and of the place names.
const TITLES_SOURCE: &str = "wikipedia-ja";
const POSTAL_SOURCE: &str = "japanpost-ken-all";
/// Where the dictionaries and the model that ship are kept, and their notices.
const KEPT: &str = "dictionaries";
const NOTICES: &str = "notices";
const DIST: &str = "build/dist";
/// The ranking model, paired with the base dictionary.
const MODEL: &str = "build/dictionaries/base.model";
const RANKING_EVALUATION: &str = "build/ranking-evaluation.tsv";
/// Where the training examples wait, in a file of each run's own, while the
/// model trains.
const RANKING_WORK: &str = "build/ranking";
const MODEL_BITS: u8 = 20;
const EPOCHS: usize = 3;
const RATE: f32 = 0.05;
/// Documents become examples a block at a time; a block closes once its units
/// reach this many or its text [`BLOCK_BYTES`], and training holds one block
/// at a time.
const BLOCK_UNITS: usize = 100_000;
const BLOCK_BYTES: usize = 4_000_000;
/// Units of each field's train documents the model also learns from, the
/// same for every field however much text it has.
const FIELD_UNITS: usize = 250_000;

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
         build/dictionaries/base.tsv already gives
       kanaemi-dict ranking
         train the ranking model on the candidates of
         build/dictionaries/base-train.tsv from the train documents of the
         base and of each field's additional dictionary, write it to
         build/dictionaries/base.model paired with
         build/dictionaries/base.tsv, and measure the eval documents
         without and with it into build/ranking-evaluation.tsv
       kanaemi-dict take
         put the dictionaries and the model of build/dictionaries/ that have
         a sources record into dictionaries/ in place of those there, once
         they check as they would ship
       kanaemi-dict dist
         check dictionaries/ and gather each dictionary with the notice of
         its sources (from notices/) and the license into build/dist/NAME/";

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
    #[error("{}: {source}", path.display())]
    Examples { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Dist(#[from] DistError),
    #[error("{}: Kanaemi does not read the model: {source}", path.display())]
    Model {
        path: PathBuf,
        source: kanaemi_engine::ModelError,
    },
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
        ["ranking"] => train_ranking(),
        ["take"] => take_built(),
        ["dist"] => build_dist(),
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
    if !train_only {
        let mut sources =
            document_sources(open(DOCS)?, |_| true).map_err(|source| Error::Documents {
                path: DOCS.into(),
                source,
            })?;
        sources.extend([ANALYZER_SOURCE, LEXICON_SOURCE].map(String::from));
        write_sources(outputs.dictionary, text.as_bytes(), None, &sources)?;
    }
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
    let dictionary = train_dictionary()?;
    let docs = eval_documents(open(UNITS)?, open(DOCS)?)?;
    let evaluation = evaluate_documents(&docs, &dictionary, None);
    write_atomically(EVALUATION, |w| {
        w.write_all(evaluation.to_tsv().as_bytes())
            .map_err(write_error(EVALUATION))
    })?;
    print_scores("", &evaluation);
    println!("documents: {}, out: {EVALUATION}", docs.len());
    Ok(())
}

/// The base dictionary built from the train documents, as Kanaemi reads it.
fn train_dictionary() -> Result<Arc<TextDictionary>, Error> {
    let path = BASE_TRAIN.dictionary;
    let text = std::fs::read(path).map_err(read_error(path))?;
    let (dictionary, invalid) = TextDictionary::parse(text);
    if !invalid.is_empty() {
        return Err(Error::Unreadable {
            path: path.into(),
            source: RejectedLines(invalid),
        });
    }
    Ok(Arc::new(dictionary))
}

fn print_scores(label: &str, evaluation: &Evaluation) {
    for (class, scores) in evaluation.all().classes() {
        for (history, score) in [("off", scores.fresh), ("on", scores.with_history)] {
            println!(
                "{label}{class} history {history}: units {}, covered {:.2}%, first {:.2}%, mean rank {:.3}",
                score.units,
                percent(score.covered, score.units),
                percent(score.first, score.units),
                score.mean_rank(),
            );
        }
    }
}

/// Trains the ranking model on the candidates the dictionary of the train
/// documents gives, writes it, and measures the eval documents without and
/// with it.
fn train_ranking() -> Result<(), Error> {
    let dictionary = train_dictionary()?;
    let examples_path = format!("{RANKING_WORK}/examples-{}.bin", std::process::id());
    std::fs::create_dir_all(RANKING_WORK).map_err(read_error(RANKING_WORK))?;
    let mut examples = ExampleFile::create(&examples_path).map_err(|source| Error::Examples {
        path: examples_path.clone().into(),
        source,
    })?;
    let base = std::fs::read(BASE.dictionary).map_err(read_error(BASE.dictionary))?;
    let mut taken: HashSet<String> = HashSet::new();
    let mut sources = BTreeSet::from([ANALYZER_SOURCE.to_owned(), LEXICON_SOURCE.to_owned()]);
    let mut add = |units: &str, docs: &str, budget: usize| -> Result<(), Error> {
        let added = add_examples(
            units,
            docs,
            budget,
            &taken,
            &dictionary,
            &mut examples,
            &mut sources,
        )?;
        println!("{docs}: {} documents", added.len());
        taken.extend(added);
        Ok(())
    };
    add(UNITS, DOCS, usize::MAX)?;
    for name in field_names()? {
        let dir = format!("{ADDITIONAL_BUILD}/{name}");
        add(
            &format!("{dir}/units.jsonl"),
            &format!("{dir}/docs.jsonl"),
            FIELD_UNITS,
        )?;
    }
    println!("examples: {}", examples.len());
    let weights = train(&examples, MODEL_BITS, EPOCHS, RATE).map_err(|source| Error::Examples {
        path: examples_path.clone().into(),
        source,
    })?;
    drop(examples);
    let model_bytes = model_file(MODEL_BITS, &weights);
    write_atomically(MODEL, |w| {
        w.write_all(&model_bytes).map_err(write_error(MODEL))
    })?;
    write_sources(MODEL, &model_bytes, Some(&base), &sources)?;
    let model = RankingModel::open(MODEL).map_err(|source| Error::Model {
        path: MODEL.into(),
        source,
    })?;
    let docs = eval_documents(open(UNITS)?, open(DOCS)?)?;
    print_scores(
        "without the model: ",
        &evaluate_documents(&docs, &dictionary, None),
    );
    let with = evaluate_documents(&docs, &dictionary, Some(&Arc::new(model)));
    print_scores("with the model: ", &with);
    write_atomically(RANKING_EVALUATION, |w| {
        w.write_all(with.to_tsv().as_bytes())
            .map_err(write_error(RANKING_EVALUATION))
    })?;
    println!("out: {MODEL}, evaluation: {RANKING_EVALUATION}");
    Ok(())
}

/// The additional dictionaries built from a field's documents, whose writing
/// the model learns too: every one with units but a year's new words.
fn field_names() -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(ADDITIONAL).map_err(read_error(ADDITIONAL))? {
        let name = entry
            .map_err(read_error(ADDITIONAL))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        let year =
            std::path::Path::new(&format!("{ADDITIONAL}/{name}/wikipedia-since.txt")).exists();
        let built =
            std::path::Path::new(&format!("{ADDITIONAL_BUILD}/{name}/units.jsonl")).exists();
        if !name.starts_with('.') && !year && built {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Adds the examples of the train documents of `units` and `docs` that are
/// not `taken`, until their units reach `budget`, a block at a time, and to
/// `sources` their source IDs; returns the doc IDs added.
fn add_examples(
    units: &str,
    docs: &str,
    budget: usize,
    taken: &HashSet<String>,
    dictionary: &Arc<TextDictionary>,
    examples: &mut ExampleFile,
    sources: &mut BTreeSet<String>,
) -> Result<Vec<String>, Error> {
    let mut added = Vec::new();
    let mut units_taken = 0usize;
    let mut batch: Vec<EvalDocument> = Vec::new();
    let (mut batch_units, mut batch_bytes) = (0usize, 0usize);
    let flush = |batch: &mut Vec<EvalDocument>, examples: &mut ExampleFile| {
        let parts: Vec<Examples> = batch
            .par_iter()
            .map_init(
                || engine(dictionary.clone()),
                |engine, doc| examples_of_document(engine, &doc.units, &doc.text),
            )
            .collect();
        let mut block = Examples::default();
        for part in parts {
            block.append(part);
        }
        batch.clear();
        examples.push(&block).map_err(|source| Error::Examples {
            path: examples.path().into(),
            source,
        })
    };
    each_document_with_units(
        open(units)?,
        open(docs)?,
        |doc_id| split_of(doc_id) == Split::Train && !taken.contains(doc_id),
        |doc| {
            if units_taken >= budget {
                return Ok(());
            }
            units_taken += doc.units.len();
            batch_units += doc.units.len();
            batch_bytes += doc.text.len();
            added.push(doc.doc_id.clone());
            sources.insert(doc.source_id.clone());
            batch.push(doc);
            if batch_units >= BLOCK_UNITS || batch_bytes >= BLOCK_BYTES {
                flush(&mut batch, examples)?;
                (batch_units, batch_bytes) = (0, 0);
            }
            Ok::<_, Error>(())
        },
    )?;
    flush(&mut batch, examples)?;
    Ok(added)
}

/// Gathers what ships from dictionaries/ into build/dist/, a folder per
/// dictionary, in place of what was there.
fn build_dist() -> Result<(), Error> {
    let files = gather(KEPT.as_ref(), NOTICES.as_ref())?;
    let partial = format!("{DIST}.{}.tmp", std::process::id());
    let _ = std::fs::remove_dir_all(&partial);
    let written = (|| {
        for (path, bytes) in &files {
            let target = std::path::Path::new(&partial).join(path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir).map_err(write_error(&dir.to_string_lossy()))?;
            }
            std::fs::write(&target, bytes).map_err(write_error(&target.to_string_lossy()))?;
        }
        match std::fs::remove_dir_all(DIST) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(write_error(DIST)(e)),
            _ => std::fs::rename(&partial, DIST).map_err(write_error(DIST)),
        }
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    let mut names: Vec<String> = files
        .iter()
        .filter_map(|(path, _)| Some(path.parent()?.to_string_lossy().into_owned()))
        .collect();
    names.dedup();
    println!("dictionaries: {}, out: {DIST}", names.join(", "));
    Ok(())
}

/// Puts the built dictionaries and model into dictionaries/, once they check
/// as they would ship.
fn take_built() -> Result<(), Error> {
    let taken = take(DICTIONARIES.as_ref(), KEPT.as_ref(), NOTICES.as_ref())?;
    println!("taken: {}, out: {KEPT}", taken.join(", "));
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
        let (dictionary, sources) = if name == PLACE {
            (
                place_dictionary_of(&format!("{ADDITIONAL_BUILD}/{name}/ken_all.csv"))?,
                BTreeSet::from([POSTAL_SOURCE.to_owned()]),
            )
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
        write_sources(&dictionary_path, text.as_bytes(), None, &sources)?;
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
/// names the first article of its year, or else a field's words; and the
/// sources it was built from.
fn sourced_dictionary(
    name: &str,
    base: &Base,
    analyzer: &Analyzer,
) -> Result<(Dictionary, BTreeSet<String>), Error> {
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
    let mut sources =
        document_sources(open(&docs)?, |_| true).map_err(|source| Error::Documents {
            path: docs.clone().into(),
            source,
        })?;
    sources.insert(ANALYZER_SOURCE.to_owned());
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
        return Ok((field_dictionary(&units, &titles, &texts), sources));
    };
    // Text from before the year: the works and laws, and the articles
    // created before it.
    let older_doc = |doc_id: &str| {
        doc_id.starts_with("aozora:")
            || doc_id.starts_with("law:")
            || doc_id
                .strip_prefix("wikipedia:")
                .and_then(|id| id.parse::<u64>().ok())
                .is_some_and(|id| id < since)
    };
    let documents_error = |source| Error::Documents {
        path: DOCS.into(),
        source,
    };
    let older = document_texts(open(DOCS)?, older_doc).map_err(documents_error)?;
    sources.extend(document_sources(open(DOCS)?, older_doc).map_err(documents_error)?);
    sources.insert(TITLES_SOURCE.to_owned());
    Ok((
        year_dictionary(&titles, base, &texts, &older, units.len()),
        sources,
    ))
}

/// Writes beside `path` the record of the sources its `bytes` were built from.
fn write_sources(
    path: &str,
    bytes: &[u8],
    paired: Option<&[u8]>,
    sources: &BTreeSet<String>,
) -> Result<(), Error> {
    let record = format!("{path}.sources.txt");
    write_atomically(&record, |w| {
        w.write_all(sources_file(bytes, paired, sources).as_bytes())
            .map_err(write_error(&record))
    })
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

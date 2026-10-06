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
    Agreement, Analyzer, AnalyzerError, BASE_LABEL, Base, Corrections, CorrectionsError, CutError,
    Dictionary, DistError, DocumentsError, EvalDocument, EvalDocumentsError, Evaluation,
    ExampleFile, Examples, ReadingsError, RejectedLines, Split, TitlesError, UnidicError,
    UnidicReadings, UnidicWord, Unit, UnitsError, WordCasesError, WordResult, WriteError,
    agreement, check_words, counted_reading, counted_words, counter_words, cut_documents,
    dictionary_lines, document_sources, document_texts, each_document_with_units, engine,
    eval_documents, evaluate_documents, examples_of_document, field_dictionary, gather, is_word,
    model_file, okurigana_dictionary, parse_corrections, parse_word_cases, paths_of,
    place_dictionary, place_names, plain_words, read_titles, read_units, reading_form, sample_tsv,
    shared_readings, sources_file, split_of, take, train, word_misses_tsv, word_scores_tsv,
    write_atomically, year_dictionary,
};
use kanaemi_engine::{RankingModel, TextDictionary};
use rayon::prelude::*;

const SYSTEM_DICTIONARY: &str = "build/sudachi/system_full.dic";
/// SudachiDict small, which checks the analyzer's readings against UniDic.
const CHECKER_DICTIONARY: &str = "build/sudachi/system_small.dic";
const LEXICON: &str = "build/sudachi/small_lex.csv";
/// The words the analyzer reads wrong, applied to what it reads.
const CORRECTIONS: &str = "analyzer/corrections.tsv";
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
/// The additional dictionary of the words the base dictionary's documents
/// spell with other okurigana than the usual, which the base leaves out.
const OKURIGANA: &str = "okurigana";
/// The sources every dictionary read with the analyzer has: the analyzer's
/// dictionary, the dictionary that checks its readings, and the UniDic
/// lexicon they are checked against and the base takes words from.
const ANALYZER_SOURCES: [&str; 3] = [
    "sudachidict-full",
    "sudachidict-small",
    "sudachidict-small-lex",
];
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
/// The word set people wrote, and where its scores and misses go.
const WORDS: &str = "evaluation/words.tsv";
const CHECK_WORDS: &str = "build/check-words.tsv";
const CHECK_WORD_MISSES: &str = "build/check-words-misses.tsv";
/// The model that ships with the base dictionary.
const KEPT_MODEL: &str = "dictionaries/base.model";
const CHECK_SAMPLE: &str = "build/check-sample.tsv";
/// Items sampled from each stratum of the base dictionary and from each
/// additional dictionary.
const SAMPLE_BASE: usize = 60;
const SAMPLE_ADDITIONAL: usize = 15;
const CHECK_READINGS: &str = "build/check-readings.tsv";
/// The paths of each surface MeCab gives, best first.
const MECAB_PATHS: usize = 5;

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
         reading them with build/sudachi/system_full.dic, checking the
         readings against build/sudachi/small_lex.csv with
         build/sudachi/system_small.dic, and applying the corrections of
         analyzer/corrections.tsv
       kanaemi-dict dictionary [--train-only]
         build build/dictionaries/base.tsv from build/units.jsonl and the
         UniDic lexicon build/sudachi/small_lex.csv, with the words of one
         to ten with each counter read by the analyzer, reporting to
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
         its sources (from notices/) and the license into build/dist/NAME/
       kanaemi-dict check-words
         convert the words of evaluation/words.tsv with the base dictionary
         of dictionaries/, without and with its model, and write the scores
         per category to build/check-words.tsv and the words whose right
         surface did not come first to build/check-words-misses.tsv
       kanaemi-dict check-sample
         sample the items of the dictionaries of dictionaries/ by stratum
         into build/check-sample.tsv for people to judge
       kanaemi-dict check-readings
         read the surfaces with kanji of the dictionaries of dictionaries/
         with MeCab and write the items no path of it reads as the
         dictionary does to build/check-readings.tsv";

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
    Corrections {
        path: PathBuf,
        source: CorrectionsError,
    },
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
    #[error("{}: {source}", path.display())]
    WordCases {
        path: PathBuf,
        source: WordCasesError,
    },
    #[error("running mecab: {0}")]
    Mecab(io::Error),
    #[error("mecab: {0}")]
    MecabReadings(#[from] ReadingsError),
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
        ["check-words"] => check_kept_words(),
        ["check-sample"] => sample_kept(),
        ["check-readings"] => check_kept_readings(),
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
    let readings = UnidicReadings::new(unidic_words()?);
    Analyzer::open(
        SYSTEM_DICTIONARY,
        CHECKER_DICTIONARY,
        readings,
        corrections()?,
    )
    .map_err(|source| Error::Analyzer {
        path: SYSTEM_DICTIONARY.into(),
        source,
    })
}

fn corrections() -> Result<Corrections, Error> {
    parse_corrections(read_to_string(CORRECTIONS)?)
        .map(Corrections::new)
        .map_err(|source| Error::Corrections {
            path: CORRECTIONS.into(),
            source,
        })
}

fn unidic_words() -> Result<Vec<UnidicWord>, Error> {
    plain_words(open(LEXICON)?).map_err(|source| Error::Unidic {
        path: LEXICON.into(),
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
        cut_documents(docs, |text| analyzer.words(text), analyzer.readings(), out).map_err(
            |source| Error::Cut {
                path: DOCS.into(),
                source,
            },
        )
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
    let unidic = unidic_words()?;
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
        let mut dictionary = Dictionary::build(units, &unidic);
        if let Some(e) = failure {
            return Err(e);
        }
        let counted = {
            let counters =
                UnidicReadings::new(counter_words(open(LEXICON)?).map_err(|source| {
                    Error::Unidic {
                        path: LEXICON.into(),
                        source,
                    }
                })?);
            let analyzer = open_analyzer()?;
            counted_words(&dictionary.numeric, &counters, |surface| {
                analyzer
                    .tokens(format!("{surface}の"))
                    .map(|tokens| counted_reading(&tokens, surface))
            })
            .map_err(|source| Error::Analyzer {
                path: SYSTEM_DICTIONARY.into(),
                source,
            })?
        };
        dictionary.add_words(counted);
        let corrections = corrections()?;
        dictionary.drop_words(|reading, surface| {
            corrections.drops(surface) || !is_word(reading, surface)
        });
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
        sources.extend(ANALYZER_SOURCES.map(String::from));
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
    parsed_dictionary(BASE_TRAIN.dictionary)
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
    let mut sources = BTreeSet::from(ANALYZER_SOURCES.map(String::from));
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
        .filter(|name| !name.is_empty())
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

/// Converts the word set with the base dictionary that ships, without and
/// with its model.
fn check_kept_words() -> Result<(), Error> {
    let cases = parse_word_cases(read_to_string(WORDS)?).map_err(|source| Error::WordCases {
        path: WORDS.into(),
        source,
    })?;
    let dictionary = parsed_dictionary(&format!("{KEPT}/base.tsv"))?;
    let model = RankingModel::open(KEPT_MODEL).map_err(|source| Error::Model {
        path: KEPT_MODEL.into(),
        source,
    })?;
    let off = check_words(&cases, &dictionary, None);
    let on = check_words(&cases, &dictionary, Some(&Arc::new(model)));
    let runs: [(&str, &[WordResult]); 2] = [("off", &off), ("on", &on)];
    for (path, tsv) in [
        (CHECK_WORDS, word_scores_tsv(&cases, &runs)),
        (CHECK_WORD_MISSES, word_misses_tsv(&cases, &runs)),
    ] {
        write_atomically(path, |w| {
            w.write_all(tsv.as_bytes()).map_err(write_error(path))
        })?;
    }
    for (label, results) in runs {
        let first = results.iter().filter(|r| r.rank == Some(1)).count();
        let covered = results.iter().filter(|r| r.rank.is_some()).count();
        println!(
            "model {label}: words {}, covered {:.2}%, first {:.2}%",
            results.len(),
            percent(covered, results.len()),
            percent(first, results.len()),
        );
    }
    println!("out: {CHECK_WORDS}, misses: {CHECK_WORD_MISSES}");
    Ok(())
}

/// A dictionary as Kanaemi reads it, failing on any line it rejects.
fn parsed_dictionary(path: &str) -> Result<Arc<TextDictionary>, Error> {
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

/// The dictionaries that ship, by name, the base first and the rest in the
/// order of their names.
fn kept_dictionaries() -> Result<Vec<(String, String)>, Error> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(KEPT).map_err(read_error(KEPT))? {
        let path = entry.map_err(read_error(KEPT))?.path();
        if path.extension().is_some_and(|e| e == "tsv")
            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(name.to_owned());
        }
    }
    names.sort_by_key(|name| (name != "base", name.clone()));
    names
        .into_iter()
        .map(|name| {
            let text = read_to_string(format!("{KEPT}/{name}.tsv"))?;
            Ok((name, text))
        })
        .collect()
}

/// Samples the items of the dictionaries that ship for people to judge.
fn sample_kept() -> Result<(), Error> {
    let dictionaries = kept_dictionaries()?;
    let borrowed: Vec<(&str, &str)> = dictionaries
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let tsv = sample_tsv(&borrowed, SAMPLE_BASE, SAMPLE_ADDITIONAL);
    write_atomically(CHECK_SAMPLE, |w| {
        w.write_all(tsv.as_bytes())
            .map_err(write_error(CHECK_SAMPLE))
    })?;
    println!("items: {}, out: {CHECK_SAMPLE}", tsv.lines().count() - 1);
    Ok(())
}

/// Reads the surfaces with kanji of the dictionaries that ship with MeCab,
/// and lists the items no path of it reads as the dictionary does.
fn check_kept_readings() -> Result<(), Error> {
    let dictionaries = kept_dictionaries()?;
    let mut items = Vec::new();
    let mut forms: Vec<String> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (name, text) in &dictionaries {
        for (_, line) in dictionary_lines(text) {
            let Some((form, reading)) = reading_form(&line) else {
                continue;
            };
            let at = *index.entry(form.clone()).or_insert_with(|| {
                forms.push(form);
                forms.len() - 1
            });
            items.push((name.as_str(), line, reading, at));
        }
    }
    let paths = mecab_paths(&forms)?;
    let unidic = UnidicReadings::new(unidic_words()?);
    let mut counts: Vec<(&str, [usize; 4])> = Vec::new();
    let mut disagreements = Vec::new();
    for (name, line, reading, at) in &items {
        let agreement = agreement(reading, &paths[*at]);
        if counts.last().is_none_or(|(n, _)| n != name) {
            counts.push((name, [0; 4]));
        }
        let count = &mut counts.last_mut().expect("pushed above").1;
        match agreement {
            Agreement::Agrees => count[0] += 1,
            Agreement::Disagrees => {
                count[1] += 1;
                let known = unidic.of(&forms[*at]);
                let suggested = shared_readings(known, &paths[*at]);
                let likely_wrong = !suggested.is_empty() && !known.contains(reading);
                count[3] += usize::from(likely_wrong);
                disagreements.push((name, line, &paths[*at], suggested, likely_wrong));
            }
            Agreement::Unread => count[2] += 1,
        }
    }
    disagreements.sort_by_key(|(name, line, _, _, likely_wrong)| {
        (*name, !*likely_wrong, line.cost.unwrap_or(u32::MAX))
    });
    let mut tsv = String::from(
        "dictionary\treading\tsurface\tconjugation\tcost\tmecab\tsuggested\tlikely_wrong\n",
    );
    for (name, line, paths, suggested, likely_wrong) in disagreements {
        let mut readings: Vec<String> = Vec::new();
        for path in paths.iter().flatten() {
            if !readings.contains(path) {
                readings.push(path.clone());
            }
        }
        tsv.push_str(&format!(
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            line.reading,
            line.surface,
            line.conjugation,
            line.cost.map(|c| c.to_string()).unwrap_or_default(),
            readings.join("|"),
            suggested.join("|"),
            if likely_wrong { "yes" } else { "" },
        ));
    }
    write_atomically(CHECK_READINGS, |w| {
        w.write_all(tsv.as_bytes())
            .map_err(write_error(CHECK_READINGS))
    })?;
    for (name, [agrees, disagrees, unread, likely_wrong]) in counts {
        println!(
            "{name}: read {}, agree {agrees}, disagree {disagrees} (likely wrong {likely_wrong}), unread {unread}",
            agrees + disagrees + unread
        );
    }
    println!("out: {CHECK_READINGS}");
    Ok(())
}

/// The n-best paths of MeCab for each of `inputs`, one per line.
fn mecab_paths(inputs: &[String]) -> Result<Vec<Vec<Option<String>>>, Error> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("mecab")
        .arg(format!("-N{MECAB_PATHS}"))
        .args(["-F", "%m\\t%f[7]\\n", "-U", "%m\\t\\n", "-E", "EOS\\n"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(Error::Mecab)?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let paths = std::thread::scope(|scope| {
        let writer = scope.spawn(move || -> io::Result<()> {
            let mut stdin = io::BufWriter::new(&mut stdin);
            for input in inputs {
                writeln!(stdin, "{input}")?;
            }
            stdin.flush()
        });
        let paths = paths_of(inputs, BufReader::new(stdout));
        writer
            .join()
            .expect("the writer does not panic")
            .map_err(Error::Mecab)?;
        Ok::<_, Error>(paths?)
    })?;
    let status = child.wait().map_err(Error::Mecab)?;
    if !status.success() {
        return Err(Error::Mecab(io::Error::other(format!(
            "exited with {status}"
        ))));
    }
    Ok(paths)
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
    let corrections = corrections()?;
    let mut analyzer = None;
    for name in names {
        let label = read_to_string(format!("{ADDITIONAL}/{name}/label.txt"))?;
        let (mut dictionary, sources) = if name == PLACE {
            (
                place_dictionary_of(&format!("{ADDITIONAL_BUILD}/{name}/ken_all.csv"))?,
                BTreeSet::from([POSTAL_SOURCE.to_owned()]),
            )
        } else if name == OKURIGANA {
            let mut sources =
                document_sources(open(DOCS)?, |_| true).map_err(|source| Error::Documents {
                    path: DOCS.into(),
                    source,
                })?;
            sources.extend(ANALYZER_SOURCES.map(String::from));
            let mut failure = None;
            let dictionary = okurigana_dictionary(read_units(open(UNITS)?).map_while(|unit| {
                unit.map_err(|source| {
                    failure = Some(Error::Units {
                        path: UNITS.into(),
                        source,
                    })
                })
                .ok()
            }));
            if let Some(e) = failure {
                return Err(e);
            }
            (dictionary, sources)
        } else {
            if analyzer.is_none() {
                analyzer = Some(open_analyzer()?);
            }
            sourced_dictionary(&name, &base, analyzer.as_ref().unwrap())?
        };
        dictionary.drop_words(|reading, surface| {
            corrections.drops(surface) || !is_word(reading, surface)
        });
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
        cut_documents(
            open(&docs)?,
            |text| analyzer.words(text),
            analyzer.readings(),
            out,
        )
        .map_err(|source| Error::Cut {
            path: docs.clone().into(),
            source,
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
    sources.extend(ANALYZER_SOURCES.map(String::from));
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

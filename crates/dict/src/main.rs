//! `kanaemi-dict`: cuts the collected documents into units, builds the base
//! dictionary from them and evaluates it, and builds the additional
//! dictionaries, reading and writing under the current directory.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use kanaemi_dict::{
    Agreement, Analyzer, AnalyzerError, BASE_LABEL, Base, ChainCounter, Class, Corrections,
    CorrectionsError, CutError, Dictionary, DistError, DocumentsError, Established, EvalDocument,
    EvalDocumentsError, Evaluation, ExampleFile, Examples, KanjiError, Readings, ReadingsError,
    RejectedLines, Split, TitleReadings, TitlesError, UnidicError, UnidicReadings, UnidicWord,
    Unit, UnitsError, WordCase, WordCasesError, WordResult, WordScore, WriteError, agreement,
    as_typed, check_words, clans, common_words, compound_parts, correction_uses,
    correction_uses_tsv, counted_reading, counted_voicings, counted_words, counter_words,
    cut_documents, dictionary_lines, document_ids, document_sources, document_texts,
    each_document_with_units, engine, engine_with, eval_documents, evaluate_documents,
    examples_of_document, excluded_readings, field_dictionary, fragment_surfaces, gather, is_word,
    kanji_entries, keys_diff_tsv, listed_dictionary, loss_reasons_tsv, mismatches_tsv,
    miss_reasons_tsv, model_file, mozc_emoji, mozc_emoticons, mozc_readings, mozc_symbols,
    name_readings, nihon_readings, okurigana_dictionary, parse_check_cases, parse_corrections,
    parse_word_cases, paths_of, person_dictionary, place_dictionary, place_names, plain_words,
    read_names, read_title_entries, read_titles, read_titles_of, read_units, reading_form,
    sample_tsv, shared_readings, sources_file, split_of, station_dictionary, take, text_units,
    title_entries, train, typed_by_analyzer, typed_units, unihan_readings, unmet_checks_tsv,
    unnamed_words, wikidata_mismatches, wikidata_people, wikidata_readings, wikidata_stations,
    with_people_read_by_leads, word_misses_tsv, word_scores_tsv, write_atomically, year_dictionary,
};
use kanaemi_engine::{RankingModel, TextDictionary};
use rayon::prelude::*;

const SYSTEM_DICTIONARY: &str = "build/sudachi/system_full.dic";
/// SudachiDict small, which checks the analyzer's readings against UniDic.
const CHECKER_DICTIONARY: &str = "build/sudachi/system_small.dic";
const LEXICON: &str = "build/sudachi/small_lex.csv";
/// The rest of SudachiDict's lexicon, beside SudachiDict small's.
const FULL_LEXICONS: [&str; 2] = [
    "build/sudachi/core_lex.csv",
    "build/sudachi/notcore_lex.csv",
];
/// The words the analyzer reads wrong, applied to what it reads.
const CORRECTIONS: &str = "analyzer/corrections.tsv";
const DOCS: &str = "build/docs.jsonl";
/// The titles of the Wikipedia articles, which `scripts/base-titles.ts` writes.
const BASE_TITLES: &str = "build/base-titles.tsv";
/// The names of the Wikipedia articles and redirects, which
/// `scripts/base-titles.ts` writes.
const BASE_NAMES: &str = "build/base-names.txt";
/// Mozc's table of single kanji and the Unihan readings, which
/// `scripts/kanji.ts` writes, and the pairs kept out of them.
const KANJI_TABLE: &str = "build/kanji/single_kanji.tsv";
const UNIHAN_READINGS: &str = "build/kanji/Unihan_Readings.txt";
const KANJI_EXCLUDED: &str = "kanji/excluded.tsv";
/// The sources of the single kanji readings.
const KANJI_SOURCE: &str = "mozc-single-kanji";
const UNIHAN_SOURCE: &str = "unicode-unihan";
const UNITS: &str = "build/units.jsonl";
/// Where an evaluation of the documents writes: its scores, the units not
/// offered their surface, the keys whose first candidates lost to the best
/// order, every key, the keys of the run before, and what changed since.
struct EvaluationOutputs {
    scores: &'static str,
    misses: &'static str,
    losses: &'static str,
    keys: &'static str,
    previous_keys: &'static str,
    diff: &'static str,
    /// A sample of the units not offered their surface for people to judge,
    /// written once, without the model.
    missing_sample: Option<&'static str>,
}

const EVALUATION: EvaluationOutputs = EvaluationOutputs {
    scores: "build/evaluation.tsv",
    misses: "build/evaluation-misses.tsv",
    losses: "build/evaluation-losses.tsv",
    keys: "build/evaluation-keys.tsv",
    previous_keys: "build/evaluation-keys.previous.tsv",
    diff: "build/evaluation-diff.tsv",
    missing_sample: Some("build/evaluation-missing-sample.tsv"),
};
/// Why [`EVALUATION`]'s units were not offered their surface, as Kanaemi
/// explains each conversion.
const MISSING_REASONS: &str = "build/evaluation-missing-reasons.tsv";
/// Why the first candidates of [`EVALUATION`]'s keys lost to the best order.
const LOSS_REASONS: &str = "build/evaluation-loss-reasons.tsv";
/// [`EVALUATION`] with additional dictionaries after the base.
const WITH_ADDITIONAL: EvaluationOutputs = EvaluationOutputs {
    scores: "build/evaluation-with.tsv",
    misses: "build/evaluation-with-misses.tsv",
    losses: "build/evaluation-with-losses.tsv",
    keys: "build/evaluation-with-keys.tsv",
    previous_keys: "build/evaluation-with-keys.previous.tsv",
    diff: "build/evaluation-with-diff.tsv",
    missing_sample: None,
};
/// [`MISSING_REASONS`] and [`LOSS_REASONS`] of [`WITH_ADDITIONAL`].
const WITH_ADDITIONAL_REASONS: (&str, &str) = (
    "build/evaluation-with-missing-reasons.tsv",
    "build/evaluation-with-loss-reasons.tsv",
);
const DICTIONARIES: &str = "build/dictionaries";
/// Where the additional dictionaries built against the dictionary of the
/// train documents go, for the evaluation alone: built against the base, an
/// additional dictionary leaves out the words of the eval documents the base
/// has from them.
const TRAIN_ADDITIONAL: &str = "build/dictionaries/train";
/// Where each additional dictionary is set up, one directory per dictionary.
const ADDITIONAL: &str = "additional";
/// Where `scripts/additional.ts` leaves what each additional dictionary is built from.
const ADDITIONAL_BUILD: &str = "build/additional";
/// The additional dictionary built from the postal code data, not from documents.
const PLACE: &str = "place";
/// The additional dictionary of the words the base dictionary's documents
/// spell with other okurigana than the usual, which the base leaves out.
const OKURIGANA: &str = "okurigana";
/// The additional dictionaries taken from Mozc's tables, not from documents.
const SYMBOL: &str = "symbol";
const EMOJI: &str = "emoji";
const EMOTICON: &str = "emoticon";
/// The additional dictionary of person names, taken from Wikidata's people.
const PERSON: &str = "person";
const PEOPLE_SOURCE: &str = "wikidata-people";
/// Wikidata's people with an article in Japanese and no reading, whom the
/// leads of their articles read.
const UNREAD_PEOPLE_SOURCE: &str = "wikidata-people-unread";
/// The additional dictionary of station names, taken from Wikidata's stations.
const STATION: &str = "station";
const STATIONS_SOURCE: &str = "wikidata-stations";
/// The additional dictionaries made from lists, whose costs start at the cost
/// of a word met once in the base's units.
const LISTS: [&str; 3] = [PERSON, PLACE, STATION];
/// The sources every dictionary read with the analyzer has: the analyzer's
/// dictionary, the dictionary that checks its readings, and the UniDic
/// lexicon they are checked against and the base takes words from, and the
/// rest of SudachiDict's lexicon, which reads the words the build joins.
const ANALYZER_SOURCES: [&str; 5] = [
    "sudachidict-full",
    "sudachidict-small",
    "sudachidict-small-lex",
    "sudachidict-core-lex",
    "sudachidict-notcore-lex",
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
/// The rows of [`EVALUATION`]'s sample of units not offered their surface.
const MISSING_SAMPLE: usize = 200;
/// [`EVALUATION`] with the model.
const RANKING_EVALUATION: EvaluationOutputs = EvaluationOutputs {
    scores: "build/ranking-evaluation.tsv",
    misses: "build/ranking-misses.tsv",
    losses: "build/ranking-losses.tsv",
    keys: "build/ranking-keys.tsv",
    previous_keys: "build/ranking-keys.previous.tsv",
    diff: "build/ranking-diff.tsv",
    missing_sample: None,
};
/// [`RANKING_EVALUATION`] with every additional dictionary after the base.
const RANKING_WITH_ADDITIONAL: EvaluationOutputs = EvaluationOutputs {
    scores: "build/ranking-with.tsv",
    misses: "build/ranking-with-misses.tsv",
    losses: "build/ranking-with-losses.tsv",
    keys: "build/ranking-with-keys.tsv",
    previous_keys: "build/ranking-with-keys.previous.tsv",
    diff: "build/ranking-with-diff.tsv",
    missing_sample: None,
};
/// Where the training examples wait, in a file of each run's own, while the
/// model trains.
const RANKING_WORK: &str = "build/ranking";
const MODEL_BITS: u8 = 22;
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
/// The word set taken from lists outside the analyzer, and where its scores
/// and misses go.
const WORDS: &str = "evaluation/words.tsv";
const CHECK_WORDS: &str = "build/check-words.tsv";
const CHECK_WORD_MISSES: &str = "build/check-words-misses.tsv";
/// What each feature expects of its words, and where the checks it does not
/// meet go.
const CHECKS: &str = "evaluation/checks.tsv";
const CHECK_LIST: &str = "build/check-list.tsv";
/// The model that ships with the base dictionary.
const KEPT_MODEL: &str = "dictionaries/base.model";
const CHECK_SAMPLE: &str = "build/check-sample.tsv";
/// The readings of Wikidata's items, which `scripts/wikidata.ts` writes, and
/// the words of the base dictionary that read none of their ways.
const WIKIDATA_READINGS: &str = "build/wikidata/readings.tsv";
const CHECK_WIKIDATA: &str = "build/check-wikidata.tsv";
/// Items sampled from each stratum of the base dictionary and from each
/// additional dictionary.
const SAMPLE_BASE: usize = 60;
const SAMPLE_ADDITIONAL: usize = 15;
const CHECK_READINGS: &str = "build/check-readings.tsv";
/// What each correction changes in the documents.
const CHECK_CORRECTIONS: &str = "build/check-corrections.tsv";
/// The paths of each surface MeCab gives, best first.
const MECAB_PATHS: usize = 5;

/// Where a base dictionary build writes.
struct Outputs {
    dictionary: &'static str,
    report: &'static str,
    /// The cost of a word met once in the units the dictionary counts, which
    /// the dictionaries made from lists built against it start at.
    seen_once: &'static str,
}

const BASE: Outputs = Outputs {
    dictionary: "build/dictionaries/base.tsv",
    report: "build/dictionaries/base-report.tsv",
    seen_once: "build/dictionaries/base-seen-once.txt",
};
const BASE_TRAIN: Outputs = Outputs {
    dictionary: "build/dictionaries/base-train.tsv",
    report: "build/dictionaries/base-train-report.tsv",
    seen_once: "build/dictionaries/base-train-seen-once.txt",
};

const USAGE: &str = "\
usage: kanaemi-dict units
         cut every document of build/docs.jsonl into build/units.jsonl,
         reading them with build/sudachi/system_full.dic, checking the
         readings against build/sudachi/small_lex.csv with
         build/sudachi/system_small.dic, reading the words it joins as
         build/sudachi/{small,core,notcore}_lex.csv read them, and applying
         the corrections of analyzer/corrections.tsv
       kanaemi-dict dictionary [--train-only]
         build build/dictionaries/base.tsv from build/units.jsonl, the
         UniDic lexicon build/sudachi/small_lex.csv and the readings
         build/sudachi/{core,notcore}_lex.csv give the surfaces of the
         units, with the words of one
         to ten with each counter read by the analyzer, reporting to
         build/dictionaries/base-report.tsv, and the cost of a word met
         once to build/dictionaries/base-seen-once.txt; with --train-only,
         from the units of the train documents alone into
         build/dictionaries/base-train.tsv, base-train-report.tsv and
         base-train-seen-once.txt
       kanaemi-dict evaluate
         convert the units of the eval documents with
         build/dictionaries/base-train.tsv, and write the units not offered
         their surface and the loss against the best order without context
         to build/evaluation.tsv, with the units not offered, the readings
         that lost, every reading and what changed since the run before in
         build/evaluation-{misses,losses,keys,diff}.tsv, and why units were
         not offered and readings lost in
         build/evaluation-{missing,loss}-reasons.tsv
       kanaemi-dict evaluate --with NAME...
         evaluate with the additional dictionaries
         build/dictionaries/train/NAME.tsv after the base (build them with
         additional --train-only), writing build/evaluation-with*.tsv
         instead
       kanaemi-dict additional [--train-only] [NAME...]
         build the additional dictionaries NAME, or every one under
         additional/, from build/additional/NAME/ into
         build/dictionaries/NAME.tsv and NAME-report.tsv, without what
         build/dictionaries/base.tsv already gives, the costs of person,
         place and station starting at build/dictionaries/base-seen-once.txt;
         with --train-only, without what build/dictionaries/base-train.tsv
         gives and from base-train-seen-once.txt into
         build/dictionaries/train/, for evaluate --with
       kanaemi-dict ranking
         train the ranking model on the candidates of
         build/dictionaries/base-train.tsv from the train documents of the
         base and of each field's additional dictionary, write it to
         build/dictionaries/base.model paired with
         build/dictionaries/base.tsv, and measure the eval documents
         without and with it, writing the tables of evaluate with it as
         build/ranking-*.tsv, and again with every additional dictionary of
         build/dictionaries/train/ after the base, writing
         build/ranking-with*.tsv
       kanaemi-dict take
         put the dictionaries and the model of build/dictionaries/ that have
         a sources record into dictionaries/ in place of those there, once
         they check as they would ship
       kanaemi-dict dist
         check dictionaries/ and gather each dictionary with the notice of
         its sources (from notices/) and the license into build/dist/NAME/
       kanaemi-dict check-words
         convert the words of evaluation/words.tsv and the checks of
         evaluation/checks.tsv with the base dictionary of dictionaries/,
         without and with its model, and write the scores per category to
         build/check-words.tsv, the words not offered or not first after
         their text to build/check-words-misses.tsv, and the checks not met
         to build/check-list.tsv
       kanaemi-dict check-words --build
         the same with build/dictionaries/base.tsv, without a model
       kanaemi-dict check-words [--build] --with NAME...
         the same with the additional dictionaries NAME of
         build/dictionaries/ listed after the base dictionary
       kanaemi-dict check-sample
         sample the items of the dictionaries of dictionaries/ by stratum
         into build/check-sample.tsv for people to judge
       kanaemi-dict check-readings
         read the surfaces with kanji of the dictionaries of dictionaries/
         with MeCab and write the items no path of it reads as the
         dictionary does to build/check-readings.tsv
       kanaemi-dict check-corrections
         cut each line of the documents that holds a surface of
         analyzer/corrections.tsv, and each surface alone, with and without
         the corrections, and write what each correction changes to
         build/check-corrections.tsv
       kanaemi-dict check-wikidata
         compare the base dictionary of dictionaries/ with the readings of
         Wikidata's items in build/wikidata/readings.tsv and write the words
         that read none of their items' ways to build/check-wikidata.tsv";

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
    Kanji { path: PathBuf, source: KanjiError },
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
    #[error("{}: not a cost", path.display())]
    SeenOnce { path: PathBuf },
    #[error("{}: cannot remove: {source}", path.display())]
    Remove { path: PathBuf, source: io::Error },
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
        ["evaluate"] => evaluate(&[]),
        ["evaluate", "--with", ref names @ ..] if !names.is_empty() => evaluate(names),
        ["additional", "--train-only", ref names @ ..] => build_additional(names, true),
        ["additional", ref names @ ..] => build_additional(names, false),
        ["ranking"] => train_ranking(),
        ["take"] => take_built(),
        ["dist"] => build_dist(),
        ["check-words"] => check_kept_words(&[]),
        ["check-words", "--build"] => check_built_words(&[]),
        ["check-words", "--with", ref names @ ..] if !names.is_empty() => check_kept_words(names),
        ["check-words", "--build", "--with", ref names @ ..] if !names.is_empty() => {
            check_built_words(names)
        }
        ["check-sample"] => sample_kept(),
        ["check-readings"] => check_kept_readings(),
        ["check-corrections"] => check_corrections(),
        ["check-wikidata"] => check_wikidata(),
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
    let counters = counter_words(open(LEXICON)?).map_err(|source| Error::Unidic {
        path: LEXICON.into(),
        source,
    })?;
    let readings = UnidicReadings::new(unidic_words()?)
        .with_counters(counters)
        .with_lexicon(lexicon_words(unnamed_words)?);
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

/// The words of the full lexicons `take` picks out of each.
fn lexicon_words(
    take: fn(BufReader<File>) -> Result<Vec<UnidicWord>, UnidicError>,
) -> Result<Vec<UnidicWord>, Error> {
    let mut words = Vec::new();
    for path in FULL_LEXICONS {
        words.extend(take(open(path)?).map_err(|source| Error::Unidic {
            path: path.into(),
            source,
        })?);
    }
    Ok(words)
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
    // A correction says how its surface reads; the lexicon reading it away
    // would bring back the misreading it corrects (一回 as いちかい).
    let lexicon: Vec<UnidicWord> = {
        let corrections = corrections()?;
        lexicon_words(common_words)?
            .into_iter()
            .filter(|w| !corrections.contradicts(&w.reading, &w.surface))
            .collect()
    };
    let names = read_names(open(BASE_NAMES)?).map_err(read_error(BASE_NAMES))?;
    let titles = read_titles(open(BASE_TITLES)?).map_err(|source| Error::Titles {
        path: BASE_TITLES.into(),
        source,
    })?;
    let established = established_words(|doc_id| !train_only || split_of(doc_id) == Split::Train)?;
    let (dictionary, took_titles, took_unihan) = {
        let mut chains = ChainCounter::new(&names);
        let mut reads = TitleReadings::new(&titles);
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
        let mut total = 0usize;
        let mut dictionary = Dictionary::build_with_lexicon(
            units
                .inspect(|unit| {
                    chains.observe(unit);
                    reads.observe(unit);
                })
                .flat_map(|unit| as_typed(&unit, &established))
                // Titles and chains cost against the units as the dictionary
                // counts them.
                .inspect(|_| total += 1),
            &unidic,
            &lexicon,
        );
        if let Some(e) = failure {
            return Err(e);
        }
        let texts = document_texts(open(DOCS)?, |doc_id| {
            !train_only || split_of(doc_id) == Split::Train
        })
        .map_err(|source| Error::Documents {
            path: DOCS.into(),
            source,
        })?;
        let titles = read_title_entries(
            &titles,
            &reads.finish(),
            texts.iter().map(String::as_str),
            total,
        );
        let mut took_titles = !titles.is_empty();
        dictionary.add_words(titles);
        let chained = {
            let known: HashSet<&str> = dictionary
                .entries
                .iter()
                .map(|e| e.surface.as_str())
                .collect();
            let chains = chains
                .finish()
                .titles(|s| known.contains(s), texts.iter().map(String::as_str));
            title_entries(&chains, texts.iter().map(String::as_str), total)
        };
        println!("chains: {}", chained.len());
        took_titles |= !chained.is_empty();
        dictionary.add_words(chained);
        let counted = {
            let counters =
                UnidicReadings::new(counter_words(open(LEXICON)?).map_err(|source| {
                    Error::Unidic {
                        path: LEXICON.into(),
                        source,
                    }
                })?);
            let analyzer = open_analyzer()?;
            counted_words(
                &dictionary.numeric,
                &counters,
                &dictionary.numeric_written,
                |surface| {
                    analyzer
                        .tokens(format!("{surface}の"))
                        .map(|tokens| counted_reading(&tokens, surface))
                },
            )
            .map_err(|source| Error::Analyzer {
                path: SYSTEM_DICTIONARY.into(),
                source,
            })?
        };
        let before = dictionary.numeric.len();
        let voiced = counted_voicings(
            &dictionary.numeric,
            dictionary.entries.iter().chain(&counted),
        );
        dictionary.add_numeric(voiced);
        println!("counted voicings: {}", dictionary.numeric.len() - before);
        dictionary.add_words(counted);
        let nihon = nihon_readings(&dictionary.entries);
        println!("nihon: {}", nihon.len());
        dictionary.add_words(nihon);
        let (kanji, took_unihan) = {
            let excluded =
                excluded_readings(&read_to_string(KANJI_EXCLUDED)?).map_err(|source| {
                    Error::Kanji {
                        path: KANJI_EXCLUDED.into(),
                        source,
                    }
                })?;
            kanji_entries(
                &dictionary.entries,
                &mozc_readings(&read_to_string(KANJI_TABLE)?),
                &unihan_readings(&read_to_string(UNIHAN_READINGS)?),
                &excluded,
            )
        };
        println!("single kanji: {}", kanji.len());
        dictionary.add_words(kanji);
        let corrections = corrections()?;
        dictionary.drop_words(|reading, surface| {
            corrections.drops(surface) || !is_word(reading, surface)
        });
        (dictionary, took_titles, took_unihan)
    };
    let text = dictionary
        .to_checked_text(BASE_LABEL)
        .map_err(|source| Error::Rejected {
            path: outputs.dictionary.into(),
            source,
        })?;
    // Removed before the base is replaced: left from an earlier build, it
    // would not be this base's, should the build stop before writing it.
    match std::fs::remove_file(outputs.seen_once) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => {
            return Err(Error::Remove {
                path: outputs.seen_once.into(),
                source,
            });
        }
        _ => {}
    }
    write_atomically(outputs.dictionary, |w| {
        w.write_all(text.as_bytes())
            .map_err(write_error(outputs.dictionary))
    })?;
    write_atomically(outputs.report, |w| {
        w.write_all(dictionary.report.to_tsv().as_bytes())
            .map_err(write_error(outputs.report))
    })?;
    if let Some(cost) = dictionary.seen_once_cost {
        write_atomically(outputs.seen_once, |w| {
            writeln!(w, "{cost}").map_err(write_error(outputs.seen_once))
        })?;
    }
    if !train_only {
        let mut sources =
            document_sources(open(DOCS)?, |_| true).map_err(|source| Error::Documents {
                path: DOCS.into(),
                source,
            })?;
        sources.extend(ANALYZER_SOURCES.map(String::from));
        if took_titles {
            sources.insert(TITLES_SOURCE.to_owned());
        }
        sources.insert(KANJI_SOURCE.to_owned());
        if took_unihan {
            sources.insert(UNIHAN_SOURCE.to_owned());
        }
        write_sources(outputs.dictionary, text.as_bytes(), None, &sources)?;
    }
    println!(
        "entries: {}, okurigana lines: {} (particle lines: {}), numeric lines: {}, unknown types: {}, disallowed okurigana: {}, out: {}, report: {}",
        dictionary.entries.len(),
        dictionary.okuri.len(),
        dictionary.particles.len(),
        dictionary.numeric.len(),
        dictionary.report.unknown_conjugations.len(),
        dictionary.report.disallowed_okurigana.len(),
        outputs.dictionary,
        outputs.report,
    );
    Ok(())
}

/// Converts the units of the eval documents with the dictionary built from
/// the train documents, followed by the additional dictionaries `with` built
/// against it under [`TRAIN_ADDITIONAL`], and writes the scores per kind of
/// source and class: those of the base alone to [`EVALUATION`], those with
/// additional dictionaries to [`WITH_ADDITIONAL`].
fn evaluate(with: &[&str]) -> Result<(), Error> {
    let (outputs, reasons) = if with.is_empty() {
        (&EVALUATION, (MISSING_REASONS, LOSS_REASONS))
    } else {
        (&WITH_ADDITIONAL, WITH_ADDITIONAL_REASONS)
    };
    let dictionaries = with_additional_in(TRAIN_ADDITIONAL, train_dictionary()?, with)?;
    let docs = typed_eval_documents(&train_established()?)?;
    let agreed = agreed_readings(&docs)?;
    let evaluation = evaluate_documents(&docs, &dictionaries, None, &|unit| agrees(&agreed, unit));
    print_scores("", &evaluation);
    write_evaluation(outputs, &evaluation)?;
    let engine = engine_with(&dictionaries);
    let all = evaluation.all();
    let texts = std::iter::once(BASE_TRAIN.dictionary.to_owned())
        .chain(
            with.iter()
                .map(|name| format!("{TRAIN_ADDITIONAL}/{name}.tsv")),
        )
        .map(|path| read_to_string(&path))
        .collect::<Result<String, Error>>()?;
    let (missing, by_reason) = miss_reasons_tsv(&engine, &all.missing(), &Readings::parse(texts));
    write_reasons(reasons.0, "missing", &missing, &by_reason)?;
    let (losses, by_reason) = loss_reasons_tsv(&engine, &all.losses());
    write_reasons(reasons.1, "loss", &losses, &by_reason)?;
    println!("documents: {}, out: {}", docs.len(), outputs.scores);
    Ok(())
}

/// Writes a table of reasons to `path`, and prints how much of `what` each
/// reason holds.
fn write_reasons<N: std::fmt::Display>(
    path: &str,
    what: &str,
    tsv: &str,
    by_reason: &BTreeMap<&str, N>,
) -> Result<(), Error> {
    write_atomically(path, |w| {
        w.write_all(tsv.as_bytes()).map_err(write_error(path))
    })?;
    let reasons: Vec<String> = by_reason
        .iter()
        .map(|(reason, n)| format!("{reason} {n}"))
        .collect();
    println!("{what} by reason: {}, out: {path}", reasons.join(", "));
    Ok(())
}

/// Writes every table of `evaluation` to `out`, keeping the keys of the run
/// before to write what changed since.
fn write_evaluation(out: &EvaluationOutputs, evaluation: &Evaluation) -> Result<(), Error> {
    let all = evaluation.all();
    if std::path::Path::new(out.keys).exists() {
        std::fs::rename(out.keys, out.previous_keys).map_err(write_error(out.previous_keys))?;
    }
    let keys = all.keys_tsv();
    for (path, tsv) in [
        (out.scores, evaluation.to_tsv()),
        (out.misses, all.misses_tsv()),
        (out.losses, all.losses_tsv()),
        (out.keys, keys.clone()),
    ] {
        write_atomically(path, |w| {
            w.write_all(tsv.as_bytes()).map_err(write_error(path))
        })?;
    }
    if let Some(path) = out.missing_sample {
        let tsv = all.missing_sample_tsv(MISSING_SAMPLE);
        write_atomically(path, |w| {
            w.write_all(tsv.as_bytes()).map_err(write_error(path))
        })?;
    }
    if std::path::Path::new(out.previous_keys).exists() {
        let diff = keys_diff_tsv(&read_to_string(out.previous_keys)?, &keys);
        println!(
            "changed keys since the run before: {}, out: {}",
            diff.lines().count() - 1,
            out.diff
        );
        write_atomically(out.diff, |w| {
            w.write_all(diff.as_bytes()).map_err(write_error(out.diff))
        })?;
    }
    Ok(())
}

/// The joined words typed whole in the documents of build/units.jsonl that
/// `wanted` takes (see [`Established`]).
fn established_words(wanted: impl Fn(&str) -> bool) -> Result<Established, Error> {
    let mut failure = None;
    let established = Established::of(
        read_units(open(UNITS)?)
            .map_while(|unit| {
                unit.map_err(|source| {
                    failure = Some(Error::Units {
                        path: UNITS.into(),
                        source,
                    })
                })
                .ok()
            })
            .filter(|unit| wanted(&unit.doc_id)),
    );
    match failure {
        Some(e) => Err(e),
        None => Ok(established),
    }
}

/// [`established_words`] of the train documents, which the dictionary the
/// evaluation and the model use is built from.
fn train_established() -> Result<Established, Error> {
    established_words(|doc_id| split_of(doc_id) == Split::Train)
}

/// The eval documents with their units as the evaluation types them (see
/// [`typed_by_analyzer`]).
fn typed_eval_documents(established: &Established) -> Result<Vec<EvalDocument>, Error> {
    let mut docs = eval_documents(open(UNITS)?, open(DOCS)?)?;
    for doc in &mut docs {
        let typed = typed_by_analyzer(&doc.units, &doc.compounds, established);
        doc.parts = compound_parts(&doc.units, &typed, established);
        doc.units = typed;
    }
    Ok(docs)
}

/// The readings MeCab gives the surfaces of the units of `docs` the same as
/// the analyzer, by surface: answers another analyzer agrees with, less
/// likely to be the analyzer's errors. Units with a number are left out, as
/// MeCab reads digits one by one.
fn agreed_readings(docs: &[EvalDocument]) -> Result<HashMap<String, HashSet<String>>, Error> {
    let pairs: BTreeSet<(&str, &str)> = docs
        .iter()
        .flat_map(|d| d.units.iter().chain(d.parts.values().flatten()))
        .filter(|u| u.numeric.is_none())
        .map(|u| (u.surface.as_str(), u.reading.as_str()))
        .collect();
    let forms: Vec<String> = pairs
        .iter()
        .map(|(surface, _)| *surface)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(String::from)
        .collect();
    let paths = mecab_paths(&forms)?;
    let at: HashMap<&str, usize> = forms
        .iter()
        .enumerate()
        .map(|(i, f)| (f.as_str(), i))
        .collect();
    let mut agreed: HashMap<String, HashSet<String>> = HashMap::new();
    for (surface, reading) in pairs {
        if agreement(reading, &paths[at[surface]]) == Agreement::Agrees {
            agreed
                .entry(surface.to_owned())
                .or_default()
                .insert(reading.to_owned());
        }
    }
    Ok(agreed)
}

fn agrees(agreed: &HashMap<String, HashSet<String>>, unit: &Unit) -> bool {
    agreed
        .get(&unit.surface)
        .is_some_and(|readings| readings.contains(&unit.reading))
}

/// The base dictionary built from the train documents, as Kanaemi reads it.
fn train_dictionary() -> Result<Arc<TextDictionary>, Error> {
    parsed_dictionary(BASE_TRAIN.dictionary)
}

fn print_scores(label: &str, evaluation: &Evaluation) {
    let all = evaluation.all();
    let rows = Class::ALL
        .map(|class| (class.name(), all.score(class)))
        .into_iter()
        .chain([
            ("all", all.total()),
            ("agreed with mecab", evaluation.agreed.total()),
        ]);
    for (class, s) in rows {
        println!(
            "{label}{class}: units {}, missing {} ({:.2}%), first {} of best {} (loss {}), page loss {}",
            s.units,
            s.missing,
            percent(s.missing as usize, s.units as usize),
            s.first,
            s.best_first,
            s.loss(),
            s.page_loss(),
        );
    }
    println!(
        "{label}compounds not offered, typed as their parts: {}",
        evaluation.split
    );
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
    // The fragments the train dictionary counted so, from the same units.
    let established = train_established()?;
    let fragments = {
        let mut failure = None;
        let fragments = fragment_surfaces(
            read_units(open(UNITS)?)
                .map_while(|unit| {
                    unit.map_err(|source| {
                        failure = Some(Error::Units {
                            path: UNITS.into(),
                            source,
                        })
                    })
                    .ok()
                })
                .filter(|unit| split_of(&unit.doc_id) == Split::Train)
                .flat_map(|unit| as_typed(&unit, &established)),
        );
        if let Some(failure) = failure {
            return Err(failure);
        }
        fragments
    };
    println!("fragments: {}", fragments.len());
    let mut taken: HashSet<String> = HashSet::new();
    let mut sources = BTreeSet::from(ANALYZER_SOURCES.map(String::from));
    let mut add = |units: &str, docs: &str, budget: usize| -> Result<(), Error> {
        let added = add_examples(
            units,
            docs,
            budget,
            &taken,
            &Teaching {
                dictionary: &dictionary,
                fragments: &fragments,
                established: &established,
            },
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
    let docs = typed_eval_documents(&established)?;
    let agreed = agreed_readings(&docs)?;
    let agreed = |unit: &Unit| agrees(&agreed, unit);
    let model = Arc::new(model);
    let names = additional_names()?;
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let runs = [
        ("", vec![dictionary.clone()], &RANKING_EVALUATION),
        (
            "with the additional dictionaries, ",
            with_additional_in(TRAIN_ADDITIONAL, dictionary, &names)?,
            &RANKING_WITH_ADDITIONAL,
        ),
    ];
    for (label, dictionaries, outputs) in runs {
        print_scores(
            &format!("{label}without the model: "),
            &evaluate_documents(&docs, &dictionaries, None, &agreed),
        );
        let with = evaluate_documents(&docs, &dictionaries, Some(&model), &agreed);
        print_scores(&format!("{label}with the model: "), &with);
        write_evaluation(outputs, &with)?;
    }
    println!(
        "out: {MODEL}, evaluation: {} (with the additional dictionaries {})",
        RANKING_EVALUATION.scores, RANKING_WITH_ADDITIONAL.scores
    );
    Ok(())
}

/// Every additional dictionary set up under additional/, in the order of
/// their names.
fn additional_names() -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(ADDITIONAL).map_err(read_error(ADDITIONAL))? {
        let entry = entry.map_err(read_error(ADDITIONAL))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // A dot directory is a year's setup left half-written, as
        // scripts/additional.ts skips it too.
        if entry.path().is_dir() && !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
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

/// What examples are made with: the dictionary that gives the candidates,
/// and the fragments typed only as their compounds (see [`typed_units`]).
struct Teaching<'a> {
    dictionary: &'a Arc<TextDictionary>,
    fragments: &'a HashSet<String>,
    established: &'a Established,
}

/// Adds the examples of the train documents of `units` and `docs` that are
/// not `taken`, until their units reach `budget`, a block at a time, and to
/// `sources` their source IDs; returns the doc IDs added.
fn add_examples(
    units: &str,
    docs: &str,
    budget: usize,
    taken: &HashSet<String>,
    teaching: &Teaching,
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
                || engine(teaching.dictionary.clone()),
                |engine, doc| {
                    let units = typed_units(
                        &doc.units,
                        &doc.compounds,
                        teaching.fragments,
                        teaching.established,
                    );
                    examples_of_document(engine, &units, &doc.text)
                },
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

/// Converts the word set and the checks with the base dictionary that ships
/// and the built additional dictionaries `with` after it, without and with
/// its model.
fn check_kept_words(with: &[&str]) -> Result<(), Error> {
    let dictionaries = with_additional(parsed_dictionary(&format!("{KEPT}/base.tsv"))?, with)?;
    let model = RankingModel::open(KEPT_MODEL).map_err(|source| Error::Model {
        path: KEPT_MODEL.into(),
        source,
    })?;
    check_words_with(
        &dictionaries,
        &[("off", None), ("on", Some(Arc::new(model)))],
    )
}

/// [`check_kept_words`] with the base dictionary just built, which has no
/// model paired with it until the model is trained again.
fn check_built_words(with: &[&str]) -> Result<(), Error> {
    let dictionaries = with_additional(parsed_dictionary(BASE.dictionary)?, with)?;
    check_words_with(&dictionaries, &[("off", None)])
}

/// `base` followed by the additional dictionaries `names` built under
/// build/dictionaries/, as a typist lists them.
fn with_additional(
    base: Arc<TextDictionary>,
    names: &[&str],
) -> Result<Vec<Arc<TextDictionary>>, Error> {
    with_additional_in(DICTIONARIES, base, names)
}

/// [`with_additional`] with the additional dictionaries built under `dir`.
fn with_additional_in(
    dir: &str,
    base: Arc<TextDictionary>,
    names: &[&str],
) -> Result<Vec<Arc<TextDictionary>>, Error> {
    let mut dictionaries = vec![base];
    for name in names {
        dictionaries.push(parsed_dictionary(&format!("{dir}/{name}.tsv"))?);
    }
    Ok(dictionaries)
}

/// Converts the word set and the checks with `dictionaries` and each model
/// of `models`, labeled, and writes their tables.
fn check_words_with(
    dictionaries: &[Arc<TextDictionary>],
    models: &[(&str, Option<Arc<RankingModel>>)],
) -> Result<(), Error> {
    let cases = parse_word_cases(read_to_string(WORDS)?).map_err(|source| Error::WordCases {
        path: WORDS.into(),
        source,
    })?;
    let checks = parse_check_cases(read_to_string(CHECKS)?).map_err(|source| Error::WordCases {
        path: CHECKS.into(),
        source,
    })?;
    let check_words_of: Vec<WordCase> = checks.iter().map(|c| c.case.clone()).collect();
    let mut word_runs = Vec::new();
    let mut check_runs = Vec::new();
    for (label, model) in models {
        word_runs.push((*label, check_words(&cases, dictionaries, model.as_ref())));
        check_runs.push((
            *label,
            check_words(&check_words_of, dictionaries, model.as_ref()),
        ));
    }
    let word_runs: Vec<(&str, &[WordResult])> =
        word_runs.iter().map(|(l, r)| (*l, r.as_slice())).collect();
    let check_runs: Vec<(&str, &[WordResult])> =
        check_runs.iter().map(|(l, r)| (*l, r.as_slice())).collect();
    let unmet = unmet_checks_tsv(&checks, &check_runs);
    for (path, tsv) in [
        (CHECK_WORDS, word_scores_tsv(&cases, &word_runs)),
        (CHECK_WORD_MISSES, word_misses_tsv(&cases, &word_runs)),
        (CHECK_LIST, unmet.clone()),
    ] {
        write_atomically(path, |w| {
            w.write_all(tsv.as_bytes()).map_err(write_error(path))
        })?;
    }
    for (label, results) in &word_runs {
        let mut score = WordScore::default();
        for (case, result) in cases.iter().zip(results.iter()) {
            score.add(case, result);
        }
        println!(
            "model {label}: words {}, missing {}, first after a text {} of {}",
            score.words, score.missing, score.after_text_first, score.after_text,
        );
    }
    println!(
        "checks: {}, not met: {}",
        checks.len() * models.len(),
        unmet.lines().count() - 1
    );
    println!("out: {CHECK_WORDS}, misses: {CHECK_WORD_MISSES}, checks not met: {CHECK_LIST}");
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

/// Lists the words of the base dictionary that ships whose readings are none
/// of the readings of the Wikidata items of their surface.
fn check_wikidata() -> Result<(), Error> {
    let readings = wikidata_readings(&read_to_string(WIKIDATA_READINGS)?);
    let base_path = format!("{KEPT}/base.tsv");
    let base = read_to_string(&base_path)?;
    let (summary, mismatches) =
        wikidata_mismatches(&readings, dictionary_lines(&base).map(|(_, line)| line));
    write_atomically(CHECK_WIKIDATA, |w| {
        w.write_all(mismatches_tsv(&mismatches).as_bytes())
            .map_err(write_error(CHECK_WIKIDATA))
    })?;
    println!(
        "labels: {}, in the base: {}, mismatched: {}, out: {CHECK_WIKIDATA}",
        summary.labels, summary.in_base, summary.mismatched
    );
    Ok(())
}

/// Cuts the documents of the base and of every additional dictionary where
/// they hold a correction's surface, with and without the corrections, and
/// writes what each correction changes. A surface to drop also counts as
/// needed where a name or a title holds it, since the dictionaries take
/// those whole.
fn check_corrections() -> Result<(), Error> {
    let corrections =
        parse_corrections(read_to_string(CORRECTIONS)?).map_err(|source| Error::Corrections {
            path: CORRECTIONS.into(),
            source,
        })?;
    // Without the additional dictionaries' documents, the base's alone.
    let additional: Vec<String> = match std::fs::read_dir(ADDITIONAL_BUILD) {
        Ok(entries) => {
            let mut dirs: Vec<String> = entries
                .filter_map(|e| e.ok().map(|e| e.path().display().to_string()))
                .collect();
            dirs.sort();
            dirs
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(source) => {
            return Err(Error::Read {
                path: ADDITIONAL_BUILD.into(),
                source,
            });
        }
    };
    let listed: HashSet<String> = {
        let mut listed = read_names(open(BASE_NAMES)?).map_err(read_error(BASE_NAMES))?;
        // The base's titles must be there; an additional dictionary may have none.
        let titles = std::iter::once(BASE_TITLES.to_owned()).chain(
            additional
                .iter()
                .map(|dir| format!("{dir}/titles.tsv"))
                .filter(|path| std::path::Path::new(path).exists()),
        );
        for path in titles {
            let read = read_titles(open(&path)?).map_err(|source| Error::Titles {
                path: path.clone().into(),
                source,
            })?;
            listed.extend(read.into_iter().map(|t| t.surface));
        }
        listed
    };
    // The base's documents must be there; an additional dictionary made
    // from a list has none.
    let docs: Vec<String> = std::iter::once(DOCS.to_owned())
        .chain(
            additional
                .iter()
                .map(|dir| format!("{dir}/docs.jsonl"))
                .filter(|path| std::path::Path::new(path).exists()),
        )
        .collect();
    let analyzer = open_analyzer()?;
    let leaving_out: Vec<Corrections> = (0..corrections.len())
        .map(|i| {
            Corrections::new(
                corrections
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .map(|(_, c)| c.clone()),
            )
        })
        .collect();
    let mut failure = None;
    let texts = docs.iter().flat_map(|path| {
        let texts = open(path).and_then(|docs| {
            document_texts(docs, |_| true).map_err(|source| Error::Documents {
                path: path.into(),
                source,
            })
        });
        texts.unwrap_or_else(|e| {
            failure.get_or_insert(e);
            Vec::new()
        })
    });
    let uses = correction_uses(
        &corrections,
        texts,
        |surface| listed.contains(surface),
        |text, left_out| {
            let tokenize = |t: &str| match left_out {
                None => analyzer.words(t),
                Some(i) => analyzer.words_corrected_by(t, &leaving_out[i]),
            };
            text_units(text, tokenize, analyzer.readings())
        },
    )
    .map_err(|source| Error::Analyzer {
        path: CORRECTIONS.into(),
        source,
    })?;
    if let Some(e) = failure {
        return Err(e);
    }
    write_atomically(CHECK_CORRECTIONS, |w| {
        w.write_all(correction_uses_tsv(&corrections, &uses).as_bytes())
            .map_err(write_error(CHECK_CORRECTIONS))
    })?;
    let unneeded = uses.iter().filter(|u| !u.is_needed()).count();
    println!(
        "corrections: {}, not needed: {unneeded}, out: {CHECK_CORRECTIONS}",
        corrections.len()
    );
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
/// additional/, each without what the base dictionary already gives; with
/// `train_only`, against the dictionary of the train documents into
/// [`TRAIN_ADDITIONAL`], for the evaluation, with no sources record.
fn build_additional(names: &[&str], train_only: bool) -> Result<(), Error> {
    let (base_outputs, out_dir) = if train_only {
        std::fs::create_dir_all(TRAIN_ADDITIONAL).map_err(write_error(TRAIN_ADDITIONAL))?;
        (BASE_TRAIN, TRAIN_ADDITIONAL)
    } else {
        (BASE, DICTIONARIES)
    };
    let base_path = base_outputs.dictionary;
    let names: Vec<String> = if names.is_empty() {
        additional_names()?
    } else {
        names.iter().map(|&n| n.to_owned()).collect()
    };
    // Built for the evaluation, a dictionary made from documents leaves out
    // the ones that are eval documents of the base too.
    let eval_ids = if train_only {
        document_ids(open(DOCS)?, |doc_id| split_of(doc_id) == Split::Eval).map_err(|source| {
            Error::Documents {
                path: DOCS.into(),
                source,
            }
        })?
    } else {
        HashSet::new()
    };
    let eval_ids = &eval_ids;
    let wanted = move |doc_id: &str| !eval_ids.contains(doc_id);
    let base = {
        let text = read_to_string(base_path)?;
        Base::parse(text).map_err(|source| Error::Unreadable {
            path: base_path.into(),
            source,
        })?
    };
    // Read before any dictionary is built, so a missing cost fails the build
    // up front.
    let seen_once = names
        .iter()
        .any(|name| LISTS.contains(&name.as_str()))
        .then(|| seen_once_cost(base_outputs.seen_once))
        .transpose()?;
    let corrections = corrections()?;
    let mut analyzer = None;
    for name in names {
        let label = read_to_string(format!("{ADDITIONAL}/{name}/label.txt"))?;
        let listed = [SYMBOL, EMOJI, EMOTICON].contains(&name.as_str());
        let (mut dictionary, sources) = if listed {
            let table = |file: &str| read_to_string(format!("{ADDITIONAL_BUILD}/{name}/{file}"));
            let (pairs, sources) = match name.as_str() {
                SYMBOL => (mozc_symbols(&table("symbol.tsv")?), vec!["mozc-symbol"]),
                EMOJI => (
                    mozc_emoji(&table("emoji_data.tsv")?, &table("manual_emoji_data.tsv")?),
                    vec!["mozc-emoji:emoji_data", "mozc-emoji:manual_emoji_data"],
                ),
                _ => (
                    mozc_emoticons(&table("emoticon.tsv")?),
                    vec!["mozc-emoticon"],
                ),
            };
            (
                listed_dictionary(pairs),
                sources.into_iter().map(String::from).collect(),
            )
        } else if name == PERSON {
            let mut sources = BTreeSet::from([PEOPLE_SOURCE.to_owned(), KANJI_SOURCE.to_owned()]);
            let mut people = wikidata_people(&read_to_string(format!(
                "{ADDITIONAL_BUILD}/{name}/people.tsv"
            ))?);
            let read_by_leads = format!("{ADDITIONAL_BUILD}/{name}/wikipedia-people.tsv");
            if std::path::Path::new(&read_by_leads).exists() {
                people = with_people_read_by_leads(
                    people,
                    wikidata_people(&read_to_string(read_by_leads)?),
                );
                sources.extend([UNREAD_PEOPLE_SOURCE, TITLES_SOURCE].map(String::from));
            }
            let readings = name_readings(&mozc_readings(&read_to_string(KANJI_TABLE)?));
            let clans = clans(&read_to_string(format!("{ADDITIONAL}/{name}/uji.txt"))?);
            (person_dictionary(&people, &readings, &clans), sources)
        } else if name == STATION {
            let stations = wikidata_stations(&read_to_string(format!(
                "{ADDITIONAL_BUILD}/{name}/stations.tsv"
            ))?);
            (
                station_dictionary(&stations),
                BTreeSet::from([STATIONS_SOURCE.to_owned()]),
            )
        } else if name == PLACE {
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
            let dictionary = okurigana_dictionary(
                read_units(open(UNITS)?)
                    .map_while(|unit| {
                        unit.map_err(|source| {
                            failure = Some(Error::Units {
                                path: UNITS.into(),
                                source,
                            })
                        })
                        .ok()
                    })
                    .filter(|unit| !train_only || split_of(&unit.doc_id) == Split::Train),
            );
            if let Some(e) = failure {
                return Err(e);
            }
            (dictionary, sources)
        } else {
            if analyzer.is_none() {
                analyzer = Some(open_analyzer()?);
            }
            let (mut dictionary, sources) =
                sourced_dictionary(&name, &base, analyzer.as_ref().unwrap(), wanted)?;
            dictionary.add_words(nihon_readings(&dictionary.entries));
            (dictionary, sources)
        };
        // Symbols and emoticons are not words, and may hold spaces or end in 〜.
        dictionary.drop_words(|reading, surface| {
            corrections.drops(surface) || !listed && !is_word(reading, surface)
        });
        let dictionary_path = format!("{out_dir}/{name}.tsv");
        let report_path = format!("{out_dir}/{name}-report.tsv");
        let keep = |line: &kanaemi_engine::ItemLine<'_>, kind| base.keeps(line, kind);
        let text = if LISTS.contains(&name.as_str()) {
            let start = seen_once.expect("read up front whenever a list dictionary is built");
            dictionary.to_text_where_starting_at(label.trim(), start, keep)
        } else {
            dictionary.to_text_where(label.trim(), keep)
        };
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
        if !train_only {
            write_sources(&dictionary_path, text.as_bytes(), None, &sources)?;
        }
        println!(
            "{name}\tlines: {}\tout: {dictionary_path}",
            text.lines().count() - 1
        );
    }
    Ok(())
}

/// The cost of a word met once in the base's units, as its build wrote it to
/// `path`.
fn seen_once_cost(path: &str) -> Result<u32, Error> {
    read_to_string(path)?
        .trim()
        .parse()
        .map_err(|_| Error::SeenOnce { path: path.into() })
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
    wanted: impl Fn(&str) -> bool + Copy,
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
    let units: Vec<Unit> = read_all_units(&units_path)?
        .into_iter()
        .filter(|u| wanted(&u.doc_id))
        .collect();
    let texts = document_texts(open(&docs)?, wanted).map_err(|source| Error::Documents {
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
            Ok(file) => {
                read_titles_of(BufReader::new(file), wanted).map_err(|source| Error::Titles {
                    path: path.into(),
                    source,
                })?
            }
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
    let older_wanted = |doc_id: &str| older_doc(doc_id) && wanted(doc_id);
    let older = document_texts(open(DOCS)?, older_wanted).map_err(documents_error)?;
    sources.extend(document_sources(open(DOCS)?, older_wanted).map_err(documents_error)?);
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

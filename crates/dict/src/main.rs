//! `kanaemi-dict`: cuts the collected documents into units and builds the base
//! dictionary from them, reading and writing under the current directory.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use kanaemi_dict::{
    Analyzer, AnalyzerError, BASE_LABEL, CutError, Dictionary, RejectedLines, UnidicError, Unit,
    WriteError, cut_documents, plain_words, write_atomically,
};

const SYSTEM_DICTIONARY: &str = "build/sudachi/system_full.dic";
const LEXICON: &str = "build/sudachi/small_lex.csv";
const DOCS: &str = "build/docs.jsonl";
const UNITS: &str = "build/units.jsonl";
const DICTIONARY: &str = "build/dictionaries/base.tsv";
const REPORT: &str = "build/dictionaries/base-report.tsv";

const USAGE: &str = "\
usage: kanaemi-dict units
         cut every document of build/docs.jsonl into build/units.jsonl,
         reading them with build/sudachi/system_full.dic
       kanaemi-dict dictionary
         build build/dictionaries/base.tsv from build/units.jsonl and the
         UniDic lexicon build/sudachi/small_lex.csv, reporting to
         build/dictionaries/base-report.tsv";

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
    #[error("{}: line {line}: {source}", path.display())]
    Units {
        path: PathBuf,
        line: usize,
        source: io::Error,
    },
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error("{DICTIONARY} left as it was: {0}")]
    Rejected(#[from] RejectedLines),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["units"] => cut_units(),
        ["dictionary"] => build_dictionary(),
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

/// Builds the base dictionary, and writes it and its report only when Kanaemi
/// reads every line.
fn build_dictionary() -> Result<(), Error> {
    let unidic = plain_words(open(LEXICON)?).map_err(|source| Error::Unidic {
        path: LEXICON.into(),
        source,
    })?;
    let dictionary = {
        let mut failure = None;
        let units =
            read_units(open(UNITS)?).map_while(|unit| unit.map_err(|e| failure = Some(e)).ok());
        let dictionary = Dictionary::build(units, &unidic);
        if let Some(e) = failure {
            return Err(e);
        }
        dictionary
    };
    let text = dictionary.to_checked_text(BASE_LABEL)?;
    write_atomically(DICTIONARY, |w| {
        w.write_all(text.as_bytes())
            .map_err(write_error(DICTIONARY))
    })?;
    write_atomically(REPORT, |w| {
        w.write_all(dictionary.report.to_tsv().as_bytes())
            .map_err(write_error(REPORT))
    })?;
    println!(
        "entries: {}, okurigana lines: {}, unknown types: {}, disallowed okurigana: {}, out: {DICTIONARY}, report: {REPORT}",
        dictionary.entries.len(),
        dictionary.okuri.len(),
        dictionary.report.unknown_conjugations.len(),
        dictionary.report.disallowed_okurigana.len(),
    );
    Ok(())
}

fn write_error(path: &str) -> impl FnOnce(io::Error) -> Error {
    move |source| {
        Error::Write(WriteError {
            path: path.into(),
            source,
        })
    }
}

/// The units of `build/units.jsonl`, one per line.
fn read_units(reader: impl BufRead) -> impl Iterator<Item = Result<Unit, Error>> {
    reader.lines().enumerate().map(|(i, line)| {
        let units_error = |source| Error::Units {
            path: UNITS.into(),
            line: i + 1,
            source,
        };
        let line = line.map_err(units_error)?;
        serde_json::from_str(&line).map_err(|e| units_error(e.into()))
    })
}

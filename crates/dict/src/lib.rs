//! Building Kanaemi's official dictionaries and ranking model.
//!
//! It reads collected text with an analyzer and keeps the words that text
//! actually uses, so the dictionaries never copy another dictionary's list.

mod additional;
mod analyzer;
mod chains;
mod checks;
mod conjugation;
mod corrections;
mod counted;
mod dictionary;
mod dist;
mod documents;
mod eval_documents;
mod evaluation;
mod kana;
mod kanji;
mod numeral;
mod output;
mod place;
mod ranking;
mod split;
mod symbols;
mod unidic;
mod units;
mod wikidata;

pub use additional::*;
pub use analyzer::*;
pub use chains::*;
pub use checks::*;
pub use corrections::*;
pub use counted::*;
pub use dictionary::*;
pub use dist::*;
pub use documents::*;
pub use eval_documents::*;
pub use evaluation::*;
pub use kana::*;
pub use kanji::*;
pub use numeral::*;
pub use output::*;
pub use place::*;
pub use ranking::*;
pub use split::*;
pub use symbols::*;
pub use unidic::*;
pub use units::*;
pub use wikidata::*;

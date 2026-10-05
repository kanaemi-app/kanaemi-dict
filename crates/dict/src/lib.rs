//! Building Kanaemi's official dictionaries and ranking model.
//!
//! It reads collected text with an analyzer and keeps the words that text
//! actually uses, so the dictionaries never copy another dictionary's list.

mod additional;
mod analyzer;
mod conjugation;
mod dictionary;
mod documents;
mod eval_documents;
mod evaluation;
mod kana;
mod numeral;
mod output;
mod place;
mod ranking;
mod split;
mod unidic;
mod units;

pub use additional::*;
pub use analyzer::*;
pub use dictionary::*;
pub use documents::*;
pub use eval_documents::*;
pub use evaluation::*;
pub use kana::*;
pub use numeral::*;
pub use output::*;
pub use place::*;
pub use ranking::*;
pub use split::*;
pub use unidic::*;
pub use units::*;

//! Building Kanaemi's official dictionaries and ranking model.
//!
//! It reads collected text with an analyzer and keeps the words that text
//! actually uses, so the dictionaries never copy another dictionary's list.

mod analyzer;
mod conjugation;
mod dictionary;
mod documents;
mod kana;
mod output;
mod unidic;
mod units;

pub use analyzer::*;
pub use dictionary::*;
pub use documents::*;
pub use kana::*;
pub use output::*;
pub use unidic::*;
pub use units::*;

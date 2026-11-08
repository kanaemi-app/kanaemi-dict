//! The readings people confirmed for a surface beside the one the analyzer
//! gives it, as `analyzer/added-readings.tsv` writes them.

use crate::kana::is_reading_kana;

/// A reading added to a surface the text has.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddedReading {
    pub surface: String,
    /// In hiragana.
    pub reading: String,
}

/// What an added reading did to the dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddedReadingUse {
    /// It added the line.
    Added,
    /// Its surface occurs too rarely outside compounds to enter.
    Rare,
    /// The dictionary has the line without it.
    Given,
}

#[derive(Debug, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct AddedReadingsError {
    pub line: usize,
    pub reason: String,
}

/// The added readings of `text`, a surface and a reading per tab-separated
/// line; lines starting with `#` and blank lines are skipped.
pub fn parse_added_readings(
    text: impl AsRef<str>,
) -> Result<Vec<AddedReading>, AddedReadingsError> {
    text.as_ref()
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|(i, line)| {
            parse_line(line).map_err(|reason| AddedReadingsError {
                line: i + 1,
                reason,
            })
        })
        .collect()
}

fn parse_line(line: &str) -> Result<AddedReading, String> {
    let [surface, reading] = line.split('\t').collect::<Vec<_>>()[..] else {
        return Err(format!("{line:?} does not have two tab-separated fields"));
    };
    if surface.is_empty() {
        return Err("the surface is empty".into());
    }
    if reading.is_empty() || !reading.chars().all(is_reading_kana) {
        return Err(format!("the reading {reading:?} is not hiragana"));
    }
    Ok(AddedReading {
        surface: surface.to_owned(),
        reading: reading.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_give_a_surface_and_a_reading_skipping_comments_and_blank_lines() {
        let text = "# 表記\t読み\n空行\tからぎょう\n\n未払い\tみばらい\n";

        assert_eq!(
            parse_added_readings(text).unwrap(),
            [
                AddedReading {
                    surface: "空行".into(),
                    reading: "からぎょう".into(),
                },
                AddedReading {
                    surface: "未払い".into(),
                    reading: "みばらい".into(),
                },
            ]
        );
    }

    #[test]
    fn a_reading_not_in_hiragana_is_an_error_with_its_line() {
        for reading in ["-", "みばら*い", "カラギョウ", ""] {
            let error = parse_added_readings(format!("# c\n空行\t{reading}\n")).unwrap_err();
            assert_eq!(error.line, 2, "{reading:?}");
        }
    }

    #[test]
    fn a_line_without_two_fields_is_an_error() {
        assert!(parse_added_readings("空行\n").is_err());
        assert!(parse_added_readings("空行\tからぎょう\tx\n").is_err());
        assert!(parse_added_readings("\tからぎょう\n").is_err());
    }
}

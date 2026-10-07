//! The symbol, emoji and emoticon dictionaries, taken from Mozc's tables:
//! words text cannot make, since text never writes how a symbol is read.

use std::collections::{BTreeMap, BTreeSet};

use crate::kana::is_hiragana;
use crate::{Dictionary, Entry, katakana_to_hiragana};

/// The cost of the first symbol of a reading: band 13 of the ranking model,
/// behind every word of the base dictionary.
const FIRST_COST: u32 = 8192;
const LAST_COST: u32 = 16383;

/// `reading` as it is typed with Kanaemi's default romaji table: ASCII
/// symbols and digits full-width, `- , . [ ] / ~` as `ー 、 。 「 」 ・ 〜`, and
/// katakana as hiragana; none when anything else stays, such as letters,
/// which type as kana, or kanji.
pub fn typed_reading(reading: &str) -> Option<String> {
    let typed: String = katakana_to_hiragana(reading)
        .chars()
        .map(|c| match c {
            '-' => 'ー',
            ',' => '、',
            '.' => '。',
            '[' => '「',
            ']' => '」',
            '/' => '・',
            '~' => '〜',
            '!'..='~' if !c.is_ascii_alphabetic() => {
                char::from_u32(c as u32 - 0x21 + 0xFF01).unwrap_or(c)
            }
            c => c,
        })
        .collect();
    let typable = |c: char| {
        is_hiragana(c)
            || matches!(c, 'ー' | '、' | '。' | '「' | '」' | '・' | '〜')
            || matches!(c, '\u{FF01}'..='\u{FF5E}')
                && !matches!(c, '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}')
    };
    (!typed.is_empty() && typed.chars().all(typable)).then_some(typed)
}

/// The (reading, symbol) pairs of Mozc's symbol.tsv, in its order, without
/// its header and hentaigana, which few fonts show.
pub fn mozc_symbols(tsv: &str) -> Vec<(String, String)> {
    let rows = tsv.strip_prefix("POS\t").map_or(tsv, |rest| {
        rest.split_once('\n').map_or("", |(_, rows)| rows)
    });
    pairs(rows, 1, 2)
        .into_iter()
        .filter(|(_, symbol)| !symbol.chars().any(is_hentaigana))
        .collect()
}

/// The (reading, emoji) pairs of Mozc's emoji_data.tsv and then of its
/// manual_emoji_data.tsv, in their order.
pub fn mozc_emoji(data: &str, manual: &str) -> Vec<(String, String)> {
    let mut emoji = pairs(data, 1, 2);
    emoji.extend(pairs(manual, 1, 2));
    emoji
}

/// The (reading, emoticon) pairs of Mozc's emoticon.tsv, in its order.
pub fn mozc_emoticons(tsv: &str) -> Vec<(String, String)> {
    pairs(tsv, 0, 1)
}

/// The (reading, surface) pairs of a Mozc table whose `surface` column holds
/// the character and whose `readings` column its readings split by spaces,
/// skipping comments and rows without either.
fn pairs(tsv: &str, surface: usize, readings: usize) -> Vec<(String, String)> {
    tsv.lines()
        .filter(|line| !line.starts_with('#'))
        .flat_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let symbol = fields.get(surface).copied().unwrap_or_default();
            let readings = fields.get(readings).copied().unwrap_or_default();
            let symbol = (!symbol.is_empty() && !symbol.starts_with('#')).then_some(symbol);
            readings
                .split([' ', '\u{3000}'])
                .filter(|r| !r.is_empty())
                .filter_map(move |r| Some((r.to_owned(), symbol?.to_owned())))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn is_hentaigana(c: char) -> bool {
    matches!(c, '\u{1B000}'..='\u{1B16F}')
}

/// The dictionary of `pairs`, each reading typed as [`typed_reading`] gives
/// and each pair once; a pair read as it is written is left out, since
/// Kanaemi gives what was typed. Within a reading, the pairs cost more in
/// their order.
pub fn listed_dictionary(pairs: impl IntoIterator<Item = (String, String)>) -> Dictionary {
    let mut seen = BTreeSet::new();
    let mut order: BTreeMap<String, u32> = BTreeMap::new();
    let mut entries = Vec::new();
    for (reading, surface) in pairs {
        let Some(reading) = typed_reading(&reading) else {
            continue;
        };
        if reading == surface || !seen.insert((reading.clone(), surface.clone())) {
            continue;
        }
        let n = order.entry(reading.clone()).or_default();
        let cost = (FIRST_COST + *n).min(LAST_COST);
        *n += 1;
        entries.push(Entry {
            reading,
            surface,
            conjugation: None,
            cost,
        });
    }
    Dictionary {
        entries,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_is_typed_as_the_default_romaji_table_types_it() {
        assert_eq!(typed_reading("やじるし").as_deref(), Some("やじるし"));
        assert_eq!(typed_reading("#").as_deref(), Some("＃"));
        assert_eq!(typed_reading("1").as_deref(), Some("１"));
        assert_eq!(typed_reading(",").as_deref(), Some("、"));
        assert_eq!(typed_reading("-").as_deref(), Some("ー"));
        assert_eq!(typed_reading("~").as_deref(), Some("〜"));
        assert_eq!(typed_reading("ふりヴにゃ").as_deref(), Some("ふりゔにゃ"));
    }

    #[test]
    fn a_reading_with_letters_or_kanji_is_not_typed() {
        assert_eq!(typed_reading("ok"), None);
        assert_eq!(typed_reading("ｏｋ"), None);
        assert_eq!(typed_reading("四角数字"), None);
        assert_eq!(typed_reading(""), None);
    }

    #[test]
    fn a_symbol_row_gives_its_readings_in_order_without_comments() {
        let tsv = "POS\tCHAR\tReading (space separated)\tdescription\n\
            句読点\t、\tとうてん , 、\t読点\t\tOTHER\n\
            \t\t\t\t\t\t\t# ⇒ defined at above\n\
            # Hentaigana\n\
            記号\t\u{1B000}\tへんたいがな え\t変体仮名\n\
            記号\t→\tやじるし　みぎ\t右矢印\n";

        assert_eq!(
            mozc_symbols(tsv),
            [
                ("とうてん".into(), "、".into()),
                (",".into(), "、".into()),
                ("、".into(), "、".into()),
                ("やじるし".into(), "→".into()),
                ("みぎ".into(), "→".into()),
            ]
        );
    }

    #[test]
    fn emoji_take_both_tables_and_emoticons_their_first_column() {
        let data = "# comment\n1F431\t🐱\tねこ かお\tCAT FACE\t猫の顔\t\tE0.6\n";
        let manual = "1F431\t🐱\tにゃー\t\t\t\n";
        let emoticons = "\tkeys\tcategories\n(^^)\tにこにこ にこ\tSMILE\n";

        assert_eq!(
            mozc_emoji(data, manual),
            [
                ("ねこ".into(), "🐱".into()),
                ("かお".into(), "🐱".into()),
                ("にゃー".into(), "🐱".into()),
            ]
        );
        assert_eq!(
            mozc_emoticons(emoticons),
            [
                ("にこにこ".into(), "(^^)".into()),
                ("にこ".into(), "(^^)".into()),
            ]
        );
    }

    #[test]
    fn the_dictionary_costs_more_down_a_reading_and_skips_what_types_as_is() {
        let pairs = [
            ("まる", "○"),
            ("まる", "●"),
            ("まる", "○"),
            ("ok", "👌"),
            (",", "、"),
            (",", "，"),
            ("やじるし", "→"),
        ]
        .map(|(r, s)| (r.to_owned(), s.to_owned()));

        let dict = listed_dictionary(pairs);

        let entries: Vec<_> = dict
            .entries
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str(), e.cost))
            .collect();
        assert_eq!(
            entries,
            [
                ("まる", "○", 8192),
                ("まる", "●", 8193),
                ("、", "，", 8192),
                ("やじるし", "→", 8192),
            ]
        );
    }
}

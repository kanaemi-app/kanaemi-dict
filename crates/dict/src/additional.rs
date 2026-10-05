//! Additional dictionaries: words a field or a year adds to the base
//! dictionary, without what the base dictionary already gives.

use std::collections::HashSet;
use std::io::{self, BufRead};
use std::sync::Arc;

use aho_corasick::AhoCorasick;
use kanaemi_engine::{Engine, ItemLine, TextDictionary};

use crate::dictionary::{MIN_COUNT, cost_of, okuri_lines};
use crate::evaluation::{Query, engine, rank_of};
use crate::kana::{is_hiragana, is_kanji, is_katakana};
use crate::{Dictionary, Entry, LineKind, RejectedLines, Unit};

/// A Wikipedia article's title, without its qualifier, with the reading its
/// text gives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Title {
    pub reading: String,
    pub surface: String,
}

/// How many times more often, per character, a year's new word is used in
/// its year than in older text.
const NOVELTY: f64 = 10.0;

/// Words written whole from `titles`, each costing by how often its surface
/// occurs in `texts` against `total` units. Like a unit, a title needs kanji
/// or katakana and at least two characters, and enters when it occurs, as a
/// word of its own (see [`word_counts`]), as often as a counted word must.
pub fn title_entries<'a>(
    titles: &[Title],
    texts: impl IntoIterator<Item = &'a str>,
    total: usize,
) -> Vec<Entry> {
    let titles: Vec<&Title> = titles
        .iter()
        .filter(|t| t.surface.chars().nth(1).is_some() && needs_conversion(&t.surface))
        .collect();
    let surfaces: Vec<&str> = titles.iter().map(|t| t.surface.as_str()).collect();
    titles
        .into_iter()
        .zip(word_counts(&surfaces, texts))
        .filter(|&(_, n)| n >= MIN_COUNT)
        .map(|(t, n)| Entry {
            reading: t.reading.clone(),
            surface: t.surface.clone(),
            conjugation: None,
            cost: cost_of(n, total),
        })
        .collect()
}

/// The `titles` used in `year` at least [`NOVELTY`] times as often, per
/// character, as in `older`, counted as words of their own. One occurrence is
/// added to the older count, so that words older text never uses compare by
/// how often the year uses them.
pub fn novel_titles<'a, 'b>(
    titles: &[Title],
    year: impl IntoIterator<Item = &'a str>,
    older: impl IntoIterator<Item = &'b str>,
) -> Vec<Title> {
    let surfaces: Vec<&str> = titles.iter().map(|t| t.surface.as_str()).collect();
    let (in_year, year_chars) = counts_and_chars(&surfaces, year);
    let (in_older, older_chars) = counts_and_chars(&surfaces, older);
    titles
        .iter()
        .zip(in_year.into_iter().zip(in_older))
        .filter(|(_, (y, o))| *y as f64 / year_chars >= NOVELTY * (*o + 1) as f64 / older_chars)
        .map(|(t, _)| t.clone())
        .collect()
}

fn counts_and_chars<'a>(
    words: &[&str],
    texts: impl IntoIterator<Item = &'a str>,
) -> (Vec<usize>, f64) {
    let mut chars = 0usize;
    let counts = word_counts(
        words,
        texts.into_iter().inspect(|t| chars += t.chars().count()),
    );
    (counts, chars.max(1) as f64)
}

/// How often each of `words` occurs in `texts` as a word of its own: an
/// occurrence that a kanji, a katakana, or a letter or digit continues on
/// either side is part of a longer word and not counted.
pub fn word_counts<'a>(words: &[&str], texts: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut counts = vec![0usize; words.len()];
    let Ok(matcher) = AhoCorasick::new(words) else {
        return counts;
    };
    let joins = |a: Option<char>, b: Option<char>| {
        a.zip(b)
            .and_then(|(a, b)| Some(script_of(a)? == script_of(b)?))
            .unwrap_or(false)
    };
    for text in texts {
        for m in matcher.find_overlapping_iter(text) {
            let (before, word, after) = (&text[..m.start()], &text[m.range()], &text[m.end()..]);
            if !joins(before.chars().next_back(), word.chars().next())
                && !joins(word.chars().next_back(), after.chars().next())
            {
                counts[m.pattern().as_usize()] += 1;
            }
        }
    }
    counts
}

#[derive(PartialEq)]
enum Script {
    Kanji,
    Katakana,
    Alphanumeric,
}

/// The kind of character a word runs on in. Hiragana and symbols end words.
fn script_of(c: char) -> Option<Script> {
    if is_kanji(c) {
        Some(Script::Kanji)
    } else if is_katakana(c) || c == 'ー' {
        Some(Script::Katakana)
    } else if c.is_alphanumeric() && !is_hiragana(c) {
        Some(Script::Alphanumeric)
    } else {
        None
    }
}

fn needs_conversion(s: &str) -> bool {
    s.chars().any(|c| is_kanji(c) || is_katakana(c))
}

/// The base dictionary as an additional dictionary is measured against: its
/// lines, and Kanaemi converting with it alone.
pub struct Base {
    /// Every line without its cost, as written: reading, surface, and the
    /// conjugation type when there is one.
    keys: HashSet<String>,
    /// Every line's surface, as written.
    surfaces: HashSet<String>,
    engine: Engine,
}

impl Base {
    /// The base dictionary of a text dictionary, unless Kanaemi would not
    /// read some line of it.
    pub fn parse(text: impl AsRef<str>) -> Result<Self, RejectedLines> {
        let text = text.as_ref();
        let (dictionary, invalid) = TextDictionary::parse(text);
        if !invalid.is_empty() {
            return Err(RejectedLines(invalid));
        }
        // Lines are compared as written: both sides are written by
        // `ItemLine`, which escapes every tab inside a field, so splitting
        // on tabs splits fields, and equal fields mean equal values.
        let mut keys = HashSet::new();
        let mut surfaces = HashSet::new();
        for line in item_lines(text) {
            keys.insert(line_key(line));
            if let Some(surface) = line.split('\t').nth(1) {
                surfaces.insert(surface.to_owned());
            }
        }
        Ok(Self {
            keys,
            surfaces,
            engine: engine(Arc::new(dictionary)),
        })
    }

    /// The first item line of `text` the base dictionary has too, cost aside.
    pub fn shared_line<'a>(&self, text: &'a str) -> Option<&'a str> {
        item_lines(text).find(|line| self.keys.contains(&line_key(line)))
    }

    /// The `titles` whose surface no line of the base dictionary has, under
    /// any reading.
    pub fn new_titles(&self, titles: &[Title]) -> Vec<Title> {
        titles
            .iter()
            .filter(|t| !self.surfaces.contains(&written_surface(&t.surface)))
            .cloned()
            .collect()
    }

    /// Whether an additional dictionary keeps `line`: a line the base
    /// dictionary has is left out, and so is a word written whole or with
    /// okurigana that Kanaemi already offers for its reading with the base
    /// dictionary alone.
    pub fn keeps(&self, line: &ItemLine, kind: LineKind) -> bool {
        let key = ItemLine {
            cost: None,
            ..line.clone()
        }
        .to_string();
        if self.keys.contains(&key) {
            return false;
        }
        let query = match kind {
            LineKind::Word if line.conjugation.is_none() => Query {
                reading: line.reading.to_owned(),
                okurigana: None,
                expected: line.surface.to_owned(),
            },
            LineKind::Okurigana => {
                let kana = line.okurigana.unwrap_or_default();
                Query {
                    reading: format!("{}{kana}", line.reading),
                    okurigana: Some(kana.to_owned()),
                    expected: line.surface.to_owned(),
                }
            }
            _ => return true,
        };
        rank_of(&self.engine, &query).is_none()
    }
}

fn item_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

/// A line's reading, surface and conjugation type, as written, without its
/// cost.
fn line_key(line: &str) -> String {
    let fields: Vec<&str> = line.split('\t').collect();
    let named = match fields.get(2) {
        Some(conjugation) if !conjugation.is_empty() => 3,
        _ => 2,
    };
    fields[..named.min(fields.len())].join("\t")
}

/// `surface` as a line of a text dictionary writes it.
fn written_surface(surface: &str) -> String {
    let line = ItemLine {
        surface,
        ..Default::default()
    }
    .to_string();
    line.split_once('\t')
        .map_or(line.clone(), |(_, s)| s.to_owned())
}

#[derive(Debug, thiserror::Error)]
pub enum TitlesError {
    #[error("line {line}: {source}")]
    Read { line: usize, source: io::Error },
    #[error("line {line}: not a doc ID, a non-empty reading and a non-empty surface")]
    Fields { line: usize },
}

/// The titles of a `doc_id<TAB>reading<TAB>surface` file, sorted, without
/// repeats.
pub fn read_titles(tsv: impl BufRead) -> Result<Vec<Title>, TitlesError> {
    let mut titles = Vec::new();
    for (i, line) in tsv.lines().enumerate() {
        let line = line.map_err(|source| TitlesError::Read {
            line: i + 1,
            source,
        })?;
        // An empty surface would be an empty pattern for the matcher, which
        // matches between the bytes of a character.
        let [_, reading, surface] = line.split('\t').collect::<Vec<_>>()[..] else {
            return Err(TitlesError::Fields { line: i + 1 });
        };
        if reading.is_empty() || surface.is_empty() {
            return Err(TitlesError::Fields { line: i + 1 });
        }
        titles.push(Title {
            reading: reading.to_owned(),
            surface: surface.to_owned(),
        });
    }
    titles.sort();
    titles.dedup();
    Ok(titles)
}

/// A field's dictionary: the stems and words of the units of its documents,
/// and the `titles` its documents' `texts` use, costing against the units.
/// UniDic's words are left out; the base dictionary has them.
pub fn field_dictionary(units: &[Unit], titles: &[Title], texts: &[String]) -> Dictionary {
    let mut dictionary = Dictionary::build(units, &[]);
    let added: Vec<Entry> = {
        let have: HashSet<(&str, &str)> = dictionary
            .entries
            .iter()
            .filter(|e| e.conjugation.is_none())
            .map(|e| (e.reading.as_str(), e.surface.as_str()))
            .collect();
        title_entries(titles, texts.iter().map(String::as_str), units.len())
            .into_iter()
            .filter(|e| !have.contains(&(e.reading.as_str(), e.surface.as_str())))
            .collect()
    };
    dictionary.entries.extend(added);
    dictionary.okuri = okuri_lines(&dictionary.entries);
    dictionary
}

/// A year's dictionary of new words: the `titles` whose surface the base
/// dictionary lacks and that the `year`'s texts use far more than `older`
/// texts, costing against the year's `units`.
pub fn year_dictionary(
    titles: &[Title],
    base: &Base,
    year: &[String],
    older: &[String],
    units: usize,
) -> Dictionary {
    let titles = novel_titles(
        &base.new_titles(titles),
        year.iter().map(String::as_str),
        older.iter().map(String::as_str),
    );
    let entries = title_entries(&titles, year.iter().map(String::as_str), units);
    Dictionary {
        okuri: okuri_lines(&entries),
        entries,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dictionary;

    fn title(reading: &str, surface: &str) -> Title {
        Title {
            reading: reading.into(),
            surface: surface.into(),
        }
    }

    fn surfaces(entries: Vec<Entry>) -> Vec<String> {
        entries.into_iter().map(|e| e.surface).collect()
    }

    #[test]
    fn titles_the_texts_use_often_enough_enter_with_their_reading() {
        let titles = [
            title("れいわのこめそうどう", "令和の米騒動"),
            title("べきとう", "冪等"),
        ];
        let texts = ["令和の米騒動が続く。冪等な処理。", "令和の米騒動。"];

        assert_eq!(
            title_entries(&titles, texts, 100),
            [Entry {
                reading: "れいわのこめそうどう".into(),
                surface: "令和の米騒動".into(),
                conjugation: None,
                cost: cost_of(2, 100),
            }]
        );
    }

    #[test]
    fn a_title_inside_a_longer_word_is_not_counted() {
        let titles = [
            title("にっちゅうせん", "日中戦"),
            title("ちゅうどうかいかく", "中道改革"),
        ];
        let texts = [
            "日中戦争と日中戦争。",
            "中道改革が進む。中道改革連合の中道改革。",
        ];

        assert_eq!(surfaces(title_entries(&titles, texts, 100)), ["中道改革"]);
    }

    #[test]
    fn titles_in_hiragana_or_of_one_character_do_not_enter() {
        let titles = [
            title("なふさしょっく", "ナフサショック"),
            title("かみ", "上"),
            title("ぬい", "ぬい"),
        ];
        let texts = ["ナフサショックの上でぬい。ナフサショックの上でぬい。"];

        assert_eq!(
            surfaces(title_entries(&titles, texts, 100)),
            ["ナフサショック"]
        );
    }

    #[test]
    fn titles_used_far_more_in_the_year_than_in_older_text_are_novel() {
        let titles = [
            title("どぱがき", "ドパガキ"),
            title("こかいん", "コカイン"),
            title("うつぼ", "ウツボ"),
        ];
        // The year: 39 characters; older text: 200.
        let year = [
            "ドパガキとコカインとウツボの話。ドパガキ。",
            "ウツボの話をする今年の記事。ドパガキ",
        ];
        let older = format!("{}コカイン。ウツボ。{}", "古".repeat(93), "古".repeat(98));

        // ドパガキ 3 in 39 against (0+1) in 200: 15.4 times. ウツボ 2 in 39
        // against (1+1) in 200: 5.1 times. コカイン 1 in 39 against (1+1) in
        // 200: 2.6 times.
        assert_eq!(
            novel_titles(&titles, year, [older.as_str()]),
            [title("どぱがき", "ドパガキ")]
        );
    }

    #[test]
    fn only_titles_whose_surface_the_base_lacks_under_every_reading_are_new() {
        let base = Base::parse("# 基本\nうえの\t上野\t\t971\nか\t書\t五段-カ行\t10\n").unwrap();

        assert_eq!(
            base.new_titles(&[
                title("うわの", "上野"),
                title("しょ", "書"),
                title("べきとう", "冪等")
            ]),
            [title("べきとう", "冪等")]
        );
    }

    #[test]
    fn a_surface_with_a_tab_is_compared_as_written() {
        let base = Base::parse("たぶ\ta\\tb\t\t10\n").unwrap();

        assert_eq!(base.new_titles(&[title("たぶ", "a\tb")]), []);
    }

    fn kept(base: &str, dictionary: &Dictionary) -> Vec<String> {
        let base = Base::parse(base).unwrap();
        dictionary
            .to_text_where("IT", |line, kind| base.keeps(line, kind))
            .lines()
            .skip(1)
            .map(str::to_owned)
            .collect()
    }

    fn entry(reading: &str, surface: &str, conjugation: Option<&str>, cost: u32) -> Entry {
        Entry {
            reading: reading.into(),
            surface: surface.into(),
            conjugation: conjugation.map(Into::into),
            cost,
        }
    }

    #[test]
    fn words_kanaemi_already_offers_with_the_base_go_and_the_rest_stays() {
        let base = "# 基本\nか\t書\t五段-カ行\t10\nきしゃ\t記者\t\t20\n";
        let dictionary = Dictionary {
            entries: vec![
                entry("かきます", "書きます", None, 5),
                entry("きしゃ", "記者", None, 3),
                entry("べきとう", "冪等", None, 9),
                entry("こうぞう", "構造", Some("五段-カ行"), 4),
            ],
            okuri: vec![
                entry("か*き", "書き", None, 5),
                entry("かま*え", "構え", None, 4),
            ],
            ..Default::default()
        };

        assert_eq!(
            kept(base, &dictionary),
            [
                "かま*え\t構え\t\t4",
                "こうぞう\t構造\t五段-カ行\t4",
                "べきとう\t冪等\t\t9"
            ]
        );
    }

    #[test]
    fn a_stem_line_the_base_has_goes_whatever_its_cost() {
        let base = "か\t書\t五段-カ行\t10\n";
        let dictionary = Dictionary {
            entries: vec![
                entry("か", "書", Some("五段-カ行"), 3),
                entry("か", "書", Some("五段-ガ行"), 3),
            ],
            ..Default::default()
        };

        assert_eq!(kept(base, &dictionary), ["か\t書\t五段-ガ行\t3"]);
    }

    #[test]
    fn a_katakana_word_kanaemi_offers_as_the_reading_goes() {
        let dictionary = Dictionary {
            entries: vec![entry("なふさ", "ナフサ", None, 3)],
            ..Default::default()
        };

        assert_eq!(kept("", &dictionary), Vec::<String>::new());
    }

    #[test]
    fn a_word_with_a_backslash_is_checked_like_any_other() {
        let base = "えん\t\\\\\t\t10\n";
        let dictionary = Dictionary {
            entries: vec![entry("えん", "\\", None, 3), entry("えん", "円", None, 3)],
            ..Default::default()
        };

        assert_eq!(kept(base, &dictionary), ["えん\t円\t\t3"]);
    }
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use crate::{Dictionary, Unit};

    fn title(reading: &str, surface: &str) -> Title {
        Title {
            reading: reading.into(),
            surface: surface.into(),
        }
    }

    fn word(reading: &str, surface: &str) -> Unit {
        Unit {
            doc_id: "wikipedia:1".into(),
            position: 0,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        }
    }

    #[test]
    fn titles_are_read_without_repeats_in_order() {
        let tsv = "wikipedia:2\tべきとう\t冪等\nwikipedia:1\tえーぴーあい\tAPI\nwikipedia:3\tべきとう\t冪等\n";

        assert_eq!(
            read_titles(tsv.as_bytes()).unwrap(),
            [title("えーぴーあい", "API"), title("べきとう", "冪等")]
        );
    }

    #[test]
    fn a_titles_line_without_three_fields_is_an_error_naming_its_line() {
        let err = read_titles("wikipedia:1\tべきとう\n".as_bytes()).unwrap_err();

        assert!(err.to_string().starts_with("line 1:"), "{err}");
    }

    #[test]
    fn a_titles_line_with_an_empty_surface_is_an_error_naming_its_line() {
        let err = read_titles("wikipedia:1\tべきとう\t冪等\nwikipedia:2\twara\t\n".as_bytes())
            .unwrap_err();

        assert!(err.to_string().starts_with("line 2:"), "{err}");
    }

    #[test]
    fn a_field_dictionary_has_its_units_and_the_titles_its_texts_use() {
        let units = [
            word("こうぞう", "構造"),
            word("こうぞう", "構造"),
            word("べきとう", "冪等"),
        ];
        let titles = [
            title("べきとう", "冪等"),
            title("ちゅうどうかいかく", "中道改革"),
        ];
        let texts = [
            "冪等な構造。".to_owned(),
            "冪等。中道改革。中道改革。".to_owned(),
        ];

        let dictionary = field_dictionary(&units, &titles, &texts);

        let mut words: Vec<(&str, u32)> = dictionary
            .entries
            .iter()
            .map(|e| (e.surface.as_str(), e.cost))
            .collect();
        words.sort();
        assert_eq!(
            words,
            [
                ("中道改革", cost_of(2, 3)),
                ("冪等", cost_of(2, 3)),
                ("構造", cost_of(2, 3)),
            ]
        );
    }

    #[test]
    fn a_title_the_units_have_already_is_not_added_again() {
        let units = [word("べきとう", "冪等"), word("べきとう", "冪等")];
        let titles = [title("べきとう", "冪等")];
        let texts = ["冪等。冪等。冪等。".to_owned()];

        let dictionary = field_dictionary(&units, &titles, &texts);

        assert_eq!(dictionary.entries.len(), 1);
    }

    #[test]
    fn a_year_dictionary_has_the_new_titles_its_year_uses_far_more_than_older_text() {
        let base = Base::parse("うえの\t上野\t\t10\n").unwrap();
        let titles = [
            title("どぱがき", "ドパガキ"),
            title("うえの", "上野"),
            title("ういるす", "ウイルス"),
        ];
        let year = ["ドパガキと上野。ドパガキと上野。ウイルス。ウイルス。".to_owned()];
        let older = [format!("{}ウイルス。", "古".repeat(200))];

        let dictionary: Dictionary = year_dictionary(&titles, &base, &year, &older, 10);

        let surfaces: Vec<&str> = dictionary
            .entries
            .iter()
            .map(|e| e.surface.as_str())
            .collect();
        assert_eq!(surfaces, ["ドパガキ"]);
        assert_eq!(dictionary.entries[0].cost, cost_of(2, 10));
    }
}

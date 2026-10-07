//! The person name dictionary, taken from the readings in kana (P1814) of
//! Wikidata's people: full names, and the family and given names they split
//! into.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::dictionary::cost_of;
use crate::kana::{has_kanji, is_hiragana, is_kanji, is_katakana};
use crate::wikidata::{entity, literal, plain_reading, unqualified};
use crate::{Dictionary, Entry, katakana_to_hiragana};

/// Before this year, a family name of an old clan reads with の before the
/// given name (平清盛 たいらのきよもり).
const NO_BEFORE: i32 = 1600;
/// A family or given name becomes a line of its own once this many people
/// bear it, as a word of the base dictionary once it is seen this often.
const MIN_BEARERS: usize = 2;

/// One person of Wikidata as the people's export gives them, with the
/// readings and the facts of every row of the person gathered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    pub item: String,
    pub label: String,
    /// Each reading as its parts split by spaces, in plain hiragana.
    pub readings: BTreeSet<Vec<String>>,
    pub sitelinks: usize,
    pub ja_article: bool,
    /// The earliest year of birth, if any.
    pub born: Option<i32>,
}

/// The people of a QLever TSV export of `?item ?kana ?label ?links ?ja
/// ?birth`, in the order of their items. A row without a label with kanji,
/// with a label holding other characters than kana and kanji, or with a
/// reading holding other characters than kana, is left out.
pub fn wikidata_people(text: &str) -> Vec<Person> {
    let mut people: BTreeMap<String, Person> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(item) = fields.first().and_then(|f| entity(f)) else {
            continue;
        };
        let Some(reading) = fields
            .get(1)
            .and_then(|f| literal(f))
            .and_then(split_reading)
        else {
            continue;
        };
        let label = fields
            .get(2)
            .and_then(|f| literal(f))
            .map(unqualified)
            .unwrap_or_default();
        if !has_kanji(&label) || !label.chars().all(is_name_char) {
            continue;
        }
        let person = people.entry(item.clone()).or_insert_with(|| Person {
            item,
            label,
            ..Default::default()
        });
        person.readings.insert(reading);
        if let Some(n) = fields.get(3).and_then(|f| number(f)) {
            person.sitelinks = person.sitelinks.max(n as usize);
        }
        person.ja_article |= fields.get(4).is_some_and(|f| !f.is_empty());
        if let Some(year) = fields.get(5).and_then(|f| number(f)) {
            person.born = Some(person.born.map_or(year as i32, |y| y.min(year as i32)));
        }
    }
    people.into_values().collect()
}

/// The parts of a reading split by spaces, each in plain hiragana; none
/// when a part keeps other characters than kana.
fn split_reading(reading: &str) -> Option<Vec<String>> {
    reading
        .split([' ', '\u{3000}'])
        .filter(|part| !part.is_empty())
        .map(plain_reading)
        .collect::<Option<Vec<_>>>()
        .filter(|parts| !parts.is_empty())
}

fn is_name_char(c: char) -> bool {
    is_kanji(c) || is_hiragana(c) || is_katakana(c) || matches!(c, '々' | 'ー')
}

/// The leading signed integer of a field, quoted or not (`"9"`, `1955-12-16T…`,
/// `-0100-01-01T…`).
fn number(field: &str) -> Option<i64> {
    let text = field.trim_start_matches('"');
    let sign = if text.starts_with('-') { -1 } else { 1 };
    let digits: String = text
        .trim_start_matches(['-', '+'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<i64>().ok().map(|n| sign * n)
}

/// The readings each kanji can take in a name, from Mozc's single kanji
/// table as [`crate::mozc_readings`] gives it.
pub fn name_readings(mozc: &[(String, char)]) -> HashMap<char, Vec<String>> {
    let mut readings: HashMap<char, Vec<String>> = HashMap::new();
    for (reading, kanji) in mozc {
        readings.entry(*kanji).or_default().push(reading.clone());
    }
    readings
}

/// Whether `chars` can be read as `reading`, each kanji in one of its
/// readings or their voiced or doubled forms, 々 as the kanji before it, and
/// kana as themselves.
fn reads_as(chars: &[char], reading: &str, readings: &HashMap<char, Vec<String>>) -> bool {
    fn go(chars: &[char], at: usize, reading: &str, readings: &HashMap<char, Vec<String>>) -> bool {
        let Some(&c) = chars.get(at) else {
            return reading.is_empty();
        };
        let ways: Vec<String> = match c {
            '々' if at > 0 => kanji_ways(chars[at - 1], readings),
            'ヶ' | 'ヵ' | 'ケ' | 'カ' => ["か", "が", "け", "こ"].map(String::from).to_vec(),
            c if is_kanji(c) => kanji_ways(c, readings),
            c => vec![katakana_to_hiragana(c.to_string())],
        };
        ways.iter().any(|way| {
            reading
                .strip_prefix(way.as_str())
                .is_some_and(|rest| go(chars, at + 1, rest, readings))
        })
    }
    go(chars, 0, reading, readings)
}

/// The readings of kanji `c`, with the first kana voiced or half-voiced and
/// a last つ, ち, く or き doubled, as a kanji reads inside a name.
fn kanji_ways(c: char, readings: &HashMap<char, Vec<String>>) -> Vec<String> {
    let mut ways = Vec::new();
    for reading in readings.get(&c).into_iter().flatten() {
        let mut forms = vec![reading.clone()];
        let mut chars = reading.chars();
        if let Some(first) = chars.next() {
            let rest: String = chars.collect();
            for changed in [voiced(first), half_voiced(first)].into_iter().flatten() {
                forms.push(format!("{changed}{rest}"));
            }
        }
        for form in forms.clone() {
            if let Some(stem) = form.strip_suffix(['つ', 'ち', 'く', 'き']) {
                forms.push(format!("{stem}っ"));
            }
        }
        ways.extend(forms);
    }
    ways
}

fn voiced(c: char) -> Option<char> {
    let plain = "かきくけこさしすせそたちつてとはひふへほ";
    let voiced = "がぎぐげござじずぜぞだぢづでどばびぶべぼ";
    plain
        .chars()
        .position(|p| p == c)
        .and_then(|i| voiced.chars().nth(i))
}

fn half_voiced(c: char) -> Option<char> {
    let plain = "はひふへほ";
    let half = "ぱぴぷぺぽ";
    plain
        .chars()
        .position(|p| p == c)
        .and_then(|i| half.chars().nth(i))
}

/// A person's name as the dictionary takes it: the label split into family
/// and given names where a reading splits, or else whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    Split {
        family: (String, String),
        given: (String, String),
        /// Whether the reading put の between them (みなもと の よりとも).
        with_no: bool,
    },
    Whole(String, String),
}

/// How `reading` names `label`: split after a head of the label that reads as
/// the family name, the first whose rest reads as the given name or else the
/// longest; or whole when the reading has no space and the
/// whole label reads as it; none when the family name reads as no head of
/// the label, as when the label is a stage name and the reading the real one.
pub fn name_of(
    label: &str,
    reading: &[String],
    readings: &HashMap<char, Vec<String>>,
) -> Option<Name> {
    let chars: Vec<char> = label.chars().collect();
    let (family, given, with_no) = match reading {
        [whole] => {
            return reads_as(&chars, whole, readings)
                .then(|| Name::Whole(whole.clone(), label.to_owned()));
        }
        [family, given] => (family, given, false),
        [family, no, given] if no == "の" => (family, given, true),
        _ => return None,
    };
    let cuts: Vec<usize> = (1..chars.len())
        .filter(|&n| reads_as(&chars[..n], family, readings))
        .collect();
    // Mozc reads some kanji as a whole family name (牧 まきの), so the
    // shortest head can swallow the family name; the longest one cannot.
    let cut = cuts
        .iter()
        .copied()
        .find(|&n| reads_as(&chars[n..], given, readings))
        .or(cuts.last().copied())?;
    Some(Name::Split {
        family: (family.clone(), chars[..cut].iter().collect()),
        given: (given.clone(), chars[cut..].iter().collect()),
        with_no,
    })
}

/// The old clans whose names read with の before a given name, one a line
/// in `text`, skipping comments and blank lines.
pub fn clans(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The person name dictionary of `people`: each full name costing by its
/// person's site links, counted twice with an article in Japanese, and each
/// family and given name with kanji that two or more people bear, costing by
/// the people who bear it. A full name of
/// an old clan born before 1600 is also read with の.
pub fn person_dictionary(
    people: &[Person],
    readings: &HashMap<char, Vec<String>>,
    clans: &BTreeSet<String>,
) -> Dictionary {
    let mut full: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut parts: BTreeMap<(String, String), BTreeSet<&str>> = BTreeMap::new();
    let mut named = BTreeSet::new();
    for person in people {
        let weight = (person.sitelinks.max(1)) * if person.ja_article { 2 } else { 1 };
        for reading in &person.readings {
            let Some(name) = name_of(&person.label, reading, readings) else {
                continue;
            };
            named.insert(person.item.as_str());
            let mut full_readings = Vec::new();
            match name {
                Name::Whole(reading, label) => full_readings.push((reading, label)),
                Name::Split {
                    family,
                    given,
                    with_no,
                } => {
                    let old = clans.contains(&family.1)
                        && person.born.is_some_and(|year| year < NO_BEFORE);
                    full_readings.push((format!("{}{}", family.0, given.0), person.label.clone()));
                    if with_no || old {
                        full_readings
                            .push((format!("{}の{}", family.0, given.0), person.label.clone()));
                    }
                    for part in [family, given] {
                        parts.entry(part).or_default().insert(&person.item);
                    }
                }
            }
            for key in full_readings {
                let w = full.entry(key).or_default();
                *w = (*w).max(weight);
            }
        }
    }
    let full_total: usize = full.values().sum();
    let mut entries: Vec<Entry> = full
        .iter()
        .map(|((reading, surface), &w)| entry(reading, surface, cost_of(w, full_total)))
        .collect();
    entries.extend(
        parts
            .iter()
            .filter(|((reading, surface), bearers)| {
                !reading.is_empty() && has_kanji(surface) && bearers.len() >= MIN_BEARERS
            })
            .map(|((reading, surface), bearers)| {
                entry(reading, surface, cost_of(bearers.len(), named.len()))
            }),
    );
    Dictionary {
        entries,
        ..Default::default()
    }
}

fn entry(reading: &str, surface: &str, cost: u32) -> Entry {
    Entry {
        reading: reading.to_owned(),
        surface: surface.to_owned(),
        conjugation: None,
        cost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings() -> HashMap<char, Vec<String>> {
        let mozc = [
            ("とく", '徳'),
            ("かわ", '川'),
            ("いえ", '家'),
            ("やす", '康'),
            ("たいら", '平'),
            ("きよ", '清'),
            ("もり", '盛'),
            ("おお", '大'),
            ("いし", '石'),
            ("さ", '沙'),
            ("や", '也'),
            ("か", '加'),
            ("さ", '佐'),
            ("さ", '々'),
            ("き", '木'),
            ("はな", '花'),
            ("まえ", '前'),
            ("まき", '牧'),
            ("まきの", '牧'),
            ("の", '野'),
            ("ま", '真'),
            ("と", '人'),
        ]
        .map(|(r, k)| (r.to_owned(), k));
        name_readings(&mozc)
    }

    fn parts(reading: &str) -> Vec<String> {
        reading.split(' ').map(String::from).collect()
    }

    #[test]
    fn rows_gather_into_people_with_plain_readings() {
        let text = "?item\t?kana\t?label\t?links\t?ja\t?birth\n\
            <http://www.wikidata.org/entity/Q1>\t\"とくがわ いえやす\"\t\"徳川家康\"@ja\t\"95\"\t<https://ja.wikipedia.org/wiki/x>\t\"1543-01-31T00:00:00Z\"\n\
            <http://www.wikidata.org/entity/Q1>\t\"トクガワ・イエヤス\"\t\"徳川家康\"@ja\t\"95\"\t<https://ja.wikipedia.org/wiki/x>\t\"1542-12-26T00:00:00Z\"\n\
            <http://www.wikidata.org/entity/Q2>\t\"とむ くるーず\"\t\"トム・クルーズ\"@ja\t\"120\"\t\t\n\
            <http://www.wikidata.org/entity/Q3>\t\"なかむら 4だいめ\"\t\"中村\"@ja\t\t\t\n\
            <http://www.wikidata.org/entity/Q4>\t\"ほげ\"\t\"豊英秋 (雅楽師)\"@ja\t\t\t\"-0100-01-01T00:00:00Z\"\n";

        let people = wikidata_people(text);

        assert_eq!(
            people,
            [
                Person {
                    item: "Q1".into(),
                    label: "徳川家康".into(),
                    readings: BTreeSet::from([
                        parts("とくがわいえやす"),
                        parts("とくがわ いえやす")
                    ]),
                    sitelinks: 95,
                    ja_article: true,
                    born: Some(1542),
                },
                Person {
                    item: "Q4".into(),
                    label: "豊英秋".into(),
                    readings: BTreeSet::from([parts("ほげ")]),
                    sitelinks: 0,
                    ja_article: false,
                    born: Some(-100),
                },
            ]
        );
    }

    #[test]
    fn a_name_splits_at_the_shortest_head_that_reads_as_the_family_name() {
        assert_eq!(
            name_of("徳川家康", &parts("とくがわ いえやす"), &readings()),
            Some(Name::Split {
                family: ("とくがわ".into(), "徳川".into()),
                given: ("いえやす".into(), "家康".into()),
                with_no: false,
            })
        );
        assert_eq!(
            name_of("佐々木加", &parts("ささき か"), &readings()),
            Some(Name::Split {
                family: ("ささき".into(), "佐々木".into()),
                given: ("か".into(), "加".into()),
                with_no: false,
            })
        );
        assert_eq!(
            name_of("花ヶ前加", &parts("はながさき か"), &readings()),
            None,
            "まえ is no さき"
        );
    }

    #[test]
    fn a_cut_whose_given_name_reads_too_wins_and_else_the_longest_family_name() {
        let split = |label: &str, reading: &str| match name_of(label, &parts(reading), &readings())
        {
            Some(Name::Split { family, given, .. }) => Some((family.1, given.1)),
            _ => None,
        };

        assert_eq!(
            split("牧野真人", "まきの まさと"),
            Some(("牧野".into(), "真人".into()))
        );
        assert_eq!(
            split("牧野覚", "まきの さとる"),
            Some(("牧野".into(), "覚".into()))
        );
    }

    #[test]
    fn a_stage_name_read_as_the_real_name_is_not_a_name() {
        assert_eq!(
            name_of("大石沙也加", &parts("ふじさわ さやか"), &readings()),
            None
        );
    }

    #[test]
    fn a_reading_without_a_space_names_the_label_only_when_it_reads_whole() {
        assert_eq!(
            name_of("家康", &parts("いえやす"), &readings()),
            Some(Name::Whole("いえやす".into(), "家康".into()))
        );
        assert_eq!(name_of("家康", &parts("ほげ"), &readings()), None);
    }

    #[test]
    fn a_reading_with_no_between_the_names_splits_around_it() {
        assert_eq!(
            name_of("平清盛", &parts("たいら の きよもり"), &readings()),
            Some(Name::Split {
                family: ("たいら".into(), "平".into()),
                given: ("きよもり".into(), "清盛".into()),
                with_no: true,
            })
        );
    }

    #[test]
    fn the_dictionary_has_full_names_and_their_parts_and_no_for_old_clans() {
        let person = |item: &str, label: &str, reading: &str, links, born| Person {
            item: item.into(),
            label: label.into(),
            readings: BTreeSet::from([parts(reading)]),
            sitelinks: links,
            ja_article: false,
            born,
        };
        let people = [
            person("Q1", "徳川家康", "とくがわ いえやす", 90, Some(1543)),
            person("Q2", "徳川家", "とくがわ いえ", 10, Some(1900)),
            person("Q3", "平清盛", "たいら きよもり", 50, Some(1118)),
            person("Q4", "大石沙也加", "ふじさわ さやか", 5, None),
            person("Q5", "平家康", "たいら いえやす", 1, Some(1900)),
            person("Q6", "徳川さやか", "とくがわ さやか", 1, None),
        ];
        let clans = clans("# old clans\n平\n源\n");

        let dict = person_dictionary(&people, &readings(), &clans);

        let lines: BTreeSet<(&str, &str)> = dict
            .entries
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str()))
            .collect();
        assert_eq!(
            lines,
            BTreeSet::from([
                ("とくがわいえやす", "徳川家康"),
                ("とくがわいえ", "徳川家"),
                ("たいらきよもり", "平清盛"),
                ("たいらのきよもり", "平清盛"),
                ("たいらいえやす", "平家康"),
                ("とくがわさやか", "徳川さやか"),
                ("とくがわ", "徳川"),
                ("いえやす", "家康"),
                ("たいら", "平"),
            ]),
            "a name one person bears, or without kanji, is no line of its own"
        );
        let cost = |reading: &str| {
            dict.entries
                .iter()
                .find(|e| e.reading == reading)
                .unwrap()
                .cost
        };
        assert!(cost("とくがわいえやす") < cost("とくがわいえ"));
        assert!(
            cost("とくがわ") < cost("いえやす"),
            "three bear 徳川, two 家康"
        );
    }
}

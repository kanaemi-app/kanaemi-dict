//! Checking the base dictionary's readings against the readings in kana
//! (P1814) of Wikidata's items, which are never taken into a dictionary.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::kana::{has_kanji, is_hiragana};
use crate::{DictionaryLine, katakana_to_hiragana};

const ENTITY: &str = "http://www.wikidata.org/entity/";

/// One row of the readings QLever exports: an item, one of its readings in
/// kana, its Japanese label and one thing it is (P31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikidataReading {
    pub item: String,
    pub reading: String,
    pub label: String,
    pub p31: Option<String>,
}

/// The rows of a QLever TSV export of items' readings, labels and P31, with
/// the reading in plain hiragana and the label without a qualifier in
/// brackets; a row without a label, or whose reading keeps other characters
/// than kana (なかむら 4だいめ), is left out.
pub fn wikidata_readings(text: &str) -> Vec<WikidataReading> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let item = entity(fields.next()?)?;
            let reading = plain_reading(literal(fields.next()?)?)?;
            let label = unqualified(literal(fields.next()?)?);
            if label.is_empty() {
                return None;
            }
            let p31 = fields.next().and_then(entity);
            Some(WikidataReading {
                item,
                reading,
                label,
                p31,
            })
        })
        .collect()
}

/// `Q55488` of `<http://www.wikidata.org/entity/Q55488>`.
fn entity(field: &str) -> Option<String> {
    let inner = field.strip_prefix('<')?.strip_suffix('>')?;
    inner.strip_prefix(ENTITY).map(str::to_owned)
}

/// The text of a quoted literal, without its language tag.
fn literal(field: &str) -> Option<&str> {
    let quoted = field.rsplit_once('@').map_or(field, |(text, _)| text);
    quoted.strip_prefix('"')?.strip_suffix('"')
}

/// `reading` in hiragana without spaces and middle dots, ゐ and ゑ as い
/// and え; none when other characters than kana stay.
fn plain_reading(reading: &str) -> Option<String> {
    let plain: String = katakana_to_hiragana(reading)
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '\u{200b}' | '・'))
        .map(|c| match c {
            'ゐ' => 'い',
            'ゑ' => 'え',
            c => c,
        })
        .collect();
    (!plain.is_empty() && plain.chars().all(|c| is_hiragana(c) || c == 'ー')).then_some(plain)
}

/// `label` without a qualifier in brackets at its end (内藤忠政 (鳥羽藩主)).
fn unqualified(label: &str) -> String {
    let cut = if label.ends_with(')') {
        label.rfind(" (")
    } else if label.ends_with('）') {
        label.rfind('（')
    } else {
        None
    };
    cut.map_or(label, |at| &label[..at]).trim().to_owned()
}

/// A word of the base dictionary that reads none of the ways the Wikidata
/// items of its surface read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub surface: String,
    /// The base dictionary's readings and their costs.
    pub base: Vec<(String, u32)>,
    pub wikidata: Vec<String>,
    pub items: Vec<String>,
    pub p31: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WikidataSummary {
    /// Labels with kanji compared.
    pub labels: usize,
    /// Those whose surface the base dictionary has as a whole word.
    pub in_base: usize,
    pub mismatched: usize,
}

/// The words of the base dictionary's `lines` whose surface is a label of
/// `readings` with kanji and whose readings are none of the label's.
pub fn wikidata_mismatches<'a>(
    readings: &[WikidataReading],
    lines: impl IntoIterator<Item = DictionaryLine<'a>>,
) -> (WikidataSummary, Vec<Mismatch>) {
    #[derive(Default)]
    struct Label {
        readings: BTreeSet<String>,
        items: BTreeSet<String>,
        p31: BTreeSet<String>,
    }
    let mut labels: BTreeMap<&str, Label> = BTreeMap::new();
    for r in readings.iter().filter(|r| has_kanji(&r.label)) {
        let label = labels.entry(r.label.as_str()).or_default();
        // Many stations read only their name (天王寺駅 てんのうじ).
        let reading = if r.label.ends_with('駅') && !r.reading.ends_with("えき") {
            format!("{}えき", r.reading)
        } else {
            r.reading.clone()
        };
        label.readings.insert(reading);
        label.items.insert(r.item.clone());
        label.p31.extend(r.p31.clone());
    }
    let mut base: HashMap<&str, Vec<(String, u32)>> = HashMap::new();
    for line in lines {
        if line.conjugation.is_empty()
            && !line.surface.contains('{')
            && labels.contains_key(line.surface)
        {
            base.entry(line.surface)
                .or_default()
                .push((line.reading.to_owned(), line.cost.unwrap_or(u32::MAX)));
        }
    }
    let mut mismatches: Vec<Mismatch> = labels
        .iter()
        .filter_map(|(surface, label)| {
            let words = base.get(surface)?;
            if words.iter().any(|(r, _)| label.readings.contains(r)) {
                return None;
            }
            let mut words = words.clone();
            words.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
            Some(Mismatch {
                surface: (*surface).to_owned(),
                base: words,
                wikidata: label.readings.iter().cloned().collect(),
                items: label.items.iter().cloned().collect(),
                p31: label.p31.iter().cloned().collect(),
            })
        })
        .collect();
    mismatches.sort_by_key(|m| {
        (
            !m.surface.ends_with('駅'),
            m.base.first().map_or(u32::MAX, |(_, cost)| *cost),
            m.surface.clone(),
        )
    });
    let summary = WikidataSummary {
        labels: labels.len(),
        in_base: base.len(),
        mismatched: mismatches.len(),
    };
    (summary, mismatches)
}

/// `mismatches` as the TSV of build/check-wikidata.tsv: the base readings as
/// `reading:cost`, and each list joined with `|`.
pub fn mismatches_tsv(mismatches: &[Mismatch]) -> String {
    let mut tsv = String::from("surface\tbase\twikidata\titems\tp31\n");
    for m in mismatches {
        let base: Vec<String> = m.base.iter().map(|(r, c)| format!("{r}:{c}")).collect();
        tsv.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            m.surface,
            base.join("|"),
            m.wikidata.join("|"),
            m.items.join("|"),
            m.p31.join("|"),
        ));
    }
    tsv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(item: &str, reading: &str, label: &str) -> WikidataReading {
        WikidataReading {
            item: item.into(),
            reading: reading.into(),
            label: label.into(),
            p31: None,
        }
    }

    fn line<'a>(reading: &'a str, surface: &'a str, cost: u32) -> DictionaryLine<'a> {
        DictionaryLine {
            reading,
            surface,
            conjugation: "",
            cost: Some(cost),
        }
    }

    #[test]
    fn a_row_gives_its_item_reading_label_and_p31_in_plain_form() {
        let text = "?item\t?kana\t?label\t?p31\n\
            <http://www.wikidata.org/entity/Q6849859>\t\"ミキ・\u{200b}えき\"\t\"三木駅\"@ja\t<http://www.wikidata.org/entity/Q55488>\n\
            <http://www.wikidata.org/entity/Q1>\t\"めいぢこうゑん\"\t\"明治公園 (東京都)\"@ja\t\n";

        assert_eq!(
            wikidata_readings(text),
            [
                WikidataReading {
                    item: "Q6849859".into(),
                    reading: "みきえき".into(),
                    label: "三木駅".into(),
                    p31: Some("Q55488".into()),
                },
                WikidataReading {
                    item: "Q1".into(),
                    reading: "めいぢこうえん".into(),
                    label: "明治公園".into(),
                    p31: None,
                },
            ]
        );
    }

    #[test]
    fn a_row_without_a_label_or_with_a_reading_not_in_kana_is_left_out() {
        let text = "<http://www.wikidata.org/entity/Q2>\t\"なかむら 4だいめ\"\t\"中村\"@ja\t\n\
            <http://www.wikidata.org/entity/Q3>\t\"かがみ\"\t\t\n";

        assert_eq!(wikidata_readings(text), []);
    }

    #[test]
    fn a_word_reading_none_of_its_items_ways_is_a_mismatch() {
        let readings = [
            reading("Q6849859", "みきえき", "三木駅"),
            reading("Q10", "まち", "町"),
            reading("Q11", "ちょう", "町"),
            reading("Q12", "あいうえ", "未知語"),
        ];
        let lines = [line("きみえき", "三木駅", 1500), line("まち", "町", 900)];

        let (summary, mismatches) = wikidata_mismatches(&readings, lines);

        assert_eq!(
            summary,
            WikidataSummary {
                labels: 3,
                in_base: 2,
                mismatched: 1,
            }
        );
        assert_eq!(
            mismatches,
            [Mismatch {
                surface: "三木駅".into(),
                base: vec![("きみえき".into(), 1500)],
                wikidata: vec!["みきえき".into()],
                items: vec!["Q6849859".into()],
                p31: vec![],
            }]
        );
    }

    #[test]
    fn a_station_reading_without_eki_reads_the_name_with_eki() {
        let readings = [reading("Q1", "てんのうじ", "天王寺駅")];
        let lines = [line("てんのうじえき", "天王寺駅", 1391)];

        let (_, mismatches) = wikidata_mismatches(&readings, lines);

        assert_eq!(mismatches, []);
    }

    #[test]
    fn labels_without_kanji_and_conjugating_or_numeric_lines_are_not_compared() {
        let readings = [
            reading("Q1", "てすと", "テスト"),
            reading("Q2", "かく", "書"),
        ];
        let lines = [
            line("てすとー", "テスト", 1000),
            DictionaryLine {
                conjugation: "五段-カ行",
                ..line("か", "書", 900)
            },
        ];

        let (summary, mismatches) = wikidata_mismatches(&readings, lines);

        assert_eq!(summary.in_base, 0);
        assert_eq!(mismatches, []);
    }

    #[test]
    fn station_names_come_first_then_the_cheapest_word() {
        let readings = [
            reading("Q1", "みきえき", "三木駅"),
            reading("Q2", "ささやまじょう", "篠山城"),
            reading("Q3", "かあいがもん", "河相我聞"),
        ];
        let lines = [
            line("しのやまじょう", "篠山城", 1300),
            line("かわいがもん", "河相我聞", 1200),
            line("きみえき", "三木駅", 1500),
        ];

        let (_, mismatches) = wikidata_mismatches(&readings, lines);

        let order: Vec<&str> = mismatches.iter().map(|m| m.surface.as_str()).collect();
        assert_eq!(order, ["三木駅", "河相我聞", "篠山城"]);
    }

    #[test]
    fn the_tsv_has_a_header_and_a_line_a_word() {
        let mismatches = [Mismatch {
            surface: "三木駅".into(),
            base: vec![("きみえき".into(), 1500)],
            wikidata: vec!["みきえき".into()],
            items: vec!["Q6849859".into()],
            p31: vec!["Q55488".into()],
        }];

        assert_eq!(
            mismatches_tsv(&mismatches),
            "surface\tbase\twikidata\titems\tp31\n三木駅\tきみえき:1500\tみきえき\tQ6849859\tQ55488\n"
        );
    }
}

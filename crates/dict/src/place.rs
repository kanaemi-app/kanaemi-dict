//! The place name dictionary: place names from Japan Post's postal code
//! data, with their readings.

use std::collections::BTreeMap;

use crate::dictionary::cost_of;
use crate::kana::katakana_to_hiragana;
use crate::{Dictionary, Entry};

/// Every place name of the postal code data's rows as (reading, surface)
/// with the rows it appears in: prefectures, municipalities and towns, and
/// a district or a city split off a municipality where the reading splits
/// one way only. Towns with notes, numbers, catch-all wording or a list of
/// towns in one field are left out.
pub fn place_names(csv: &str) -> BTreeMap<(String, String), usize> {
    let mut names = BTreeMap::new();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(csv.as_bytes());
    for record in reader.records().filter_map(Result::ok) {
        let field = |i: usize| record.get(i).unwrap_or_default();
        let mut row: Vec<(String, String)> = vec![
            (katakana_to_hiragana(field(3)), field(6).to_owned()),
            (katakana_to_hiragana(field(4)), field(7).to_owned()),
        ];
        row.extend(split_municipality(&row[1].0, &row[1].1));
        if is_town(field(8), field(5)) {
            row.push((katakana_to_hiragana(field(5)), field(8).to_owned()));
        }
        row.sort();
        row.dedup();
        for name in row {
            if !name.0.is_empty() && !name.1.is_empty() {
                *names.entry(name).or_default() += 1;
            }
        }
    }
    names
}

/// The district or city and the rest of a municipality name (上川郡 and
/// 東神楽町, 札幌市 and 中央区), when the surface and the reading split there
/// one way only.
fn split_municipality(reading: &str, surface: &str) -> Vec<(String, String)> {
    let mut splits = Vec::new();
    for (mark, kana) in [('郡', "ぐん"), ('市', "し")] {
        for (at, _) in surface.match_indices(mark) {
            let (head, rest) = surface.split_at(at + mark.len_utf8());
            if at == 0 || rest.is_empty() {
                continue;
            }
            for (r, _) in reading.match_indices(kana) {
                let (rhead, rrest) = reading.split_at(r + kana.len());
                if r > 0 && !rrest.is_empty() {
                    splits.push([
                        (rhead.to_owned(), head.to_owned()),
                        (rrest.to_owned(), rest.to_owned()),
                    ]);
                }
            }
        }
    }
    match splits.as_slice() {
        [only] => only.to_vec(),
        _ => Vec::new(),
    }
}

/// Whether a town field and its reading name one town, not a note, a
/// catch-all, a list of towns, or a numbered block whose reading spells the
/// number in digits.
fn is_town(town: &str, reading: &str) -> bool {
    let has_digit = |s: &str| {
        s.chars()
            .any(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
    };
    !town.is_empty()
        && !town.contains(['（', '(', '、'])
        && !has_digit(town)
        && !has_digit(reading)
        && !town.contains("以下に掲載がない場合")
        && !town.contains("の次に番地がくる場合")
        && !town.ends_with("一円")
}

/// The place names as a dictionary of plain words, costing by the share of
/// the `rows` each appears in.
pub fn place_dictionary(names: &BTreeMap<(String, String), usize>, rows: usize) -> Dictionary {
    Dictionary {
        entries: names
            .iter()
            .map(|((reading, surface), &n)| Entry {
                reading: reading.clone(),
                surface: surface.clone(),
                conjugation: None,
                cost: cost_of(n, rows),
            })
            .collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_seen_in_more_rows_cost_less() {
        let names = BTreeMap::from([
            (("ほっかいどう".to_owned(), "北海道".to_owned()), 8),
            (("ひじりの".to_owned(), "ひじり野".to_owned()), 1),
        ]);

        let dict = place_dictionary(&names, 10);

        let costs: Vec<_> = dict
            .entries
            .iter()
            .map(|e| (e.surface.as_str(), e.cost))
            .collect();
        assert_eq!(costs, [("ひじり野", 230), ("北海道", 22)]);
    }

    fn row(pref: (&str, &str), city: (&str, &str), town: (&str, &str)) -> String {
        format!(
            "01101,\"060  \",\"0600000\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",0,0,0,0,0,0\n",
            pref.1, city.1, town.1, pref.0, city.0, town.0
        )
    }

    fn names(csv: &str) -> Vec<(String, String, usize)> {
        place_names(csv)
            .into_iter()
            .map(|((r, s), n)| (r, s, n))
            .collect()
    }

    #[test]
    fn prefectures_municipalities_and_towns_come_out_in_hiragana_with_their_rows() {
        let hokkaido = ("北海道", "ホッカイドウ");
        let csv = [
            row(
                hokkaido,
                ("上川郡東神楽町", "カミカワグンヒガシカグラチョウ"),
                ("ひじり野", "ヒジリノ"),
            ),
            row(
                hokkaido,
                ("上川郡東神楽町", "カミカワグンヒガシカグラチョウ"),
                ("以下に掲載がない場合", "イカニケイサイガナイバアイ"),
            ),
        ]
        .concat();

        assert_eq!(
            names(&csv),
            [
                ("かみかわぐん".into(), "上川郡".into(), 2),
                (
                    "かみかわぐんひがしかぐらちょう".into(),
                    "上川郡東神楽町".into(),
                    2
                ),
                ("ひがしかぐらちょう".into(), "東神楽町".into(), 2),
                ("ひじりの".into(), "ひじり野".into(), 1),
                ("ほっかいどう".into(), "北海道".into(), 2),
            ]
        );
    }

    #[test]
    fn a_ward_splits_off_its_city_only_where_the_reading_splits_one_way() {
        let csv = [
            row(
                ("北海道", "ホッカイドウ"),
                ("札幌市中央区", "サッポロシチュウオウク"),
                ("大通東", "オオドオリヒガシ"),
            ),
            row(
                ("東京都", "トウキョウト"),
                ("西東京市", "ニシトウキョウシ"),
                ("泉町", "イズミチョウ"),
            ),
        ]
        .concat();

        let names: Vec<String> = names(&csv).into_iter().map(|(_, s, _)| s).collect();

        assert_eq!(
            names,
            [
                "泉町",
                "大通東",
                "札幌市",
                "札幌市中央区",
                "中央区",
                "東京都",
                "西東京市",
                "北海道"
            ]
        );
    }

    #[test]
    fn towns_with_notes_numbers_or_catch_all_wording_are_left_out() {
        let tokyo = ("東京都", "トウキョウト");
        let city = ("千代田区", "チヨダク");
        let csv = [
            row(
                tokyo,
                city,
                (
                    "大手町（次のビルを除く）",
                    "オオテマチ（ツギノビルヲノゾク）",
                ),
            ),
            row(tokyo, city, ("第１地割", "ダイ１チワリ")),
            row(tokyo, city, ("千代田区一円", "チヨダクイチエン")),
            row(tokyo, city, ("北一条東", "キタ１ジョウヒガシ")),
            row(
                tokyo,
                city,
                ("井道、奥井道、内井道", "イミチ、オクイミチ、ウチイミチ"),
            ),
            row(tokyo, city, ("番町", "バンチョウ")),
        ]
        .concat();

        let towns: Vec<String> = names(&csv).into_iter().map(|(_, s, _)| s).collect();

        assert_eq!(towns, ["千代田区", "東京都", "番町"]);
    }
}

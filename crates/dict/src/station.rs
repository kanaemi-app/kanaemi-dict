//! The station name dictionary, taken from the readings in kana (P1814) of
//! Wikidata's railway stations in Japan: each station's name with 駅 and
//! without it.

use std::collections::{BTreeMap, BTreeSet};

use crate::dictionary::cost_of;
use crate::kana::has_kanji;
use crate::person::{is_name_char, number};
use crate::wikidata::{entity, literal, plain_reading, station_reading, unqualified};
use crate::{Dictionary, Entry};

/// One station of Wikidata as the stations' export gives it, with the
/// readings of every row of the station gathered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Station {
    pub item: String,
    pub label: String,
    /// Each reading in plain hiragana, with えき when the label ends in 駅.
    pub readings: BTreeSet<String>,
    pub sitelinks: usize,
}

/// The stations of a QLever TSV export of `?item ?kana ?label ?links`, in
/// the order of their items. A row without a label with kanji, with a label
/// holding other characters than kana and kanji, or with a reading holding
/// other characters than kana, is left out.
pub fn wikidata_stations(text: &str) -> Vec<Station> {
    let mut stations: BTreeMap<String, Station> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(item) = fields.first().and_then(|f| entity(f)) else {
            continue;
        };
        let Some(reading) = fields
            .get(1)
            .and_then(|f| literal(f))
            .and_then(plain_reading)
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
        let station = stations.entry(item.clone()).or_insert_with(|| Station {
            item,
            label,
            ..Default::default()
        });
        station
            .readings
            .insert(station_reading(&station.label, &reading));
        if let Some(n) = fields.get(3).and_then(|f| number(f)) {
            station.sitelinks = station.sitelinks.max(n as usize);
        }
    }
    stations.into_values().collect()
}

/// The station name dictionary of `stations`: each label with its reading,
/// and a label ending in 駅 also without 駅 and えき. A line costs by the
/// site links of the stations giving it, summed.
pub fn station_dictionary(stations: &[Station]) -> Dictionary {
    let mut weights: BTreeMap<(String, String), usize> = BTreeMap::new();
    for station in stations {
        let mut lines = BTreeSet::new();
        for reading in &station.readings {
            lines.insert((reading.clone(), station.label.clone()));
            let name = station
                .label
                .strip_suffix('駅')
                .zip(reading.strip_suffix("えき"))
                .filter(|(surface, reading)| !reading.is_empty() && has_kanji(surface));
            if let Some((surface, reading)) = name {
                lines.insert((reading.to_owned(), surface.to_owned()));
            }
        }
        for line in lines {
            *weights.entry(line).or_default() += station.sitelinks.max(1);
        }
    }
    let total: usize = weights.values().sum();
    let entries = weights
        .into_iter()
        .map(|((reading, surface), w)| Entry {
            reading,
            surface,
            conjugation: None,
            cost: cost_of(w, total),
        })
        .collect();
    Dictionary {
        entries,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(item: &str, label: &str, readings: &[&str], sitelinks: usize) -> Station {
        Station {
            item: item.into(),
            label: label.into(),
            readings: readings.iter().map(|&r| r.to_owned()).collect(),
            sitelinks,
        }
    }

    fn lines(dict: &Dictionary) -> BTreeSet<(&str, &str)> {
        dict.entries
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str()))
            .collect()
    }

    fn cost(dict: &Dictionary, reading: &str, surface: &str) -> u32 {
        dict.entries
            .iter()
            .find(|e| e.reading == reading && e.surface == surface)
            .unwrap()
            .cost
    }

    #[test]
    fn rows_gather_into_stations_with_unqualified_labels_and_plain_readings() {
        let text = "?item\t?kana\t?label\t?links\n\
            <http://www.wikidata.org/entity/Q2256123>\t\"でまちやなぎえき\"\t\"出町柳駅\"@ja\t\"12\"\n\
            <http://www.wikidata.org/entity/Q2256123>\t\"デマチヤナギ\"\t\"出町柳駅\"@ja\t\"12\"\n\
            <http://www.wikidata.org/entity/Q1>\t\"さんじょう\"\t\"三条駅 (京都府)\"@ja\t\n";

        assert_eq!(
            wikidata_stations(text),
            [
                station("Q1", "三条駅", &["さんじょうえき"], 0),
                station("Q2256123", "出町柳駅", &["でまちやなぎえき"], 12),
            ]
        );
    }

    #[test]
    fn a_label_without_kanji_or_with_other_characters_or_a_reading_not_in_kana_is_left_out() {
        let text = "<http://www.wikidata.org/entity/Q1>\t\"ゆーかりがおか\"\t\"ユーカリが丘駅\"@ja\t\n\
            <http://www.wikidata.org/entity/Q2>\t\"いすみ\"\t\"いすみ\"@ja\t\n\
            <http://www.wikidata.org/entity/Q3>\t\"ないえ\"\t\"JR奈井江駅\"@ja\t\n\
            <http://www.wikidata.org/entity/Q4>\t\"とうきょう2\"\t\"東京駅\"@ja\t\n\
            <http://www.wikidata.org/entity/Q5>\t\"いちがや\"\t\"市ヶ谷駅\"@ja\t\n";

        let items: Vec<String> = wikidata_stations(text)
            .into_iter()
            .map(|s| s.item)
            .collect();

        assert_eq!(items, ["Q1", "Q5"]);
    }

    #[test]
    fn a_station_reads_with_eki_and_its_name_without_it() {
        let dict = station_dictionary(&[
            station("Q1", "出町柳駅", &["でまちやなぎえき"], 12),
            station("Q2", "中書島", &["ちゅうしょじま"], 3),
            station("Q3", "駅", &["えき"], 1),
            station("Q4", "いすみ駅", &["いすみえき"], 1),
        ]);

        assert_eq!(
            lines(&dict),
            BTreeSet::from([
                ("でまちやなぎえき", "出町柳駅"),
                ("でまちやなぎ", "出町柳"),
                ("ちゅうしょじま", "中書島"),
                ("えき", "駅"),
                ("いすみえき", "いすみ駅"),
            ]),
            "a name left empty or without kanji is no line of its own"
        );
    }

    #[test]
    fn stations_sharing_a_line_add_their_weights_and_more_site_links_cost_less() {
        let dict = station_dictionary(&[
            station("Q1", "三条駅", &["さんじょうえき"], 10),
            station("Q2", "三条駅", &["さんじょうえき"], 10),
            station("Q3", "青梅駅", &["おうめえき"], 15),
            station("Q4", "中書島駅", &["ちゅうしょじまえき"], 0),
        ]);

        let sanjo = cost(&dict, "さんじょうえき", "三条駅");
        let ome = cost(&dict, "おうめえき", "青梅駅");
        let chushojima = cost(&dict, "ちゅうしょじまえき", "中書島駅");
        assert!(sanjo < ome, "two stations of 10 outweigh one of 15");
        assert!(ome < chushojima);
        assert_eq!(chushojima, cost_of(1, 2 * (10 + 10 + 15 + 1)));
    }
}

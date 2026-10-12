//! Why the evaluation's units were not offered their surface, as Kanaemi
//! explains the conversion of each.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use kanaemi_engine::{DropReason, Engine, Explanation, OriginItem};

use crate::{Class, Key};

/// The readings a dictionary text writes each surface under: those of its
/// words and okurigana lines, and apart those of its stems.
#[derive(Debug, Default)]
pub struct Readings {
    words: HashMap<String, BTreeSet<String>>,
    stems: HashMap<String, BTreeSet<String>>,
}

impl Readings {
    pub fn parse(text: impl AsRef<str>) -> Self {
        let mut readings = Self::default();
        for line in text.as_ref().lines().filter(|l| !l.starts_with('#')) {
            let mut fields = line.split('\t');
            let (Some(reading), Some(surface)) = (fields.next(), fields.next()) else {
                continue;
            };
            let into = match fields.next() {
                Some(conjugation) if !conjugation.is_empty() => &mut readings.stems,
                _ => &mut readings.words,
            };
            into.entry(surface.to_owned())
                .or_default()
                .insert(reading.to_owned());
        }
        readings
    }

    /// The readings of `surface`, or else, when it is typed with its
    /// okurigana marked, of the stems it is built from (see [`stems_of`]).
    fn of(&self, surface: &str, marked: bool) -> BTreeSet<&str> {
        let mut found: BTreeSet<&str> = self
            .words
            .get(surface)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if found.is_empty() && marked {
            for stem in stems_of(surface) {
                found.extend(
                    self.stems
                        .get(stem)
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            }
        }
        found
    }
}

/// The stems a surface typed with its okurigana marked is built from: the
/// surface itself (食べ of 食べる) or its part before the hiragana it ends in
/// (書 of 書い, 取り組 of 取り組み), never a shorter one (取, another word).
fn stems_of(surface: &str) -> BTreeSet<&str> {
    let before = surface.trim_end_matches(|c: char| crate::kana::is_hiragana(c));
    [surface, before]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect()
}

/// Why `expected` was not among the candidates of `explanation`: the reason
/// Kanaemi left the item out for, `other-reading` when the dictionary has the
/// surface only under other readings, or `not-in-dictionary`; and a detail to
/// look at.
pub fn miss_reason(
    explanation: &Explanation,
    expected: &str,
    readings: &Readings,
) -> (&'static str, String) {
    // An item left out is the one for `expected` when it made that surface,
    // or, typed with the okurigana marked, when it is a stem `expected` is
    // built from (分 of 分から, which a stem read わか builds into 分ら).
    // Typed whole, a stem only shares the start by chance (深 of the place
    // 深良), and so does a word written whole (会 of 会い).
    let dropped = explanation
        .dropped
        .iter()
        .find(|d| d.surface.as_deref() == Some(expected))
        .or_else(|| {
            explanation.okurigana.as_ref()?;
            let stems = stems_of(expected);
            explanation.dropped.iter().find(|d| {
                matches!(&d.origin.item, OriginItem::Conjugated { surface, .. } if stems.contains(surface.as_str()))
            })
        });
    if let Some(dropped) = dropped {
        let reason = match dropped.reason {
            DropReason::Hidden => "hidden",
            DropReason::NotFilled => "not-filled",
            DropReason::NotConjugated => "not-conjugated",
            DropReason::OkuriganaMismatch => "okurigana-mismatch",
            DropReason::Shadowed { .. } => "shadowed",
            _ => "dropped",
        };
        return (reason, shown_item(&dropped.origin.item));
    }
    let others = readings.of(expected, explanation.okurigana.is_some());
    if others.is_empty() {
        ("not-in-dictionary", String::new())
    } else {
        (
            "other-reading",
            others.into_iter().collect::<Vec<_>>().join(" "),
        )
    }
}

fn shown_item(item: &OriginItem) -> String {
    match item {
        OriginItem::Word { reading, surface } | OriginItem::Placeholders { reading, surface } => {
            format!("{reading} {surface}")
        }
        OriginItem::Okuri {
            stem,
            head,
            surface,
        } => format!("{stem}*{head:?} {surface}"),
        OriginItem::Conjugated {
            stem,
            surface,
            conjugation,
            rest,
        } => format!("{stem} {surface} {conjugation} +{rest}"),
        _ => format!("{item:?}"),
    }
}

/// The table of the surfaces not offered with why: class, reading as typed,
/// expected surface, units, reason and detail, the most units first; and the
/// units of each reason. A numeric unit's key keeps its number as a
/// placeholder, which Kanaemi cannot be asked about, so it falls under
/// `numeric`.
pub fn miss_reasons_tsv(
    engine: &Engine,
    missing: &[(Class, Key, String, u64)],
    readings: &Readings,
) -> (String, BTreeMap<&'static str, u64>) {
    let mut rows: Vec<(u64, String)> = Vec::new();
    let mut by_reason: BTreeMap<&'static str, u64> = BTreeMap::new();
    for (class, (reading, okurigana), expected, units) in missing {
        let (reason, detail) = if *class == Class::Numeric {
            ("numeric", String::new())
        } else {
            miss_reason(
                &engine.explain(reading, okurigana.as_deref()),
                expected,
                readings,
            )
        };
        *by_reason.entry(reason).or_default() += units;
        let typed = match okurigana {
            Some(first) => format!(
                "{}*{first}",
                reading.strip_suffix(first.as_str()).unwrap_or(reading)
            ),
            None => reading.clone(),
        };
        rows.push((
            *units,
            format!(
                "{}\t{typed}\t{expected}\t{units}\t{reason}\t{detail}",
                class.name()
            ),
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut tsv = String::from("class\treading\texpected\tunits\treason\tdetail\n");
    for (_, row) in rows {
        writeln!(tsv, "{row}").expect("writing to a String never fails");
    }
    (tsv, by_reason)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kanaemi_engine::TextDictionary;

    use super::*;
    use crate::engine;

    fn engine_of(text: &str) -> Engine {
        let (dictionary, invalid) = TextDictionary::parse(text);
        assert!(invalid.is_empty(), "{invalid:?}");
        engine(Arc::new(dictionary))
    }

    #[test]
    fn a_missing_surface_falls_under_why_kanaemi_left_it_out_or_what_the_dictionary_lacks() {
        let text = "わか\t分\t五段-タ行\t5\nふか\t深\t形容詞\t5\nこうしょう\t交渉\t\t5\nこうしょう\t高尚\t\t9\nたかし\t高尚\t\t9\n";
        let engine = engine_of(text);
        let readings = Readings::parse(text);
        let missing = [
            (
                Class::Word,
                ("わから".to_owned(), Some("ら".to_owned())),
                "分から".to_owned(),
                3,
            ),
            (
                Class::Word,
                ("こうしょう".to_owned(), None),
                "公証".to_owned(),
                2,
            ),
            (
                Class::Word,
                ("ふから".to_owned(), None),
                "深良".to_owned(),
                2,
            ),
            (
                Class::Compound,
                ("こうしょうか".to_owned(), None),
                "高尚".to_owned(),
                1,
            ),
            (
                Class::Numeric,
                ("{}がい".to_owned(), None),
                "{half-num}階".to_owned(),
                1,
            ),
        ];

        let (tsv, by_reason) = miss_reasons_tsv(&engine, &missing, &readings);

        let reasons: Vec<(&str, &str)> = tsv
            .lines()
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                (f[2], f[4])
            })
            .collect();
        assert_eq!(
            reasons,
            [
                ("分から", "not-conjugated"),
                ("公証", "not-in-dictionary"),
                ("深良", "not-in-dictionary"),
                ("高尚", "other-reading"),
                ("{half-num}階", "numeric"),
            ]
        );
        assert!(
            tsv.contains("\tother-reading\tこうしょう たかし\n"),
            "{tsv}"
        );
        assert_eq!(by_reason["not-conjugated"], 3);
    }

    #[test]
    fn a_marked_surface_is_looked_up_under_the_stems_it_is_built_from_alone() {
        let text = "ひらけ\t開け\t下一段-カ行\t5\nと\t取\t五段-ラ行\t5\nかい\t会\t\t5\n";
        let engine = engine_of(text);
        let readings = Readings::parse(text);
        let reason = |reading: &str, first: &str, expected: &str| {
            miss_reason(&engine.explain(reading, Some(first)), expected, &readings).0
        };

        assert_eq!(
            reason("あけ", "け", "開け"),
            "other-reading",
            "the whole surface is a stem"
        );
        assert_eq!(
            reason("とりくみ", "み", "取り組み"),
            "not-in-dictionary",
            "取 is another word"
        );
        assert_eq!(
            reason("かい", "い", "会い"),
            "not-in-dictionary",
            "会 is a word written whole, no stem of 会い"
        );
    }
}

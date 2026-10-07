//! Words of a small number and a counter (一匹, 三本), typed with the number in
//! kana (いっぴき, さんぼん), which numeric items cannot give: Kanaemi takes a
//! number in a reading only as digits.

use std::collections::BTreeMap;

use crate::kana::{
    HA_ROW, PLAIN, SEMI_VOICED, VOICED, is_hiragana, is_kanji, is_katakana, is_reading_kana,
    unvoiced,
};
use crate::units::reading_after_number;
use crate::{Entry, Token, UnidicReadings};

/// The numbers the words are made for, as kanji.
const NUMBERS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
/// Readings of a number in Sino-Japanese, which a counter's first sound
/// changes; any other reading (みっ, ふた) is Japanese and already changed.
const SINO: [&str; 13] = [
    "いち",
    "に",
    "さん",
    "よん",
    "し",
    "ご",
    "ろく",
    "なな",
    "しち",
    "はち",
    "きゅう",
    "く",
    "じゅう",
];
/// Added to a counter's cost for choosing one of the ten numbers: −ln(1/10) × 100.
const ONE_OF_TEN: u32 = 230;

/// How the analyzer reads a word of a number and a counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountedReading {
    /// Read as a common noun, or one and the words after it (一人 as ひとり,
    /// 九時間 as くじ＋かん).
    Word(String),
    /// Read as the number and the counter after it.
    Counted { number: String, counter: String },
}

/// How `tokens`, the analyzer's tokens of a text starting with `surface`,
/// read `surface`: from a common noun at its head, or as its leading numeral
/// and the tokens after it. None when the tokens do not end where `surface`
/// does.
pub fn counted_reading(tokens: &[Token], surface: &str) -> Option<CountedReading> {
    let len = surface.chars().count();
    let within: Vec<&Token> = tokens.iter().take_while(|t| t.begin < len).collect();
    let last = within.last()?;
    if last.begin + last.surface.chars().count() != len {
        return None;
    }
    let [head, rest @ ..] = within.as_slice() else {
        return None;
    };
    // The tokens after the number and its counter, a common noun at the head
    // holding both (一人).
    let read_after = |counter_at: usize| -> String {
        rest.iter()
            .enumerate()
            .map(|(i, t)| reading_after_number(i + counter_at, t))
            .collect()
    };
    if head.pos[0] == "名詞" && head.pos[1] == "普通名詞" {
        Some(CountedReading::Word(format!(
            "{}{}",
            head.reading,
            read_after(1)
        )))
    } else if head.pos[1] == "数詞" && !rest.is_empty() {
        let rest = read_after(0);
        Some(CountedReading::Counted {
            number: head.reading.clone(),
            counter: rest,
        })
    } else {
        None
    }
}

/// The words of one to ten with each counter of `numeric`, read with `read`
/// (see [`counted_reading`]) and `counter_readings`, UniDic's readings of
/// its counters (see [`crate::counter_words`]), each at its counter's cost
/// and one of ten numbers more.
pub fn counted_words<E>(
    numeric: &[Entry],
    counter_readings: &UnidicReadings,
    mut read: impl FnMut(&str) -> Result<Option<CountedReading>, E>,
) -> Result<Vec<Entry>, E> {
    let mut words = Vec::new();
    for (counter, cost) in counters(numeric) {
        for number in NUMBERS {
            let surface = format!("{number}{counter}");
            let reading = match read(&surface)? {
                Some(CountedReading::Word(reading)) => reading,
                Some(CountedReading::Counted {
                    number,
                    counter: counter_reading,
                }) if SINO.contains(&number.as_str()) => sound_changes(
                    &number,
                    &base_reading(&counter, &counter_reading, counter_readings),
                ),
                Some(CountedReading::Counted {
                    number,
                    counter: counter_reading,
                }) => format!("{number}{counter_reading}"),
                None => continue,
            };
            if reading.is_empty() || !reading.chars().all(is_reading_kana) {
                continue;
            }
            words.push(Entry {
                reading,
                surface,
                conjugation: None,
                cost: cost + ONE_OF_TEN,
            });
        }
    }
    words.sort_by(|a, b| (&a.reading, &a.surface).cmp(&(&b.reading, &b.surface)));
    Ok(words)
}

/// The counters of the numeric items that start with their one placeholder
/// (`{kanji}匹` read `{}ひき` has 匹), written from a kanji or a kana and with
/// no bracket (not mm, nor 日（セル）), each at the smallest cost among its
/// items.
fn counters(numeric: &[Entry]) -> BTreeMap<String, u32> {
    let mut counters: BTreeMap<String, u32> = BTreeMap::new();
    for entry in numeric {
        let Some(after) = entry.reading.strip_prefix("{}") else {
            continue;
        };
        let Some((_, counter)) = entry
            .surface
            .strip_prefix('{')
            .and_then(|s| s.split_once('}'))
        else {
            continue;
        };
        let written_in_kana_or_kanji = counter
            .chars()
            .next()
            .is_some_and(|c| is_kanji(c) || is_hiragana(c) || is_katakana(c));
        if after.contains("{}")
            || !written_in_kana_or_kanji
            || counter.contains(['{', '}', '（', '）', '(', ')'])
        {
            continue;
        }
        counters
            .entry(counter.to_owned())
            .and_modify(|c| *c = (*c).min(entry.cost))
            .or_insert(entry.cost);
    }
    counters
}

/// The reading of the counter `surface`, read `read` by the analyzer, that
/// the sound changes start from, by UniDic's readings of it as a counter:
/// `read` with its first kana unvoiced when UniDic gives that (杯 as はい for
/// ばい), else `read` when UniDic gives it (倍 stays ばい), else UniDic's only
/// reading (話 as わ for はなし); `read` when none of them is there.
fn base_reading(surface: &str, read: &str, counters: &UnidicReadings) -> String {
    base_reading_of(counters.of(surface), read)
}

/// [`base_reading`] of a counter read `read`, UniDic reading it `known`.
fn base_reading_of(known: &[String], read: &str) -> String {
    let mut chars = read.chars();
    let plain = chars
        .next()
        .map(|first| format!("{}{}", unvoiced(first), chars.as_str()));
    match (plain, known) {
        (Some(plain), _) if known.contains(&plain) => plain,
        _ if known.iter().any(|k| k == read) => read.to_owned(),
        (_, [only]) => only.clone(),
        _ => read.to_owned(),
    }
}

/// The reading of numerals read `numeral` that write no number (なん, すうじゅう)
/// and the counter `surface` after them, read `read` by the analyzer: the
/// counter from its UniDic counter reading, changed as after 三 when the
/// numerals end in ん, after 十 when in じゅう and after 六 when in ひゃく,
/// びゃく or ぴゃく.
pub(crate) fn reading_counted_after(
    numeral: &str,
    surface: &str,
    read: &str,
    readings: &UnidicReadings,
) -> String {
    let counter = base_reading_of(readings.as_counter(surface), read);
    for (ending, number) in [
        ("ん", "さん"),
        ("じゅう", "じゅう"),
        ("ひゃく", "ろく"),
        ("びゃく", "ろく"),
        ("ぴゃく", "ろく"),
    ] {
        let Some(head) = numeral.strip_suffix(ending) else {
            continue;
        };
        let changed = sound_changes(number, &counter);
        if let Some(rest) = changed.strip_prefix(&geminated(number)) {
            return format!("{head}{}{rest}", geminated(ending));
        }
        if let Some(rest) = changed.strip_prefix(number) {
            return format!("{head}{ending}{rest}");
        }
    }
    format!("{numeral}{counter}")
}

/// `kana` with its last kana made a small っ (じゅう as じゅっ).
fn geminated(kana: &str) -> String {
    let mut s = kana.to_owned();
    s.pop();
    s.push('っ');
    s
}

/// The reading of `number`, read in Sino-Japanese, before a counter read
/// `counter`, changed the common way by the counter's first sound: か row
/// geminates 一・六・八・十, さ and た rows 一・八・十, and は row geminates
/// 一・六・八・十 into its semi-voiced form and voices it after 三.
fn sound_changes(number: &str, counter: &str) -> String {
    let unchanged = || format!("{number}{counter}");
    let mut chars = counter.chars();
    let Some(first) = chars.next() else {
        return unchanged();
    };
    let rest = chars.as_str();
    let Some(i) = PLAIN.chars().position(|k| k == first) else {
        return unchanged();
    };
    let geminated = match number {
        "いち" => "いっ",
        "ろく" => "ろっ",
        "はち" => "はっ",
        "じゅう" => "じゅっ",
        _ => number,
    };
    let (number, kana) = match (i / 5, number) {
        (0, "いち" | "ろく" | "はち" | "じゅう") | (1 | 2, "いち" | "はち" | "じゅう") => {
            (geminated, first)
        }
        (3, "いち" | "ろく" | "はち" | "じゅう") => (
            geminated,
            SEMI_VOICED
                .chars()
                .nth(i - HA_ROW)
                .expect("SEMI_VOICED pairs the は row"),
        ),
        (3, "さん") => (number, VOICED.chars().nth(i).expect("VOICED pairs PLAIN")),
        _ => return unchanged(),
    };
    format!("{number}{kana}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnidicWord;

    #[test]
    fn a_ka_row_counter_geminates_one_six_eight_and_ten() {
        assert_eq!(sound_changes("いち", "こ"), "いっこ");
        assert_eq!(sound_changes("ろく", "こ"), "ろっこ");
        assert_eq!(sound_changes("はち", "こ"), "はっこ");
        assert_eq!(sound_changes("じゅう", "かい"), "じゅっかい");
        assert_eq!(sound_changes("さん", "かい"), "さんかい");
        assert_eq!(sound_changes("に", "こ"), "にこ");
    }

    #[test]
    fn a_sa_or_ta_row_counter_geminates_one_eight_and_ten() {
        assert_eq!(sound_changes("いち", "たく"), "いったく");
        assert_eq!(sound_changes("はち", "さつ"), "はっさつ");
        assert_eq!(sound_changes("じゅう", "さつ"), "じゅっさつ");
        assert_eq!(sound_changes("ろく", "さつ"), "ろくさつ");
        assert_eq!(sound_changes("さん", "たく"), "さんたく");
    }

    #[test]
    fn a_ha_row_counter_takes_the_semi_voiced_form_after_a_gemination_and_voices_after_three() {
        assert_eq!(sound_changes("いち", "ひき"), "いっぴき");
        assert_eq!(sound_changes("ろく", "ほん"), "ろっぽん");
        assert_eq!(sound_changes("はち", "ひき"), "はっぴき");
        assert_eq!(sound_changes("じゅう", "ほん"), "じゅっぽん");
        assert_eq!(sound_changes("さん", "ほん"), "さんぼん");
        assert_eq!(sound_changes("よん", "ほん"), "よんほん");
        assert_eq!(sound_changes("に", "ひき"), "にひき");
    }

    #[test]
    fn a_counter_of_another_sound_changes_nothing() {
        assert_eq!(sound_changes("いち", "まい"), "いちまい");
        assert_eq!(sound_changes("さん", "ばい"), "さんばい");
    }

    fn readings(words: &[(&str, &str)]) -> UnidicReadings {
        UnidicReadings::new(words.iter().map(|(reading, surface)| UnidicWord {
            reading: (*reading).into(),
            surface: (*surface).into(),
        }))
    }

    #[test]
    fn a_counter_starts_from_its_unidic_counter_reading() {
        let unidic = readings(&[
            ("はい", "杯"),
            ("ばい", "杯"),
            ("ばい", "倍"),
            ("だい", "台"),
            ("わ", "話"),
        ]);

        assert_eq!(base_reading("杯", "ばい", &unidic), "はい");
        assert_eq!(base_reading("倍", "ばい", &unidic), "ばい");
        assert_eq!(base_reading("台", "だい", &unidic), "だい");
        assert_eq!(base_reading("話", "はなし", &unidic), "わ");
        assert_eq!(base_reading("匹", "ひき", &unidic), "ひき");
    }

    fn as_counters(words: &[(&str, &str)]) -> UnidicReadings {
        UnidicReadings::default().with_counters(words.iter().map(|(reading, surface)| UnidicWord {
            reading: (*reading).into(),
            surface: (*surface).into(),
        }))
    }

    #[test]
    fn a_counter_after_numerals_ending_in_n_ju_or_hyaku_changes_as_after_three_ten_or_six() {
        let unidic = as_counters(&[("ほん", "本"), ("ひき", "匹"), ("かい", "回")]);

        assert_eq!(
            reading_counted_after("なん", "匹", "ひき", &unidic),
            "なんびき"
        );
        assert_eq!(
            reading_counted_after("なん", "本", "ぽん", &unidic),
            "なんぼん"
        );
        assert_eq!(
            reading_counted_after("すうじゅう", "本", "ぽん", &unidic),
            "すうじゅっぽん"
        );
        assert_eq!(
            reading_counted_after("すうひゃく", "回", "かい", &unidic),
            "すうひゃっかい"
        );
        assert_eq!(
            reading_counted_after("すうひゃく", "本", "ぽん", &unidic),
            "すうひゃっぽん"
        );
        assert_eq!(
            reading_counted_after("なんびゃく", "本", "ぽん", &unidic),
            "なんびゃっぽん"
        );
        assert_eq!(
            reading_counted_after("さんぴゃく", "匹", "ひき", &unidic),
            "さんぴゃっぴき"
        );
    }

    #[test]
    fn a_counter_after_other_numerals_keeps_its_unidic_counter_reading() {
        let unidic = as_counters(&[("ほん", "本"), ("ばい", "倍")]);

        assert_eq!(
            reading_counted_after("すう", "本", "ぽん", &unidic),
            "すうほん"
        );
        assert_eq!(
            reading_counted_after("いく", "本", "ぽん", &unidic),
            "いくほん"
        );
        assert_eq!(
            reading_counted_after("なん", "倍", "ばい", &unidic),
            "なんばい"
        );
    }

    #[test]
    fn a_counter_starts_from_unidic_s_counter_readings_not_its_other_words() {
        let unidic = readings(&[("たい", "台")]).with_counters([UnidicWord {
            reading: "だい".into(),
            surface: "台".into(),
        }]);

        assert_eq!(
            reading_counted_after("すう", "台", "だい", &unidic),
            "すうだい"
        );
    }

    fn tok(surface: &str, reading: &str, pos: &str, begin: usize) -> Token {
        Token {
            surface: surface.into(),
            reading: reading.into(),
            pos: pos.split(',').map(str::to_owned).collect(),
            dictionary_form: surface.into(),
            normalized_form: surface.into(),
            begin,
        }
    }

    const NUMERAL: &str = "名詞,数詞,*,*,*,*";
    const SUFFIX: &str = "接尾辞,名詞的,助数詞,*,*,*";
    const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const PARTICLE: &str = "助詞,格助詞,*,*,*,*";

    #[test]
    fn a_number_and_a_counter_read_apart_give_both_readings() {
        let tokens = [
            tok("三", "みっ", NUMERAL, 0),
            tok("日", "か", SUFFIX, 1),
            tok("の", "の", PARTICLE, 2),
        ];

        assert_eq!(
            counted_reading(&tokens, "三日"),
            Some(CountedReading::Counted {
                number: "みっ".into(),
                counter: "か".into()
            })
        );
    }

    #[test]
    fn i_after_the_counter_reads_kurai() {
        let tokens = [
            tok("六", "ろく", NUMERAL, 0),
            tok("秒", "びょう", SUFFIX, 1),
            tok("位", "い", SUFFIX, 2),
            tok("の", "の", PARTICLE, 3),
        ];

        assert_eq!(
            counted_reading(&tokens, "六秒位"),
            Some(CountedReading::Counted {
                number: "ろく".into(),
                counter: "びょうくらい".into()
            })
        );
    }

    #[test]
    fn i_after_a_noun_that_holds_the_counter_reads_kurai() {
        let tokens = [
            tok("一人", "ひとり", NOUN, 0),
            tok("位", "い", SUFFIX, 2),
            tok("の", "の", PARTICLE, 3),
        ];

        assert_eq!(
            counted_reading(&tokens, "一人位"),
            Some(CountedReading::Word("ひとりくらい".into()))
        );
    }

    #[test]
    fn a_word_read_as_one_noun_gives_its_reading() {
        let tokens = [tok("一人", "ひとり", NOUN, 0), tok("の", "の", PARTICLE, 2)];

        assert_eq!(
            counted_reading(&tokens, "一人"),
            Some(CountedReading::Word("ひとり".into()))
        );
    }

    #[test]
    fn a_word_read_as_a_noun_and_more_gives_their_readings_joined() {
        let tokens = [
            tok("九時", "くじ", NOUN, 0),
            tok("間", "かん", SUFFIX, 2),
            tok("の", "の", PARTICLE, 3),
        ];

        assert_eq!(
            counted_reading(&tokens, "九時間"),
            Some(CountedReading::Word("くじかん".into()))
        );
    }

    #[test]
    fn a_word_read_otherwise_gives_nothing() {
        let proper = [
            tok("三本", "みもと", "名詞,固有名詞,人名,姓,*,*", 0),
            tok("の", "の", PARTICLE, 2),
        ];
        let past_the_end = [
            tok("一", "いち", NUMERAL, 0),
            tok("日の", "にちの", NOUN, 1),
        ];

        assert_eq!(counted_reading(&proper, "三本"), None);
        assert_eq!(counted_reading(&past_the_end, "一日"), None);
    }

    fn numeric(reading: &str, surface: &str, cost: u32) -> Entry {
        Entry {
            reading: reading.into(),
            surface: surface.into(),
            conjugation: None,
            cost,
        }
    }

    #[test]
    fn each_counter_gets_one_to_ten_read_by_the_analyzer_and_the_sound_changes() {
        let numeric = [
            numeric("{}ひき", "{kanji}匹", 900),
            numeric("{}ひき", "{}匹", 800),
            numeric("だい{}かい", "第{kanji}回", 500),
            numeric("{}がつ{}にち", "{kanji}月{kanji}日", 500),
            numeric("{}みりめーとる", "{}mm", 500),
            numeric("{}ひき", "{kanji}匹（セル）", 500),
        ];
        let read = |surface: &str| -> Result<Option<CountedReading>, ()> {
            let number = match surface.chars().next().unwrap() {
                '一' => "いち",
                '二' => "に",
                '三' => "さん",
                _ => return Ok(None),
            };
            Ok(Some(CountedReading::Counted {
                number: number.into(),
                counter: "ひき".into(),
            }))
        };

        let words = counted_words(&numeric, &UnidicReadings::default(), read).unwrap();

        let view: Vec<_> = words
            .iter()
            .map(|w| (w.reading.as_str(), w.surface.as_str(), w.cost))
            .collect();
        assert_eq!(
            view,
            [
                ("いっぴき", "一匹", 1030),
                ("さんびき", "三匹", 1030),
                ("にひき", "二匹", 1030),
            ]
        );
    }

    #[test]
    fn a_japanese_reading_of_the_number_is_kept_as_read() {
        let numeric = [numeric("{}か", "{kanji}日", 900)];
        let read = |surface: &str| -> Result<Option<CountedReading>, ()> {
            Ok((surface == "三日").then(|| CountedReading::Counted {
                number: "みっ".into(),
                counter: "か".into(),
            }))
        };

        let words = counted_words(&numeric, &UnidicReadings::default(), read).unwrap();

        assert_eq!(words.len(), 1);
        assert_eq!(words[0].reading, "みっか");
    }
}

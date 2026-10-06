//! Reading a number as the text writes it: its value, and the notation of
//! Kanaemi's numeric conversion that writes the value back the same way.

/// How a number is written, named as Kanaemi's numeric placeholders name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notation {
    /// ASCII digits (12).
    Plain,
    /// Full-width digits (１２).
    WideNum,
    /// Kanji digits one by one (一二, 二〇二六).
    KanjiNum,
    /// Positional kanji (十二, 二千二十六).
    Kanji,
    /// Positional kanji with 壱弐参拾 (壱拾弐).
    Daiji,
    /// ASCII digits with a comma every three digits (1,000).
    GroupedNum,
}

/// A number with its value as the ASCII digits a typist enters for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Number {
    pub notation: Notation,
    pub value: String,
}

impl Notation {
    /// The placeholder that writes a number in this notation. ASCII digits
    /// take `half-num`, as `{}` gives the number as typed, full-width when
    /// the typist enters it so.
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Plain => "{half-num}",
            Self::WideNum => "{wide-num}",
            Self::KanjiNum => "{kanji-num}",
            Self::Kanji => "{kanji}",
            Self::Daiji => "{daiji}",
            Self::GroupedNum => "{grouped-num}",
        }
    }
}

const KANJI_DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const DAIJI_DIGITS: [(char, u128); 3] = [('壱', 1), ('弐', 2), ('参', 3)];
const SMALL_UNITS: [(char, u128); 4] = [('千', 1000), ('百', 100), ('十', 10), ('拾', 10)];
const LARGE_UNITS: [char; 4] = ['万', '億', '兆', '京'];
/// Ten thousand 京: neither `kanji` nor `daiji` writes a number this large.
const POSITIONAL_LIMIT: u128 = 10u128.pow(20);

/// The number `s` writes, when one of the notations writes its value back
/// as exactly `s`.
pub fn read_number(s: impl AsRef<str>) -> Option<Number> {
    let s = s.as_ref();
    let number = |notation, value: String| Some(Number { notation, value });
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return number(Notation::Plain, s.to_owned());
    }
    if s.chars().all(|c| ('０'..='９').contains(&c)) {
        let value = s
            .chars()
            .map(|c| char::from(b'0' + (c as u32 - '０' as u32) as u8))
            .collect();
        return number(Notation::WideNum, value);
    }
    if s.contains(',') {
        return is_grouped(s).then(|| Number {
            notation: Notation::GroupedNum,
            value: s.replace(',', ""),
        });
    }
    let digits: Option<String> = s.chars().map(kanji_digit).collect();
    if let Some(value) = digits.filter(|d| d.len() > 1) {
        return number(Notation::KanjiNum, value);
    }
    let daiji = s.chars().any(|c| "壱弐参拾".contains(c));
    let value = positional_value(s).filter(|v| *v < POSITIONAL_LIMIT)?;
    if write_positional(value, daiji) != s {
        return None;
    }
    let notation = if daiji {
        Notation::Daiji
    } else {
        Notation::Kanji
    };
    number(notation, value.to_string())
}

fn kanji_digit(c: char) -> Option<char> {
    let d = KANJI_DIGITS.iter().position(|k| *k == c)?;
    Some(char::from(b'0' + d as u8))
}

fn is_grouped(s: &str) -> bool {
    let mut groups = s.split(',');
    let head = groups.next().unwrap_or_default();
    (1..=3).contains(&head.len())
        && !head.starts_with('0')
        && head.bytes().all(|b| b.is_ascii_digit())
        && groups.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
}

/// The value of positional kanji, read leniently: [`write_positional`]
/// decides whether the writing is one a notation produces.
fn positional_value(s: &str) -> Option<u128> {
    let mut total: u128 = 0;
    let mut group: u128 = 0;
    let mut digit: Option<u128> = None;
    for c in s.chars() {
        if let Some(d) = KANJI_DIGITS.iter().position(|k| *k == c) {
            if digit.replace(d as u128).is_some() {
                return None;
            }
        } else if let Some((_, d)) = DAIJI_DIGITS.iter().find(|(k, _)| *k == c) {
            if digit.replace(*d).is_some() {
                return None;
            }
        } else if let Some((_, unit)) = SMALL_UNITS.iter().find(|(k, _)| *k == c) {
            group += digit.take().unwrap_or(1) * unit;
        } else {
            let i = LARGE_UNITS.iter().position(|k| *k == c)?;
            group += digit.take().unwrap_or(0);
            if group == 0 {
                return None;
            }
            let unit = 10u128.pow(4 * (i as u32 + 1));
            total = total.checked_add(group.checked_mul(unit)?)?;
            group = 0;
        }
    }
    total.checked_add(group + digit.unwrap_or(0))
}

/// `value` as Kanaemi's `kanji` notation writes it, or as `daiji` writes it.
fn write_positional(value: u128, daiji: bool) -> String {
    if value == 0 {
        return KANJI_DIGITS[0].to_string();
    }
    let digit = |d: u128| match (daiji, d) {
        (true, 1..=3) => DAIJI_DIGITS[d as usize - 1].0,
        _ => KANJI_DIGITS[d as usize],
    };
    let mut out = String::new();
    for i in (0..=LARGE_UNITS.len()).rev() {
        let group = value / 10u128.pow(4 * i as u32) % 10_000;
        if group == 0 {
            continue;
        }
        for (unit, kanji) in [
            (1000, '千'),
            (100, '百'),
            (10, if daiji { '拾' } else { '十' }),
        ] {
            let d = group / unit % 10;
            if d == 0 {
                continue;
            }
            if d != 1 || daiji {
                out.push(digit(d));
            }
            out.push(kanji);
        }
        if !group.is_multiple_of(10) {
            out.push(digit(group % 10));
        }
        if i > 0 {
            out.push(LARGE_UNITS[i - 1]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(s: &str) -> Option<(Notation, String)> {
        read_number(s).map(|n| (n.notation, n.value))
    }

    fn num(notation: Notation, value: &str) -> Option<(Notation, String)> {
        Some((notation, value.to_owned()))
    }

    #[test]
    fn each_notation_has_the_placeholder_kanaemi_names_it_by() {
        let placeholders = [
            Notation::Plain,
            Notation::WideNum,
            Notation::KanjiNum,
            Notation::Kanji,
            Notation::Daiji,
            Notation::GroupedNum,
        ]
        .map(Notation::placeholder);

        assert_eq!(
            placeholders,
            [
                "{half-num}",
                "{wide-num}",
                "{kanji-num}",
                "{kanji}",
                "{daiji}",
                "{grouped-num}"
            ]
        );
    }

    #[test]
    fn ascii_digits_are_plain_and_keep_their_leading_zeros() {
        assert_eq!(read("3"), num(Notation::Plain, "3"));
        assert_eq!(read("2026"), num(Notation::Plain, "2026"));
        assert_eq!(read("007"), num(Notation::Plain, "007"));
    }

    #[test]
    fn full_width_digits_are_wide_and_typed_as_ascii_digits() {
        assert_eq!(read("３"), num(Notation::WideNum, "3"));
        assert_eq!(read("０５０"), num(Notation::WideNum, "050"));
    }

    #[test]
    fn ascii_and_full_width_digits_mixed_are_no_number() {
        assert_eq!(read("1２"), None);
    }

    #[test]
    fn commas_between_groups_of_three_digits_are_grouped() {
        assert_eq!(read("1,000"), num(Notation::GroupedNum, "1000"));
        assert_eq!(read("12,345,678"), num(Notation::GroupedNum, "12345678"));
    }

    #[test]
    fn commas_off_the_groups_of_three_are_no_number() {
        assert_eq!(read("1,00"), None);
        assert_eq!(read("1000,000"), None);
        assert_eq!(read("01,000"), None);
        assert_eq!(read(",100"), None);
        assert_eq!(read("100,"), None);
        assert_eq!(read("１,０００"), None);
    }

    #[test]
    fn several_kanji_digits_without_positions_are_kanji_digits() {
        assert_eq!(read("二〇二六"), num(Notation::KanjiNum, "2026"));
        assert_eq!(read("〇七"), num(Notation::KanjiNum, "07"));
    }

    #[test]
    fn a_single_kanji_digit_is_positional_kanji() {
        assert_eq!(read("三"), num(Notation::Kanji, "3"));
        assert_eq!(read("〇"), num(Notation::Kanji, "0"));
    }

    #[test]
    fn positional_kanji_are_read_to_their_value() {
        assert_eq!(read("十二"), num(Notation::Kanji, "12"));
        assert_eq!(read("二十六"), num(Notation::Kanji, "26"));
        assert_eq!(read("百"), num(Notation::Kanji, "100"));
        assert_eq!(read("千百十一"), num(Notation::Kanji, "1111"));
        assert_eq!(read("一万"), num(Notation::Kanji, "10000"));
        assert_eq!(read("十万十"), num(Notation::Kanji, "100010"));
        assert_eq!(read("二千二十六"), num(Notation::Kanji, "2026"));
        assert_eq!(read("三億五千万"), num(Notation::Kanji, "350000000"));
    }

    #[test]
    fn positional_kanji_the_kanji_notation_would_write_otherwise_are_no_number() {
        assert_eq!(read("一千"), None, "kanji omits 一 before 千");
        assert_eq!(read("万"), None, "kanji writes 一 before 万");
        assert_eq!(read("二三百"), None, "an estimate, not a value");
        assert_eq!(read("千〇十"), None);
        assert_eq!(read("十百"), None);
        assert_eq!(read("万億"), None);
        assert_eq!(read("二十〇"), None);
    }

    #[test]
    fn positional_kanji_reach_up_to_just_below_ten_thousand_kei() {
        assert_eq!(
            read("九千九百九十九京"),
            num(Notation::Kanji, "99990000000000000000")
        );
        assert_eq!(read("一万京"), None);
    }

    #[test]
    fn positional_kanji_with_daiji_are_daiji_and_write_every_one() {
        assert_eq!(read("壱千弐百"), num(Notation::Daiji, "1200"));
        assert_eq!(read("壱"), num(Notation::Daiji, "1"));
        assert_eq!(read("参拾"), num(Notation::Daiji, "30"));
        assert_eq!(read("壱千壱百壱拾壱"), num(Notation::Daiji, "1111"));
        assert_eq!(read("千弐百"), None, "daiji writes 壱 before 千");
        assert_eq!(read("壱千二百"), None, "daiji writes 二 as 弐");
    }

    #[test]
    fn digits_mixed_with_kanji_decimals_and_other_text_are_no_number() {
        assert_eq!(read("3万"), None);
        assert_eq!(read("1.5"), None);
        assert_eq!(read("三・五"), None);
        assert_eq!(read("数"), None);
        assert_eq!(read(""), None);
    }
}

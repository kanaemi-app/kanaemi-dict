//! Telling kana and kanji apart, and turning katakana into hiragana.

/// Katakana to hiragana, leaving every other character alone.
pub fn katakana_to_hiragana(s: impl AsRef<str>) -> String {
    s.as_ref()
        .chars()
        .map(|c| match c {
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Hiragana to katakana, leaving every other character alone.
pub(crate) fn hiragana_to_katakana(s: impl AsRef<str>) -> String {
    s.as_ref()
        .chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

pub(crate) fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}')
}

/// Hiragana or the long vowel mark: what a reading Kanaemi takes is made of.
pub(crate) fn is_reading_kana(c: char) -> bool {
    is_hiragana(c) || c == 'ー'
}

pub(crate) fn is_katakana(c: char) -> bool {
    matches!(c, 'ァ'..='ヺ')
}

pub(crate) fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '々' | '〆' | 'ヶ')
}

pub(crate) fn has_kanji(s: &str) -> bool {
    s.chars().any(is_kanji)
}

/// The kana that have voiced forms, in the order of [`VOICED`].
pub(crate) const PLAIN: &str = "かきくけこさしすせそたちつてとはひふへほ";
pub(crate) const VOICED: &str = "がぎぐげござじずぜぞだぢづでどばびぶべぼ";
pub(crate) const SEMI_VOICED: &str = "ぱぴぷぺぽ";
/// Where the は row starts in [`PLAIN`], the only row with semi-voiced forms.
pub(crate) const HA_ROW: usize = 15;

/// Whether `voiced` is `plain` voiced or semi-voiced, as it is where words
/// join (rendaku), ち and つ also as じ and ず the way they are written then.
pub(crate) fn is_voicing_of(plain: char, voiced: char) -> bool {
    let Some(i) = PLAIN.chars().position(|k| k == plain) else {
        return false;
    };
    VOICED.chars().nth(i) == Some(voiced)
        || i.checked_sub(HA_ROW)
            .and_then(|j| SEMI_VOICED.chars().nth(j))
            == Some(voiced)
        || matches!((plain, voiced), ('ち', 'じ') | ('つ', 'ず'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katakana_becomes_hiragana_and_the_rest_stays() {
        assert_eq!(
            katakana_to_hiragana("カキクケコヴヵヶーABCかな漢"),
            "かきくけこゔゕゖーABCかな漢"
        );
    }

    #[test]
    fn hiragana_becomes_katakana_and_the_rest_stays() {
        assert_eq!(
            hiragana_to_katakana("ぁかきゔゕゖーABCカナ漢"),
            "ァカキヴヵヶーABCカナ漢"
        );
    }

    #[test]
    fn a_kana_voices_into_its_voiced_and_semi_voiced_forms_and_chi_tsu_also_into_ji_zu() {
        assert!(is_voicing_of('す', 'ず'));
        assert!(is_voicing_of('は', 'ば') && is_voicing_of('は', 'ぱ'));
        assert!(is_voicing_of('ち', 'ぢ') && is_voicing_of('ち', 'じ'));
        assert!(is_voicing_of('つ', 'ず'));
        assert!(!is_voicing_of('か', 'ぱ'));
        assert!(!is_voicing_of('ず', 'す'));
        assert!(!is_voicing_of('あ', 'あ'));
    }

    #[test]
    fn the_long_vowel_mark_is_reading_kana_but_not_hiragana() {
        assert!(is_reading_kana('ー'));
        assert!(!is_hiragana('ー'));
        assert!(is_hiragana('ぁ') && is_hiragana('ゖ'));
        assert!(!is_reading_kana('ア'));
    }

    #[test]
    fn iteration_marks_and_small_ke_count_as_kanji() {
        assert!(
            ['漢', '々', '〆', 'ヶ', '㐀', '豈']
                .into_iter()
                .all(is_kanji)
        );
        assert!(!is_kanji('か') && !is_kanji('カ'));
        assert!(has_kanji("書く") && !has_kanji("かく"));
    }

    #[test]
    fn katakana_runs_from_small_a_to_vo() {
        assert!(['ァ', 'カ', 'ヺ'].into_iter().all(is_katakana));
        assert!(!is_katakana('ー') && !is_katakana('か'));
    }
}

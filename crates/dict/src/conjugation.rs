//! Sudachi's conjugation types as Kanaemi names them.
//!
//! Sudachi names its types as UniDic does, which Kanaemi follows, except for
//! the words Kanaemi gives a type of their own and the types and words its
//! table leaves out.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// How Kanaemi takes a word Sudachi conjugates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KanaemiType<'a> {
    /// The type Kanaemi conjugates the word by; it may still be one Kanaemi's
    /// table does not know.
    Named(&'a str),
    /// A word whose stem's reading changes from form to form (来る, する alone,
    /// いい): the dictionary lists each of its forms as a word that does not
    /// conjugate.
    OutsideTable,
}

/// How Kanaemi takes the word of `dictionary_form` that Sudachi conjugates by
/// `sudachi_type`.
pub(crate) fn kanaemi_type<'a>(sudachi_type: &'a str, dictionary_form: &str) -> KanaemiType<'a> {
    let renames = &*RENAMES;
    if renames.outside_types.contains(sudachi_type)
        || renames.outside_words.contains(dictionary_form)
    {
        return KanaemiType::OutsideTable;
    }
    KanaemiType::Named(
        renames
            .words
            .get(dictionary_form)
            .map_or(sudachi_type, String::as_str),
    )
}

struct Renames {
    words: HashMap<String, String>,
    outside_types: HashSet<String>,
    outside_words: HashSet<String>,
}

static RENAMES: LazyLock<Renames> = LazyLock::new(|| {
    Renames::parse(
        include_str!("../assets/conjugation/words.tsv"),
        include_str!("../assets/conjugation/outside.tsv"),
    )
});

impl Renames {
    /// Panics on a malformed line: the data is bundled and its tests load it.
    fn parse(words: &str, outside: &str) -> Self {
        let mut renames = Self {
            words: HashMap::new(),
            outside_types: HashSet::new(),
            outside_words: HashSet::new(),
        };
        for fields in data_lines(words) {
            let [word, conjugation] = fields[..] else {
                panic!("words.tsv: {fields:?}");
            };
            renames
                .words
                .insert(word.to_owned(), conjugation.to_owned());
        }
        for fields in data_lines(outside) {
            match fields[..] {
                ["型", conjugation] => renames.outside_types.insert(conjugation.to_owned()),
                ["語", word] => renames.outside_words.insert(word.to_owned()),
                _ => panic!("outside.tsv: {fields:?}"),
            };
        }
        renames
    }
}

fn data_lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
}

#[cfg(test)]
mod tests {
    use kanaemi_engine::terminal_ending;

    use super::*;

    #[test]
    fn words_with_a_type_of_their_own_are_renamed() {
        assert_eq!(
            kanaemi_type("五段-カ行", "行く"),
            KanaemiType::Named("五段-カ行-促音便")
        );
        assert_eq!(
            kanaemi_type("五段-ワア行", "問う"),
            KanaemiType::Named("五段-ワア行-ウ音便")
        );
        assert_eq!(
            kanaemi_type("五段-ラ行", "下さる"),
            KanaemiType::Named("五段-ラ行-特殊")
        );
    }

    #[test]
    fn other_words_keep_sudachi_s_type() {
        assert_eq!(
            kanaemi_type("五段-カ行", "書く"),
            KanaemiType::Named("五段-カ行")
        );
        assert_eq!(
            kanaemi_type("文語四段-ハ行", "候ふ"),
            KanaemiType::Named("文語四段-ハ行")
        );
    }

    #[test]
    fn the_types_and_words_the_table_leaves_out_are_outside_it() {
        assert_eq!(kanaemi_type("カ行変格", "来る"), KanaemiType::OutsideTable);
        assert_eq!(kanaemi_type("サ行変格", "する"), KanaemiType::OutsideTable);
        assert_eq!(kanaemi_type("形容詞", "いい"), KanaemiType::OutsideTable);
        assert_eq!(
            kanaemi_type("サ行変格", "勉強する"),
            KanaemiType::Named("サ行変格")
        );
    }

    #[test]
    fn every_type_words_are_renamed_to_is_one_kanaemi_knows() {
        for conjugation in RENAMES.words.values() {
            assert!(terminal_ending(conjugation).is_some(), "{conjugation}");
        }
    }

    #[test]
    fn a_comment_or_blank_line_is_not_data() {
        let renames = Renames::parse("# 辞書形\t活用型\n\n行く\tA\n", "# 種類\n型\tB\n語\tC\n");

        assert_eq!(renames.words.len(), 1);
        assert!(renames.outside_types.contains("B"));
        assert!(renames.outside_words.contains("C"));
    }

    #[test]
    #[should_panic(expected = "outside.tsv")]
    fn an_outside_line_of_an_unknown_kind_is_rejected() {
        Renames::parse("", "品詞\t名詞\n");
    }
}

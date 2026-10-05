//! Conversion units: the pieces of text Kanaemi converts in one go, cut from
//! Sudachi's morphemes.

use kanaemi_engine::{MAX_SUFFIX_KANA, terminal_ending};

use crate::Token;
use crate::analyzer::is_break;
use crate::conjugation::{KanaemiType, kanaemi_type};
use crate::kana::{has_kanji, is_kanji, is_katakana, is_reading_kana};

/// A unit with its document and its position in the document's text, as
/// written to `build/units.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Unit {
    pub doc_id: String,
    /// Characters from the start of the document's text.
    pub position: usize,
    pub reading: String,
    pub surface: String,
    pub stem_reading: Option<String>,
    pub stem_surface: Option<String>,
    /// The conjugation type of the stem, as Kanaemi names it.
    pub conjugation: Option<String>,
    /// A conjugation type Kanaemi's table does not know; such a unit never
    /// enters the dictionary.
    pub unknown_conjugation: Option<String>,
}

/// One unit within a line. `begin` counts characters from the line's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineUnit {
    pub(crate) begin: usize,
    pub(crate) reading: String,
    pub(crate) surface: String,
    pub(crate) stem: Option<Stem>,
    pub(crate) unknown_conjugation: Option<String>,
}

/// The stem of a word Kanaemi conjugates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stem {
    pub(crate) reading: String,
    pub(crate) surface: String,
    pub(crate) conjugation: String,
}

/// The most UTF-8 bytes handed to the analyzer at once; longer lines are read
/// in pieces.
pub(crate) const MAX_ANALYZER_BYTES: usize = 40_000;

/// Cuts a document's text into units line by line, reading each line with
/// `tokenize`, and hands each unit to `emit` as its line is cut so a long
/// document's units need not be held at once.
pub(crate) fn each_unit<E>(
    doc_id: &str,
    text: &str,
    mut tokenize: impl FnMut(&str) -> Result<Vec<Token>, E>,
    mut emit: impl FnMut(Unit),
) -> Result<(), E> {
    let mut line_start = 0;
    for line in text.split('\n') {
        let mut tokens = Vec::new();
        for (piece_start, piece) in pieces(line) {
            tokens.extend(tokenize(piece)?.into_iter().map(|mut token| {
                token.begin += piece_start;
                token
            }));
        }
        for u in cut_line(&tokens) {
            let stem = u.stem;
            emit(Unit {
                doc_id: doc_id.to_owned(),
                position: line_start + u.begin,
                reading: u.reading,
                surface: u.surface,
                stem_reading: stem.as_ref().map(|s| s.reading.clone()),
                stem_surface: stem.as_ref().map(|s| s.surface.clone()),
                conjugation: stem.map(|s| s.conjugation),
                unknown_conjugation: u.unknown_conjugation,
            });
        }
        line_start += line.chars().count() + 1;
    }
    Ok(())
}

/// `line` in pieces the analyzer accepts, each with its character offset.
/// A piece of a quarter of the byte limit in characters never exceeds it; a
/// piece ends after the last break in its second half when there is one.
fn pieces(line: &str) -> Vec<(usize, &str)> {
    if line.len() <= MAX_ANALYZER_BYTES {
        return vec![(0, line)];
    }
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let byte_at = |i: usize| chars.get(i).map_or(line.len(), |(b, _)| *b);
    let step = MAX_ANALYZER_BYTES / 4;
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + step).min(chars.len());
        if end < chars.len()
            && let Some(k) = (start + step / 2..end)
                .rev()
                .find(|&k| is_break(chars[k].1))
        {
            end = k + 1;
        }
        out.push((start, &line[byte_at(start)..byte_at(end)]));
        start = end;
    }
    out
}

/// Parts of speech a prefix joins.
const PREFIX_HEADS: [&str; 4] = ["名詞", "動詞", "形容詞", "形状詞"];
/// Stands in for the conjugation type of a word outside Kanaemi's table once
/// the token carries Kanaemi's types.
const OUTSIDE_TABLE: &str = "(outside the table)";

/// Cuts one line's tokens into units.
pub(crate) fn cut_line(tokens: &[Token]) -> Vec<LineUnit> {
    let renamed: Vec<Token> = tokens.iter().map(with_kanaemi_type).collect();
    let tokens = join_affixes(&renamed);
    let mut units = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        if !needs_conversion(&token.surface) || is_numeral(token) {
            continue;
        }
        let Some(conjugation) = conjugation_of(token) else {
            units.push(plain(token));
            continue;
        };
        if conjugation == OUTSIDE_TABLE {
            let okuri = after_last_kanji(&token.surface);
            let (unit, taken) = take_in(plain(token), okuri, &tokens[i..]);
            i += taken;
            units.push(unit);
            continue;
        }
        let Some(ending) = terminal_ending(conjugation) else {
            units.push(LineUnit {
                unknown_conjugation: Some(conjugation.to_owned()),
                ..plain(token)
            });
            continue;
        };
        match stem_of(token, ending, conjugation) {
            Some((stem, okuri)) => {
                let unit = LineUnit {
                    stem: Some(stem),
                    ..plain(token)
                };
                let (unit, taken) = take_in(unit, okuri, &tokens[i..]);
                i += taken;
                units.push(unit);
            }
            None => units.push(plain(token)),
        }
    }
    units.retain(|u| {
        is_plain_reading(&u.reading) && u.stem.as_ref().is_none_or(|s| is_plain_reading(&s.reading))
    });
    units
}

fn plain(token: &Token) -> LineUnit {
    LineUnit {
        begin: token.begin,
        reading: token.reading.clone(),
        surface: token.surface.clone(),
        stem: None,
        unknown_conjugation: None,
    }
}

/// The stem from the dictionary form, and the okurigana's kana count, when
/// the surface and reading line up with it.
fn stem_of(token: &Token, ending: &str, conjugation: &str) -> Option<(Stem, usize)> {
    let stem = token.dictionary_form.strip_suffix(ending)?;
    let okuri = token.surface.strip_prefix(stem)?;
    if !has_kanji(stem) || !okuri.chars().all(is_reading_kana) {
        return None;
    }
    let reading = token.reading.strip_suffix(okuri)?;
    let stem = Stem {
        reading: reading.to_owned(),
        surface: stem.to_owned(),
        conjugation: conjugation.to_owned(),
    };
    Some((stem, okuri.chars().count()))
}

/// Takes in the auxiliaries and the conjunctive て/で that follow, while the
/// okurigana stays within what Kanaemi lets follow a stem. Returns the unit
/// and how many tokens it took.
fn take_in(mut unit: LineUnit, mut okuri: usize, rest: &[Token]) -> (LineUnit, usize) {
    let mut taken = 0;
    for next in rest {
        let attaches = next.pos[0] == "助動詞"
            || (next.pos[0] == "助詞"
                && next.pos[1] == "接続助詞"
                && matches!(next.surface.as_str(), "て" | "で"));
        let kana = next.surface.chars().count();
        if !attaches || !next.surface.chars().all(is_reading_kana) || okuri + kana > MAX_SUFFIX_KANA
        {
            break;
        }
        unit.surface.push_str(&next.surface);
        unit.reading.push_str(&next.reading);
        okuri += kana;
        taken += 1;
    }
    (unit, taken)
}

/// Joins a prefix to the content word after it and a suffix to the noun or
/// pronoun before it, never to a numeral.
fn join_affixes(tokens: &[Token]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut prefix: Option<Token> = None;
    for token in tokens {
        let mut token = token.clone();
        if let Some(p) = prefix.take() {
            if PREFIX_HEADS.contains(&token.pos[0].as_str()) && !is_numeral(&token) {
                token = Token {
                    surface: p.surface + &token.surface,
                    reading: p.reading + &token.reading,
                    dictionary_form: p.dictionary_form + &token.dictionary_form,
                    begin: p.begin,
                    pos: token.pos,
                };
            } else {
                out.push(p);
            }
        }
        if token.pos[0] == "接頭辞" {
            prefix = Some(token);
            continue;
        }
        if token.pos[0] == "接尾辞"
            && let Some(prev) = out.last_mut().filter(|prev| takes_suffix(prev))
        {
            prev.surface.push_str(&token.surface);
            prev.reading.push_str(&token.reading);
            prev.dictionary_form.push_str(&token.dictionary_form);
            prev.pos = token.pos;
            continue;
        }
        out.push(token);
    }
    out.extend(prefix);
    out
}

fn takes_suffix(token: &Token) -> bool {
    let noun_like = matches!(token.pos[0].as_str(), "名詞" | "代名詞")
        || (token.pos[0] == "接尾辞" && token.pos[1] == "名詞的");
    noun_like && !is_numeral(token)
}

fn conjugation_of(token: &Token) -> Option<&str> {
    token.pos.get(4).map(String::as_str).filter(|t| *t != "*")
}

/// The token with its conjugation type as Kanaemi names it, or
/// [`OUTSIDE_TABLE`], decided from its own dictionary form before any prefix
/// joins it.
fn with_kanaemi_type(token: &Token) -> Token {
    let mut token = token.clone();
    if let Some(sudachi_type) = conjugation_of(&token) {
        let renamed = match kanaemi_type(sudachi_type, &token.dictionary_form) {
            KanaemiType::Named(conjugation) => conjugation.to_owned(),
            KanaemiType::OutsideTable => OUTSIDE_TABLE.to_owned(),
        };
        token.pos[4] = renamed;
    }
    token
}

fn is_numeral(token: &Token) -> bool {
    token.pos.get(1).is_some_and(|p| p == "数詞")
}

/// Whether a word typed as its hiragana reading needs converting: it has kanji
/// or katakana.
fn needs_conversion(s: &str) -> bool {
    s.chars().any(|c| is_kanji(c) || is_katakana(c))
}

fn is_plain_reading(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_reading_kana)
}

/// Kana after the last kanji of `s`.
fn after_last_kanji(s: &str) -> usize {
    s.chars().rev().take_while(|c| !is_kanji(*c)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token from `surface/reading/pos/dictionary_form`, `pos` being the six
    /// comma-separated fields.
    fn tok(spec: &str, begin: usize) -> Token {
        let [surface, reading, pos, dictionary_form] = spec.split('/').collect::<Vec<_>>()[..]
        else {
            panic!("{spec}");
        };
        Token {
            surface: surface.into(),
            reading: reading.into(),
            pos: pos.split(',').map(str::to_owned).collect(),
            dictionary_form: dictionary_form.into(),
            begin,
        }
    }

    fn line(specs: &[&str]) -> Vec<Token> {
        let mut begin = 0;
        specs
            .iter()
            .map(|spec| {
                let t = tok(spec, begin);
                begin += t.surface.chars().count();
                t
            })
            .collect()
    }

    fn cut(specs: &[&str]) -> Vec<LineUnit> {
        cut_line(&line(specs))
    }

    fn plain(begin: usize, reading: &str, surface: &str) -> LineUnit {
        LineUnit {
            begin,
            reading: reading.into(),
            surface: surface.into(),
            stem: None,
            unknown_conjugation: None,
        }
    }

    fn conjugated(
        begin: usize,
        reading: &str,
        surface: &str,
        stem: (&str, &str, &str),
    ) -> LineUnit {
        LineUnit {
            stem: Some(Stem {
                reading: stem.0.into(),
                surface: stem.1.into(),
                conjugation: stem.2.into(),
            }),
            ..plain(begin, reading, surface)
        }
    }

    const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const PARTICLE: &str = "助詞,格助詞,*,*,*,*";

    /// One noun per character, each read as "よみ".
    fn fake_tokens(text: &str) -> Result<Vec<Token>, String> {
        Ok(text
            .chars()
            .enumerate()
            .map(|(begin, c)| Token {
                surface: c.to_string(),
                reading: "よみ".into(),
                pos: NOUN.split(',').map(str::to_owned).collect(),
                dictionary_form: c.to_string(),
                begin,
            })
            .collect())
    }

    fn cut_document<E>(
        text: &str,
        tokenize: impl FnMut(&str) -> Result<Vec<Token>, E>,
    ) -> Result<Vec<Unit>, E> {
        let mut units = Vec::new();
        each_unit("d", text, tokenize, |unit| units.push(unit))?;
        Ok(units)
    }

    #[test]
    fn words_with_kanji_or_katakana_are_units_and_hiragana_only_words_are_not() {
        assert_eq!(
            cut(&[
                &format!("手紙/てがみ/{NOUN}/手紙"),
                &format!("を/を/{PARTICLE}/を"),
                &format!("ぼく/ぼく/{NOUN}/ぼく"),
                &format!("ページ/ぺーじ/{NOUN}/ページ")
            ]),
            [plain(0, "てがみ", "手紙"), plain(5, "ぺーじ", "ページ")]
        );
    }

    #[test]
    fn a_conjugating_word_takes_in_auxiliaries_and_te_up_to_four_kana_after_the_stem() {
        assert_eq!(
            cut(&[
                "食べ/たべ/動詞,一般,*,*,下一段-バ行,未然形-一般/食べる",
                "られ/られ/助動詞,*,*,*,助動詞-レル,未然形-一般/られる",
                "ない/ない/助動詞,*,*,*,助動詞-ナイ,終止形-一般/ない",
            ]),
            [conjugated(
                0,
                "たべられない",
                "食べられない",
                ("たべ", "食べ", "下一段-バ行")
            )]
        );
        assert_eq!(
            cut(&[
                "書い/かい/動詞,一般,*,*,五段-カ行,連用形-イ音便/書く",
                "て/て/助詞,接続助詞,*,*,*,*/て",
                "い/い/動詞,非自立可能,*,*,上一段-ア行,連用形-一般/いる",
            ]),
            [conjugated(0, "かいて", "書いて", ("か", "書", "五段-カ行"))]
        );
    }

    #[test]
    fn taking_in_stops_before_going_past_four_kana() {
        assert_eq!(
            cut(&[
                "書か/かか/動詞,一般,*,*,五段-カ行,未然形-一般/書く",
                "せ/せ/助動詞,*,*,*,助動詞-セル,未然形-一般/せる",
                "られ/られ/助動詞,*,*,*,助動詞-レル,未然形-一般/られる",
                "ない/ない/助動詞,*,*,*,助動詞-ナイ,終止形-一般/ない",
            ]),
            [conjugated(
                0,
                "かかせられ",
                "書かせられ",
                ("か", "書", "五段-カ行")
            )]
        );
    }

    #[test]
    fn a_prefix_does_not_hide_a_word_outside_the_table() {
        assert_eq!(
            cut(&[
                "超/ちょう/接頭辞,*,*,*,*,*/超",
                "いい/いい/形容詞,非自立可能,*,*,形容詞,終止形-一般/いい",
                "です/です/助動詞,*,*,*,助動詞-デス,終止形-一般/です",
            ]),
            [plain(0, "ちょういいです", "超いいです")]
        );
    }

    #[test]
    fn long_lines_break_after_punctuation_near_the_limit() {
        let step = MAX_ANALYZER_BYTES / 4;
        let line = format!("{}。{}", "あ".repeat(step - 10), "い".repeat(step + 5));

        let starts: Vec<usize> = pieces(&line).iter().map(|(start, _)| *start).collect();

        assert_eq!(starts[..2], [0, step - 9]);
        assert!(
            pieces(&line)
                .iter()
                .all(|(_, p)| p.len() <= MAX_ANALYZER_BYTES)
        );
        assert_eq!(
            pieces(&line).iter().map(|(_, p)| *p).collect::<String>(),
            line
        );
    }

    #[test]
    fn a_prefix_does_not_hide_the_word_kanaemi_renames() {
        let units = cut(&[
            "お/お/接頭辞,*,*,*,*,*/お",
            "下さい/ください/動詞,非自立可能,*,*,五段-ラ行,連用形-イ音便/下さる",
        ]);

        assert_eq!(
            units,
            [conjugated(
                0,
                "おください",
                "お下さい",
                ("おくださ", "お下さ", "五段-ラ行-特殊")
            )]
        );
    }

    #[test]
    fn kanaemi_names_the_types_it_splits_off() {
        let units = cut(&[
            "行っ/いっ/動詞,非自立可能,*,*,五段-カ行,連用形-促音便/行く",
            "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            "、/、/補助記号,読点,*,*,*,*/、",
            "問う/とう/動詞,一般,*,*,五段-ワア行,連用形-ウ音便/問う",
            "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            "、/、/補助記号,読点,*,*,*,*/、",
            "下さい/ください/動詞,非自立可能,*,*,五段-ラ行,連用形-イ音便/下さる",
            "、/、/補助記号,読点,*,*,*,*/、",
            "請う/こう/動詞,一般,*,*,五段-ワア行,連用形-ウ音便/請う",
            "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            "、/、/補助記号,読点,*,*,*,*/、",
            "御坐い/ござい/動詞,非自立可能,*,*,五段-ラ行,連用形-イ音便/御坐る",
        ]);

        let types: Vec<_> = units
            .iter()
            .map(|u| u.stem.as_ref().unwrap().conjugation.as_str())
            .collect();
        assert_eq!(
            types,
            [
                "五段-カ行-促音便",
                "五段-ワア行-ウ音便",
                "五段-ラ行-特殊",
                "五段-ワア行-ウ音便",
                "五段-ラ行-特殊"
            ]
        );
    }

    #[test]
    fn words_outside_the_table_become_whole_plain_units() {
        assert_eq!(
            cut(&[
                "来/き/動詞,非自立可能,*,*,カ行変格,連用形-一般/来る",
                "まし/まし/助動詞,*,*,*,助動詞-マス,連用形-一般/ます",
                "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            ]),
            [plain(0, "きました", "来ました")]
        );
    }

    #[test]
    fn an_unknown_conjugation_type_is_kept_without_a_stem() {
        let units = cut(&["候/そうろう/動詞,一般,*,*,文語四段-ハ行,終止形-一般/候ふ"]);

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].stem, None);
        assert_eq!(
            units[0].unknown_conjugation.as_deref(),
            Some("文語四段-ハ行")
        );
    }

    #[test]
    fn a_surface_off_its_stem_becomes_a_plain_unit_alone() {
        assert_eq!(
            cut(&[
                "逝っ/いっ/動詞,非自立可能,*,*,五段-カ行,連用形-促音便/行く",
                "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            ]),
            [plain(0, "いっ", "逝っ")]
        );
    }

    #[test]
    fn prefixes_and_suffixes_join_their_words() {
        assert_eq!(
            cut(&[
                "お/お/接頭辞,*,*,*,*,*/お",
                "待ち/まち/動詞,一般,*,*,五段-タ行,連用形-一般/待つ",
                "、/、/補助記号,読点,*,*,*,*/、",
                "山田/やまだ/名詞,固有名詞,人名,姓,*,*/山田",
                "さん/さん/接尾辞,名詞的,一般,*,*,*/さん",
                "と/と/助詞,格助詞,*,*,*,*/と",
                "彼/かれ/代名詞,*,*,*,*,*/彼",
                "ら/ら/接尾辞,名詞的,一般,*,*,*/ら",
            ]),
            [
                conjugated(0, "おまち", "お待ち", ("おま", "お待", "五段-タ行")),
                plain(4, "やまださん", "山田さん"),
                plain(9, "かれら", "彼ら"),
            ]
        );
    }

    #[test]
    fn numerals_are_not_units_and_affixes_do_not_join_them() {
        assert_eq!(
            cut(&[
                "第/だい/接頭辞,*,*,*,*,*/第",
                "3/さん/名詞,数詞,*,*,*,*/3",
                "条/じょう/名詞,普通名詞,助数詞可能,*,*,*/条",
                "の/の/助詞,格助詞,*,*,*,*/の",
                "千/せん/名詞,数詞,*,*,*,*/千",
                "円/えん/名詞,普通名詞,助数詞可能,*,*,*/円",
            ]),
            [
                plain(0, "だい", "第"),
                plain(2, "じょう", "条"),
                plain(5, "えん", "円")
            ]
        );
    }

    #[test]
    fn a_document_s_units_carry_their_position_in_the_whole_text() {
        let units = cut_document("あ漢い\nう字", fake_tokens).unwrap();

        let view: Vec<_> = units
            .iter()
            .map(|u| (u.position, u.surface.as_str(), u.doc_id.as_str()))
            .collect();
        assert_eq!(view, [(1, "漢", "d"), (5, "字", "d")]);
    }

    #[test]
    fn a_word_split_across_pieces_still_takes_in_what_follows() {
        let step = MAX_ANALYZER_BYTES / 4;
        let text = format!("{}食べ{}", "あ".repeat(step - 2), "あ".repeat(step));
        // The first piece ends with 食べ; the second starts with られない.
        let tokenize = |piece: &str| -> Result<Vec<Token>, String> {
            if let Some(head) = piece.strip_suffix("食べ") {
                let mut tokens = fake_tokens(head)?;
                tokens.push(tok(
                    "食べ/たべ/動詞,一般,*,*,下一段-バ行,未然形-一般/食べる",
                    step - 2,
                ));
                Ok(tokens)
            } else {
                Ok(vec![
                    tok("られ/られ/助動詞,*,*,*,助動詞-レル,未然形-一般/られる", 0),
                    tok("ない/ない/助動詞,*,*,*,助動詞-ナイ,終止形-一般/ない", 2),
                ])
            }
        };

        let units = cut_document(&text, tokenize).unwrap();

        let words: Vec<_> = units.iter().map(|u| u.surface.as_str()).collect();
        assert!(words.contains(&"食べられない"), "{words:?}");
    }

    #[test]
    fn a_line_too_long_for_the_analyzer_is_read_in_pieces() {
        let mut lengths = Vec::new();
        let long = "あ".repeat(MAX_ANALYZER_BYTES / 3 + 1);
        let text = format!("{long}漢");

        let units = cut_document(&text, |piece: &str| {
            lengths.push(piece.len());
            fake_tokens(piece)
        })
        .unwrap();

        assert!(
            lengths.iter().all(|&n| n <= MAX_ANALYZER_BYTES),
            "{lengths:?}"
        );
        assert_eq!(
            units.iter().map(|u| u.position).collect::<Vec<_>>(),
            [MAX_ANALYZER_BYTES / 3 + 1]
        );
    }

    #[test]
    fn units_whose_reading_is_not_plain_hiragana_are_dropped() {
        assert_eq!(
            cut(&[
                &format!("ＡＩ技術/えーあいぎじゅつ/{NOUN}/ＡＩ技術"),
                &format!("Ｘ線/Ｘせん/{NOUN}/Ｘ線")
            ]),
            [plain(0, "えーあいぎじゅつ", "ＡＩ技術")]
        );
    }

    #[test]
    fn an_analyzer_error_stops_the_document() {
        let result = cut_document("漢\n字", |_| Err::<Vec<Token>, _>("broken"));

        assert_eq!(result, Err("broken"));
    }
}

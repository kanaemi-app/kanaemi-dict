//! Conversion units: the pieces of text Kanaemi converts in one go, cut from
//! Sudachi's morphemes.

use std::io::{self, BufRead};

use kanaemi_engine::{MAX_SUFFIX_KANA, terminal_ending};

use crate::analyzer::is_break;
use crate::conjugation::{KanaemiType, kanaemi_type};
use crate::kana::{has_kanji, is_kanji, is_katakana, is_reading_kana, is_voicing_of};
use crate::numeral::{Number, read_number};
use crate::{Token, UnidicReadings, Words, is_word, katakana_to_hiragana};

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
    pub numeric: Option<Numeric>,
    /// A compound the analyzer groups from the units at its place, which it
    /// overlaps: it counts as a word, but is no unit typed in the evaluation
    /// or the model's training.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compound: bool,
}

/// A unit that holds a number, in the form of Kanaemi's numeric items: the
/// number's placeholder in reading and surface, and the number typed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Numeric {
    /// The reading with `{}` for the number: `だい{}かい`.
    pub reading: String,
    /// The surface with the number's notation placeholder: `第{kanji}回`.
    pub surface: String,
    /// The number as the ASCII digits typed for it: `3` for 三.
    pub value: String,
}

/// A line of `build/units.jsonl` that could not be read as a unit.
#[derive(Debug, thiserror::Error)]
#[error("line {line}: {source}")]
pub struct UnitsError {
    pub line: usize,
    pub source: io::Error,
}

/// The units of `reader`, one JSON object per line.
pub fn read_units(reader: impl BufRead) -> impl Iterator<Item = Result<Unit, UnitsError>> {
    reader.lines().enumerate().map(|(i, line)| {
        let failed = |source| UnitsError {
            line: i + 1,
            source,
        };
        let line = line.map_err(failed)?;
        serde_json::from_str(&line).map_err(|e| failed(e.into()))
    })
}

/// One unit within a line. `begin` counts characters from the line's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineUnit {
    pub(crate) begin: usize,
    pub(crate) reading: String,
    pub(crate) surface: String,
    pub(crate) stem: Option<Stem>,
    pub(crate) unknown_conjugation: Option<String>,
    pub(crate) numeric: Option<Numeric>,
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
pub(crate) fn each_unit<W: Into<Words>, E>(
    doc_id: &str,
    text: &str,
    mut tokenize: impl FnMut(&str) -> Result<W, E>,
    readings: &UnidicReadings,
    mut emit: impl FnMut(Unit),
) -> Result<(), E> {
    let mut line_start = 0;
    for line in text.split('\n') {
        let mut words = Words::default();
        for (piece_start, piece) in pieces(line) {
            let shift = |mut token: Token| {
                token.begin += piece_start;
                token
            };
            let piece_words: Words = tokenize(piece)?.into();
            words
                .tokens
                .extend(piece_words.tokens.into_iter().map(shift));
            words
                .compounds
                .extend(piece_words.compounds.into_iter().map(shift));
        }
        let units = cut_line(&words.tokens, readings);
        let compounds = compound_units(&words, &units);
        let mut line_units: Vec<(LineUnit, bool)> = units
            .into_iter()
            .map(|u| (u, false))
            .chain(compounds.into_iter().map(|u| (u, true)))
            .collect();
        line_units.sort_by_key(|(u, _)| u.begin);
        for (u, compound) in line_units {
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
                numeric: u.numeric,
                compound,
            });
        }
        line_start += line.chars().count() + 1;
    }
    Ok(())
}

/// The compounds of `words` that make words: nouns with kanji or katakana
/// and no numeral among their tokens, read in plain kana, other than a unit
/// already cut at the same place.
fn compound_units(words: &Words, units: &[LineUnit]) -> Vec<LineUnit> {
    words
        .compounds
        .iter()
        .filter(|c| {
            let end = c.begin + c.surface.chars().count();
            c.pos[0] == "名詞"
                && needs_conversion(&c.surface)
                && is_plain_reading(&c.reading)
                && is_word(&c.reading, &c.surface)
                && !words
                    .tokens
                    .iter()
                    .any(|t| t.begin >= c.begin && t.begin < end && is_numeral(t))
                && !units
                    .iter()
                    .any(|u| u.begin == c.begin && u.surface == c.surface)
        })
        .map(plain)
        .collect()
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

/// Parts of speech a prefix joins. A prefix joined to a verb or an adjective
/// makes a word that does not exist (お願い into お願う).
const PREFIX_HEADS: [&str; 2] = ["名詞", "形状詞"];
/// Stands in for the conjugation type of a word outside Kanaemi's table once
/// the token carries Kanaemi's types.
const OUTSIDE_TABLE: &str = "(outside the table)";

/// Cuts one line's tokens into units.
pub(crate) fn cut_line(tokens: &[Token], readings: &UnidicReadings) -> Vec<LineUnit> {
    let renamed: Vec<Token> = tokens
        .iter()
        .map(|t| with_kanaemi_type(&read_as_written(t)))
        .collect();
    let mut units = Vec::new();
    let mut rest = renamed.as_slice();
    while let Some(n) = rest.iter().position(is_numeral) {
        let start = n - usize::from(n > 0 && rest[n - 1].pos[0] == "接頭辞");
        units.extend(cut_words(&rest[..start], readings));
        let (unit, taken) = cut_numeric(&rest[start..]);
        units.extend(unit);
        rest = &rest[start + taken..];
    }
    units.extend(cut_words(rest, readings));
    units
}

/// Cuts tokens holding no numeral into units.
fn cut_words(tokens: &[Token], readings: &UnidicReadings) -> Vec<LineUnit> {
    let tokens = join_affixes(tokens, readings);
    let mut units = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        if !needs_conversion(&token.surface) || matches!(token.pos[0].as_str(), "補助記号" | "空白")
        {
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
            // Cut short at a small つ (ハマっ), the form is no word.
            None if token.reading.ends_with('っ') => {}
            None => units.push(plain(token)),
        }
    }
    units.retain(|u| {
        is_plain_reading(&u.reading)
            && u.stem.as_ref().is_none_or(|s| is_plain_reading(&s.reading))
            && is_word(&u.reading, &u.surface)
    });
    units
}

/// The token read as its katakana is written (ランウェイ as らんうぇい), when it
/// is written in katakana alone: the analyzer gives a variant spelling the
/// reading of the usual one (らんうえい). ヵ and ヶ stand for kanji (3ヶ月),
/// read か or が, and ヷ, ヸ, ヹ and ヺ have no hiragana, so a surface with
/// any of them keeps the analyzer's reading.
fn read_as_written(token: &Token) -> Token {
    let mut token = token.clone();
    let katakana_only = !token.surface.is_empty()
        && token.surface.chars().all(|c| {
            (is_katakana(c) && !matches!(c, 'ヵ' | 'ヶ' | 'ヷ' | 'ヸ' | 'ヹ' | 'ヺ')) || c == 'ー'
        });
    if katakana_only {
        token.reading = katakana_to_hiragana(&token.surface)
            .chars()
            .map(|c| match c {
                'ゐ' => 'い',
                'ゑ' => 'え',
                c => c,
            })
            .collect();
    }
    token
}

/// Cuts the numeral at the head of `tokens`, with the prefix before it, into
/// a numeric unit: the prefix, the number, the counter after it and the
/// suffixes after the counter. Returns the unit, if they make one, and how
/// many tokens it took, which it takes either way.
fn cut_numeric(tokens: &[Token]) -> (Option<LineUnit>, usize) {
    let number_start = usize::from(tokens[0].pos[0] == "接頭辞");
    let after = number_start
        + tokens[number_start..]
            .iter()
            .take_while(|t| is_numeral(t))
            .count();
    if !tokens.get(after).is_some_and(is_counter) {
        return (None, after);
    }
    let taken = after
        + 1
        + tokens[after + 1..]
            .iter()
            .take_while(|t| t.pos[0] == "接尾辞")
            .count();
    let unit = numeric_unit(
        &tokens[..number_start],
        &tokens[number_start..after],
        &tokens[after..taken],
    );
    (unit, taken)
}

fn numeric_unit(before: &[Token], number: &[Token], after: &[Token]) -> Option<LineUnit> {
    let surface = |ts: &[Token]| ts.iter().map(|t| t.surface.as_str()).collect::<String>();
    let kana = |ts: &[Token]| {
        let reading: String = ts.iter().map(|t| t.reading.as_str()).collect();
        reading.chars().all(is_reading_kana).then_some(reading)
    };
    let Number { notation, value } = read_number(surface(number))?;
    let (kana_before, kana_after) = (kana(before)?, kana(after)?);
    let (surface_before, surface_after) = (surface(before), surface(after));
    let numeric = Numeric {
        reading: format!("{kana_before}{{}}{kana_after}"),
        surface: format!("{surface_before}{}{surface_after}", notation.placeholder()),
        value,
    };
    (numeric.reading != numeric.surface).then(|| LineUnit {
        begin: before.first().unwrap_or(&number[0]).begin,
        reading: format!("{kana_before}{}{kana_after}", numeric.value),
        surface: format!("{surface_before}{}{surface_after}", surface(number)),
        stem: None,
        unknown_conjugation: None,
        numeric: Some(numeric),
    })
}

/// A suffix, or a noun that can count, after a number.
fn is_counter(token: &Token) -> bool {
    token.pos[0] == "接尾辞" || (token.pos[0] == "名詞" && token.pos[2] == "助数詞可能")
}

fn plain(token: &Token) -> LineUnit {
    LineUnit {
        begin: token.begin,
        reading: token.reading.clone(),
        surface: token.surface.clone(),
        stem: None,
        unknown_conjugation: None,
        numeric: None,
    }
}

/// The stem from the dictionary form, and the okurigana's kana count, when
/// the surface and reading line up with it. A godan form written without its
/// okurigana (有 for あり) and an adjective's ウ音便 (早う) read the stem
/// otherwise than the dictionary form does, so they make none.
fn stem_of(token: &Token, ending: &str, conjugation: &str) -> Option<(Stem, usize)> {
    let stem = token.dictionary_form.strip_suffix(ending)?;
    let okuri = token.surface.strip_prefix(stem)?;
    if !has_kanji(stem) || !okuri.chars().all(is_reading_kana) {
        return None;
    }
    if (okuri.is_empty() && conjugation.starts_with("五段"))
        || (conjugation == "形容詞" && token.pos.get(5).is_some_and(|f| f.ends_with("ウ音便")))
    {
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
/// pronoun before it, unless UniDic has the joined word and no reading of it
/// can stand for the joined readings.
fn join_affixes(tokens: &[Token], readings: &UnidicReadings) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut prefix: Option<Token> = None;
    for token in tokens {
        let mut token = token.clone();
        if let Some(p) = prefix.take() {
            match PREFIX_HEADS
                .contains(&token.pos[0].as_str())
                .then(|| joined(&p, &token, readings))
                .flatten()
            {
                Some(j) => token = j,
                None => out.push(p),
            }
        }
        if token.pos[0] == "接頭辞" {
            prefix = Some(token);
            continue;
        }
        if token.pos[0] == "接尾辞"
            && let Some(prev) = out.last_mut().filter(|prev| takes_suffix(prev))
            && let Some(j) = joined(prev, &token, readings)
        {
            *prev = j;
            continue;
        }
        out.push(token);
    }
    out.extend(prefix);
    out
}

/// `head` and `tail` as one word with the part of speech of `tail`, read as
/// UniDic reads it when UniDic has it.
fn joined(head: &Token, tail: &Token, readings: &UnidicReadings) -> Option<Token> {
    let surface = format!("{}{}", head.surface, tail.surface);
    let reading = format!("{}{}", head.reading, tail.reading);
    let reading = checked_reading(readings.of(&surface), reading, head.reading.chars().count())?;
    Some(Token {
        surface,
        reading,
        dictionary_form: format!("{}{}", head.dictionary_form, tail.dictionary_form),
        begin: head.begin,
        pos: tail.pos.clone(),
    })
}

/// `reading` when UniDic does not know the word or reads it so; else UniDic's
/// reading that differs only by voicing the kana at `seam`, or UniDic's only
/// reading. None when neither is there.
fn checked_reading(known: &[String], reading: String, seam: usize) -> Option<String> {
    if known.is_empty() || known.contains(&reading) {
        return Some(reading);
    }
    let voiced_at_seam = |other: &String| {
        other.chars().count() == reading.chars().count()
            && reading
                .chars()
                .zip(other.chars())
                .enumerate()
                .all(|(i, (a, b))| a == b || (i == seam && is_voicing_of(a, b)))
    };
    known
        .iter()
        .find(|r| voiced_at_seam(r))
        .or(match known {
            [only] => Some(only),
            _ => None,
        })
        .cloned()
}

fn takes_suffix(token: &Token) -> bool {
    matches!(token.pos[0].as_str(), "名詞" | "代名詞")
        || (token.pos[0] == "接尾辞" && token.pos[1] == "名詞的")
}

fn conjugation_of(token: &Token) -> Option<&str> {
    token.pos.get(4).map(String::as_str).filter(|t| *t != "*")
}

/// The token with its conjugation type as Kanaemi names it, or
/// [`OUTSIDE_TABLE`], decided from its dictionary form.
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
    use crate::UnidicWord;

    #[test]
    fn units_are_read_one_per_line_and_a_bad_line_is_an_error_with_its_number() {
        let unit = Unit {
            compound: false,
            doc_id: "a:1".into(),
            position: 3,
            reading: "ã¦ãã¿".into(),
            surface: "æç´".into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: Some(Numeric {
                reading: "{}ぽん".into(),
                surface: "{kanji}本".into(),
                value: "3".into(),
            }),
        };
        let text = format!(
            "{}\n{{\"doc_id\":\"a:1\"}}\n",
            serde_json::to_string(&unit).unwrap()
        );

        let read: Vec<_> = read_units(text.as_bytes()).collect();

        assert_eq!(read.len(), 2);
        assert_eq!(read[0].as_ref().unwrap(), &unit);
        assert!(
            matches!(read[1], Err(UnitsError { line: 2, .. })),
            "{read:?}"
        );
    }

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
        cut_line(&line(specs), &UnidicReadings::default())
    }

    /// Cuts with UniDic giving `known` as (reading, surface) pairs.
    fn cut_knowing(known: &[(&str, &str)], specs: &[&str]) -> Vec<LineUnit> {
        let readings = UnidicReadings::new(known.iter().map(|(reading, surface)| UnidicWord {
            reading: (*reading).into(),
            surface: (*surface).into(),
        }));
        cut_line(&line(specs), &readings)
    }

    fn plain(begin: usize, reading: &str, surface: &str) -> LineUnit {
        LineUnit {
            begin,
            reading: reading.into(),
            surface: surface.into(),
            stem: None,
            unknown_conjugation: None,
            numeric: None,
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

    fn cut_document<W: Into<Words>, E>(
        text: &str,
        tokenize: impl FnMut(&str) -> Result<W, E>,
    ) -> Result<Vec<Unit>, E> {
        let mut units = Vec::new();
        each_unit("d", text, tokenize, &UnidicReadings::default(), |unit| {
            units.push(unit)
        })?;
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
    fn a_prefix_does_not_join_a_verb_or_an_adjective() {
        assert_eq!(
            cut(&[
                "突/とっ/接頭辞,*,*,*,*,*/突",
                "当る/あたる/動詞,一般,*,*,五段-ラ行,終止形-一般/当る",
                "、/、/補助記号,読点,*,*,*,*/、",
                "超/ちょう/接頭辞,*,*,*,*,*/超",
                "安い/やすい/形容詞,一般,*,*,形容詞,終止形-一般/安い",
            ]),
            [
                plain(0, "とっ", "突"),
                conjugated(1, "あたる", "当る", ("あた", "当", "五段-ラ行")),
                plain(4, "ちょう", "超"),
                conjugated(5, "やすい", "安い", ("やす", "安", "形容詞")),
            ]
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
                "逝き/いき/動詞,非自立可能,*,*,五段-カ行,連用形-一般/行く",
                "ます/ます/助動詞,*,*,*,助動詞-マス,終止形-一般/ます",
            ]),
            [plain(0, "いき", "逝き")]
        );
    }

    #[test]
    fn a_form_written_without_its_okurigana_makes_no_stem() {
        assert_eq!(
            cut(&[
                "有/あり/動詞,非自立可能,*,*,五段-ラ行,連用形-一般/有る",
                "、/、/補助記号,読点,*,*,*,*/、",
                "見/み/動詞,非自立可能,*,*,上一段-マ行,連用形-一般/見る",
                "た/た/助動詞,*,*,*,助動詞-タ,終止形-一般/た",
            ]),
            [
                plain(0, "あり", "有"),
                conjugated(2, "みた", "見た", ("み", "見", "上一段-マ行")),
            ]
        );
    }

    #[test]
    fn an_adjective_s_u_sound_change_makes_no_stem() {
        assert_eq!(
            cut(&["早う/はよう/形容詞,一般,*,*,形容詞,連用形-ウ音便/早い"]),
            [plain(0, "はよう", "早う")]
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
                conjugated(1, "まち", "待ち", ("ま", "待", "五段-タ行")),
                plain(4, "やまださん", "山田さん"),
                plain(9, "かれら", "彼ら"),
            ]
        );
    }

    #[test]
    fn a_katakana_word_is_read_as_it_is_written() {
        assert_eq!(
            cut(&[
                &format!("ランウェイ/らんうえい/{NOUN}/ランウェイ"),
                &format!("キターーー/きたー/{NOUN}/キターーー"),
                &format!("ヱビス/えびす/{NOUN}/ヱビス"),
            ]),
            [
                plain(0, "らんうぇい", "ランウェイ"),
                plain(5, "きたーーー", "キターーー"),
                plain(10, "えびす", "ヱビス"),
            ]
        );
    }

    #[test]
    fn a_katakana_word_with_a_letter_hiragana_lacks_keeps_the_analyzer_s_reading() {
        assert_eq!(
            cut(&[&format!("ヷイオリン/ゔぁいおりん/{NOUN}/ヷイオリン")]),
            [plain(0, "ゔぁいおりん", "ヷイオリン")]
        );
    }

    #[test]
    fn a_small_ke_counter_keeps_the_analyzer_s_reading() {
        assert_eq!(
            cut(&[
                &format!("3/さん/{NUMERAL}/3"),
                &format!("ヶ/か/{COUNTER_SUFFIX}/ヶ"),
            ]),
            [numeric(0, "3か", "3ヶ", ("{}か", "{}ヶ", "3"))]
        );
    }

    #[test]
    fn a_piece_read_from_a_small_kana_a_moraic_nasal_or_a_long_vowel_is_not_a_unit() {
        assert_eq!(
            cut(&[
                &format!("ッス/っす/{NOUN}/ッス"),
                &format!("ンな/んな/{NOUN}/ンな"),
                &format!("放/っぱなし/{NOUN}/放"),
                &format!("ァ/ぁ/{NOUN}/ァ"),
                &format!("手紙/てがみ/{NOUN}/手紙"),
            ]),
            [plain(6, "てがみ", "手紙")]
        );
    }

    #[test]
    fn a_conjugating_word_without_a_stem_cut_short_at_a_small_tsu_is_not_a_unit() {
        assert_eq!(
            cut(&[
                "ハマっ/はまっ/動詞,一般,*,*,五段-ラ行,連用形-促音便/ハマる",
                "、/、/補助記号,読点,*,*,*,*/、",
                &format!("パッ/ぱっ/{NOUN}/パッ"),
            ]),
            [plain(4, "ぱっ", "パッ")]
        );
    }

    #[test]
    fn a_symbol_a_spaced_surface_and_a_loose_voicing_mark_are_not_units() {
        assert_eq!(
            cut(&[
                "☆彡/きごう/補助記号,一般,*,*,*,*/☆彡",
                &format!("美 少年/びしょうねん/{NOUN}/美 少年"),
                &format!("ヘ゛ヒ゛ー/べびー/{NOUN}/ヘ゛ヒ゛ー"),
                &format!("手紙/てがみ/{NOUN}/手紙"),
            ]),
            [plain(11, "てがみ", "手紙")]
        );
    }

    #[test]
    fn a_joined_word_unidic_has_takes_unidic_s_reading_voiced_where_the_words_join() {
        assert_eq!(
            cut_knowing(
                &[("おおずもう", "大相撲"), ("たいすもう", "大相撲")],
                &[
                    "大/おお/接頭辞,*,*,*,*,*/大",
                    &format!("相撲/すもう/{NOUN}/相撲"),
                ]
            ),
            [plain(0, "おおずもう", "大相撲")]
        );
    }

    #[test]
    fn a_joined_word_unidic_reads_one_way_takes_that_reading() {
        assert_eq!(
            cut_knowing(
                &[("りゅうじょう", "粒状")],
                &[
                    &format!("粒/つぶ/{NOUN}/粒"),
                    "状/じょう/接尾辞,名詞的,一般,*,*,*/状",
                ]
            ),
            [plain(0, "りゅうじょう", "粒状")]
        );
    }

    #[test]
    fn a_joined_word_whose_reading_unidic_has_keeps_it() {
        assert_eq!(
            cut_knowing(
                &[("おおずもう", "大相撲")],
                &[
                    "大/おお/接頭辞,*,*,*,*,*/大",
                    &format!("相撲/ずもう/{NOUN}/相撲"),
                ]
            ),
            [plain(0, "おおずもう", "大相撲")]
        );
    }

    #[test]
    fn a_joined_word_whose_unidic_reading_cannot_be_chosen_stays_apart() {
        assert_eq!(
            cut_knowing(
                &[("さんじょう", "山上"), ("やまがみ", "山上")],
                &[
                    &format!("山/やま/{NOUN}/山"),
                    "上/じょう/接尾辞,名詞的,一般,*,*,*/上",
                ]
            ),
            [plain(0, "やま", "山"), plain(1, "じょう", "上")]
        );
    }

    fn numeric(begin: usize, typed: &str, surface: &str, item: (&str, &str, &str)) -> LineUnit {
        LineUnit {
            numeric: Some(Numeric {
                reading: item.0.into(),
                surface: item.1.into(),
                value: item.2.into(),
            }),
            ..plain(begin, typed, surface)
        }
    }

    const NUMERAL: &str = "名詞,数詞,*,*,*,*";
    const COUNTER_SUFFIX: &str = "接尾辞,名詞的,助数詞,*,*,*";
    const COUNTER_NOUN: &str = "名詞,普通名詞,助数詞可能,*,*,*";
    const PREFIX: &str = "接頭辞,*,*,*,*,*";

    #[test]
    fn a_number_and_the_counter_after_it_are_a_numeric_unit() {
        assert_eq!(
            cut(&[
                &format!("3/さん/{NUMERAL}/3"),
                &format!("本/ぽん/{COUNTER_SUFFIX}/本"),
            ]),
            [numeric(0, "3ぽん", "3本", ("{}ぽん", "{}本", "3"))]
        );
    }

    #[test]
    fn a_noun_that_takes_a_number_counts_it() {
        assert_eq!(
            cut(&[
                &format!("2026/にせんにじゅうろく/{NUMERAL}/2026"),
                &format!("年/ねん/{COUNTER_NOUN}/年"),
                &format!("10/じゅう/{NUMERAL}/10"),
                &format!("月/がつ/{COUNTER_NOUN}/月"),
                &format!("5/ご/{NUMERAL}/5"),
                &format!("日/か/{COUNTER_SUFFIX}/日"),
            ]),
            [
                numeric(0, "2026ねん", "2026年", ("{}ねん", "{}年", "2026")),
                numeric(5, "10がつ", "10月", ("{}がつ", "{}月", "10")),
                numeric(8, "5か", "5日", ("{}か", "{}日", "5")),
            ]
        );
    }

    #[test]
    fn the_prefix_before_a_number_and_the_suffixes_after_its_counter_join_it() {
        assert_eq!(
            cut(&[
                &format!("第/だい/{PREFIX}/第"),
                &format!("3/さん/{NUMERAL}/3"),
                &format!("回/かい/{COUNTER_NOUN}/回"),
                &format!("の/の/{PARTICLE}/の"),
                &format!("3/さん/{NUMERAL}/3"),
                "冊/さつ/接尾辞,名詞的,一般,*,*,*/冊",
                "目/め/接尾辞,名詞的,一般,*,*,*/目",
            ]),
            [
                numeric(0, "だい3かい", "第3回", ("だい{}かい", "第{}回", "3")),
                numeric(4, "3さつめ", "3冊目", ("{}さつめ", "{}冊目", "3")),
            ]
        );
    }

    #[test]
    fn the_numeric_surface_writes_the_number_as_the_text_does() {
        let units = cut(&[
            &format!("三/さん/{NUMERAL}/三"),
            &format!("本/ぽん/{COUNTER_SUFFIX}/本"),
            &format!("３/さん/{NUMERAL}/３"),
            &format!("個/こ/{COUNTER_SUFFIX}/個"),
            &format!("1,000/せん/{NUMERAL}/1000"),
            &format!("円/えん/{COUNTER_NOUN}/円"),
            &format!("二〇二六/にぜろにろく/{NUMERAL}/二〇二六"),
            &format!("年/ねん/{COUNTER_NOUN}/年"),
            &format!("壱千/いっせん/{NUMERAL}/壱千"),
            &format!("円/えん/{COUNTER_NOUN}/円"),
        ]);

        let items: Vec<_> = units
            .iter()
            .map(|u| {
                let n = u.numeric.as_ref().unwrap();
                (u.reading.as_str(), n.surface.as_str(), n.value.as_str())
            })
            .collect();
        assert_eq!(
            items,
            [
                ("3ぽん", "{kanji}本", "3"),
                ("3こ", "{wide-num}個", "3"),
                ("1000えん", "{grouped-num}円", "1000"),
                ("2026ねん", "{kanji-num}年", "2026"),
                ("1000えん", "{daiji}円", "1000"),
            ]
        );
    }

    #[test]
    fn numerals_the_analyzer_splits_are_one_number() {
        assert_eq!(
            cut(&[
                &format!("二十/にじゅう/{NUMERAL}/二十"),
                &format!("六/ろく/{NUMERAL}/六"),
                &format!("歳/さい/{COUNTER_SUFFIX}/歳"),
            ]),
            [numeric(
                0,
                "26さい",
                "二十六歳",
                ("{}さい", "{kanji}歳", "26")
            )]
        );
    }

    #[test]
    fn a_number_without_a_counter_makes_no_unit_and_takes_its_prefix_along() {
        assert_eq!(
            cut(&[
                &format!("第/だい/{PREFIX}/第"),
                &format!("3/さん/{NUMERAL}/3"),
                &format!("の/の/{PARTICLE}/の"),
                &format!("手紙/てがみ/{NOUN}/手紙"),
            ]),
            [plain(3, "てがみ", "手紙")]
        );
    }

    #[test]
    fn a_number_no_notation_writes_takes_its_neighbours_out_of_the_units() {
        assert_eq!(
            cut(&[
                &format!("約/やく/{PREFIX}/約"),
                &format!("3/さん/{NUMERAL}/3"),
                &format!("万/まん/{NUMERAL}/万"),
                &format!("円/えん/{COUNTER_NOUN}/円"),
                &format!("1.5/いってんご/{NUMERAL}/1.5"),
                &format!("倍/ばい/{COUNTER_SUFFIX}/倍"),
            ]),
            []
        );
    }

    #[test]
    fn a_counter_read_other_than_in_hiragana_makes_no_unit() {
        assert_eq!(
            cut(&[
                &format!("3/さん/{NUMERAL}/3"),
                &format!("Ｘ/Ｘ/{COUNTER_SUFFIX}/Ｘ"),
            ]),
            []
        );
    }

    #[test]
    fn a_numeric_unit_typed_as_its_own_surface_is_not_a_unit() {
        assert_eq!(
            cut(&[
                &format!("3/さん/{NUMERAL}/3"),
                &format!("つ/つ/{COUNTER_SUFFIX}/つ"),
                &format!("三/さん/{NUMERAL}/三"),
                &format!("つ/つ/{COUNTER_SUFFIX}/つ"),
            ]),
            [numeric(2, "3つ", "三つ", ("{}つ", "{kanji}つ", "3"))]
        );
    }

    #[test]
    fn a_document_s_numeric_units_carry_their_numbers() {
        let tokenize = |_: &str| -> Result<Vec<Token>, String> {
            Ok(line(&[
                &format!("3/さん/{NUMERAL}/3"),
                &format!("本/ぽん/{COUNTER_SUFFIX}/本"),
            ]))
        };

        let units = cut_document("3本", tokenize).unwrap();

        assert_eq!(
            units[0].numeric,
            Some(Numeric {
                reading: "{}ぽん".into(),
                surface: "{}本".into(),
                value: "3".into(),
            })
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

    /// The units of `text` read as `specs`, with the compounds `compounds`
    /// as `(surface, reading, pos)` at the place their surface first shows.
    fn cut_with_compounds(
        text: &str,
        specs: &[&str],
        compounds: &[(&str, &str, &str)],
    ) -> Vec<(usize, String, String, bool)> {
        let words = Words {
            tokens: line(specs),
            compounds: compounds
                .iter()
                .map(|(surface, reading, pos)| {
                    let at = text.find(surface).unwrap();
                    tok(
                        &format!("{surface}/{reading}/{pos}/{surface}"),
                        text[..at].chars().count(),
                    )
                })
                .collect(),
        };
        let units = cut_document(text, |_| Ok::<_, ()>(words.clone())).unwrap();
        units
            .into_iter()
            .map(|u| (u.position, u.reading, u.surface, u.compound))
            .collect()
    }

    #[test]
    fn a_noun_compound_is_a_unit_of_its_own_beside_its_parts_in_position_order() {
        let units = cut_with_compounds(
            "露天風呂に入る",
            &[
                &format!("露天/ろてん/{NOUN}/露天"),
                &format!("風呂/ふろ/{NOUN}/風呂"),
                &format!("に/に/{PARTICLE}/に"),
                "入る/はいる/動詞,一般,*,*,五段-ラ行,終止形-一般/入る",
            ],
            &[("露天風呂", "ろてんぶろ", NOUN)],
        );

        assert_eq!(
            units,
            [
                (0, "ろてん".into(), "露天".into(), false),
                (0, "ろてんぶろ".into(), "露天風呂".into(), true),
                (2, "ふろ".into(), "風呂".into(), false),
                (5, "はいる".into(), "入る".into(), false),
            ]
        );
    }

    #[test]
    fn a_compound_that_is_no_noun_holds_a_numeral_or_is_a_unit_already_is_left_out() {
        let numeral = "名詞,数詞,*,*,*,*";
        let suffix = "接尾辞,名詞的,一般,*,*,*";
        let units = cut_with_compounds(
            "取り扱う第三章山田さん",
            &[
                "取り/とり/動詞,一般,*,*,五段-ラ行,連用形-一般/取る",
                "扱う/あつかう/動詞,一般,*,*,五段-ワア行,終止形-一般/扱う",
                "第/だい/接頭辞,*,*,*,*,*/第",
                &format!("三/さん/{numeral}/三"),
                &format!("章/しょう/{NOUN}/章"),
                "山田/やまだ/名詞,固有名詞,人名,姓,*,*/山田",
                &format!("さん/さん/{suffix}/さん"),
            ],
            &[
                (
                    "取り扱う",
                    "とりあつかう",
                    "動詞,一般,*,*,五段-ワア行,終止形-一般",
                ),
                ("第三章", "だいさんしょう", NOUN),
                ("山田さん", "やまださん", NOUN),
            ],
        );

        assert!(units.iter().all(|u| !u.3), "{units:?}");
    }

    #[test]
    fn a_compound_unit_keeps_its_mark_through_a_units_line_and_others_write_none() {
        let mut unit = Unit {
            compound: true,
            doc_id: "a:1".into(),
            position: 0,
            reading: "ろてんぶろ".into(),
            surface: "露天風呂".into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        };

        let line = serde_json::to_string(&unit).unwrap();
        assert_eq!(read_units(line.as_bytes()).next().unwrap().unwrap(), unit);

        unit.compound = false;
        assert!(!serde_json::to_string(&unit).unwrap().contains("compound"));
    }
}

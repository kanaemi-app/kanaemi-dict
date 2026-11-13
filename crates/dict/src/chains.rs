//! Words the analyzer splits and no compound groups (多種／多様): chains of
//! units whose surface Wikipedia names as an article or a redirect, and
//! chains no name holds that many documents write (起動時), read the way the
//! analyzer's compounds join their parts.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{self, BufRead};

use aho_corasick::AhoCorasick;

use crate::dictionary::{MIN_COUNT, SURU, cost_of};
use crate::eval_documents::span_of;
use crate::kana::{HA_ROW, PLAIN, SEMI_VOICED, VOICED, is_kanji, is_katakana};
use crate::{Entry, Established, Title, Unit, title_entries, typed_by_analyzer};

/// How often a chain must occur to be written.
const MIN_CHAINS: usize = 5;
/// How often a chain no name holds must occur to be written.
const MIN_UNNAMED: usize = 10;
/// In how many documents a chain no name holds must occur to be written, so
/// that one site repeating it is not enough.
const MIN_UNNAMED_DOCS: usize = 5;
/// The most units of a chain no name holds.
const MAX_UNNAMED_UNITS: usize = 3;
/// How often a chain no name holds must occur to be reported.
const REPORT_MIN: usize = 3;
/// The share of the places before a chain no name holds that, once digits
/// take it, makes the chain the tail of a number (19世紀初頭) rather than a
/// word.
const MAX_AFTER_NUMBER: f64 = 0.5;
/// The least share, on each side, of the places a chain occurs at where no
/// kanji or katakana runs on into it.
const MIN_APART: f64 = 0.5;
/// The least entropy, in bits, of the characters on each side of a chain.
const MIN_ENTROPY: f64 = 1.5;
/// How many compounds must show how a part joins for a chain to follow them.
const MIN_COMPOUNDS: usize = 2;
/// The kana that turn into a small tsu before another part (てつ＋きょう).
const GEMINATING: [char; 4] = ['つ', 'ち', 'く', 'き'];

/// One part of a chain or a compound: its surface and reading.
type Part = (String, String);

/// Counts the chains of units whose surface `names` holds, the chains of the
/// units as the evaluation types them, which `established` decides, and the
/// compounds that teach how readings change where parts join. Units come
/// document by document, as `build/units.jsonl` holds them.
pub struct ChainCounter<'a> {
    names: &'a HashSet<String>,
    established: &'a Established,
    doc_id: String,
    units: Vec<Piece>,
    compounds: Vec<Piece>,
    whole_units: Vec<Unit>,
    whole_compounds: Vec<Unit>,
    chains: HashMap<String, Chain>,
    unnamed: HashMap<String, Unnamed>,
    /// Places of each surface of the units chains are made of.
    unit_counts: HashMap<String, usize>,
    documents: usize,
    /// Each compound once: its reading and parts.
    taught: HashSet<(String, Vec<Part>)>,
}

/// What a chain or a compound needs of a unit.
struct Piece {
    position: usize,
    surface: String,
    reading: String,
    numeric: bool,
}

#[derive(Default)]
struct Chain {
    count: usize,
    /// How often each run of parts made the chain.
    parts: HashMap<Vec<Part>, usize>,
}

#[derive(Default)]
struct Unnamed {
    count: usize,
    documents: usize,
    last_document: usize,
    /// Places whose last unit takes する after it (有効化した).
    suru: usize,
    parts: HashMap<Vec<Part>, usize>,
}

impl<'a> ChainCounter<'a> {
    pub fn new(names: &'a HashSet<String>, established: &'a Established) -> Self {
        Self {
            names,
            established,
            doc_id: String::new(),
            units: Vec::new(),
            compounds: Vec::new(),
            whole_units: Vec::new(),
            whole_compounds: Vec::new(),
            chains: HashMap::new(),
            unnamed: HashMap::new(),
            unit_counts: HashMap::new(),
            documents: 0,
            taught: HashSet::new(),
        }
    }

    pub fn observe(&mut self, unit: &Unit) {
        if unit.doc_id != self.doc_id {
            self.flush();
            self.doc_id.clone_from(&unit.doc_id);
        }
        // A numeric unit reads its number in digits; its kana reads it out.
        let reading = match &unit.numeric {
            Some(numeric) => numeric.kana.clone(),
            None => unit.reading.clone(),
        };
        let piece = Piece {
            position: unit.position,
            surface: unit.surface.clone(),
            reading,
            numeric: unit.numeric.is_some(),
        };
        if unit.compound {
            self.compounds.push(piece);
            self.whole_compounds.push(unit.clone());
        } else {
            self.units.push(piece);
            self.whole_units.push(unit.clone());
        }
    }

    pub fn finish(mut self) -> Chains {
        self.flush();
        let mut junctions = Junctions::default();
        for (reading, parts) in &self.taught {
            junctions.learn(reading, parts);
        }
        Chains {
            chains: self.chains,
            unnamed: self.unnamed,
            unit_counts: self.unit_counts,
            junctions,
        }
    }

    fn flush(&mut self) {
        let units = std::mem::take(&mut self.units);
        let compounds = std::mem::take(&mut self.compounds);
        let mut run: Vec<&Piece> = Vec::new();
        for u in &units {
            if !is_chain_surface(&u.surface) {
                self.count_run(&run);
                run.clear();
                continue;
            }
            if run.last().is_some_and(|last| !touches(last, u)) {
                self.count_run(&run);
                run.clear();
            }
            run.push(u);
        }
        self.count_run(&run);
        let at: HashMap<usize, &Piece> = units.iter().map(|u| (u.position, u)).collect();
        for c in &compounds {
            if let Some(parts) = parts_of(c, &at) {
                self.taught.insert((c.reading.clone(), parts));
            }
        }
        self.count_typed();
    }

    /// Counts the chains of the units as the evaluation types them: a
    /// compound typed whole is one unit of a chain, and a chain inside it is
    /// typed as the compound.
    fn count_typed(&mut self) {
        self.documents += 1;
        let units = std::mem::take(&mut self.whole_units);
        let compounds = std::mem::take(&mut self.whole_compounds);
        let typed = typed_by_analyzer(&units, &compounds, self.established);
        let mut run: Vec<&Unit> = Vec::new();
        for u in &typed {
            if !is_chain_unit(u) {
                self.count_unnamed(&run);
                run.clear();
                continue;
            }
            if run.last().is_some_and(|last| span_of(last).1 != u.position) {
                self.count_unnamed(&run);
                run.clear();
            }
            *self.unit_counts.entry(u.surface.clone()).or_default() += 1;
            run.push(u);
        }
        self.count_unnamed(&run);
    }

    /// Counts every run of two up to [`MAX_UNNAMED_UNITS`] units of `run`
    /// that does not end in a numeric unit.
    fn count_unnamed(&mut self, run: &[&Unit]) {
        for len in 2..=MAX_UNNAMED_UNITS {
            for window in run.windows(len) {
                let last = window[len - 1];
                if last.numeric.is_some() {
                    continue;
                }
                let surface: String = window.iter().map(|u| u.surface.as_str()).collect();
                let chain = self.unnamed.entry(surface).or_default();
                chain.count += 1;
                if chain.last_document != self.documents {
                    chain.documents += 1;
                    chain.last_document = self.documents;
                }
                if last.suru.is_some() {
                    chain.suru += 1;
                }
                let parts = window
                    .iter()
                    .map(|u| (u.surface.clone(), reading_of(u).to_owned()))
                    .collect();
                *chain.parts.entry(parts).or_default() += 1;
            }
        }
    }

    /// Counts every two and three units in a row of `run` that the names
    /// hold, leaving out those that end in a numeric unit (一月一日, 平成十年),
    /// which numeric items count, and those with a unit not read in kana.
    fn count_run(&mut self, run: &[&Piece]) {
        for len in 2..=3 {
            for window in run.windows(len) {
                if window.last().is_some_and(|u| u.numeric)
                    || window.iter().any(|u| u.reading.is_empty())
                {
                    continue;
                }
                let surface: String = window.iter().map(|u| u.surface.as_str()).collect();
                if !self.names.contains(&surface) {
                    continue;
                }
                let chain = self.chains.entry(surface).or_default();
                chain.count += 1;
                let parts = window
                    .iter()
                    .map(|u| (u.surface.clone(), u.reading.clone()))
                    .collect();
                *chain.parts.entry(parts).or_default() += 1;
            }
        }
    }
}

/// A unit read in kana whose surface is kanji, katakana and the long vowel
/// mark only, and no stem.
fn is_chain_unit(unit: &Unit) -> bool {
    is_chain_surface(&unit.surface)
        && unit.stem_surface.is_none()
        && !unit.okurigana_variant
        && !reading_of(unit).is_empty()
}

/// A numeric unit reads its number in digits; its kana reads it out.
fn reading_of(unit: &Unit) -> &str {
    unit.numeric
        .as_ref()
        .map_or(unit.reading.as_str(), |n| n.kana.as_str())
}

/// Kanji, katakana and the long vowel mark only.
fn is_chain_surface(surface: &str) -> bool {
    !surface.is_empty() && surface.chars().all(is_chain_char)
}

fn is_chain_char(c: char) -> bool {
    is_kanji(c) || is_katakana(c) || c == 'ー'
}

fn touches(before: &Piece, after: &Piece) -> bool {
    before.position + before.surface.chars().count() == after.position
}

/// The units that make up `compound`, when they cover it exactly and are more
/// than one.
fn parts_of(compound: &Piece, at: &HashMap<usize, &Piece>) -> Option<Vec<Part>> {
    let end = compound.position + compound.surface.chars().count();
    let mut parts = Vec::new();
    let mut position = compound.position;
    while position < end {
        let u = at.get(&position)?;
        if u.reading.is_empty() {
            return None;
        }
        parts.push((u.surface.clone(), u.reading.clone()));
        position += u.surface.chars().count();
    }
    (position == end && parts.len() > 1).then_some(parts)
}

/// The chains counted, and what the compounds taught.
pub struct Chains {
    chains: HashMap<String, Chain>,
    unnamed: HashMap<String, Unnamed>,
    unit_counts: HashMap<String, usize>,
    junctions: Junctions,
}

/// A chain no name holds, with what decides whether it is written.
#[derive(Debug, Clone, PartialEq)]
pub struct UnnamedChain {
    pub title: Title,
    pub count: usize,
    pub documents: usize,
    pub suru: usize,
    /// The share of the places of its rarest unit the chain takes up.
    pub cohesion: f64,
    /// The share of places where no kanji or katakana runs on into it, and
    /// the entropy of the characters, before and after it.
    pub apart: (f64, f64),
    pub entropy: (f64, f64),
    /// The share of the places before it a digit takes.
    pub after_number: f64,
    pub taken: bool,
}

/// The entries of the chains taken of `chains`: each as a word costed by
/// the places of `texts` it stands apart in against `total` units, and as the
/// stem of する when that many places take する after it (有効化 of 有効化した).
pub fn unnamed_entries<'t>(
    chains: &[UnnamedChain],
    texts: impl IntoIterator<Item = &'t str>,
    total: usize,
) -> Vec<Entry> {
    let taken: Vec<&UnnamedChain> = chains.iter().filter(|c| c.taken).collect();
    let titles: Vec<Title> = taken.iter().map(|c| c.title.clone()).collect();
    let mut entries = title_entries(&titles, texts, total);
    entries.extend(
        taken
            .iter()
            .filter(|c| c.suru >= MIN_COUNT && c.title.surface.chars().last().is_some_and(is_kanji))
            .map(|c| Entry {
                reading: c.title.reading.clone(),
                surface: c.title.surface.clone(),
                conjugation: Some(SURU.to_owned()),
                cost: cost_of(c.suru, total),
            }),
    );
    entries
}

/// The table of `chains` for people to judge: one row per chain, the most
/// often first.
pub fn unnamed_tsv(chains: &[UnnamedChain]) -> String {
    let mut rows: Vec<&UnnamedChain> = chains.iter().collect();
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.title.surface.cmp(&b.title.surface))
    });
    let mut tsv = String::from(
        "reading\tsurface\tcount\tdocuments\tsuru\tcohesion\tapart_before\tapart_after\tentropy_before\tentropy_after\tafter_number\ttaken\n",
    );
    for c in rows {
        writeln!(
            tsv,
            "{}\t{}\t{}\t{}\t{}\t{:.4}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}",
            c.title.reading,
            c.title.surface,
            c.count,
            c.documents,
            c.suru,
            c.cohesion,
            c.apart.0,
            c.apart.1,
            c.entropy.0,
            c.entropy.1,
            c.after_number,
            u8::from(c.taken),
        )
        .expect("writing to a String never fails");
    }
    tsv
}

impl Chains {
    /// The chains to write as words, read through their junctions: those
    /// counted often enough whose surface `known` lacks, and which stand
    /// apart from what is around them where they occur in `texts`.
    pub fn titles<'t>(
        &self,
        known: impl Fn(&str) -> bool,
        texts: impl IntoIterator<Item = &'t str>,
    ) -> Vec<Title> {
        let mut candidates: Vec<(&str, &Vec<Part>)> = self
            .chains
            .iter()
            .filter(|(surface, chain)| chain.count >= MIN_CHAINS && !known(surface))
            .filter_map(|(surface, chain)| {
                let parts = chain
                    .parts
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))?
                    .0;
                Some((surface.as_str(), parts))
            })
            .collect();
        candidates.sort();
        let surfaces: Vec<&str> = candidates.iter().map(|&(s, _)| s).collect();
        candidates
            .iter()
            .zip(sides(&surfaces, texts))
            .filter(|(_, sides)| sides.stand_apart())
            .map(|(&(surface, parts), _)| Title {
                reading: self.junctions.read(parts),
                surface: surface.to_owned(),
            })
            .collect()
    }

    /// The chains of the units as the evaluation types them that no name
    /// holds, met at least [`REPORT_MIN`] times, whose surface `known` lacks,
    /// read through their junctions. One is taken when it occurs at least
    /// [`MIN_UNNAMED`] times in at least [`MIN_UNNAMED_DOCS`] documents and
    /// stands apart from what is around it where it occurs in `texts`.
    pub fn unnamed<'t>(
        &self,
        known: impl Fn(&str) -> bool,
        texts: impl IntoIterator<Item = &'t str>,
    ) -> Vec<UnnamedChain> {
        let mut candidates: Vec<(&str, &Unnamed, &Vec<Part>)> = self
            .unnamed
            .iter()
            .filter(|(surface, chain)| chain.count >= REPORT_MIN && !known(surface))
            .filter_map(|(surface, chain)| {
                let parts = chain
                    .parts
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))?
                    .0;
                Some((surface.as_str(), chain, parts))
            })
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(b.0));
        let surfaces: Vec<&str> = candidates.iter().map(|&(s, _, _)| s).collect();
        candidates
            .iter()
            .zip(sides(&surfaces, texts))
            .map(|(&(surface, chain, parts), sides)| {
                let rarest = parts
                    .iter()
                    .map(|(s, _)| self.unit_counts.get(s).copied().unwrap_or_default())
                    .min()
                    .unwrap_or_default()
                    .max(1);
                let ((apart_before, entropy_before), (apart_after, entropy_after)) =
                    (Sides::of(&sides.before), Sides::of(&sides.after));
                let after_number = Sides::after_number(&sides.before);
                UnnamedChain {
                    title: Title {
                        reading: self.junctions.read(&plural_gata(parts)),
                        surface: surface.to_owned(),
                    },
                    count: chain.count,
                    documents: chain.documents,
                    suru: chain.suru,
                    cohesion: chain.count as f64 / rarest as f64,
                    apart: (apart_before, apart_after),
                    entropy: (entropy_before, entropy_after),
                    after_number,
                    taken: chain.count >= MIN_UNNAMED
                        && chain.documents >= MIN_UNNAMED_DOCS
                        && sides.stand_apart()
                        && after_number < MAX_AFTER_NUMBER,
                }
            })
            .collect()
    }

    /// The reading of a chain of `parts` (surface, reading).
    pub fn read(&self, parts: &[(&str, &str)]) -> String {
        let parts: Vec<Part> = parts
            .iter()
            .map(|&(s, r)| (s.to_owned(), r.to_owned()))
            .collect();
        self.junctions.read(&parts)
    }
}

/// How the compounds join their parts: per part, how often its first kana
/// stays, voices and semi-voices after another part, and how often its last
/// kana stays and turns into a small tsu before another part.
#[derive(Default)]
struct Junctions {
    heads: HashMap<Part, [usize; 3]>,
    tails: HashMap<Part, [usize; 2]>,
}

#[derive(Clone, Copy)]
enum Head {
    Plain,
    Voiced,
    SemiVoiced,
}

const HEADS: [Head; 3] = [Head::Plain, Head::Voiced, Head::SemiVoiced];

impl Junctions {
    /// Counts how the parts of a compound read `reading` join, when kana
    /// voicing and small tsu where they join explain it.
    fn learn(&mut self, reading: &str, parts: &[Part]) {
        let Some(joins) = joins_of(reading, parts) else {
            return;
        };
        for (i, (small_tsu, head)) in joins.into_iter().enumerate() {
            self.heads.entry(parts[i + 1].clone()).or_default()[head as usize] += 1;
            if ends_geminating(&parts[i].1) {
                self.tails.entry(parts[i].clone()).or_default()[usize::from(small_tsu)] += 1;
            }
        }
    }

    fn read(&self, parts: &[Part]) -> String {
        let mut reading = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i == 0 {
                reading.push_str(&part.1);
                continue;
            }
            if self.small_tsu(&parts[i - 1]) && starts_voiceless(&part.1) {
                reading.pop();
                reading.push('っ');
            }
            reading
                .push_str(&with_head(&part.1, self.head(part)).unwrap_or_else(|| part.1.clone()));
        }
        reading
    }

    fn head(&self, part: &Part) -> Head {
        self.heads
            .get(part)
            .and_then(|counts| most(counts))
            .map_or(Head::Plain, |i| HEADS[i])
    }

    fn small_tsu(&self, part: &Part) -> bool {
        ends_geminating(&part.1) && self.tails.get(part).and_then(|counts| most(counts)) == Some(1)
    }
}

/// The index of the largest of `counts`, the first on a tie, when they add
/// up to [`MIN_COMPOUNDS`].
fn most(counts: &[usize]) -> Option<usize> {
    if counts.iter().sum::<usize>() < MIN_COMPOUNDS {
        return None;
    }
    (0..counts.len()).rev().max_by_key(|&i| counts[i])
}

/// For each place two of `parts` join in `reading`: whether the part before
/// ends in a small tsu, and how the part after begins.
fn joins_of(reading: &str, parts: &[Part]) -> Option<Vec<(bool, Head)>> {
    let mut rest = reading;
    let mut current = parts.first()?.1.clone();
    let mut joins = Vec::new();
    for part in &parts[1..] {
        let mut befores = vec![(false, current.clone())];
        if ends_geminating(&current) {
            let mut small = current.clone();
            small.pop();
            small.push('っ');
            befores.push((true, small));
        }
        let (small_tsu, used, head, after) = befores
            .into_iter()
            .filter(|(_, before)| rest.starts_with(before.as_str()))
            .find_map(|(small_tsu, before)| {
                let tail = &rest[before.len()..];
                HEADS.into_iter().find_map(|head| {
                    let after = with_head(&part.1, head)?;
                    tail.starts_with(&after)
                        .then_some((small_tsu, before.len(), head, after))
                })
            })?;
        joins.push((small_tsu, head));
        rest = &rest[used..];
        current = after;
    }
    (rest == current).then_some(joins)
}

/// `reading` with its first kana changed as `head` says, when it can be.
fn with_head(reading: &str, head: Head) -> Option<String> {
    let mut chars = reading.chars();
    let first = chars.next()?;
    let changed = match head {
        Head::Plain => first,
        Head::Voiced => {
            let i = PLAIN.chars().position(|k| k == first)?;
            VOICED.chars().nth(i)?
        }
        Head::SemiVoiced => {
            let i = PLAIN.chars().position(|k| k == first)?;
            SEMI_VOICED.chars().nth(i.checked_sub(HA_ROW)?)?
        }
    };
    Some(std::iter::once(changed).chain(chars).collect())
}

fn ends_geminating(reading: &str) -> bool {
    reading
        .chars()
        .next_back()
        .is_some_and(|c| GEMINATING.contains(&c))
}

fn starts_voiceless(reading: &str) -> bool {
    reading
        .chars()
        .next()
        .is_some_and(|c| PLAIN.contains(c) || SEMI_VOICED.contains(c))
}

/// The characters right before and after the places a surface occurs at;
/// `None` for the start or the end of a text.
#[derive(Default)]
struct Sides {
    before: HashMap<Option<char>, usize>,
    after: HashMap<Option<char>, usize>,
}

impl Sides {
    fn stand_apart(&self) -> bool {
        [&self.before, &self.after].into_iter().all(|side| {
            let (apart, entropy) = Self::of(side);
            apart >= MIN_APART && entropy >= MIN_ENTROPY
        })
    }

    /// The share of the places on `side` a digit takes.
    fn after_number(side: &HashMap<Option<char>, usize>) -> f64 {
        let total: usize = side.values().sum();
        let digits: usize = side
            .iter()
            .filter(|(c, _)| c.is_some_and(|c| c.is_ascii_digit() || ('０'..='９').contains(&c)))
            .map(|(_, n)| n)
            .sum();
        if total == 0 {
            0.0
        } else {
            digits as f64 / total as f64
        }
    }

    /// The share of the places on `side` where no kanji or katakana runs on
    /// into the surface, and the entropy of the characters there; nothing
    /// for a surface found nowhere.
    fn of(side: &HashMap<Option<char>, usize>) -> (f64, f64) {
        let total: usize = side.values().sum();
        if total == 0 {
            return (0.0, 0.0);
        }
        let apart: usize = side
            .iter()
            .filter(|(c, _)| !c.is_some_and(is_chain_char))
            .map(|(_, n)| n)
            .sum();
        (apart as f64 / total as f64, entropy(side, total))
    }
}

/// `parts` with a lone 方 at the end read がた, the plural of people
/// (先生方), which the analyzer reads as the noun ほう.
fn plural_gata(parts: &[Part]) -> Vec<Part> {
    let mut parts = parts.to_vec();
    if let Some(last) = parts.last_mut().filter(|(s, _)| s == "方") {
        last.1 = "がた".to_owned();
    }
    parts
}

fn entropy(counts: &HashMap<Option<char>, usize>, total: usize) -> f64 {
    counts
        .values()
        .map(|&n| {
            let p = n as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}

/// The [`Sides`] of each of `surfaces` in `texts`.
fn sides<'t>(surfaces: &[&str], texts: impl IntoIterator<Item = &'t str>) -> Vec<Sides> {
    let mut sides: Vec<Sides> = surfaces.iter().map(|_| Sides::default()).collect();
    let Ok(matcher) = AhoCorasick::new(surfaces) else {
        return sides;
    };
    for text in texts {
        for m in matcher.find_overlapping_iter(text) {
            let s = &mut sides[m.pattern().as_usize()];
            *s.before
                .entry(text[..m.start()].chars().next_back())
                .or_default() += 1;
            *s.after.entry(text[m.end()..].chars().next()).or_default() += 1;
        }
    }
    sides
}

/// The names of `build/base-names.txt`, one per line.
pub fn read_names(lines: impl BufRead) -> io::Result<HashSet<String>> {
    lines
        .lines()
        .filter(|l| l.as_ref().map_or(true, |l| !l.is_empty()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Numeric;

    fn unit(doc_id: &str, position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            particle: None,
            suru: None,
            copula: None,
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
            doc_id: doc_id.into(),
            position,
            reading: reading.into(),
            surface: surface.into(),
            stem_reading: None,
            stem_surface: None,
            conjugation: None,
            unknown_conjugation: None,
            numeric: None,
        }
    }

    fn compound(doc_id: &str, position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            compound: true,
            ..unit(doc_id, position, reading, surface)
        }
    }

    fn names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|&n| n.to_owned()).collect()
    }

    /// `parts` (surface, reading) as units that touch, starting at `position`.
    fn touching(doc_id: &str, position: usize, parts: &[(&str, &str)]) -> Vec<Unit> {
        let mut at = position;
        parts
            .iter()
            .map(|&(s, r)| {
                let u = unit(doc_id, at, r, s);
                at += s.chars().count();
                u
            })
            .collect()
    }

    /// A compound over `parts` and its parts, at the start of document `doc_id`.
    fn compound_of(doc_id: &str, reading: &str, parts: &[(&str, &str)]) -> Vec<Unit> {
        let surface: String = parts.iter().map(|&(s, _)| s).collect();
        let mut units = vec![compound(doc_id, 0, reading, &surface)];
        units.extend(touching(doc_id, 0, parts));
        units
    }

    fn count(names: &HashSet<String>, units: &[Unit]) -> Chains {
        let established = Established::default();
        let mut counter = ChainCounter::new(names, &established);
        for u in units {
            counter.observe(u);
        }
        counter.finish()
    }

    /// Text in which `word` stands apart from varied neighbours `n` times.
    fn apart(word: &str, n: usize) -> String {
        const AROUND: [(&str, &str); 4] = [("。", "は"), ("、", "が"), ("「", "」"), ("\n", "を")];
        (0..n)
            .map(|i| {
                let (before, after) = AROUND[i % AROUND.len()];
                format!("{before}{word}{after}")
            })
            .collect()
    }

    fn titles(chains: &Chains, texts: &[String]) -> Vec<(String, String)> {
        chains
            .titles(|_| false, texts.iter().map(String::as_str))
            .into_iter()
            .map(|t| (t.reading, t.surface))
            .collect()
    }

    #[test]
    fn units_that_touch_chain_in_twos_and_threes_that_wikipedia_names() {
        let names = names(&["多種多様", "自律神経失調", "神経失調"]);
        let units: Vec<Unit> = (0..5)
            .flat_map(|i| {
                let doc = format!("d{i}");
                let mut units = touching(&doc, 0, &[("多種", "たしゅ"), ("多様", "たよう")]);
                units.extend(touching(
                    &doc,
                    10,
                    &[
                        ("自律", "じりつ"),
                        ("神経", "しんけい"),
                        ("失調", "しっちょう"),
                    ],
                ));
                units
            })
            .collect();
        let chains = count(&names, &units);
        let text = [
            apart("多種多様", 8),
            apart("自律神経失調", 8),
            apart("神経失調", 8),
        ]
        .concat();

        assert_eq!(
            titles(&chains, &[text]),
            [
                ("たしゅたよう".into(), "多種多様".into()),
                ("しんけいしっちょう".into(), "神経失調".into()),
                ("じりつしんけいしっちょう".into(), "自律神経失調".into()),
            ],
        );
    }

    #[test]
    fn a_chain_wikipedia_does_not_name_is_not_taken() {
        let names = names(&[]);
        let units: Vec<Unit> = (0..5)
            .flat_map(|i| {
                touching(
                    &format!("d{i}"),
                    0,
                    &[("使用", "しよう"), ("方法", "ほうほう")],
                )
            })
            .collect();

        assert!(titles(&count(&names, &units), &[apart("使用方法", 8)]).is_empty());
    }

    #[test]
    fn kana_or_a_gap_breaks_a_chain() {
        let names = names(&["多種多様"]);
        let mut units = Vec::new();
        for i in 0..5 {
            let doc = format!("d{i}");
            // 多種な多様: the kana between them is no unit of the chain.
            units.push(unit(&doc, 0, "たしゅ", "多種"));
            units.push(unit(&doc, 2, "な", "な"));
            units.push(unit(&doc, 3, "たよう", "多様"));
            // 多種 多様: a gap.
            units.push(unit(&doc, 10, "たしゅ", "多種"));
            units.push(unit(&doc, 13, "たよう", "多様"));
        }
        let text = apart("多種多様", 8);

        assert!(titles(&count(&names, &units), &[text]).is_empty());
    }

    fn numeric(doc_id: &str, position: usize, surface: &str, item: (&str, &str, &str)) -> Unit {
        let (reading, value, kana) = item;
        Unit {
            numeric: Some(Numeric {
                reading: reading.into(),
                surface: format!(
                    "{{kanji}}{}",
                    &surface[surface.char_indices().nth(1).unwrap().0..]
                ),
                value: value.into(),
                kana: kana.into(),
            }),
            ..unit(doc_id, position, &reading.replace("{}", value), surface)
        }
    }

    #[test]
    fn a_numeric_unit_and_a_word_after_it_chain_read_by_the_unit_s_kana() {
        let names = names(&["二次会"]);
        let mut units = Vec::new();
        for i in 0..5 {
            let doc = format!("d{i}");
            units.push(numeric(&doc, 0, "二次", ("{}じ", "2", "にじ")));
            units.push(unit(&doc, 2, "かい", "会"));
        }
        let text = apart("二次会", 8);

        assert_eq!(
            titles(&count(&names, &units), &[text]),
            [("にじかい".to_owned(), "二次会".to_owned())]
        );
    }

    #[test]
    fn a_chain_that_ends_in_a_numeric_unit_is_not_taken() {
        let names = names(&["一月一日", "平成十年"]);
        let mut units = Vec::new();
        for i in 0..5 {
            let doc = format!("d{i}");
            units.push(numeric(&doc, 0, "一月", ("{}がつ", "1", "いちがつ")));
            units.push(numeric(&doc, 2, "一日", ("{}にち", "1", "いちにち")));
            units.push(unit(&doc, 10, "へいせい", "平成"));
            units.push(numeric(&doc, 12, "十年", ("{}ねん", "10", "じゅうねん")));
        }
        let text = [apart("一月一日", 8), apart("平成十年", 8)].concat();

        assert!(titles(&count(&names, &units), &[text]).is_empty());
    }

    #[test]
    fn a_chain_is_taken_only_when_it_occurs_often_enough_and_the_dictionary_lacks_it() {
        let names = names(&["多種多様", "自律神経"]);
        let mut units: Vec<Unit> = (0..4)
            .flat_map(|i| {
                touching(
                    &format!("d{i}"),
                    0,
                    &[("多種", "たしゅ"), ("多様", "たよう")],
                )
            })
            .collect();
        units.extend((0..5).flat_map(|i| {
            touching(
                &format!("e{i}"),
                0,
                &[("自律", "じりつ"), ("神経", "しんけい")],
            )
        }));
        let chains = count(&names, &units);
        let texts = [[apart("多種多様", 8), apart("自律神経", 8)].concat()];

        assert_eq!(
            titles(&chains, &texts),
            [("じりつしんけい".into(), "自律神経".into())],
            "多種多様 occurs four times",
        );
        assert!(
            chains
                .titles(|s| s == "自律神経", texts.iter().map(String::as_str))
                .is_empty(),
            "the dictionary has 自律神経",
        );
    }

    #[test]
    fn a_chain_that_runs_on_into_kanji_is_a_piece_of_a_longer_word() {
        let names = names(&["厚生労働"]);
        let units: Vec<Unit> = (0..5)
            .flat_map(|i| {
                touching(
                    &format!("d{i}"),
                    0,
                    &[("厚生", "こうせい"), ("労働", "ろうどう")],
                )
            })
            .collect();
        let text = [apart("厚生労働省", 8), apart("厚生労働", 2)].concat();

        assert!(titles(&count(&names, &units), &[text]).is_empty());
    }

    #[test]
    fn a_chain_with_few_kinds_of_neighbours_is_not_taken() {
        let names = names(&["多種多様"]);
        let units: Vec<Unit> = (0..5)
            .flat_map(|i| {
                touching(
                    &format!("d{i}"),
                    0,
                    &[("多種", "たしゅ"), ("多様", "たよう")],
                )
            })
            .collect();
        let chains = count(&names, &units);

        assert!(titles(&chains, &["。多種多様は".repeat(8)]).is_empty());
    }

    #[test]
    fn compounds_teach_a_part_to_voice_where_it_joins() {
        let names = names(&[]);
        let units = [
            compound_of(
                "a",
                "かぶしきがいしゃ",
                &[("株式", "かぶしき"), ("会社", "かいしゃ")],
            ),
            compound_of(
                "b",
                "しんたくがいしゃ",
                &[("信託", "しんたく"), ("会社", "かいしゃ")],
            ),
            compound_of("c", "ろてんぶろ", &[("露天", "ろてん"), ("風呂", "ふろ")]),
        ]
        .concat();
        let chains = count(&names, &units);

        assert_eq!(
            chains.read(&[("派遣", "はけん"), ("会社", "かいしゃ")]),
            "はけんがいしゃ"
        );
        assert_eq!(
            chains.read(&[("家族", "かぞく"), ("風呂", "ふろ")]),
            "かぞくふろ",
            "one compound alone teaches nothing",
        );
        assert_eq!(
            chains.read(&[("会社", "かいしゃ"), ("派遣", "はけん")]),
            "かいしゃはけん"
        );
    }

    #[test]
    fn compounds_teach_a_part_to_end_in_a_small_tsu() {
        let names = names(&[]);
        let units = [
            compound_of("a", "てっきょう", &[("鉄", "てつ"), ("橋", "きょう")]),
            compound_of("b", "てっこつ", &[("鉄", "てつ"), ("骨", "こつ")]),
        ]
        .concat();
        let chains = count(&names, &units);

        assert_eq!(chains.read(&[("鉄", "てつ"), ("塔", "とう")]), "てっとう");
        assert_eq!(
            chains.read(&[("鉄", "てつ"), ("道", "どう")]),
            "てつどう",
            "no small tsu before a voiced kana",
        );
    }

    #[test]
    fn the_reading_most_occurrences_give_reads_the_chain() {
        let names = names(&["日本代表"]);
        let mut units: Vec<Unit> = (0..3)
            .flat_map(|i| {
                touching(
                    &format!("d{i}"),
                    0,
                    &[("日本", "にほん"), ("代表", "だいひょう")],
                )
            })
            .collect();
        units.extend((0..2).flat_map(|i| {
            touching(
                &format!("e{i}"),
                0,
                &[("日本", "にっぽん"), ("代表", "だいひょう")],
            )
        }));

        assert_eq!(
            titles(&count(&names, &units), &[apart("日本代表", 8)]),
            [("にほんだいひょう".into(), "日本代表".into())],
        );
    }

    #[test]
    fn names_come_one_per_line() {
        let read = read_names("多種多様\n\n自律神経\n".as_bytes()).unwrap();

        assert_eq!(read, names(&["多種多様", "自律神経"]));
    }

    /// `parts` touching in `documents` documents, `times` times in all, the
    /// `at`-th set of parts in a document.
    fn spread(times: usize, documents: usize, at: usize, parts: &[(&str, &str)]) -> Vec<Unit> {
        (0..times)
            .flat_map(|i| {
                let doc = format!("d{:03}", i % documents);
                touching(&doc, i * 100 + at * 10, parts)
            })
            .collect()
    }

    /// Units sorted as `build/units.jsonl` holds them.
    fn sorted(mut units: Vec<Unit>) -> Vec<Unit> {
        units.sort_by(|a, b| (&a.doc_id, a.position).cmp(&(&b.doc_id, b.position)));
        units
    }

    fn unnamed(chains: &Chains, texts: &[String]) -> Vec<(String, String, bool)> {
        chains
            .unnamed(|_| false, texts.iter().map(String::as_str))
            .into_iter()
            .map(|c| (c.title.reading, c.title.surface, c.taken))
            .collect()
    }

    #[test]
    fn a_chain_no_name_holds_is_taken_when_many_documents_write_it_apart() {
        let units = sorted(spread(
            MIN_UNNAMED,
            MIN_UNNAMED_DOCS,
            0,
            &[("起動", "きどう"), ("時", "じ")],
        ));
        let chains = count(&names(&[]), &units);

        assert_eq!(
            unnamed(&chains, &[apart("起動時", 8)]),
            [("きどうじ".into(), "起動時".into(), true)]
        );
    }

    #[test]
    fn a_chain_no_name_holds_is_not_taken_from_too_few_documents_or_places() {
        let few_documents = spread(
            MIN_UNNAMED * 3,
            MIN_UNNAMED_DOCS - 1,
            0,
            &[("送料", "そうりょう"), ("無料", "むりょう")],
        );
        let few_places = spread(
            MIN_UNNAMED - 1,
            MIN_UNNAMED - 1,
            1,
            &[("接続", "せつぞく"), ("先", "さき")],
        );
        let chains = count(&names(&[]), &sorted([few_documents, few_places].concat()));
        let texts = [[apart("送料無料", 8), apart("接続先", 8)].concat()];

        assert_eq!(
            unnamed(&chains, &texts),
            [
                ("せつぞくさき".into(), "接続先".into(), false),
                ("そうりょうむりょう".into(), "送料無料".into(), false),
            ]
        );
    }

    #[test]
    fn a_chain_inside_a_compound_typed_whole_is_typed_as_the_compound() {
        let parts = [("厚生", "こうせい"), ("労働", "ろうどう"), ("省", "しょう")];
        let units: Vec<Unit> = (0..MIN_UNNAMED)
            .flat_map(|i| {
                let doc = format!("d{i:03}");
                let mut units = vec![compound(&doc, 0, "こうせいろうどうしょう", "厚生労働省")];
                units.extend(touching(&doc, 0, &parts));
                units.extend(touching(&doc, 10, &[("同", "どう"), ("省", "しょう")]));
                units
            })
            .collect();
        let chains = count(&names(&[]), &units);

        assert_eq!(
            unnamed(&chains, &[apart("厚生労働", 8), apart("同省", 8)]),
            [("どうしょう".into(), "同省".into(), true)],
            "厚生労働 is never typed",
        );
    }

    #[test]
    fn a_chain_whose_last_unit_takes_suru_is_also_a_stem_of_suru() {
        let units: Vec<Unit> = spread(
            MIN_UNNAMED,
            MIN_UNNAMED_DOCS,
            0,
            &[("有効", "ゆうこう"), ("化", "か")],
        )
        .into_iter()
        .map(|u| match u.surface.as_str() {
            "化" => Unit {
                suru: Some("し".into()),
                ..u
            },
            _ => u,
        })
        .collect();
        let chains = count(&names(&[]), &sorted(units));
        let texts = [apart("有効化", 8)];
        let taken = chains.unnamed(|_| false, texts.iter().map(String::as_str));

        let entries: Vec<(String, String, Option<String>)> =
            unnamed_entries(&taken, texts.iter().map(String::as_str), 1000)
                .into_iter()
                .map(|e| (e.reading, e.surface, e.conjugation))
                .collect();

        assert_eq!(
            entries,
            [
                ("ゆうこうか".into(), "有効化".into(), None),
                ("ゆうこうか".into(), "有効化".into(), Some(SURU.into())),
            ]
        );
    }

    #[test]
    fn the_report_lists_every_chain_the_most_often_first() {
        let units = sorted(
            [
                spread(4, 2, 0, &[("接続", "せつぞく"), ("先", "さき")]),
                spread(3, 3, 1, &[("表示", "ひょうじ"), ("名", "めい")]),
            ]
            .concat(),
        );
        let chains = count(&names(&[]), &units);
        let report = unnamed_tsv(&chains.unnamed(|_| false, std::iter::empty()));

        let rows: Vec<Vec<&str>> = report
            .lines()
            .skip(1)
            .map(|l| l.split('\t').take(5).collect())
            .collect();
        assert_eq!(
            rows,
            [
                ["せつぞくさき", "接続先", "4", "2", "0"],
                ["ひょうじめい", "表示名", "3", "3", "0"],
            ]
        );
    }

    #[test]
    fn a_chain_a_number_comes_before_in_most_places_is_not_taken() {
        let units = sorted(spread(
            MIN_UNNAMED,
            MIN_UNNAMED_DOCS,
            0,
            &[("世紀", "せいき"), ("初頭", "しょとう")],
        ));
        let chains = count(&names(&[]), &units);
        let after_numbers: String = (0..10)
            .map(|d| format!("{d}世紀初頭{}", ["は", "が", "の", "、", "に"][d % 5]))
            .collect();
        let texts = [apart("世紀初頭", 8), after_numbers];

        assert_eq!(
            unnamed(&chains, &texts),
            [("せいきしょとう".into(), "世紀初頭".into(), false)]
        );
    }

    #[test]
    fn a_chain_ending_in_a_lone_kata_reads_it_as_the_plural_gata() {
        let units = sorted(spread(
            MIN_UNNAMED,
            MIN_UNNAMED_DOCS,
            0,
            &[("先生", "せんせい"), ("方", "ほう")],
        ));
        let chains = count(&names(&[]), &units);

        assert_eq!(
            unnamed(&chains, &[apart("先生方", 8)]),
            [("せんせいがた".into(), "先生方".into(), true)]
        );
    }
}

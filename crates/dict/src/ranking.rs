//! Training Kanaemi's ranking model: examples from the units of documents, a
//! softmax over each unit's candidates, and the weights written as Kanaemi's
//! model file.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kanaemi_engine::{
    CONTEXT_CHARS, CandidateFacts, Engine, FEATURE_NAMES, FeatureKey, HISTORY_LEN,
    MODEL_FORMAT_VERSION, RankingInput, feature_keys, key_index, key_tag, unknown_key,
};

use crate::dictionary::MIN_COUNT;
use crate::kana::is_hiragana;
use crate::{Established, Query, Spans, Unit, as_typed, query_of, span_of};

/// The tag of a place no feature was learned at.
const EMPTY_TAG: u8 = 0;

/// How often a feature must occur in the examples to be learned as itself,
/// rather than as its name's unknown feature: as often as a word must occur
/// to enter the base, each unit being an example in its field and in an
/// empty field.
pub const LEARNED_MIN_COUNT: u32 = 2 * MIN_COUNT as u32;

/// Training examples, flat: each is a unit's candidates, each candidate the
/// keys of its features and the names they have, and which candidate is the
/// unit's surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Examples {
    keys: Vec<u32>,
    names: Vec<u8>,
    candidate_ends: Vec<u32>,
    example_ends: Vec<u32>,
    correct: Vec<u32>,
}

/// The features of one candidate of an example: the key of each, and its
/// name's place in Kanaemi's feature names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub keys: &'a [u32],
    pub names: &'a [u8],
}

impl Candidate<'_> {
    fn features(&self) -> impl Iterator<Item = FeatureKey> + '_ {
        self.keys
            .iter()
            .zip(self.names)
            .map(|(&key, &name)| FeatureKey { key, name })
    }
}

impl Examples {
    pub fn len(&self) -> usize {
        self.correct.len()
    }

    pub fn is_empty(&self) -> bool {
        self.correct.is_empty()
    }

    /// Adds an example. A feature every candidate has, as many times, adds
    /// the same to every score and so neither ranks nor learns; it is not
    /// kept, which leaves out most of the history features.
    pub fn push(&mut self, candidates: impl IntoIterator<Item = Vec<FeatureKey>>, correct: usize) {
        let candidates: Vec<Vec<FeatureKey>> = candidates.into_iter().collect();
        let mut shared: HashMap<FeatureKey, usize> = HashMap::new();
        if let Some((first, rest)) = candidates.split_first() {
            for &f in first {
                *shared.entry(f).or_default() += 1;
            }
            for features in rest {
                let mut counts: HashMap<FeatureKey, usize> = HashMap::new();
                for &f in features {
                    *counts.entry(f).or_default() += 1;
                }
                shared.retain(|f, n| {
                    *n = (*n).min(counts.get(f).copied().unwrap_or(0));
                    *n > 0
                });
            }
        }
        for features in candidates {
            let mut left = shared.clone();
            for f in features {
                match left.get_mut(&f) {
                    Some(n) if *n > 0 => *n -= 1,
                    _ => {
                        self.keys.push(f.key);
                        self.names.push(f.name);
                    }
                }
            }
            self.candidate_ends.push(self.keys.len() as u32);
        }
        self.example_ends.push(self.candidate_ends.len() as u32);
        self.correct.push(correct as u32);
    }

    pub fn append(&mut self, other: Examples) {
        let features = self.keys.len() as u32;
        let candidates = self.candidate_ends.len() as u32;
        self.keys.extend(other.keys);
        self.names.extend(other.names);
        self.candidate_ends
            .extend(other.candidate_ends.into_iter().map(|e| e + features));
        self.example_ends
            .extend(other.example_ends.into_iter().map(|e| e + candidates));
        self.correct.extend(other.correct);
    }

    /// The candidates of the `i`th example.
    pub fn candidates(&self, i: usize) -> impl Iterator<Item = Candidate<'_>> {
        let first = if i == 0 {
            0
        } else {
            self.example_ends[i - 1] as usize
        };
        (first..self.example_ends[i] as usize).map(move |c| {
            let from = if c == 0 {
                0
            } else {
                self.candidate_ends[c - 1] as usize
            };
            let to = self.candidate_ends[c] as usize;
            Candidate {
                keys: &self.keys[from..to],
                names: &self.names[from..to],
            }
        })
    }

    /// Which candidate of the `i`th example is the unit's surface.
    pub fn correct(&self, i: usize) -> usize {
        self.correct[i] as usize
    }
}

/// The examples of a document read as one input field: each unit is typed as
/// the evaluation types it, with the document's earlier units as the field's
/// commits and the text before it as the field's committed text, and once
/// more into an empty field. A conjugating unit is also typed in the
/// [other ways](other_typings) it may be. Typings with a single candidate or
/// without the unit's surface among the candidates teach nothing and are left
/// out.
pub fn examples_of_document(engine: &Engine, units: &[Unit], text: &str) -> Examples {
    let chars: Vec<char> = text.chars().collect();
    let mut history: Vec<(String, String)> = Vec::new();
    let mut examples = Examples::default();
    for unit in units {
        let query = query_of(unit);
        let end = unit.position.min(chars.len());
        let context: String = chars[end.saturating_sub(CONTEXT_CHARS)..end]
            .iter()
            .collect();
        for other in other_typings(unit, &query) {
            let candidates = engine.candidate_facts(&other.reading, other.okurigana.as_deref());
            push_typing(&mut examples, &other, &candidates, &history, &context);
        }
        let candidates = engine.candidate_facts(&query.reading, query.okurigana.as_deref());
        let correct = push_typing(&mut examples, &query, &candidates, &history, &context);
        // Kanaemi records a commit as its candidate's own pair: a numeric
        // candidate as its numeric item.
        let commit = match correct.map(|c| candidates[c].recorded(&query.reading)) {
            Some((reading, surface)) => (reading.to_owned(), surface.to_owned()),
            None => (query.reading.clone(), query.expected.clone()),
        };
        history.push(commit);
        if history.len() > HISTORY_LEN {
            history.remove(0);
        }
    }
    examples
}

/// The units of a document a typist types one by one: each word joined with
/// affixes no lexicon has typed as its parts (see [`crate::as_typed`]) unless
/// `established`, and then all but those whose surface is one of `fragments`
/// and that lie inside one of `compounds` (一髪 in 危機一髪), which are typed
/// as the compound.
/// The dictionary counts them so too (see [`crate::fragment_surfaces`]);
/// taught them, the model would put such a fragment back ahead of the words
/// it lost to.
pub fn typed_units(
    units: &[Unit],
    compounds: &[Unit],
    fragments: &HashSet<String>,
    established: &Established,
) -> Vec<Unit> {
    let spans = Spans::new(compounds.iter().map(span_of));
    units
        .iter()
        .flat_map(|u| as_typed(u, established))
        .filter(|u| {
            let end = u.position + u.surface.chars().count();
            !(fragments.contains(&u.surface) && spans.covers(u.position, end))
        })
        .collect()
}

/// Adds the examples of `query` typed into the field with `history` and
/// `context`, and into an empty field, when its `candidates` teach anything;
/// gives where its expected surface is among them.
fn push_typing(
    examples: &mut Examples,
    query: &Query,
    candidates: &[CandidateFacts],
    history: &[(String, String)],
    context: &str,
) -> Option<usize> {
    let correct = candidates.iter().position(|c| c.surface == query.expected);
    if let (Some(correct), true) = (correct, candidates.len() > 1) {
        for (history, context) in [(history, context), (&[][..], "")] {
            let input = RankingInput {
                reading: &query.reading,
                history,
                context,
            };
            examples.push(candidates.iter().map(|c| feature_keys(&input, c)), correct);
        }
    }
    correct
}

/// The other ways a unit may be typed. A conjugating unit, typed with the
/// first kana of its okurigana marked, also with all of its okurigana marked
/// (か*いた for 書いた) when that is more than the first kana; not whole
/// (かいた), which a typist types for a word written whole (各 for かく). A
/// numeric unit with its number in kanji, typed with the number as digits,
/// also with the number in kana (にほん for 二本). Without them the model sees
/// such a form only as a wrong candidate of another word typed whole, and
/// learns to put it last.
fn other_typings(unit: &Unit, query: &Query) -> Vec<Query> {
    if let Some(numeric) = &unit.numeric {
        if !numeric.surface.contains("{kanji}") || numeric.kana.is_empty() {
            return Vec::new();
        }
        return vec![Query {
            reading: numeric.kana.clone(),
            okurigana: None,
            expected: unit.surface.clone(),
        }];
    }
    if query.okurigana.is_none() {
        return Vec::new();
    }
    let okurigana: String = {
        let mut kana: Vec<char> = unit
            .surface
            .chars()
            .rev()
            .take_while(|&c| is_hiragana(c))
            .collect();
        kana.reverse();
        kana.into_iter().collect()
    };
    if okurigana.chars().count() < 2 || !unit.reading.ends_with(&okurigana) {
        return Vec::new();
    }
    vec![Query {
        reading: unit.reading.clone(),
        okurigana: Some(okurigana),
        expected: unit.surface.clone(),
    }]
}

/// Examples kept as blocks, each loaded whole when trained on, so that only
/// one block need be in memory at a time.
pub trait Blocks {
    fn count(&self) -> usize;
    fn load(&self, i: usize) -> io::Result<Examples>;
}

/// Examples in memory are one block.
impl Blocks for Examples {
    fn count(&self) -> usize {
        1
    }

    fn load(&self, _i: usize) -> io::Result<Examples> {
        Ok(self.clone())
    }
}

/// Blocks of examples written one after another to a file, removed with it.
pub struct ExampleFile {
    path: PathBuf,
    out: BufWriter<File>,
    /// Where each block starts and how many bytes it takes.
    blocks: Vec<(u64, u64)>,
    end: u64,
    len: usize,
}

impl ExampleFile {
    /// An empty file at `path`, replacing what is there.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let out = BufWriter::new(File::create(&path)?);
        Ok(Self {
            path,
            out,
            blocks: Vec::new(),
            end: 0,
            len: 0,
        })
    }

    /// Appends `examples` as a block.
    pub fn push(&mut self, examples: &Examples) -> io::Result<()> {
        let mut bytes = Vec::new();
        bytes.extend((examples.names.len() as u64).to_le_bytes());
        bytes.extend(&examples.names);
        for array in [
            &examples.keys,
            &examples.candidate_ends,
            &examples.example_ends,
            &examples.correct,
        ] {
            bytes.extend((array.len() as u64).to_le_bytes());
            bytes.extend(array.iter().flat_map(|v| v.to_le_bytes()));
        }
        self.out.write_all(&bytes)?;
        self.out.flush()?;
        self.blocks.push((self.end, bytes.len() as u64));
        self.end += bytes.len() as u64;
        self.len += examples.len();
        Ok(())
    }

    /// Where the blocks are written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many examples the blocks hold.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The examples are only for the training at hand, and run to gigabytes.
impl Drop for ExampleFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Blocks for ExampleFile {
    fn count(&self) -> usize {
        self.blocks.len()
    }

    fn load(&self, i: usize) -> io::Result<Examples> {
        let (start, size) = self.blocks[i];
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![0u8; size as usize];
        file.read_exact(&mut bytes)?;
        let malformed = || io::Error::new(io::ErrorKind::InvalidData, "a malformed block");
        let head = bytes.get(..8).ok_or_else(malformed)?;
        let n = u64::from_le_bytes(head.try_into().expect("8 bytes")) as usize;
        let names = bytes.get(8..8 + n).ok_or_else(malformed)?.to_vec();
        let mut at = 8 + n;
        let mut array = || -> io::Result<Vec<u32>> {
            let head = bytes.get(at..at + 8).ok_or_else(malformed)?;
            let n = u64::from_le_bytes(head.try_into().expect("8 bytes")) as usize;
            let body = bytes.get(at + 8..at + 8 + n * 4).ok_or_else(malformed)?;
            at += 8 + n * 4;
            Ok(body
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect())
        };
        let keys = array()?;
        if keys.len() != names.len() {
            return Err(malformed());
        }
        Ok(Examples {
            keys,
            names,
            candidate_ends: array()?,
            example_ends: array()?,
            correct: array()?,
        })
    }
}

/// `0..n` in a fixed shuffled order: neighbours come apart without
/// randomness, so the same examples always give the same weights.
fn shuffled(n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| {
        (i as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
    });
    order
}

/// What training learned: a weight and the tag of the feature learned at
/// each of `2^bits` places.
#[derive(Debug, Clone, PartialEq)]
pub struct Trained {
    pub weights: Vec<f32>,
    pub tags: Vec<u8>,
}

/// The tag of the feature learned at each of `2^bits` places: at each place,
/// the tag most of its features have, when they have it at least
/// `min_count` times; the place of each name's unknown feature holds its
/// tag whatever else falls there. A feature of another tag, or of too few,
/// is learned as its name's unknown feature, as Kanaemi reads it.
pub fn owners(blocks: &impl Blocks, bits: u8, min_count: u32) -> io::Result<Vec<u8>> {
    // A majority vote in one pass, and a count of the winner in another: a
    // tag that most features of a place have wins it.
    let mut votes: Vec<(u8, u32)> = vec![(EMPTY_TAG, 0); 1 << bits];
    for b in 0..blocks.count() {
        for &key in &blocks.load(b)?.keys {
            let (tag, n) = &mut votes[key_index(key, bits)];
            if *n == 0 {
                *tag = key_tag(key);
                *n = 1;
            } else if *tag == key_tag(key) {
                *n += 1;
            } else {
                *n -= 1;
            }
        }
    }
    let mut counts = vec![0u32; 1 << bits];
    for b in 0..blocks.count() {
        for &key in &blocks.load(b)?.keys {
            let at = key_index(key, bits);
            if votes[at].0 == key_tag(key) {
                counts[at] += 1;
            }
        }
    }
    let mut tags: Vec<u8> = votes
        .iter()
        .zip(&counts)
        .map(|(&(tag, _), &n)| if n >= min_count { tag } else { EMPTY_TAG })
        .collect();
    for name in 0..FEATURE_NAMES.len() as u8 {
        let key = unknown_key(name);
        tags[key_index(key, bits)] = key_tag(key);
    }
    Ok(tags)
}

/// Weights at `bits` that rank each example's surface first, by `epochs` of
/// AdaGrad on the softmax loss over its candidates, each feature learned at
/// its place when its tag owns it ([`owners`] with `min_count`) and as its
/// name's unknown feature otherwise. Each epoch takes the blocks in a fixed
/// shuffled order and the examples of each block likewise, loading one block
/// at a time.
pub fn train(
    blocks: &impl Blocks,
    bits: u8,
    epochs: usize,
    rate: f32,
    min_count: u32,
) -> io::Result<Trained> {
    let tags = owners(blocks, bits, min_count)?;
    let place = |f: FeatureKey| {
        let at = key_index(f.key, bits);
        if tags[at] == key_tag(f.key) {
            at
        } else {
            key_index(unknown_key(f.name), bits)
        }
    };
    let mut weights = vec![0f32; 1 << bits];
    let mut squares = vec![0f32; 1 << bits];
    for _ in 0..epochs {
        for b in shuffled(blocks.count()) {
            let examples = blocks.load(b)?;
            for i in shuffled(examples.len()) {
                let correct = examples.correct(i);
                let scores: Vec<f32> = examples
                    .candidates(i)
                    .map(|c| c.features().map(|f| weights[place(f)]).sum())
                    .collect();
                let top = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let exps: Vec<f32> = scores.iter().map(|s| (s - top).exp()).collect();
                let total: f32 = exps.iter().sum();
                // A feature on several candidates takes the sum of their
                // gradients, so one every candidate shares cancels out
                // instead of drifting.
                let mut gradients: Vec<(usize, f32)> = Vec::new();
                for (c, candidate) in examples.candidates(i).enumerate() {
                    let gradient = exps[c] / total - if c == correct { 1.0 } else { 0.0 };
                    gradients.extend(candidate.features().map(|f| (place(f), gradient)));
                }
                gradients.sort_unstable_by_key(|&(at, _)| at);
                for run in gradients.chunk_by(|a, b| a.0 == b.0) {
                    let at = run[0].0;
                    let gradient: f32 = run.iter().map(|&(_, g)| g).sum();
                    if gradient.abs() < 1e-6 {
                        continue;
                    }
                    squares[at] += gradient * gradient;
                    weights[at] -= rate * gradient / (squares[at].sqrt() + 1e-6);
                }
            }
        }
    }
    Ok(Trained { weights, tags })
}

const MODEL_MAGIC: &[u8; 8] = b"KANAEMIM";
/// The weight type of a model file whose weights are bytes times a scale.
const MODEL_I8: u8 = 1;

/// A model file of Kanaemi's format with what was `trained`, `2^bits`
/// weights and tags, the weights as bytes times a scale that maps the
/// largest in magnitude to 127.
pub fn model_file(bits: u8, trained: &Trained) -> Vec<u8> {
    let Trained { weights, tags } = trained;
    let largest = weights.iter().fold(0f32, |m, w| m.max(w.abs()));
    let scale = if largest > 0.0 { largest / 127.0 } else { 1.0 };
    let mut body: Vec<u8> = weights
        .iter()
        .map(|w| (w / scale).round() as i8 as u8)
        .collect();
    body.extend_from_slice(tags);
    let mut out = Vec::with_capacity(32 + body.len());
    out.extend_from_slice(MODEL_MAGIC);
    out.extend_from_slice(&MODEL_FORMAT_VERSION.to_le_bytes());
    out.push(bits);
    out.push(MODEL_I8);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&scale.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kanaemi_engine::{RankingModel, TextDictionary};

    use super::*;
    use crate::{Numeric, engine};

    fn unit(position: usize, reading: &str, surface: &str) -> Unit {
        Unit {
            compound: false,
            okurigana_variant: false,
            parts: Vec::new(),
            doc_id: "d".into(),
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

    /// The key Kanaemi gives `feature`, its name and values joined by
    /// U+001F.
    fn key(feature: &str) -> u32 {
        xxhash_rust::xxh3::xxh3_64(feature.as_bytes()) as u32
    }

    /// Features of the keys, all of the first name.
    fn fs(keys: &[u32]) -> Vec<FeatureKey> {
        keys.iter()
            .map(|&key| FeatureKey { key, name: 0 })
            .collect()
    }

    fn engine_of(text: &str) -> Engine {
        let (dictionary, invalid) = TextDictionary::parse(text);
        assert!(invalid.is_empty(), "{invalid:?}");
        engine(Arc::new(dictionary))
    }

    #[test]
    fn each_unit_is_an_example_in_its_field_and_in_an_empty_field() {
        let engine = engine_of("かん\t缶\t\t5\nかん\t漢\t\t10\nてがみ\t手紙\t\t1\n");
        let units = [
            unit(2, "かん", "漢"),
            unit(4, "てがみ", "手紙"),
            unit(7, "かん", "漢"),
        ];

        let examples = examples_of_document(&engine, &units, "この漢と手紙と漢");

        assert_eq!(examples.len(), 4, "手紙 has no rival");
        let correct_of = |i: usize| {
            examples
                .candidates(i)
                .nth(examples.correct(i))
                .unwrap()
                .keys
        };
        let in_field = correct_of(2);
        assert!(in_field.contains(&key("s\u{1f}漢")));
        assert!(
            in_field.contains(&key("hl\u{1f}1")),
            "漢 was committed for かん before"
        );
        assert!(in_field.contains(&key("p\u{1f}手紙\u{1f}漢")));
        assert!(in_field.contains(&key("a\u{1f}と\u{1f}漢")));
        assert!(!correct_of(0).contains(&key("hl\u{1f}1")));
        let in_empty_field = correct_of(3);
        assert!(!in_empty_field.contains(&key("hl\u{1f}1")));
        assert!(!in_empty_field.contains(&key("a\u{1f}と\u{1f}漢")));
    }

    fn conjugated(position: usize, reading: &str, surface: &str, stem: (&str, &str)) -> Unit {
        Unit {
            stem_reading: Some(stem.0.into()),
            stem_surface: Some(stem.1.into()),
            conjugation: Some("五段-カ行".into()),
            ..unit(position, reading, surface)
        }
    }

    fn typing(reading: &str, okurigana: Option<&str>, expected: &str) -> Query {
        Query {
            reading: reading.into(),
            okurigana: okurigana.map(str::to_owned),
            expected: expected.into(),
        }
    }

    #[test]
    fn a_conjugated_form_is_also_typed_with_all_its_okurigana_but_never_whole() {
        let form = conjugated(0, "かいた", "書いた", ("か", "書"));
        assert_eq!(
            other_typings(&form, &query_of(&form)),
            [typing("かいた", Some("いた"), "書いた")]
        );
        let terminal = conjugated(0, "かく", "書く", ("か", "書"));
        assert_eq!(
            other_typings(&terminal, &query_of(&terminal)),
            [],
            "く alone is the first kana already"
        );
        let word = unit(0, "てがみ", "手紙");
        assert_eq!(other_typings(&word, &query_of(&word)), []);
    }

    #[test]
    fn every_typing_of_a_conjugated_form_teaches_it_first_but_one_commit_goes_into_the_history() {
        let engine = engine_of(
            "か\t書\t五段-カ行\t5\nか\t買\t五段-ワア行\t9\nかいた\t海田\t\t3\nかん\t缶\t\t5\nかん\t漢\t\t10\n",
        );
        let units = [
            conjugated(0, "かいた", "書いた", ("か", "書")),
            unit(3, "かん", "漢"),
        ];

        let examples = examples_of_document(&engine, &units, "書いた漢");

        assert_eq!(examples.len(), 6, "2 typings of 書いた and 漢, each twice");
        let correct_of = |i: usize| {
            examples
                .candidates(i)
                .nth(examples.correct(i))
                .unwrap()
                .keys
        };
        assert!(correct_of(0).contains(&key("rs\u{1f}かいた\u{1f}書いた")));
        assert!(correct_of(2).contains(&key("rs\u{1f}かい\u{1f}書い")));
        assert!(
            correct_of(4).contains(&key("hc\u{1f}1\u{1f}漢")),
            "one commit before 漢"
        );
    }

    #[test]
    fn a_unit_whose_surface_is_no_candidate_is_no_example() {
        let engine = engine_of("かん\t缶\t\t5\nかん\t漢\t\t10\n");

        let examples = examples_of_document(&engine, &[unit(0, "かん", "幹")], "幹");

        assert!(examples.is_empty());
    }

    fn numeric_unit(reading: &str, surface: &str, numeric: (&str, &str, &str)) -> Unit {
        Unit {
            numeric: Some(Numeric {
                reading: numeric.0.into(),
                surface: numeric.1.into(),
                value: reading.chars().take_while(char::is_ascii_digit).collect(),
                kana: numeric.2.into(),
            }),
            ..unit(0, reading, surface)
        }
    }

    #[test]
    fn a_fragment_inside_a_compound_is_not_typed_alone() {
        let units = [
            unit(0, "きき", "危機"),
            unit(2, "いっぱつ", "一髪"),
            unit(10, "いっぱつ", "一髪"),
        ];
        let fragments = HashSet::from(["一髪".to_owned()]);

        let compound = Unit {
            compound: true,
            ..unit(0, "ききいっぱつ", "危機一髪")
        };

        let typed = typed_units(&units, &[compound], &fragments, &Established::default());

        assert_eq!(typed, [units[0].clone(), units[2].clone()]);
    }

    #[test]
    fn a_fragment_split_out_of_a_rare_joined_word_inside_a_compound_is_not_typed_alone() {
        let joined = Unit {
            parts: vec![crate::Part {
                offset: 0,
                reading: "いっぱつ".into(),
                surface: "一髪".into(),
                ..Default::default()
            }],
            ..unit(2, "いっぱつら", "一髪ら")
        };
        let fragments = HashSet::from(["一髪".to_owned()]);
        let compound = Unit {
            compound: true,
            ..unit(0, "ききいっぱつ", "危機一髪")
        };

        let typed = typed_units(&[joined], &[compound], &fragments, &Established::default());

        assert_eq!(typed, []);
    }

    #[test]
    fn a_number_written_in_kanji_is_also_typed_in_kana() {
        let kanji = numeric_unit("2ほん", "二本", ("{}ほん", "{kanji}本", "にほん"));
        assert_eq!(
            other_typings(&kanji, &query_of(&kanji)),
            [typing("にほん", None, "二本")]
        );
        let digits = numeric_unit("2ほん", "2本", ("{}ほん", "{}本", "にほん"));
        assert_eq!(other_typings(&digits, &query_of(&digits)), []);
    }

    #[test]
    fn a_word_of_a_small_number_typed_in_kana_is_taught_first() {
        let engine = engine_of("にほん\t日本\t\t5\nにほん\t二本\t\t9\n{}ほん\t{kanji}本\t\t5\n");
        let units = [numeric_unit(
            "2ほん",
            "二本",
            ("{}ほん", "{kanji}本", "にほん"),
        )];

        let examples = examples_of_document(&engine, &units, "二本");

        let correct_of = |i: usize| {
            examples
                .candidates(i)
                .nth(examples.correct(i))
                .unwrap()
                .keys
        };
        assert!(correct_of(0).contains(&key("rs\u{1f}にほん\u{1f}二本")));
    }

    #[test]
    fn a_numeric_unit_goes_into_the_history_as_its_numeric_item() {
        let engine = engine_of("{}ほん\t{}本\t\t5\n{}ほん\t{}品\t\t9\n");
        let numeric = Unit {
            numeric: Some(Numeric {
                reading: "{}ほん".into(),
                surface: "{}本".into(),
                value: "3".into(),
                kana: String::new(),
            }),
            ..unit(0, "3ほん", "3本")
        };
        let units = [
            numeric.clone(),
            Unit {
                position: 3,
                ..numeric
            },
        ];

        let examples = examples_of_document(&engine, &units, "3本と3本");

        let second = examples
            .candidates(2)
            .nth(examples.correct(2))
            .unwrap()
            .keys;
        assert!(
            second.contains(&key("p\u{1f}\u{fdd0}\u{fdd1}本\u{1f}\u{fdd0}\u{fdd1}本")),
            "the history and the candidate both go by 本's item, as Kanaemi records it"
        );
    }

    #[test]
    fn features_every_candidate_has_as_often_are_not_kept() {
        let mut examples = Examples::default();
        examples.push([fs(&[7, 1, 9, 9]), fs(&[9, 2, 7]), fs(&[3, 7, 9])], 2);

        assert_eq!(
            examples.candidates(0).map(|c| c.keys).collect::<Vec<_>>(),
            [&[1, 9][..], &[2][..], &[3][..]]
        );
    }

    #[test]
    fn appended_examples_keep_their_candidates() {
        let mut a = Examples::default();
        a.push([fs(&[1]), fs(&[2, 3])], 1);
        let mut b = Examples::default();
        b.push([fs(&[4, 5]), fs(&[6])], 0);

        a.append(b);

        assert_eq!(a.len(), 2);
        assert_eq!(
            a.candidates(1).map(|c| c.keys).collect::<Vec<_>>(),
            [&[4, 5][..], &[6][..]]
        );
        assert_eq!(a.correct(1), 0);
    }

    fn scratch_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "kanaemi-dict-ranking-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn an_example_file_gives_back_each_block_and_goes_with_it() {
        let path = scratch_file("blocks");
        let mut first = Examples::default();
        first.push([fs(&[1]), fs(&[2, 3])], 1);
        let mut second = Examples::default();
        second.push([fs(&[4, 5]), fs(&[6])], 0);
        second.push([fs(&[7]), fs(&[8])], 1);

        let mut file = ExampleFile::create(&path).unwrap();
        file.push(&first).unwrap();
        file.push(&second).unwrap();

        assert_eq!(file.len(), 3);
        assert_eq!(file.count(), 2);
        assert_eq!(file.load(1).unwrap(), second);
        assert_eq!(file.load(0).unwrap(), first);
        drop(file);
        assert!(!path.exists());
    }

    #[test]
    fn training_from_a_file_learns_as_from_memory() {
        let path = scratch_file("train");
        let mut examples = Examples::default();
        examples.push([fs(&[1, 9]), fs(&[2, 8])], 0);
        examples.push([fs(&[2, 8]), fs(&[1, 7])], 1);
        let mut file = ExampleFile::create(&path).unwrap();
        file.push(&examples).unwrap();

        assert_eq!(
            train(&file, 10, 3, 0.5, 1).unwrap(),
            train(&examples, 10, 3, 0.5, 1).unwrap()
        );
    }

    #[test]
    fn training_puts_each_surface_first() {
        let mut examples = Examples::default();
        examples.push([fs(&[1, 9]), fs(&[2, 9])], 0);
        examples.push([fs(&[2, 8]), fs(&[1, 8])], 1);

        let weights = train(&examples, 10, 5, 0.5, 1).unwrap().weights;

        assert!(weights[1] > weights[2]);
    }

    #[test]
    fn a_feature_every_candidate_shares_learns_nothing() {
        let mut examples = Examples::default();
        examples.push([fs(&[1, 7]), fs(&[2, 7])], 0);

        let weights = train(&examples, 10, 3, 0.5, 1).unwrap().weights;

        assert_eq!(weights[7], 0.0);
    }

    #[test]
    fn a_place_goes_to_the_tag_most_of_its_features_have_often_enough() {
        // Another feature at place 1 at 10 bits, with tag 2 where 1 has 1.
        let other = (2 << 24) | 1;
        let mut examples = Examples::default();
        examples.push([fs(&[1, 5]), fs(&[other, 6])], 0);
        examples.push([fs(&[1, 6]), fs(&[5])], 0);
        examples.push([fs(&[1]), fs(&[6])], 0);

        let tags = owners(&examples, 10, 2).unwrap();

        assert_eq!(tags[1], key_tag(1), "1 thrice against the other once");
        assert_eq!(tags[5], key_tag(5));
        assert_eq!(tags[6], key_tag(6));
        assert_eq!(tags[7], EMPTY_TAG);
        let tags = owners(&examples, 10, 4).unwrap();
        assert_eq!(tags[1], EMPTY_TAG, "too few to learn");
    }

    #[test]
    fn the_place_of_every_unknown_feature_is_its_own() {
        let tags = owners(&Examples::default(), 10, 1).unwrap();
        for name in 0..FEATURE_NAMES.len() as u8 {
            let key = unknown_key(name);
            assert_eq!(tags[key_index(key, 10)], key_tag(key));
        }
    }

    #[test]
    fn a_feature_too_rare_to_learn_is_learned_as_its_name_s_unknown_feature() {
        let rare = |key: u32| FeatureKey { key, name: 0 };
        let common = FeatureKey { key: 9, name: 1 };
        let mut examples = Examples::default();
        for key in [3, 4, 5] {
            examples.push([vec![rare(key)], vec![common]], 0);
        }

        let trained = train(&examples, 10, 3, 0.5, 2).unwrap();

        assert_eq!(trained.tags[3], EMPTY_TAG);
        assert_eq!(trained.weights[3], 0.0);
        assert!(trained.weights[key_index(unknown_key(0), 10)] > 0.0);
        assert!(trained.weights[9] < 0.0);
    }

    fn trained(weights: Vec<f32>) -> Trained {
        let tags = vec![EMPTY_TAG; weights.len()];
        Trained { weights, tags }
    }

    #[test]
    fn the_model_file_holds_the_weights_as_bytes_scaled_to_the_largest_and_their_tags() {
        let mut weights = vec![0.0f32; 1 << 10];
        weights[..3].copy_from_slice(&[0.5, -2.0, 1.0]);
        let mut model = trained(weights);
        model.tags[2] = 7;

        let bytes = model_file(10, &model);

        assert_eq!(&bytes[..8], b"KANAEMIM");
        assert_eq!(&bytes[8..12], &MODEL_FORMAT_VERSION.to_le_bytes());
        assert_eq!(bytes[12..14], [10, 1]);
        assert_eq!(
            f32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            2.0 / 127.0
        );
        assert_eq!(bytes[32..35], [32, (-127i8) as u8, 64]);
        assert_eq!(bytes[32 + (1 << 10) + 2], 7);
        assert_eq!(bytes.len(), 32 + (2 << 10));
    }

    #[test]
    fn kanaemi_reads_the_model_file() {
        let path = scratch_file("model");
        std::fs::write(&path, model_file(10, &trained(vec![0.25; 1 << 10]))).unwrap();

        let opened = RankingModel::open(&path);

        std::fs::remove_file(&path).unwrap();
        assert!(opened.is_ok(), "{:?}", opened.err());
    }
}

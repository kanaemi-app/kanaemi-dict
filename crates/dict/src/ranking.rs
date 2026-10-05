//! Training Kanaemi's ranking model: examples from the units of documents, a
//! softmax over each unit's candidates, and the weights written as Kanaemi's
//! model file.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kanaemi_engine::{CONTEXT_CHARS, Engine, HISTORY_LEN, RankingInput, feature_indices};

use crate::{Unit, query_of};

/// The widest hash a model file allows.
pub const MAX_BITS: u8 = 28;

/// Training examples, flat: each is a unit's candidates, each candidate the
/// indices of its features at [`MAX_BITS`], and which candidate is the unit's
/// surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Examples {
    features: Vec<u32>,
    candidate_ends: Vec<u32>,
    example_ends: Vec<u32>,
    correct: Vec<u32>,
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
    pub fn push(&mut self, candidates: impl IntoIterator<Item = Vec<u32>>, correct: usize) {
        let candidates: Vec<Vec<u32>> = candidates.into_iter().collect();
        let mut shared: HashMap<u32, usize> = HashMap::new();
        if let Some((first, rest)) = candidates.split_first() {
            for &f in first {
                *shared.entry(f).or_default() += 1;
            }
            for features in rest {
                let mut counts: HashMap<u32, usize> = HashMap::new();
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
            self.features
                .extend(features.into_iter().filter(|f| match left.get_mut(f) {
                    Some(n) if *n > 0 => {
                        *n -= 1;
                        false
                    }
                    _ => true,
                }));
            self.candidate_ends.push(self.features.len() as u32);
        }
        self.example_ends.push(self.candidate_ends.len() as u32);
        self.correct.push(correct as u32);
    }

    pub fn append(&mut self, other: Examples) {
        let features = self.features.len() as u32;
        let candidates = self.candidate_ends.len() as u32;
        self.features.extend(other.features);
        self.candidate_ends
            .extend(other.candidate_ends.into_iter().map(|e| e + features));
        self.example_ends
            .extend(other.example_ends.into_iter().map(|e| e + candidates));
        self.correct.extend(other.correct);
    }

    /// The candidates of the `i`th example, each the indices of its features.
    pub fn candidates(&self, i: usize) -> impl Iterator<Item = &[u32]> {
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
            &self.features[from..self.candidate_ends[c] as usize]
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
/// more into an empty field. Units with a single candidate or without their
/// surface among the candidates teach nothing and are left out.
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
        let candidates = engine.candidate_facts(&query.reading, query.okurigana.as_deref());
        let correct = candidates.iter().position(|c| c.surface == query.expected);
        if let (Some(correct), true) = (correct, candidates.len() > 1) {
            for (history, context) in [(&history[..], context.as_str()), (&[][..], "")] {
                let input = RankingInput {
                    reading: &query.reading,
                    history,
                    context,
                };
                examples.push(
                    candidates.iter().map(|c| {
                        feature_indices(&input, c, MAX_BITS)
                            .into_iter()
                            .map(|f| f as u32)
                            .collect()
                    }),
                    correct,
                );
            }
        }
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
        for array in [
            &examples.features,
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
        let mut at = 0;
        let mut array = || -> io::Result<Vec<u32>> {
            let malformed = || io::Error::new(io::ErrorKind::InvalidData, "a malformed block");
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
        Ok(Examples {
            features: array()?,
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

/// Weights at `bits` that rank each example's surface first, by `epochs` of
/// AdaGrad on the softmax loss over its candidates. Each epoch takes the
/// blocks in a fixed shuffled order and the examples of each block likewise,
/// loading one block at a time.
pub fn train(blocks: &impl Blocks, bits: u8, epochs: usize, rate: f32) -> io::Result<Vec<f32>> {
    let mask = (1u32 << bits) - 1;
    let mut weights = vec![0f32; 1 << bits];
    let mut squares = vec![0f32; 1 << bits];
    for _ in 0..epochs {
        for b in shuffled(blocks.count()) {
            let examples = blocks.load(b)?;
            for i in shuffled(examples.len()) {
                let correct = examples.correct(i);
                let scores: Vec<f32> = examples
                    .candidates(i)
                    .map(|fs| fs.iter().map(|&f| weights[(f & mask) as usize]).sum())
                    .collect();
                let top = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let exps: Vec<f32> = scores.iter().map(|s| (s - top).exp()).collect();
                let total: f32 = exps.iter().sum();
                // A feature on several candidates takes the sum of their
                // gradients, so one every candidate shares cancels out
                // instead of drifting.
                let mut gradients: Vec<(usize, f32)> = Vec::new();
                for (c, fs) in examples.candidates(i).enumerate() {
                    let gradient = exps[c] / total - if c == correct { 1.0 } else { 0.0 };
                    gradients.extend(fs.iter().map(|&f| ((f & mask) as usize, gradient)));
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
    Ok(weights)
}

const MODEL_MAGIC: &[u8; 8] = b"KANAEMIM";
const MODEL_VERSION: u32 = 3;
/// The weight type of a model file whose weights are bytes times a scale.
const MODEL_I8: u8 = 1;

/// A model file of Kanaemi's format with `weights`, `2^bits` of them, as
/// bytes times a scale that maps the largest in magnitude to 127.
pub fn model_file(bits: u8, weights: &[f32]) -> Vec<u8> {
    let largest = weights.iter().fold(0f32, |m, w| m.max(w.abs()));
    let scale = if largest > 0.0 { largest / 127.0 } else { 1.0 };
    let body: Vec<u8> = weights
        .iter()
        .map(|w| (w / scale).round() as i8 as u8)
        .collect();
    let mut out = Vec::with_capacity(32 + body.len());
    out.extend_from_slice(MODEL_MAGIC);
    out.extend_from_slice(&MODEL_VERSION.to_le_bytes());
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

    /// Where Kanaemi puts the weight of `feature`, its name and values joined
    /// by U+001F, at [`MAX_BITS`].
    fn index(feature: &str) -> u32 {
        (xxhash_rust::xxh3::xxh3_64(feature.as_bytes()) & ((1 << MAX_BITS) - 1)) as u32
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
        let correct_of = |i: usize| examples.candidates(i).nth(examples.correct(i)).unwrap();
        let in_field = correct_of(2);
        assert!(in_field.contains(&index("s\u{1f}漢")));
        assert!(
            in_field.contains(&index("hl\u{1f}1")),
            "漢 was committed for かん before"
        );
        assert!(in_field.contains(&index("p\u{1f}手紙\u{1f}漢")));
        assert!(in_field.contains(&index("a\u{1f}と\u{1f}漢")));
        assert!(!correct_of(0).contains(&index("hl\u{1f}1")));
        let in_empty_field = correct_of(3);
        assert!(!in_empty_field.contains(&index("hl\u{1f}1")));
        assert!(!in_empty_field.contains(&index("a\u{1f}と\u{1f}漢")));
    }

    #[test]
    fn a_unit_whose_surface_is_no_candidate_is_no_example() {
        let engine = engine_of("かん\t缶\t\t5\nかん\t漢\t\t10\n");

        let examples = examples_of_document(&engine, &[unit(0, "かん", "幹")], "幹");

        assert!(examples.is_empty());
    }

    #[test]
    fn a_numeric_unit_goes_into_the_history_as_its_numeric_item() {
        let engine = engine_of("{}ほん\t{}本\t\t5\n{}ほん\t{}品\t\t9\n");
        let numeric = Unit {
            numeric: Some(Numeric {
                reading: "{}ほん".into(),
                surface: "{}本".into(),
                value: "3".into(),
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

        let second = examples.candidates(2).nth(examples.correct(2)).unwrap();
        assert!(
            second.contains(&index("p\u{1f}\u{fdd0}\u{fdd1}本\u{1f}\u{fdd0}\u{fdd1}本")),
            "the history and the candidate both go by 本's item, as Kanaemi records it"
        );
    }

    #[test]
    fn features_every_candidate_has_as_often_are_not_kept() {
        let mut examples = Examples::default();
        examples.push([vec![7, 1, 9, 9], vec![9, 2, 7], vec![3, 7, 9]], 2);

        assert_eq!(
            examples.candidates(0).collect::<Vec<_>>(),
            [&[1, 9][..], &[2][..], &[3][..]]
        );
    }

    #[test]
    fn appended_examples_keep_their_candidates() {
        let mut a = Examples::default();
        a.push([vec![1], vec![2, 3]], 1);
        let mut b = Examples::default();
        b.push([vec![4, 5], vec![6]], 0);

        a.append(b);

        assert_eq!(a.len(), 2);
        assert_eq!(a.candidates(1).collect::<Vec<_>>(), [&[4, 5][..], &[6][..]]);
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
        first.push([vec![1], vec![2, 3]], 1);
        let mut second = Examples::default();
        second.push([vec![4, 5], vec![6]], 0);
        second.push([vec![7], vec![8]], 1);

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
        examples.push([vec![1, 9], vec![2, 8]], 0);
        examples.push([vec![2, 8], vec![1, 7]], 1);
        let mut file = ExampleFile::create(&path).unwrap();
        file.push(&examples).unwrap();

        assert_eq!(
            train(&file, 10, 3, 0.5).unwrap(),
            train(&examples, 10, 3, 0.5).unwrap()
        );
    }

    #[test]
    fn training_puts_each_surface_first() {
        let mut examples = Examples::default();
        examples.push([vec![1, 9], vec![2, 9]], 0);
        examples.push([vec![2, 8], vec![1, 8]], 1);

        let weights = train(&examples, 10, 5, 0.5).unwrap();

        assert!(weights[1] > weights[2]);
    }

    #[test]
    fn a_feature_every_candidate_shares_learns_nothing() {
        let mut examples = Examples::default();
        examples.push([vec![1, 7], vec![2, 7]], 0);

        let weights = train(&examples, 10, 3, 0.5).unwrap();

        assert_eq!(weights[7], 0.0);
    }

    #[test]
    fn the_model_file_holds_the_weights_as_bytes_scaled_to_the_largest() {
        let mut weights = vec![0.0f32; 1 << 10];
        weights[..3].copy_from_slice(&[0.5, -2.0, 1.0]);

        let bytes = model_file(10, &weights);

        assert_eq!(&bytes[..8], b"KANAEMIM");
        assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
        assert_eq!(bytes[12..14], [10, 1]);
        assert_eq!(
            f32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            2.0 / 127.0
        );
        assert_eq!(bytes[32..35], [32, (-127i8) as u8, 64]);
        assert_eq!(bytes.len(), 32 + (1 << 10));
    }

    #[test]
    fn kanaemi_reads_the_model_file() {
        let path = scratch_file("model");
        std::fs::write(&path, model_file(10, &vec![0.25; 1 << 10])).unwrap();

        let opened = RankingModel::open(&path);

        std::fs::remove_file(&path).unwrap();
        assert!(opened.is_ok(), "{:?}", opened.err());
    }
}

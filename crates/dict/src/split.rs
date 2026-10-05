//! Splitting the documents into those the dictionaries are built from, those
//! values are compared on, and those measured once at the end.

use sha2::{Digest, Sha256};

/// The part of the documents a document belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Split {
    /// Built from.
    Train,
    /// Compared on, while choosing values.
    Dev,
    /// Measured once the values are chosen.
    Eval,
}

/// The split of a document, from the first eight bytes of the SHA-256 of its
/// ID read as a big-endian number, modulo 100: 0 is dev, 1 to 9 eval, and the
/// rest train.
pub fn split_of(doc_id: impl AsRef<str>) -> Split {
    let digest = Sha256::digest(doc_id.as_ref().as_bytes());
    let head: [u8; 8] = digest[..8].try_into().expect("a SHA-256 has 32 bytes");
    match u64::from_be_bytes(head) % 100 {
        0 => Split::Dev,
        1..=9 => Split::Eval,
        _ => Split::Train,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The remainders were computed apart from this code, with Python's hashlib.
    #[test]
    fn a_remainder_of_zero_is_dev() {
        assert_eq!(split_of("aozora:000013"), Split::Dev); // 0
    }

    #[test]
    fn remainders_from_one_to_nine_are_eval() {
        assert_eq!(split_of("aozora:000004"), Split::Eval); // 1
        assert_eq!(
            split_of("law:123AC0000000001_20200101_000000000000000"),
            Split::Eval
        ); // 4
        assert_eq!(split_of("aozora:000015"), Split::Eval); // 9
    }

    #[test]
    fn remainders_from_ten_are_train() {
        assert_eq!(split_of("aozora:000054"), Split::Train); // 10
        assert_eq!(split_of("aozora:000001"), Split::Train); // 50
        assert_eq!(split_of("pydocs:tutorial/index.html"), Split::Train); // 82
        assert_eq!(split_of("aozora:000155"), Split::Train); // 99
    }
}

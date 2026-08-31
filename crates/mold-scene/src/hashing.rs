use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};

/// A fast hasher for the short, fixed keys the scene looks up constantly.
///
/// Property names are one to twenty characters and every read of every node in
/// every frame goes through one: layout alone asks for about a dozen per node.
/// The standard hasher is SipHash, chosen to be hard to collide deliberately —
/// a property table built from a compile-time schema has no such exposure, and
/// pays about twenty nanoseconds a lookup for the protection. This is the
/// multiply-xor construction Rust's own compiler uses for the same reason.
#[derive(Default)]
pub struct FastHasher {
    pub(crate) hash: u64,
}

pub(crate) const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    pub(crate) fn add(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(5) ^ value).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while let Some((chunk, tail)) = rest.split_at_checked(8) {
            self.add(u64::from_ne_bytes(chunk.try_into().expect("eight bytes")));
            rest = tail;
        }
        for &byte in rest {
            self.add(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// Builds [`FastHasher`]s for the scene's own tables.
#[derive(Clone, Copy, Default)]
pub struct FastHash;

impl BuildHasher for FastHash {
    type Hasher = FastHasher;

    fn build_hasher(&self) -> FastHasher {
        FastHasher::default()
    }
}

/// A map keyed by something the scene controls, hashed cheaply.
pub type FastMap<K, V> = HashMap<K, V, FastHash>;

/// A set keyed by something the scene controls, hashed cheaply.
pub type FastSet<K> = HashSet<K, FastHash>;

//! Filename-resolution memo-cache backing the `StormArchive__FindFileEntry` hook.
//!
//! (weirdperformance parity). The stock resolver re-derives every answer
//! from scratch — per-character normalize, the Storm `0x7FED7FED` crypt-table
//! hash, an open-addressing probe over the archive hash table, and a walk (or
//! chain fan-out) across the open-archive list — all under two critical
//! sections, for every single file open. Archive contents never change while
//! mounted, so the (archive, scope, name) → (found archive, block index)
//! mapping is memoizable; this module is that memo.
//!
//! The cache stores only what the adapter can later replay safely:
//! archive-scoped positive results (the found handle + block index; the
//! adapter revalidates the handle against the live open-archive list before
//! trusting it) and global negative results ("no open archive has this
//! file"). Everything else — pseudo-name resolutions (return code 2), deleted
//! entries (3), odd out-pointer shapes — stays a miss and delegates to stock.
//! A miss is always correct; a hit must replay exactly what stock returned for
//! the identical query.
//!
//! Pure and host-testable: no FFI, no statics, no game-memory access. The FFI
//! adapter (`crate::win::hooks`) owns the locking and the live-handle
//! revalidation.

/// Longest cacheable name in bytes.
///
/// Longer names are legal for the game but never cached (mirrors the reference
/// implementation; MPQ paths are far shorter in practice, so the fixed-size
/// entry stays small).
pub const NAME_CAP: usize = 0x80;

/// 32 Ki buckets x 2 ways ≈ 9.5 MiB.
///
/// Sized to hold a full session's distinct lookups (a 1.12 client resolves tens
/// of thousands of names, times the patch-chain fan-out) without growth or
/// rehash on the hot path.
const BUCKET_COUNT: usize = 1 << 15;
const WAYS: usize = 2;

/// A fully-normalized cache key.
///
/// `new` is the only constructor: it applies the same character folding the
/// stock hasher applies (ASCII uppercase, `/` → `\`), so two spellings stock
/// treats as the same file share one entry. ASCII-only folding merges strictly
/// fewer names than stock's CRT `toupper` (which may also fold high-half bytes)
/// — a spelling pair we fail to merge costs one extra miss, never a wrong hit.
pub struct Key {
    hash: u32,
    archive: u32,
    scope: u32,
    len: u8,
    name: [u8; NAME_CAP],
}

impl Key {
    /// Build a key from the raw (unnormalized) name bytes.
    ///
    /// Returns `None` when the name is longer than [`NAME_CAP`] — such lookups
    /// are valid but uncacheable, and the caller falls through to stock.
    #[must_use]
    pub fn new(archive: u32, scope: u32, raw: &[u8]) -> Option<Self> {
        if raw.len() > NAME_CAP {
            return None;
        }
        let mut name = [0u8; NAME_CAP];
        for (dst, &src) in name.iter_mut().zip(raw.iter()) {
            *dst = normalize_byte(src);
        }
        let mut hash = fnv1a(0x811c_9dc5, &name[..raw.len()]);
        // Mix the archive handle and scope into the bucket selector: the chain
        // fan-out issues the same name against every sub-archive, and without
        // this those queries would pile into one 2-way bucket and thrash.
        hash = fnv1a(hash, &archive.to_le_bytes());
        hash = fnv1a(hash, &scope.to_le_bytes());
        Some(Self {
            hash: hash ^ (hash >> 16),
            archive,
            scope,
            len: u8::try_from(raw.len()).ok()?,
            name,
        })
    }

    fn matches(&self, e: &Entry) -> bool {
        e.used
            && e.archive == self.archive
            && e.scope == self.scope
            && e.len == self.len
            && e.name[..self.len as usize] == self.name[..self.len as usize]
    }
}

/// ASCII uppercase + path-separator folding.
///
/// The normalization stock applies per character before hashing.
#[inline]
const fn normalize_byte(b: u8) -> u8 {
    if b == b'/' {
        b'\\'
    } else {
        b.to_ascii_uppercase()
    }
}

#[inline]
fn fnv1a(seed: u32, bytes: &[u8]) -> u32 {
    let mut h = seed;
    for &b in bytes {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h
}

/// A replayable prior answer.
///
/// `negative` means stock returned 0 ("not found") for a global query;
/// otherwise `found`/`block_index` reproduce a scoped positive (return 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    pub negative: bool,
    /// The handle stock wrote to `*outArchive`.
    ///
    /// (the found archive, possibly a redirect of the requested one). Only
    /// meaningful for positives.
    pub found: u32,
    /// Index into the found archive's block table (`[found+0x290]`), stride 0x2c.
    ///
    /// Only meaningful for positives.
    pub block_index: u32,
}

#[derive(Clone, Copy)]
struct Entry {
    used: bool,
    archive: u32,
    scope: u32,
    len: u8,
    name: [u8; NAME_CAP],
    hit: Hit,
}

const EMPTY_ENTRY: Entry = Entry {
    used: false,
    archive: 0,
    scope: 0,
    len: 0,
    name: [0; NAME_CAP],
    hit: Hit {
        negative: false,
        found: 0,
        block_index: 0,
    },
};

#[derive(Clone, Copy)]
struct Bucket {
    ways: [Entry; WAYS],
    /// The way to evict next (flipped away from any way that hits or fills).
    evict_next: u8,
}

const EMPTY_BUCKET: Bucket = Bucket {
    ways: [EMPTY_ENTRY; WAYS],
    evict_next: 0,
};

/// Fixed-capacity, 2-way set-associative memo table.
///
/// All methods are O(ways); nothing allocates after construction.
pub struct FileCache {
    buckets: Box<[Bucket]>,
}

impl FileCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: vec![EMPTY_BUCKET; BUCKET_COUNT].into_boxed_slice(),
        }
    }

    #[inline]
    const fn bucket_index(key: &Key) -> usize {
        (key.hash as usize) & (BUCKET_COUNT - 1)
    }

    /// Look up a prior answer.
    ///
    /// Touches the pseudo-LRU marker on hit; performs no other mutation and
    /// never observes game memory, so the differential shadow path can call it
    /// freely.
    pub fn probe(&mut self, key: &Key) -> Option<Hit> {
        let bucket = &mut self.buckets[Self::bucket_index(key)];
        for (w, entry) in bucket.ways.iter().enumerate() {
            if key.matches(entry) {
                let hit = entry.hit;
                // Keep this way; evict the other next.
                bucket.evict_next = u8::from(w == 0);
                return Some(hit);
            }
        }
        None
    }

    /// Record an answer.
    ///
    /// Overwriting a same-key entry in place, filling an empty way, or evicting
    /// per the bucket's pseudo-LRU marker.
    pub fn insert(&mut self, key: &Key, hit: Hit) {
        let bucket = &mut self.buckets[Self::bucket_index(key)];
        let way = bucket
            .ways
            .iter()
            .position(|e| key.matches(e))
            .or_else(|| bucket.ways.iter().position(|e| !e.used))
            .unwrap_or(bucket.evict_next as usize % WAYS);
        bucket.ways[way] = Entry {
            used: true,
            archive: key.archive,
            scope: key.scope,
            len: key.len,
            name: key.name,
            hit,
        };
        bucket.evict_next = u8::from(way == 0);
    }
}

impl Default for FileCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests_filecache {
    use super::*;

    const POS: Hit = Hit {
        negative: false,
        found: 0x00d4_0000,
        block_index: 7,
    };
    const NEG: Hit = Hit {
        negative: true,
        found: 0,
        block_index: 0,
    };

    #[test]
    fn normalization_folds_case_and_slashes() {
        let a = Key::new(0, 1, b"Interface/GlueXML\\gLuEsTrInGs.lua").unwrap();
        let b = Key::new(0, 1, b"INTERFACE\\GLUEXML\\GLUESTRINGS.LUA").unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.name[..a.len as usize], b.name[..b.len as usize]);
        // High-half bytes are left alone (identity — never merged).
        let c = Key::new(0, 1, &[0xe9, b'a']).unwrap();
        assert_eq!(c.name[0], 0xe9);
        assert_eq!(c.name[1], b'A');
    }

    #[test]
    fn key_dimensions_are_distinct() {
        let mut cache = FileCache::new();
        let name = b"DBFilesClient\\Spell.dbc";
        let scoped = Key::new(0x1234, 0, name).unwrap();
        let other_archive = Key::new(0x5678, 0, name).unwrap();
        let other_scope = Key::new(0x1234, 1, name).unwrap();
        cache.insert(&scoped, POS);
        assert_eq!(cache.probe(&scoped), Some(POS));
        assert_eq!(cache.probe(&other_archive), None);
        assert_eq!(cache.probe(&other_scope), None);
    }

    #[test]
    fn negative_roundtrip() {
        let mut cache = FileCache::new();
        let key = Key::new(0, 0, b"World\\NoSuchFile.blp").unwrap();
        assert_eq!(cache.probe(&key), None);
        cache.insert(&key, NEG);
        assert_eq!(cache.probe(&key), Some(NEG));
    }

    #[test]
    fn same_key_overwrites_in_place() {
        let mut cache = FileCache::new();
        let key = Key::new(0x10, 0, b"a").unwrap();
        cache.insert(&key, POS);
        let newer = Hit {
            block_index: 9,
            ..POS
        };
        cache.insert(&key, newer);
        assert_eq!(cache.probe(&key), Some(newer));
    }

    #[test]
    fn long_names_are_uncacheable() {
        assert!(Key::new(0, 0, &[b'x'; NAME_CAP]).is_some());
        assert!(Key::new(0, 0, &[b'x'; NAME_CAP + 1]).is_none());
    }

    /// Three same-bucket keys through a 2-way bucket.
    ///
    /// The pseudo-LRU keeps the most-recently-touched way and the newest
    /// insert, evicting the stale way.
    #[test]
    fn two_way_eviction_prefers_recently_used() {
        let mut cache = FileCache::new();
        let base = Key::new(0, 0, b"collide").unwrap();
        let target = FileCache::bucket_index(&base);
        // Brute-force two more archives whose keys land in the same bucket.
        let mut colliders = (1u32..).filter_map(|a| {
            let k = Key::new(a, 0, b"collide").unwrap();
            (FileCache::bucket_index(&k) == target).then_some(k)
        });
        let second = colliders.next().unwrap();
        let third = colliders.next().unwrap();

        cache.insert(&base, POS);
        cache.insert(&second, POS);
        // Touch `base` so it is MRU, then overflow the bucket.
        assert_eq!(cache.probe(&base), Some(POS));
        cache.insert(&third, POS);

        assert_eq!(cache.probe(&base), Some(POS), "MRU way survived");
        assert_eq!(cache.probe(&third), Some(POS), "newest insert survived");
        assert_eq!(cache.probe(&second), None, "LRU way evicted");
    }
}

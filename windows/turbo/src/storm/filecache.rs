//! Filename-resolution memo-cache backing the `StormArchive__FindFileEntry` hook.
//!
//! The stock resolver re-derives every answer
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
/// [`Key::populate`] is the only way to fill one, and the name reaches it
/// already folded by [`normalize_byte`] the way the stock hasher folds it
/// (ASCII uppercase, `/` → `\`), so two spellings stock treats as the same file
/// share one entry. ASCII-only folding merges strictly fewer names than stock's
/// CRT `toupper` (which may also fold high-half bytes), and a spelling pair we
/// fail to merge costs one extra miss, never a wrong hit.
pub struct Key {
    hash: u32,
    archive: u32,
    scope: u32,
    len: u8,
    name: [u8; NAME_CAP],
}

impl Key {
    /// An unpopulated key: the storage [`Key::populate`] fills in place.
    ///
    /// The caller owns the storage because a `-> Option<Self>` constructor is
    /// built in a temporary and then moved into the caller's slot, and that
    /// move is a 0x90-byte copy the optimizer keeps. Owning it means the whole
    /// key, name included, is written exactly once.
    pub const EMPTY: Self = Self {
        hash: 0,
        archive: 0,
        scope: 0,
        len: 0,
        name: [0u8; NAME_CAP],
    };

    /// Populate the key from a name written straight into its own buffer.
    ///
    /// `write_name` receives the name array and writes the already
    /// [`normalize_byte`]d bytes into it, answering how many it wrote, so the
    /// normalized name lands in its final slot with no scratch buffer between.
    /// Answers `false` when `write_name` declines or the name is longer than
    /// [`NAME_CAP`], leaving the key unpopulated — such lookups are valid but
    /// uncacheable, and the caller falls through to stock instead of touching
    /// the key again.
    pub fn populate(
        &mut self,
        archive: u32,
        scope: u32,
        write_name: impl FnOnce(&mut [u8; NAME_CAP]) -> Option<usize>,
    ) -> bool {
        let Some(len) = write_name(&mut self.name) else {
            return false;
        };
        if len > NAME_CAP {
            return false;
        }
        let Ok(len_u8) = u8::try_from(len) else {
            return false;
        };
        self.archive = archive;
        self.scope = scope;
        self.len = len_u8;
        let mut hash = fnv1a(0x811c_9dc5, &self.name[..len]);
        // Mix the archive handle and scope into the bucket selector: the chain
        // fan-out issues the same name against every sub-archive, and without
        // this those queries would pile into one 2-way bucket and thrash.
        hash = fnv1a(hash, &archive.to_le_bytes());
        hash = fnv1a(hash, &scope.to_le_bytes());
        self.hash = hash ^ (hash >> 16);
        true
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
/// The normalization stock applies per character before hashing. Public because
/// the adapter folds each byte as its NUL scan copies it into the key's own
/// buffer, which is what keeps the name from being walked a second time.
#[inline]
pub const fn normalize_byte(b: u8) -> u8 {
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

/// `Hit::found` carries the handle stock wrote to `*outArchive`.
pub const KNOWN_FOUND: u8 = 1 << 0;

/// `Hit::found_in` carries the handle stock wrote to `*outFoundIn`.
pub const KNOWN_FOUND_IN: u8 = 1 << 1;

/// `Hit::block_index` locates the record stock wrote to `*outBlockEntry`.
pub const KNOWN_BLOCK: u8 = 1 << 2;

/// A replayable prior answer.
///
/// `negative` means stock returned 0 ("not found") for a global query;
/// otherwise this is a positive (return 1) and `known` says which of the three
/// fields were observed. A caller only receives what it asked for, so a
/// positive is recorded under the out-pointer shape of the call that produced
/// it, and replays only for a call whose shape it covers. A positive with no
/// fields at all is still worth keeping: three of the four call sites want
/// nothing but the return code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    pub negative: bool,
    /// The handle stock wrote to `*outArchive`.
    ///
    /// (the found archive, possibly a redirect of the requested one). Valid
    /// under [`KNOWN_FOUND`].
    pub found: u32,
    /// The handle stock wrote to `*outFoundIn`.
    ///
    /// (the archive the walk matched in, which is the requested one for a
    /// scoped query). Valid under [`KNOWN_FOUND_IN`].
    pub found_in: u32,
    /// Index into the found archive's block table (`[found+0x290]`), stride 0x2c.
    ///
    /// Valid under [`KNOWN_BLOCK`], which implies [`KNOWN_FOUND`] — the base the
    /// index is relative to belongs to the found archive.
    pub block_index: u32,
    /// Which of the three fields above this record carries.
    pub known: u8,
}

impl Hit {
    /// Whether this record carries every field in `want`.
    #[must_use]
    pub const fn serves(&self, want: u8) -> bool {
        !self.negative && self.known & want == want
    }

    /// Fold a later observation of the same key into this record.
    ///
    /// Fields accumulate rather than replace. The same name is looked up
    /// through call sites that pass different out-pointers — the file-open path
    /// asks for the archive and block record, an existence check asks for the
    /// found-in archive — and a record that kept only the newest shape would
    /// lose the other's fields on every alternation, so neither shape would ever
    /// be served. Accumulating is sound for the same reason the memo is: the
    /// answer is fixed while the archive set is.
    ///
    /// A negative on either side wins outright, since the two cannot both
    /// describe one key. The block index is relative to the found archive's
    /// table, so it is dropped if the found archive itself changed.
    #[must_use]
    pub const fn merged(self, newer: Self) -> Self {
        if self.negative || newer.negative {
            return newer;
        }
        let found = if newer.known & KNOWN_FOUND == 0 {
            self.found
        } else {
            newer.found
        };
        let found_in = if newer.known & KNOWN_FOUND_IN == 0 {
            self.found_in
        } else {
            newer.found_in
        };
        let (block_index, block_known) = if newer.known & KNOWN_BLOCK != 0 {
            (newer.block_index, KNOWN_BLOCK)
        } else if self.known & KNOWN_BLOCK != 0 && found == self.found {
            (self.block_index, KNOWN_BLOCK)
        } else {
            (0, 0)
        };
        Self {
            negative: false,
            found,
            found_in,
            block_index,
            known: ((self.known | newer.known) & !KNOWN_BLOCK) | block_known,
        }
    }
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
        found_in: 0,
        block_index: 0,
        known: 0,
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
    /// Merging into a same-key entry (see [`Hit::merged`]), filling an empty
    /// way, or evicting per the bucket's pseudo-LRU marker.
    pub fn insert(&mut self, key: &Key, hit: Hit) {
        let bucket = &mut self.buckets[Self::bucket_index(key)];
        let existing = bucket.ways.iter().position(|e| key.matches(e));
        let hit = existing.map_or(hit, |w| bucket.ways[w].hit.merged(hit));
        let way = existing
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

    /// Drop every record, for a change in the set of mounted archives.
    ///
    /// An answer is a function of the name and of which archives were mounted
    /// when it was derived. Revalidating a handle catches an archive that went
    /// away, but nothing about a stored record can notice one that arrived and
    /// now owns the name (or owns it earlier in the walk), so a mount or an
    /// unmount discards the table rather than trying to repair it.
    pub fn clear(&mut self) {
        self.buckets.fill(EMPTY_BUCKET);
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

    /// Build a key from a plain byte slice, the way the adapter's NUL scan does.
    ///
    /// The adapter folds each byte as it copies it out of the client's string;
    /// a test starts from a slice, so it does the same fold over the slice.
    fn key_of(archive: u32, scope: u32, raw: &[u8]) -> Option<Key> {
        let mut key = Key::EMPTY;
        key.populate(archive, scope, |name| {
            if raw.len() > NAME_CAP {
                return None;
            }
            for (dst, &src) in name.iter_mut().zip(raw.iter()) {
                *dst = normalize_byte(src);
            }
            Some(raw.len())
        })
        .then_some(key)
    }

    const POS: Hit = Hit {
        negative: false,
        found: 0x00d4_0000,
        found_in: 0x00d4_0000,
        block_index: 7,
        known: KNOWN_FOUND | KNOWN_FOUND_IN | KNOWN_BLOCK,
    };
    const NEG: Hit = Hit {
        negative: true,
        found: 0,
        found_in: 0,
        block_index: 0,
        known: 0,
    };

    #[test]
    fn normalization_folds_case_and_slashes() {
        let a = key_of(0, 1, b"Interface/GlueXML\\gLuEsTrInGs.lua").unwrap();
        let b = key_of(0, 1, b"INTERFACE\\GLUEXML\\GLUESTRINGS.LUA").unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.name[..a.len as usize], b.name[..b.len as usize]);
        // High-half bytes are left alone (identity — never merged).
        let c = key_of(0, 1, &[0xe9, b'a']).unwrap();
        assert_eq!(c.name[0], 0xe9);
        assert_eq!(c.name[1], b'A');
    }

    #[test]
    fn key_dimensions_are_distinct() {
        let mut cache = FileCache::new();
        let name = b"DBFilesClient\\Spell.dbc";
        let scoped = key_of(0x1234, 0, name).unwrap();
        let other_archive = key_of(0x5678, 0, name).unwrap();
        let other_scope = key_of(0x1234, 1, name).unwrap();
        cache.insert(&scoped, POS);
        assert_eq!(cache.probe(&scoped), Some(POS));
        assert_eq!(cache.probe(&other_archive), None);
        assert_eq!(cache.probe(&other_scope), None);
    }

    #[test]
    fn negative_roundtrip() {
        let mut cache = FileCache::new();
        let key = key_of(0, 0, b"World\\NoSuchFile.blp").unwrap();
        assert_eq!(cache.probe(&key), None);
        cache.insert(&key, NEG);
        assert_eq!(cache.probe(&key), Some(NEG));
    }

    #[test]
    fn same_key_overwrites_in_place() {
        let mut cache = FileCache::new();
        let key = key_of(0x10, 0, b"a").unwrap();
        cache.insert(&key, POS);
        let newer = Hit {
            block_index: 9,
            ..POS
        };
        cache.insert(&key, newer);
        assert_eq!(cache.probe(&key), Some(newer));
    }

    /// A record serves a call only when it carries every field that call wants.
    ///
    /// The shapes are the ones the client actually issues: the file-open path
    /// asks for the found archive and the block record, the existence checks
    /// ask for the found-in archive or for nothing at all.
    #[test]
    fn a_record_serves_only_the_shapes_it_covers() {
        let open_path = KNOWN_FOUND | KNOWN_BLOCK;
        let exists_path = KNOWN_FOUND_IN;

        assert!(POS.serves(open_path));
        assert!(POS.serves(exists_path));
        assert!(POS.serves(0), "a return-code-only call wants no field");

        let from_open = Hit {
            known: KNOWN_FOUND | KNOWN_BLOCK,
            ..POS
        };
        assert!(from_open.serves(open_path));
        assert!(
            !from_open.serves(exists_path),
            "the open path never delivers found-in, so it cannot answer for it"
        );
        assert!(from_open.serves(0));

        let bare = Hit { known: 0, ..POS };
        assert!(bare.serves(0));
        assert!(!bare.serves(open_path));

        assert!(!NEG.serves(0), "a negative is never a positive answer");
    }

    /// The two real call shapes alternating on one name.
    ///
    /// Each delivers a different subset, and the record has to end up serving
    /// both — otherwise every alternation evicts the other's fields and neither
    /// call is ever answered.
    #[test]
    fn alternating_shapes_accumulate_instead_of_evicting() {
        let mut cache = FileCache::new();
        let key = key_of(0, 0, b"World\\Shared.blp").unwrap();
        let open_path = KNOWN_FOUND | KNOWN_BLOCK;
        let exists_path = KNOWN_FOUND_IN;

        cache.insert(
            &key,
            Hit {
                known: open_path,
                found_in: 0,
                ..POS
            },
        );
        cache.insert(
            &key,
            Hit {
                known: exists_path,
                found: 0,
                block_index: 0,
                ..POS
            },
        );

        let merged = cache.probe(&key).unwrap();
        assert!(merged.serves(open_path), "kept the earlier shape's fields");
        assert!(merged.serves(exists_path), "took the later shape's field");
        assert_eq!(merged.found, POS.found);
        assert_eq!(merged.found_in, POS.found_in);
        assert_eq!(merged.block_index, POS.block_index);
    }

    /// A block index only survives while it still indexes the same archive.
    #[test]
    fn a_new_found_archive_drops_the_stale_block_index() {
        let older = Hit {
            known: KNOWN_FOUND | KNOWN_BLOCK,
            ..POS
        };
        let newer = Hit {
            negative: false,
            found: 0x00e5_0000,
            found_in: 0,
            block_index: 0,
            known: KNOWN_FOUND,
        };
        let merged = older.merged(newer);
        assert_eq!(merged.found, newer.found);
        assert!(
            !merged.serves(KNOWN_BLOCK),
            "the index belonged to the previous archive's table"
        );
    }

    #[test]
    fn a_negative_replaces_a_positive_outright() {
        let merged = POS.merged(NEG);
        assert_eq!(merged, NEG);
        assert_eq!(NEG.merged(POS), POS);
    }

    #[test]
    fn clear_drops_every_record() {
        let mut cache = FileCache::new();
        let positive = key_of(0, 0, b"World\\Real.blp").unwrap();
        let negative = key_of(0, 0, b"World\\NoSuchFile.blp").unwrap();
        cache.insert(&positive, POS);
        cache.insert(&negative, NEG);

        cache.clear();

        assert_eq!(cache.probe(&positive), None);
        assert_eq!(cache.probe(&negative), None);
        // Still usable afterwards.
        cache.insert(&positive, POS);
        assert_eq!(cache.probe(&positive), Some(POS));
    }

    #[test]
    fn long_names_are_uncacheable() {
        assert!(key_of(0, 0, &[b'x'; NAME_CAP]).is_some());
        assert!(key_of(0, 0, &[b'x'; NAME_CAP + 1]).is_none());
    }

    /// Three same-bucket keys through a 2-way bucket.
    ///
    /// The pseudo-LRU keeps the most-recently-touched way and the newest
    /// insert, evicting the stale way.
    #[test]
    fn two_way_eviction_prefers_recently_used() {
        let mut cache = FileCache::new();
        let base = key_of(0, 0, b"collide").unwrap();
        let target = FileCache::bucket_index(&base);
        // Brute-force two more archives whose keys land in the same bucket.
        let mut colliders = (1u32..).filter_map(|a| {
            let k = key_of(a, 0, b"collide").unwrap();
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

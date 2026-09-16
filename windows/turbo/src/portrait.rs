//! Portrait cache validity across device generations.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Player(u64),
    Creature(u32),
}

struct Entry {
    key: Key,
    generation: Option<u64>,
    sources: BTreeSet<u64>,
}

#[derive(Default)]
pub struct Registry {
    generation: u64,
    entries: BTreeMap<usize, Entry>,
}

impl Registry {
    pub fn valid(&self, texture: usize, key: &Key) -> bool {
        self.entries
            .get(&texture)
            .is_some_and(|entry| entry.generation == Some(self.generation) && &entry.key == key)
    }

    pub fn contains(&self, texture: usize) -> bool {
        self.entries.contains_key(&texture)
    }

    pub fn observe(&mut self, texture: usize, guid: u64) {
        if let Some(entry) = self.entries.get_mut(&texture) {
            entry.sources.insert(guid);
        }
    }

    /// Publish only after rendering and native ownership transfer succeed.
    pub fn publish(&mut self, texture: usize, key: Key, guid: u64) {
        let entry = self.entries.entry(texture).or_insert_with(|| Entry {
            key: match key {
                Key::Player(guid) => Key::Player(guid),
                Key::Creature(display) => Key::Creature(display),
            },
            generation: Some(self.generation),
            sources: BTreeSet::new(),
        });
        if entry.key != key {
            entry.key = key;
            entry.sources.clear();
        }
        entry.generation = Some(self.generation);
        entry.sources.insert(guid);
    }

    pub fn invalidate(&mut self, texture: usize) {
        if let Some(entry) = self.entries.get_mut(&texture) {
            entry.generation = None;
        }
    }

    pub fn textures(&self) -> Vec<usize> {
        self.entries.keys().copied().collect()
    }

    pub fn remove(&mut self, texture: usize) {
        self.entries.remove(&texture);
    }

    /// Invalidate before reset; all observed units receive a deferred refresh.
    pub fn reset(&mut self) -> BTreeSet<u64> {
        self.generation = self.generation.wrapping_add(1);
        self.entries
            .values_mut()
            .fold(BTreeSet::new(), |mut guids, entry| {
                entry.generation = None;
                guids.append(&mut entry.sources);
                guids
            })
    }
}

/// A single resource reference, released unless explicitly transferred to a native owner.
pub struct OwnedResource {
    pointer: usize,
    release: fn(usize),
}

impl OwnedResource {
    /// Take a resource reference whose matching release operation is supplied by the caller.
    ///
    /// # Safety
    /// A nonzero pointer must own one reference accepted by `release` until dropped or transferred.
    pub const unsafe fn new(pointer: usize, release: fn(usize)) -> Self {
        Self { pointer, release }
    }

    pub const fn pointer(&self) -> usize {
        self.pointer
    }

    pub fn into_raw(mut self) -> usize {
        std::mem::take(&mut self.pointer)
    }
}

impl Drop for OwnedResource {
    fn drop(&mut self) {
        if self.pointer != 0 {
            (self.release)(self.pointer);
        }
    }
}

/// Upload the client's byte mask without premultiplying portrait RGB.
pub fn mask_pixel(alpha: u8) -> u32 {
    u32::from(alpha) << 24
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_distinguishes_player_guids_and_creature_displays() {
        let mut registry = Registry::default();
        registry.publish(1, Key::Player(7), 7);
        assert!(registry.valid(1, &Key::Player(7)));
        assert!(!registry.valid(1, &Key::Creature(7)));
        assert!(!registry.valid(1, &Key::Player(8)));
    }

    #[test]
    fn reset_refreshes_every_observed_unit_and_requires_publication() {
        let mut registry = Registry::default();
        registry.publish(1, Key::Creature(10), 101);
        registry.observe(1, 102);
        registry.publish(2, Key::Player(103), 103);
        assert_eq!(registry.reset(), BTreeSet::from([101, 102, 103]));
        assert!(!registry.valid(1, &Key::Creature(10)));
        assert!(!registry.valid(2, &Key::Player(103)));
        registry.publish(1, Key::Creature(10), 102);
        assert!(registry.valid(1, &Key::Creature(10)));
        assert!(!registry.valid(2, &Key::Player(103)));
        assert_eq!(registry.reset(), BTreeSet::from([102]));
    }

    #[test]
    fn destruction_prevents_pointer_reuse_from_reusing_validity() {
        let mut registry = Registry::default();
        registry.publish(1, Key::Creature(7), 20);
        assert_eq!(registry.textures(), vec![1]);
        registry.remove(1);
        assert!(registry.textures().is_empty());
        assert!(!registry.contains(1));
        assert!(!registry.valid(1, &Key::Creature(7)));
        assert!(registry.reset().is_empty());
        registry.publish(1, Key::Player(8), 8);
        assert!(registry.valid(1, &Key::Player(8)));
    }

    #[test]
    fn invalidation_and_repeated_resets_cannot_revalidate_old_content() {
        let mut registry = Registry::default();
        registry.publish(1, Key::Player(7), 7);
        registry.invalidate(1);
        assert!(!registry.valid(1, &Key::Player(7)));
        registry.reset();
        registry.reset();
        assert!(!registry.valid(1, &Key::Player(7)));
        registry.publish(1, Key::Player(7), 7);
        assert!(registry.valid(1, &Key::Player(7)));
    }

    #[test]
    fn replacement_identity_does_not_keep_previous_sources() {
        let mut registry = Registry::default();
        registry.publish(1, Key::Creature(7), 100);
        registry.publish(1, Key::Creature(8), 101);
        assert!(!registry.valid(1, &Key::Creature(7)));
        assert_eq!(registry.reset(), BTreeSet::from([101]));
    }

    thread_local! {
        static RELEASED: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn release(pointer: usize) {
        RELEASED.with_borrow_mut(|released| released.push(pointer));
    }

    #[test]
    fn failed_publication_releases_candidate_and_preserves_old_validity() {
        RELEASED.with_borrow_mut(Vec::clear);
        let mut registry = Registry::default();
        registry.publish(1, Key::Player(7), 7);
        // SAFETY: the fake reference has no external lifetime; release records exactly one drop.
        let candidate = unsafe { OwnedResource::new(2, release) };
        let published = None::<usize>;
        if let Some(texture) = published {
            registry.publish(texture, Key::Player(7), 7);
            candidate.into_raw();
        } else {
            drop(candidate);
        }
        assert!(registry.valid(1, &Key::Player(7)));
        assert!(!registry.contains(2));
        RELEASED.with_borrow(|released| assert_eq!(released, &[2]));
    }

    #[test]
    fn transfer_releases_exactly_once_and_null_is_not_owned() {
        RELEASED.with_borrow_mut(Vec::clear);
        // SAFETY: the fake reference is accepted by the matching test release operation.
        let source = unsafe { OwnedResource::new(3, release) };
        assert_eq!(source.pointer(), 3);
        let pointer = source.into_raw();
        RELEASED.with_borrow(|released| assert!(released.is_empty()));
        // SAFETY: into_raw transferred the one reference to the simulated native owner.
        drop(unsafe { OwnedResource::new(pointer, release) });
        // SAFETY: null represents no reference and must not invoke release.
        drop(unsafe { OwnedResource::new(0, release) });
        RELEASED.with_borrow(|released| assert_eq!(released, &[3]));
    }

    #[test]
    fn every_mask_byte_survives_argb_upload() {
        for alpha in 0..=u8::MAX {
            let pixel = mask_pixel(alpha);
            assert_eq!(pixel.to_le_bytes(), [0, 0, 0, alpha]);
            let rgb = 0x0056_3412;
            assert_eq!((rgb & 0x00ff_ffff) | pixel, rgb | (u32::from(alpha) << 24));
        }
    }
}

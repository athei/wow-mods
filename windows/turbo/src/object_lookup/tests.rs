use std::collections::BTreeMap;

use super::*;

const MANAGER: u32 = 0x1000;
const BUCKETS: u32 = 0x2000;
const LOW: u32 = 5;
const HIGH: u32 = 9;
const BUCKET: u32 = BUCKETS + 12;

fn table(head: u32, link: u32) -> BTreeMap<u32, u32> {
    BTreeMap::from([
        (MANAGER + 0x24, 3),
        (MANAGER + 0x1c, BUCKETS),
        (BUCKET + 8, head),
        (BUCKET, link),
    ])
}

fn node(words: &mut BTreeMap<u32, u32>, address: u32, low: u32, high: u32) {
    words.extend([
        (address + 0x18, low),
        (address + 0x30, low),
        (address + 0x34, high),
    ]);
}

#[test]
fn empty_mask_and_sentinels_never_read_nodes() {
    assert_eq!(
        lookup(MANAGER, LOW, HIGH, |at| {
            assert_eq!(at, MANAGER + 0x24);
            u32::MAX
        }),
        0
    );
    for head in [0, 1, 0x7001, u32::MAX] {
        let words = table(head, 0x1c);
        assert_eq!(
            lookup(MANAGER, LOW, HIGH, |at| {
                assert_ne!(at, BUCKET, "empty bucket needs no link metadata");
                words[&at]
            }),
            0
        );
    }
}

#[test]
fn first_hit_needs_no_link_or_next_node_read() {
    let mut words = table(0x3000, 0x1c);
    words.remove(&BUCKET);
    node(&mut words, 0x3000, LOW, HIGH);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0x3000);
}

#[test]
fn collisions_keep_all_three_key_checks_and_hoist_metadata() {
    // A non-default link offset proves the traversal uses bucket metadata.
    let mut words = table(0x3000, 0x40);
    for address in [0x3000, 0x3100, 0x3200, 0x3300] {
        node(&mut words, address, LOW, HIGH);
        words.insert(address + 0x44, address + 0x100);
    }
    words.insert(0x3018, LOW + 1);
    words.remove(&0x3030);
    words.remove(&0x3034);
    words.insert(0x3130, LOW + 1);
    words.remove(&0x3134);
    words.insert(0x3234, HIGH + 1);
    let mut reads = BTreeMap::new();
    assert_eq!(
        lookup(MANAGER, LOW, HIGH, |at| {
            *reads.entry(at).or_insert(0) += 1;
            words[&at]
        }),
        0x3300
    );
    for address in [MANAGER + 0x24, MANAGER + 0x1c, BUCKET] {
        assert_eq!(reads[&address], 1);
    }
    for tail in [0, 0x7001] {
        words.insert(0x3334, HIGH + 1);
        words.insert(0x3344, tail);
        assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0);
    }
}

#[test]
fn mutation_and_manager_switches_between_calls_are_visible() {
    let mut words = table(0x3000, 0x1c);
    node(&mut words, 0x3000, LOW, HIGH);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0x3000);
    words.insert(BUCKET + 8, 0);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0);
    node(&mut words, 0x3000, LOW, HIGH + 1);
    words.insert(BUCKET + 8, 0x3000);
    words.insert(0x3020, 1);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0);
    // Rehash changes both the mask and the bucket array.
    words.insert(MANAGER + 0x24, 7);
    words.insert(MANAGER + 0x1c, 0x4000);
    words.insert(0x4000 + LOW * 12 + 8, 0x5000);
    node(&mut words, 0x5000, LOW, HIGH);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0x5000);
    words.insert(0x6000 + 0x24, u32::MAX);
    assert_eq!(lookup(0x6000, LOW, HIGH, |at| words[&at]), 0);
    assert_eq!(lookup(MANAGER, LOW, HIGH, |at| words[&at]), 0x5000);
}

#[test]
fn zero_guid_is_not_special_in_the_active_lookup() {
    let words = BTreeMap::from([
        (MANAGER + 0x24, 0),
        (MANAGER + 0x1c, BUCKETS),
        (BUCKETS + 8, 0x3000),
        (0x3018, 0),
        (0x3030, 0),
        (0x3034, 0),
    ]);
    assert_eq!(lookup(MANAGER, 0, 0, |at| words[&at]), 0x3000);
}

//! Portable string-library kernels for the Lua pattern-function fast paths.
//!
//! Lua 5.0's `string.gsub`/`string.gfind` always invoke the recursive pattern
//! matcher, even when the pattern is a plain literal with no magic characters —
//! the O(n*m) backtracking a chat/combat-log parser pays on every line. These
//! helpers detect the literal case and perform the work with a single-pass
//! substring search, leaving every pattern that carries a magic character (and
//! every non-literal replacement) to the original matcher. No FFI; the 32-bit
//! adapters that read/write the Lua stack live in `win::hooks`.

use memchr::memmem;

/// The Lua pattern "magic" characters. A pattern containing none of these
/// matches exactly its own bytes, so it can be handled as a literal substring
/// rather than by the recursive matcher. `^` is in the set, so an anchored
/// pattern is never treated as a literal; `)` is in the set so a bare close-paren
/// falls through to the recursive matcher (matching stock's `SPECIALS`).
const MAGIC: &[u8] = b"^$*+?.()[%-";

/// Returns `true` if `pattern` contains any Lua pattern magic character.
#[must_use]
pub fn pattern_has_magic(pattern: &[u8]) -> bool {
    pattern.iter().any(|b| MAGIC.contains(b))
}

/// Literal `string.gsub`: replace up to `max_n` non-overlapping occurrences of
/// `pattern` in `subject` with `repl`, returning the rebuilt string and the
/// replacement count.
///
/// Byte-identical to the stock collector for the caller's preconditions (a
/// non-empty no-magic `pattern` and a `repl` string with no `%` escapes): the
/// search advances past each match exactly as the original loop does (`s = e`),
/// and a non-positive `max_n` performs no replacement. An empty `pattern` is
/// rejected (it would match at every position); the caller guarantees this, the
/// guard only avoids a degenerate loop.
#[must_use]
pub fn literal_gsub(subject: &[u8], pattern: &[u8], repl: &[u8], max_n: i32) -> (Vec<u8>, i32) {
    let mut out = Vec::with_capacity(subject.len());
    let mut count: i32 = 0;
    let mut pos = 0usize;
    if !pattern.is_empty() {
        let finder = memmem::Finder::new(pattern);
        while count < max_n {
            let Some(hit) = finder.find(&subject[pos..]) else {
                break;
            };
            let start = pos + hit;
            out.extend_from_slice(&subject[pos..start]);
            out.extend_from_slice(repl);
            pos = start + pattern.len();
            count += 1;
        }
    }
    out.extend_from_slice(&subject[pos..]);
    (out, count)
}

/// Find the first occurrence of literal `pattern` in `subject` at or after byte
/// offset `from`, returning its absolute offset. Drives the `string.gfind`
/// iterator fast path (advance to the next match). Returns `None` for an empty
/// pattern, a `from` past the end of `subject`, or no further match.
#[must_use]
pub fn literal_find_from(subject: &[u8], pattern: &[u8], from: usize) -> Option<usize> {
    if pattern.is_empty() || from > subject.len() {
        return None;
    }
    memmem::find(&subject[from..], pattern).map(|rel| from + rel)
}

#[cfg(test)]
mod tests {
    use super::{literal_find_from, literal_gsub, pattern_has_magic};

    /// Position-by-position reference matching stock `str_gsub`'s loop: at each
    /// position, if the replacement budget remains and the pattern matches here,
    /// emit the replacement and skip the match; otherwise copy one byte. The
    /// `memmem`-driven kernel must agree with this on every input.
    fn oracle(subject: &[u8], pattern: &[u8], repl: &[u8], max_n: i32) -> (Vec<u8>, i32) {
        let mut out = Vec::new();
        let mut count: i32 = 0;
        let mut i = 0usize;
        while i < subject.len() {
            let here_matches = !pattern.is_empty()
                && i + pattern.len() <= subject.len()
                && &subject[i..i + pattern.len()] == pattern;
            if count < max_n && here_matches {
                out.extend_from_slice(repl);
                i += pattern.len();
                count += 1;
            } else {
                out.push(subject[i]);
                i += 1;
            }
        }
        (out, count)
    }

    #[test]
    fn detects_each_magic_char() {
        for m in b"^$*+?.()[%-" {
            assert!(
                pattern_has_magic(&[b'a', *m, b'b']),
                "magic {m:#x} not flagged"
            );
        }
    }

    #[test]
    fn plain_literal_has_no_magic() {
        assert!(!pattern_has_magic(b"Your Spell hits"));
        assert!(!pattern_has_magic(b"]}!,/ \t"));
        assert!(!pattern_has_magic(b""));
    }

    #[test]
    fn no_match_returns_subject_unchanged() {
        let (out, n) = literal_gsub(b"hello world", b"xyz", b"Q", i32::MAX);
        assert_eq!(out, b"hello world");
        assert_eq!(n, 0);
    }

    #[test]
    fn replaces_at_start_middle_end() {
        assert_eq!(
            literal_gsub(b"abXcd", b"ab", b"_", i32::MAX),
            (b"_Xcd".to_vec(), 1)
        );
        assert_eq!(
            literal_gsub(b"Xabcd", b"ab", b"_", i32::MAX),
            (b"X_cd".to_vec(), 1)
        );
        assert_eq!(
            literal_gsub(b"cdXab", b"ab", b"_", i32::MAX),
            (b"cdX_".to_vec(), 1)
        );
    }

    #[test]
    fn multiple_and_adjacent_matches() {
        assert_eq!(
            literal_gsub(b"a.a.a", b"a", b"Z", i32::MAX),
            (b"Z.Z.Z".to_vec(), 3)
        );
        // Non-overlapping: "aa" in "aaaa" matches at 0 and 2.
        assert_eq!(
            literal_gsub(b"aaaa", b"aa", b"-", i32::MAX),
            (b"--".to_vec(), 2)
        );
        // "aa" in "aaa": match at 0, then a lone "a" remains.
        assert_eq!(
            literal_gsub(b"aaa", b"aa", b"-", i32::MAX),
            (b"-a".to_vec(), 1)
        );
    }

    #[test]
    fn respects_max_n_limit() {
        assert_eq!(
            literal_gsub(b"a a a a", b"a", b"Z", 2),
            (b"Z Z a a".to_vec(), 2)
        );
        assert_eq!(
            literal_gsub(b"a a a", b"a", b"Z", 0),
            (b"a a a".to_vec(), 0)
        );
        assert_eq!(
            literal_gsub(b"a a a", b"a", b"Z", -5),
            (b"a a a".to_vec(), 0)
        );
    }

    #[test]
    fn empty_replacement_deletes() {
        assert_eq!(
            literal_gsub(b"a-b-c", b"-", b"", i32::MAX),
            (b"abc".to_vec(), 2)
        );
    }

    #[test]
    fn replacement_longer_than_pattern() {
        assert_eq!(
            literal_gsub(b"<x>", b"x", b"[wide]", i32::MAX),
            (b"<[wide]>".to_vec(), 1)
        );
    }

    #[test]
    fn empty_subject() {
        assert_eq!(literal_gsub(b"", b"a", b"Z", i32::MAX), (Vec::new(), 0));
    }

    #[test]
    fn matches_oracle_over_corpus() {
        // Deterministic LCG over a tiny alphabet so matches are frequent and
        // overlap-adjacency is exercised; compare the kernel to the oracle.
        let mut state: u32 = 0x1234_5678;
        let mut next = |modulo: u32| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) % modulo
        };
        let alphabet = b"ab-";
        for _ in 0..2000 {
            let slen = next(20) as usize;
            let subject: Vec<u8> = (0..slen).map(|_| alphabet[next(3) as usize]).collect();
            let plen = 1 + next(3) as usize;
            let pattern: Vec<u8> = (0..plen).map(|_| alphabet[next(3) as usize]).collect();
            let rlen = next(4) as usize;
            let repl: Vec<u8> = (0..rlen).map(|_| alphabet[next(3) as usize]).collect();
            let max_n = [i32::MAX, 0, 1, 2, 3][next(5) as usize];
            assert_eq!(
                literal_gsub(&subject, &pattern, &repl, max_n),
                oracle(&subject, &pattern, &repl, max_n),
                "subject={subject:?} pattern={pattern:?} repl={repl:?} max_n={max_n}",
            );
        }
    }

    #[test]
    fn find_from_walks_successive_matches() {
        let s = b"a.bc.a.bc.a";
        // Successive iteration as `gfind` would: each call starts past the prior
        // match end.
        assert_eq!(literal_find_from(s, b"bc", 0), Some(2));
        assert_eq!(literal_find_from(s, b"bc", 4), Some(7));
        assert_eq!(literal_find_from(s, b"bc", 8), None);
    }

    #[test]
    fn find_from_edges() {
        assert_eq!(literal_find_from(b"xay", b"a", 0), Some(1));
        // `from` exactly at a match, and one past it.
        assert_eq!(literal_find_from(b"aa", b"a", 1), Some(1));
        assert_eq!(literal_find_from(b"aa", b"a", 2), None);
        // out-of-range `from`, empty pattern, no match.
        assert_eq!(literal_find_from(b"aa", b"a", 3), None);
        assert_eq!(literal_find_from(b"aa", b"", 0), None);
        assert_eq!(literal_find_from(b"abc", b"z", 0), None);
    }
}

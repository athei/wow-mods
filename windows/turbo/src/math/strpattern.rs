//! Conservative mandatory-literal inspection for Lua patterns.
//!
//! Only a fully inspected, well-formed subset is eligible. A returned slice
//! occurs in every possible match. Returning `None` means the caller must let
//! the original matcher decide, including whether to raise a pattern error.

/// A mandatory contiguous run of at least four unescaped literal bytes.
///
/// Supports classes, bracket sets, captures and single-item quantifiers. Escaped
/// literals split runs rather than allocating decoded copies. Balanced matches,
/// frontier assertions, backreferences, binary patterns and malformed constructs
/// delegate. At most 32 captures are accepted, matching the client's limit.
#[must_use]
pub fn required_literal(pattern: &[u8]) -> Option<&[u8]> {
    if pattern.len() < 4 || pattern.contains(&0) {
        return None;
    }
    let mut best = 0..0;
    let mut run = 0..0;
    let mut i = 0;
    let mut depth = 0usize;
    let mut captures = 0;
    while i < pattern.len() {
        let start = i;
        let mut literal = false;
        let mut atom = true;
        match pattern[i] {
            b'(' => {
                captures += 1;
                if captures > 32 {
                    return None;
                }
                depth += 1;
                atom = false;
                i += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                atom = false;
                i += 1;
            }
            b'%' => {
                let escaped = *pattern.get(i + 1)?;
                if !b"acdlpsuwxzACDLPSUWXZ".contains(&escaped) && escaped.is_ascii_alphanumeric() {
                    return None;
                }
                i += 2;
            }
            b'[' => {
                i += 1;
                if pattern.get(i) == Some(&b'^') {
                    i += 1;
                }
                // A leading ']' is an item, not the end of an empty set.
                loop {
                    let byte = *pattern.get(i)?;
                    if byte == b'%' {
                        i += 1;
                        pattern.get(i)?;
                    }
                    i += 1;
                    if pattern.get(i) == Some(&b']') {
                        i += 1;
                        break;
                    }
                }
            }
            b'.' => i += 1,
            b'^' if i == 0 => {
                atom = false;
                i += 1;
            }
            b'$' if i + 1 == pattern.len() => {
                atom = false;
                i += 1;
            }
            b'*' | b'+' | b'-' | b'?' => return None,
            _ => {
                literal = true;
                i += 1;
            }
        }
        let quantifier = atom && pattern.get(i).is_some_and(|b| b"*+-?".contains(b));
        let optional = quantifier && pattern[i] != b'+';
        if literal && !optional {
            if run.end == start {
                run.end = i;
            } else {
                run = start..i;
            }
            if run.len() > best.len() {
                best = run.clone();
            }
        } else {
            run = i..i;
        }
        if quantifier {
            i += 1;
            run = i..i;
        }
    }
    if depth != 0 || best.len() < 4 {
        None
    } else {
        Some(&pattern[best])
    }
}

#[cfg(test)]
mod tests {
    use super::required_literal;

    #[test]
    fn mandatory_runs_respect_optional_atoms() {
        for pattern in [b"abcd*".as_slice(), b"abcd-", b"abcd?", b"a?b?c?d?"] {
            assert_eq!(required_literal(pattern), None, "{pattern:?}");
        }
        assert_eq!(required_literal(b"abcd+"), Some(b"abcd".as_slice()));
        assert_eq!(required_literal(b"abcde?"), Some(b"abcd".as_slice()));
        assert_eq!(required_literal(b"abcde*WXYZ"), Some(b"abcd".as_slice()));
        assert_eq!(required_literal(b"ab+cdef"), Some(b"cdef".as_slice()));
    }

    #[test]
    fn classes_captures_and_anchors_split_runs() {
        for pattern in [
            b"^damage (%d+)$".as_slice(),
            b"[%a]damage %s*",
            b"()damage .*",
        ] {
            assert_eq!(required_literal(pattern), Some(b"damage ".as_slice()));
        }
        assert_eq!(required_literal(b"ab(cd)ef"), None);
        assert_eq!(required_literal(b"abc%.def"), None);
        assert_eq!(required_literal(b"[]%]]abcd"), Some(b"abcd".as_slice()));
        assert_eq!(required_literal(b"[^]]abcd"), Some(b"abcd".as_slice()));
    }

    #[test]
    fn inspection_never_hides_pattern_errors() {
        for pattern in [
            b"abcd(".as_slice(),
            b"abcd)",
            b"abcd[",
            b"abcd%",
            b"abcd[]",
            b"abcd[%]",
            b"abcd%b()",
            b"abcd%f[%a]",
            b"abcd%1",
            b"abcd\0.*",
            b"abcd**",
        ] {
            assert_eq!(required_literal(pattern), None, "{pattern:?}");
        }
        let too_many = format!("{}abcd{}", "(".repeat(33), ")".repeat(33));
        assert_eq!(required_literal(too_many.as_bytes()), None);
    }

    #[test]
    fn binary_subject_literals_remain_bytes() {
        assert_eq!(
            required_literal(b"\xff\xfeabcd%d"),
            Some(b"\xff\xfeabcd".as_slice())
        );
    }
}

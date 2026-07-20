#!/bin/sh
#
# Mechanical gate for the rules in docs/CONVENTIONS.md that a linter cannot
# express. clippy covers lint-shaped rules; these are the grep-shaped ones,
# and without a gate they drift. Run by `make audit`, which `make check` calls
# before every commit.
#
#   scripts/audit.sh                  audit the whole tree
#   scripts/audit.sh --file <path>    the per-file subset, for an editor hook
#   scripts/audit.sh --update-derives regenerate scripts/derive_inventory.txt
#
# POSIX sh + awk only: no bashisms, and no GNU awk extensions (`\s`, `\b`),
# since the stock macOS awk has neither. Interactive shells here sometimes
# alias `grep` to a wrapper that mishandles multi-file argument lists; this
# runs under `#!/bin/sh`, so it sees the real one.

set -eu

cd "$(git rev-parse --show-toplevel)"

INVENTORY=scripts/derive_inventory.txt

# Files permitted to hold each narrowly-scoped exception. These are sets, not
# counts: a new site fails even if an old one was deleted in the same change.
ALLOW_SITES='unix/shared/src/tsc.rs
windows/turbo/src/math/boundsfit.rs
windows/turbo/src/math/collision.rs
windows/turbo/src/math/gx.rs
windows/turbo/src/math/lua_gc.rs
windows/turbo/src/math/m2.rs
windows/turbo/src/math/particle.rs
windows/turbo/src/math/quaternion.rs
windows/turbo/src/math/world.rs
windows/turbo/src/math.rs
windows/turbo/src/win/hooks.rs'

ONCELOCK_SITES='windows/translate/src/hook.rs
windows/turbo/build.rs
windows/turbo/src/win/hooks.rs'

INLINE_ALWAYS_SITES='unix/shared/src/crumb.rs'

# The vendored addon is upstream MIT (see THIRD-PARTY-LICENSES.md). Its
# glossary names the servers and zones it exists to translate, so the
# game-name rule cannot apply to it.
VENDORED='addon/'

status=0

# Report a finding and name the section of docs/CONVENTIONS.md it comes from,
# so the fix is one lookup away rather than a guess.
report() {
    section=$1
    shift
    printf '\n== %s\n   docs/CONVENTIONS.md § %s\n' "$1" "$section" >&2
    shift
    printf '%s\n' "$@" | sed 's/^/   /' >&2
    status=1
}

# --- individual checks -------------------------------------------------------

# Every doc block of >= 2 lines is: title / empty doc line / body. Also caps
# every doc line at the 100 columns rustfmt gives code.
doc_shape() {
    awk '
        function text(l,   t) {
            t = l
            sub(/^[ \t]*/, "", t)
            sub(/^\/\/[\/!]/, "", t)
            return t
        }
        function is_doc(l,   t) {
            t = l
            sub(/^[ \t]*/, "", t)
            if (t ~ /^\/\/\/\//) return 0      # //// is a divider, not a doc
            if (t ~ /^\/\/\//)   return 1
            if (t ~ /^\/\/!/)    return 1
            return 0
        }
        function blank(l,   t) {
            t = text(l)
            gsub(/[ \t\r]/, "", t)
            return t == ""
        }
        FNR == 1 { run = 0 }
        {
            if (!is_doc($0)) { run = 0; next }
            run++
            if (length($0) > 100)
                printf "%s:%d: doc line is %d columns (max 100)\n", FILENAME, FNR, length($0)
            if (run == 1) {
                title_line = FNR
                title_blank = blank($0)
            }
            if (run == 2) {
                if (title_blank)
                    printf "%s:%d: doc block opens with an empty line; line 1 must be the title\n", \
                        FILENAME, title_line
                else if (!blank($0))
                    printf "%s:%d: line 2 of a doc block must be an empty doc line (title / blank / body)\n", \
                        FILENAME, FNR
            }
        }
    ' "$@"
}

# Every type deriving Clone and/or Copy, as `path Type Derives`. The committed
# inventory is diffed against this, so a speculative derive cannot slip in
# unnoticed: adding one means consciously recording it.
derive_scan() {
    awk '
        /^[ \t]*#\[derive\(/ {
            if ($0 ~ /Clone/ || $0 ~ /Copy/) {
                pending = $0
                pfile = FILENAME
            }
            next
        }
        pending != "" {
            # Attributes and doc comments may sit between the derive and the item.
            if ($0 ~ /^[ \t]*#\[/ || $0 ~ /^[ \t]*\/\//) next
            if (match($0, /(struct|enum|union)[ \t]+[A-Za-z_][A-Za-z0-9_]*/)) {
                name = substr($0, RSTART, RLENGTH)
                sub(/^(struct|enum|union)[ \t]+/, "", name)
                list = pending
                sub(/^[ \t]*#\[derive\(/, "", list)
                sub(/\)\].*$/, "", list)
                gsub(/[ \t]/, "", list)
                printf "%s %s %s\n", pfile, name, list
            }
            pending = ""
        }
    ' "$@" | LC_ALL=C sort
}

# A pattern that must not appear at all, anywhere. Mentioning it in a comment is
# fine — the rules are discussed in the prose of the very files they govern.
banned() {
    pattern=$1
    section=$2
    message=$3
    shift 3
    # -H, not just -n: with a single file grep omits the filename, and the
    # comment filter below keys on the `file:line:` prefix.
    hits=$(grep -HnE "$pattern" "$@" | grep -vE '^[^:]+:[0-9]+:[ \t]*//' || true)
    if [ -n "$hits" ]; then
        report "$section" "$message" "$hits"
    fi
}

# A pattern that must not appear in a COMMENT. The inverse of `banned`: these are
# release-hygiene rules, and the only place they can be broken is the prose.
# `#` and `--` are here for TOML and Lua, which carry prose too.
banned_in_comments() {
    pattern=$1
    section=$2
    message=$3
    shift 3
    hits=$(grep -HnE "^[ \t]*(//|///|//!|#|--).*($pattern)" "$@" || true)
    if [ -n "$hits" ]; then
        report "$section" "$message" "$hits"
    fi
}

# A pattern that must not appear ANYWHERE — comment or code. For the name-leak
# rules: a private server's name is no more acceptable in a string literal or a
# path than in prose, and an absolute home directory is likeliest to show up in
# a Makefile variable or a CI step.
banned_anywhere() {
    pattern=$1
    section=$2
    message=$3
    shift 3
    hits=$(grep -HnE "$pattern" "$@" || true)
    if [ -n "$hits" ]; then
        report "$section" "$message" "$hits"
    fi
}

# A pattern confined to a known set of files: flag any file outside the set.
confined() {
    pattern=$1
    section=$2
    message=$3
    permitted=$4
    shift 4
    hits=$(grep -lE "$pattern" "$@" || true)
    strays=''
    for hit in $hits; do
        if ! printf '%s\n' "$permitted" | grep -qxF "$hit"; then
            strays="$strays$hit
"
        fi
    done
    if [ -n "$strays" ]; then
        report "$section" "$message" "$(printf '%s' "$strays")"
    fi
}

# --- modes -------------------------------------------------------------------

case "${1:-}" in
--update-derives)
    # shellcheck disable=SC2046
    derive_scan $(git ls-files '*.rs') >"$INVENTORY"
    printf 'wrote %s (%d types)\n' "$INVENTORY" "$(wc -l <"$INVENTORY" | tr -d ' ')"
    exit 0
    ;;
--file)
    file=${2:?--file needs a path}
    # An editor hook's view: only the checks that a single file can answer.
    case $file in *.rs) ;; *) exit 0 ;; esac
    [ -f "$file" ] || exit 0

    findings=$(doc_shape "$file")
    [ -z "$findings" ] || report 'Doc comments' 'doc-comment shape' "$findings"

    banned 'pub\(crate\)' 'No pub(crate) — use module hierarchy' \
        'pub(crate) visibility' "$file"
    confined 'inline\(always\)' 'Inline attributes' \
        '#[inline(always)] outside the one measured site' "$INLINE_ALWAYS_SITES" "$file"
    confined '^[ \t]*static .*: *(std::sync::)?OnceLock' 'LazyLock over OnceLock' \
        'OnceLock static outside the runtime-argument sites' "$ONCELOCK_SITES" "$file"
    confined '#\[allow\(' 'Warning suppressions' \
        'lint suppression outside the recorded exception files' "$ALLOW_SITES" "$file"

    exit $status
    ;;
-h | --help)
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac

# --- whole-tree audit --------------------------------------------------------

# shellcheck disable=SC2046
set -- $(git ls-files '*.rs')

findings=$(doc_shape "$@")
if [ -n "$findings" ]; then
    report 'Doc comments' \
        "doc-comment shape: $(printf '%s\n' "$findings" | wc -l | tr -d ' ') findings" \
        "$findings"
fi

drift=$(derive_scan "$@" | diff "$INVENTORY" - || true)
if [ -n "$drift" ]; then
    report 'No default Copy / Clone on aggregate structs' \
        "derive inventory drift (< committed, > working tree). A Clone/Copy derive needs a concrete callsite; record it with scripts/audit.sh --update-derives" \
        "$drift"
fi

banned 'pub\(crate\)' 'No pub(crate) — use module hierarchy' \
    'pub(crate) visibility' "$@"

confined 'inline\(always\)' 'Inline attributes' \
    '#[inline(always)] outside the one measured site' "$INLINE_ALWAYS_SITES" "$@"
confined '^[ \t]*static .*: *(std::sync::)?OnceLock' 'LazyLock over OnceLock' \
    'OnceLock static outside the runtime-argument sites' "$ONCELOCK_SITES" "$@"
confined '#\[allow\(' 'Warning suppressions' \
    'lint suppression outside the recorded exception files' "$ALLOW_SITES" "$@"

modules=$(git ls-files '*/mod.rs' || true)
if [ -n "$modules" ]; then
    report 'Module style: foo.rs + foo/, not foo/mod.rs' 'mod.rs file' "$modules"
fi

# --- release hygiene ---------------------------------------------------------
#
# The repository is public. Source should read as the engineering itself — not
# as a diary of how the engineering got written, and not as a document that
# assumes the reader has access to things they do not.

# Everything tracked, minus four things that cannot be held to these rules.
# The generated lockfiles carry 118 `[[package]]` stanzas that would poison a
# bracket-shaped rule. The verbatim licence texts must never be edited to
# satisfy a lint — MinHook's BSD notice names a disassembler engine and has to
# keep doing so. And this script and the document it enforces both have to
# quote the patterns they ban in order to state the rule at all.
RELEASE=$(git ls-files |
    grep -vE '^(unix|windows)/Cargo\.lock$|^LICENSE$|^THIRD-PARTY-LICENSES\.md$|^addon/WoWTranslate/LICENSE$|^scripts/audit\.sh$|^docs/CONVENTIONS\.md$' || true)

# First-party source: the release set minus the vendored addon.
SRC=$(printf '%s\n' "$RELEASE" | grep -vE "^$VENDORED" || true)

# Game-name discipline. The private server is never named, and the title is
# never spelled out — `WoW`, `WoW 1.12`, `WoW 3.3.5a` and WoW.exe addresses are
# all legitimate and deliberately not matched. Not comment-scoped: a name in a
# string literal or a path leaks exactly as much as one in prose.
# shellcheck disable=SC2086
set -- $SRC
banned_anywhere '[Tt]urtle|TURTLE|[Ww]orld[ _-][Oo]f[ _-][Ww]arcraft|WORLD OF WARCRAFT' \
    'Release hygiene' \
    'names the private server or spells out the title — say "the client" / "the launcher"' "$@"

# Reverse-engineering provenance. How the client was studied is not part of
# what this repository builds. `IDA` needs hand-written word boundaries because
# POSIX ERE has no \b and the bare letters sit inside VALIDATE and CANDIDATE;
# `leaked` is qualified because an x87 register-stack leak is a real thing this
# codebase describes.
banned_in_comments '[Gg]hidra|GHIDRA|[Dd]ecompil|[Dd]isassembl|[Dd]isasm|(^|[^A-Za-z])IDA([^A-Za-z]|$)|[Rr]everse[ -]engineer|[Rr]eversed from|[Ll]eaked (symbol|build|binar|client|pdb|source|debug)' \
    'Release hygiene' \
    'names a decompiler, a disassembler, or the workflow — keep the fact, drop the provenance' "$@"

# A competing implementation cited in source as justification. The README's
# compatibility chapter is a different act — users need to know which mods to
# remove — so it is out of scope here and stays.
COMPETITORS=$(printf '%s\n' "$SRC" | grep -v '^README\.md$' || true)
# shellcheck disable=SC2086
set -- $COMPETITORS
banned_anywhere 'SiliconPatch|[Ss]ilicon[ _-][Pp]atch|[Ww]eird[Pp]erformance|weirdperformance' \
    'Release hygiene' \
    'cites a competing mod as justification — state the behaviour in its own terms' "$@"

# A citation of a source file the reader cannot open. The behavioural claim
# never needed it: keep "the addresses are 1.12-only", drop "(foo.cpp:34)".
banned_in_comments '[A-Za-z0-9_/.-]+\.(c|cc|cpp|h|hpp|cs|py):[0-9]+' \
    'Release hygiene' \
    'cites an external source file by line — keep the fact, drop the citation' "$@"

# shellcheck disable=SC2086
set -- $RELEASE

# A reference to a private note. `project_`/`feedback_` alone is far too wide
# here — this tree has 41 legitimate `project_vertex`-style identifiers — so the
# rule keys on the wiki-link and filename forms a note reference actually takes.
banned_in_comments '\[\[(project|feedback)_|(^|[^A-Za-z0-9_])(project|feedback)_[a-z0-9_]{4,}\.md' \
    'Release hygiene' \
    'references a private note — restate the reasoning inline' "$@"

# The source is not a changelog. "commit" must be followed by an actual hash to
# count — otherwise "see committed state" reads as a reference to a commit.
banned_in_comments '\bbit us\b|[Uu]ntil commit|commit [0-9a-f]{7,}|20[0-9][0-9]-[01][0-9]-[0-3][0-9]' \
    'Release hygiene' \
    'incident provenance — state the invariant, not the history that produced it' "$@"

banned_in_comments '[Cc]laude|CLAUDE|[Aa]nthropic|Copilot|ChatGPT|(^|[^A-Za-z])[Gg]enerated with|GPT-[0-9]' \
    'Release hygiene' \
    'tooling signal' "$@"

banned_anywhere '/Users/|/home/[a-z]' \
    'Release hygiene' \
    'absolute personal path' "$@"

if [ $status -eq 0 ]; then
    printf 'audit: clean (%d files)\n' "$(git ls-files '*.rs' | wc -l | tr -d ' ')"
fi
exit $status

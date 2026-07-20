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
#   scripts/audit.sh --update-allows  regenerate scripts/allow_inventory.txt
#
# POSIX sh + awk only: no bashisms, and no GNU awk extensions (`\s`, `\b`),
# since the stock macOS awk has neither. Interactive shells here sometimes
# alias `grep` to a wrapper that mishandles multi-file argument lists; this
# runs under `#!/bin/sh`, so it sees the real one.

set -eu

cd "$(git rev-parse --show-toplevel)"

INVENTORY=scripts/derive_inventory.txt
ALLOWS=scripts/allow_inventory.txt

# Files permitted to hold each narrowly-scoped exception. These are sets, not
# counts: a new site fails even if an old one was deleted in the same change.
#
# Lint suppressions do NOT use this shape. A set says only *where* a suppression
# may live, so the largest file in the tree — already on any such list — could
# absorb an unbounded number of new ones silently. They use the inventory+diff
# shape instead (see `allow_scan`), which pins the file, the lint AND the count.
ONCELOCK_SITES='windows/translate/src/hook.rs
windows/turbo/build.rs
windows/turbo/src/win/hooks.rs'

INLINE_ALWAYS_SITES='unix/shared/src/crumb.rs'

# Files permitted a module-level `#![allow(...)]`. Everything else puts the
# suppression on the item that triggers it — see the check below for why.
MODULE_ALLOW_SITES='unix/shared/src/ftol.rs
windows/turbo/build.rs
windows/turbo/src/math/aabb.rs
windows/turbo/src/math/boundsfit.rs
windows/turbo/src/math/collision.rs
windows/turbo/src/math/fmod_mixer.rs
windows/turbo/src/math/frustum.rs
windows/turbo/src/math/gx.rs
windows/turbo/src/math/light.rs
windows/turbo/src/math/lua.rs
windows/turbo/src/math/m2.rs
windows/turbo/src/math/matrix33.rs
windows/turbo/src/math/matrix34.rs
windows/turbo/src/math/matrix44.rs
windows/turbo/src/math/misc.rs
windows/turbo/src/math/movement.rs
windows/turbo/src/math/object.rs
windows/turbo/src/math/particle.rs
windows/turbo/src/math/plane.rs
windows/turbo/src/math/quaternion.rs
windows/turbo/src/math/spline.rs
windows/turbo/src/math/ui.rs
windows/turbo/src/math/vector.rs
windows/turbo/src/math/weather.rs
windows/turbo/src/math/world.rs
windows/turbo/src/win/diff.rs
windows/turbo/src/win/hooks.rs
windows/turbo/src/win/symbols.rs'

# Suppressing a lint GROUP in source is banned outright, with no exception file
# and no inventory row that could legitimise it. This is the shape of the one
# regression this whole layer exists to prevent: a single `#![allow(...)]` naming
# two groups switches off some four hundred lints, and the tree then reports
# clean while the largest file in it goes unlinted. An individual lint can be
# argued for; a group cannot, because nobody can say what is inside it.
LINT_GROUPS='clippy::(nursery|pedantic|all|cargo|complexity|correctness|perf|restriction|style|suspicious)'

# Visibility prefixes are matched because "no `pub(crate)`" pushes any shared
# singleton to plain `pub`, which makes `pub static X: OnceLock<..>` the shape
# this rule most needs to see. `[^=]*` spans the type, so a wrapped generic or a
# fully-qualified path does not slip past.
ONCELOCK_PATTERN='^[ \t]*(pub[^ ]* )?static [^=]*OnceLock'

# The vendored addon is upstream MIT (see THIRD-PARTY-LICENSES.md). Its
# glossary names the servers and zones it exists to translate, so the
# game-name rule cannot apply to it.
VENDORED='addon/'

status=0
WHOLE_TREE=0

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
        # `///` and `//!` are different blocks even with no blank line between
        # them: a module header sitting directly above the first item doc would
        # otherwise read as one long run, and the title of that item would be
        # judged as body text, which is to say never judged at all.
        function marker(l,   t) {
            t = l
            sub(/^[ \t]*/, "", t)
            return (t ~ /^\/\/!/) ? "!" : "/"
        }
        function blank(l,   t) {
            t = text(l)
            gsub(/[ \t\r]/, "", t)
            return t == ""
        }
        FNR == 1 { run = 0; mark = "" }
        {
            # An attribute between a doc block and its item does not end the
            # block; anything else non-doc does.
            if (!is_doc($0)) {
                if ($0 !~ /^[ \t]*#!?\[/) run = 0
                next
            }
            if (marker($0) != mark) run = 0
            mark = marker($0)
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

# The doc comments a build script emits, rendered back into the shape they will
# have in the generated file, so `doc_shape` can judge them.
#
# A build script writes its output as string data, so a `///` inside
# `out.push_str("...")` is a string literal to every tool that reads the script
# and a doc comment to every reader of the result. clippy and rustdoc do see the
# generated file — it is `include!`d into the crate, which is how a missing pair
# of backticks in one of these templates was caught — but the grep-shaped rules
# never do: the generated file is untracked, so it is not in `git ls-files`.
#
# The transform emits one line per input line, so reported line numbers are the
# build script's own. Lines that are not doc templates become a non-doc
# placeholder, which is what ends a doc block for `doc_shape`.
build_doc_templates() {
    awk '
        {
            if (match($0, /push_str\("\/\/[\/!]/)) {
                s = substr($0, RSTART + 10)     # past `push_str("`, keeping the `///`
                sub(/\\n".*$/, "", s)           # drop the trailing newline escape and beyond
                gsub(/\\"/, "\"", s)
                print s
            } else {
                print "//"                       # not a doc line: ends any run
            }
        }
    ' "$1"
}

# Every type deriving Clone and/or Copy, as `path Type Derives`. The committed
# inventory is diffed against this, so a speculative derive cannot slip in
# unnoticed: adding one means consciously recording it.
#
# The attribute is accumulated until its closing bracket rather than read off one
# line, because both shapes that hide a derive from a single-line match are ones
# the tree can produce on its own: rustfmt breaks a long derive list across lines,
# and `#[cfg_attr(test, derive(Clone))]` nests the derive inside another attribute.
derive_scan() {
    awk '
        function derive_list(b,   p, rest, i) {
            p = index(b, "derive(")
            if (p == 0) return ""
            rest = substr(b, p + 7)
            i = index(rest, ")")
            if (i > 0) rest = substr(rest, 1, i - 1)
            gsub(/[ \t\r]/, "", rest)
            sub(/,$/, "", rest)
            return rest
        }
        function consider(   l) {
            l = derive_list(abuf)
            if (l ~ /(^|,)Clone($|,)/ || l ~ /(^|,)Copy($|,)/) {
                pending = l
                pfile = FILENAME
            }
            abuf = ""
        }
        function closed(l) {
            sub(/[ \t\r]+$/, "", l)
            return l ~ /\]$/
        }
        FNR == 1 { pending = ""; abuf = ""; inattr = 0 }
        {
            line = $0
            sub(/\/\/.*$/, "", line)
            if (inattr) {
                abuf = abuf line
                if (closed(line)) { inattr = 0; consider() }
                next
            }
            if (line ~ /^[ \t]*#\[/) {
                abuf = line
                if (closed(line)) consider(); else inattr = 1
                next
            }
            if (line ~ /^[ \t\r]*$/) next
            if (pending != "") {
                if (match(line, /(struct|enum|union)[ \t]+[A-Za-z_][A-Za-z0-9_]*/)) {
                    name = substr(line, RSTART, RLENGTH)
                    sub(/^(struct|enum|union)[ \t]+/, "", name)
                    printf "%s %s %s\n", pfile, name, pending
                }
                pending = ""
            }
        }
    ' "$@" | LC_ALL=C sort
}

# Every suppression must say why the structural fix does not apply. The argument
# is either a comment inside the attribute (the shape a multi-lint module header
# uses, one line of prose per lint) or a comment directly above it.
#
# clippy has `allow_attributes_without_reason` for this, but it demands
# `reason = "..."` and so rejects both of those shapes; it would fire on every
# attribute in the tree and be satisfied by moving the same words inside a string.
# The rule worth enforcing is that an argument EXISTS, not where it is spelled.
allow_argued() {
    awk '
        function check() {
            if (!argued && buf !~ /\/\//)
                printf "%s:%d: suppression with no argument; say why the structural fix does not apply\n", \
                    FILENAME, attrline
            buf = ""
        }
        FNR == 1 { lastc = 0; inattr = 0; buf = "" }
        {
            if (inattr) {
                buf = buf $0
                if ($0 ~ /\)\]/) { inattr = 0; check() }
                next
            }
            if ($0 ~ /^[ \t]*#!?\[(allow|expect)\(/) {
                buf = $0
                argued = lastc
                attrline = FNR
                if ($0 ~ /\)\]/) check(); else inattr = 1
                next
            }
            # A sibling attribute between the comment and the allow does not
            # break the association: `#[cfg(test)]` routinely sits between them.
            if ($0 ~ /^[ \t]*#!?\[/) next
            if ($0 ~ /^[ \t\r]*$/) { lastc = 0; next }
            # A plain `//` note, not `///` or `//!`. A doc comment documents the
            # item; it is not an argument for suppressing a lint on it, and
            # accepting one would let every module header pass on its `//!`
            # header alone.
            lastc = ($0 ~ /^[ \t]*\/\//) && ($0 !~ /^[ \t]*\/\/[\/!]/)
        }
    ' "$@"
}

# Every lint suppression in the tree, as `path lint count`. Covers the outer
# `#[allow(...)]`, the inner `#![allow(...)]`, `#[expect(...)]` and the
# `#[cfg_attr(..., allow(...))]` form, and accumulates multi-line attributes,
# so no spelling of a suppression is invisible to the diff.
#
# This is deliberately an inventory rather than a permitted-file set. A set can
# only answer "may this file hold a suppression at all", which grants every file
# already on it an unbounded budget — and the file carrying most of them is the
# largest in the tree. Keying on the lint and the count instead means a new file,
# a new lint in a listed file, one more site, and an entry that has gone stale
# all show up as a diff line somebody has to look at.
allow_scan() {
    awk '
        function flush(   p, q, rest, i, n, arr) {
            p = index(buf, "allow(")
            q = index(buf, "expect(")
            if (p == 0 || (q > 0 && q < p)) p = q
            if (p == 0) { buf = ""; return }
            rest = substr(buf, p)
            sub(/^(allow|expect)\(/, "", rest)
            i = index(rest, ")")
            if (i > 0) rest = substr(rest, 1, i - 1)
            n = split(rest, arr, ",")
            for (i = 1; i <= n; i++) {
                gsub(/[ \t\r]/, "", arr[i])
                if (arr[i] != "") printf "%s %s\n", FILENAME, arr[i]
            }
            buf = ""
        }
        FNR == 1 { buf = ""; inattr = 0 }
        {
            line = $0
            # Drop comments first: an allow list carries its justification inline,
            # and a `)]` inside that prose would otherwise close the attribute early.
            sub(/\/\/.*$/, "", line)
            if (!inattr) {
                if (line !~ /#!?\[(allow|expect|cfg_attr)\(/) next
                inattr = 1
                buf = line
            } else {
                buf = buf line
            }
            if (line ~ /\)\]/) { inattr = 0; flush() }
        }
    ' "$@" | LC_ALL=C sort | uniq -c |
        awk '{ printf "%s %s %s\n", $2, $3, $1 }'
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
#
# `//` and `--` are matched anywhere on the line, not just at its start: a
# trailing comment after code is still a comment, and the tree carries several
# hundred of them. `#` stays anchored — unanchored it would match any line
# containing a `#`, which in Rust is every attribute.
banned_in_comments() {
    pattern=$1
    section=$2
    message=$3
    shift 3
    hits=$(grep -HnE "(//|--|^[ \t]*(#|\*)).*($pattern)" "$@" || true)
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

# A pattern confined to a known set of files: flag any file outside the set, and
# any entry in the set that no longer matches anything.
#
# The stale half matters as much as the stray half. Each of these files earned
# its exception with an argument recorded in docs/CONVENTIONS.md; once the last
# occurrence is gone the permission has outlived what justified it, and leaving
# it in place quietly re-grants it to whatever gets added next.
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
    # Only meaningful over the whole tree: in --file mode the other permitted
    # files are simply not in the argument list.
    if [ "$WHOLE_TREE" = 1 ]; then
        stale=''
        for entry in $permitted; do
            if ! printf '%s\n' "$hits" | grep -qxF "$entry"; then
                stale="$stale$entry
"
            fi
        done
        if [ -n "$stale" ]; then
            report "$section" "stale exception: recorded file no longer matches" \
                "$(printf '%s' "$stale")"
        fi
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
--update-allows)
    # shellcheck disable=SC2046
    allow_scan $(git ls-files '*.rs') >"$ALLOWS"
    printf 'wrote %s (%d file/lint pairs)\n' "$ALLOWS" "$(wc -l <"$ALLOWS" | tr -d ' ')"
    exit 0
    ;;
--file)
    file=${2:?--file needs a path}
    # An editor hook's view: only the checks a single file can answer on its own.
    # Deliberately NOT the release-hygiene rules — those are scoped by exclusion
    # sets (the vendored addon, the licence texts, this script) that a lone path
    # cannot reconstruct, so running them here would report the exempt files as
    # violations. Nor the inventory diffs, which are whole-tree by definition.
    # `make audit` remains the gate; this is edit-time feedback, not a substitute.
    case $file in
    *.rs) ;;
    *)
        printf 'audit: --file covers Rust sources only; %s not checked\n' "$file" >&2
        exit 0
        ;;
    esac
    [ -f "$file" ] || exit 0

    findings=$(doc_shape "$file")
    [ -z "$findings" ] || report 'Doc comments' 'doc-comment shape' "$findings"

    banned 'pub\(crate\)|pub *\(in ' 'No pub(crate) — use module hierarchy' \
        'restricted visibility' "$file"
    banned "$LINT_GROUPS" 'Warning suppressions' \
        'a lint group suppressed in source; name the individual lints' "$file"
    findings=$(allow_argued "$file")
    [ -z "$findings" ] || report 'Warning suppressions' \
        'suppression without an argument' "$findings"
    confined '^[ \t]*#!\[(allow|expect)\(' 'Warning suppressions' \
        'module-level suppression outside the recorded exception files; put it on the item' \
        "$MODULE_ALLOW_SITES" "$file"
    confined 'inline\(always\)' 'Inline attributes' \
        '#[inline(always)] outside the recorded exception file' "$INLINE_ALWAYS_SITES" "$file"
    confined "$ONCELOCK_PATTERN" 'LazyLock over OnceLock' \
        'OnceLock static outside the recorded exception files' "$ONCELOCK_SITES" "$file"

    exit $status
    ;;
-h | --help)
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac

# --- whole-tree audit --------------------------------------------------------

WHOLE_TREE=1

# Every check below is driven by a file list built from `git ls-files`. If one of
# those expansions ever yields nothing — a quoting change, a `cd` that lands
# somewhere else — grep is handed no files, the `|| true` in each helper swallows
# the error, and the script reports a clean tree having scanned nothing. Assert
# the set is populated at the point it is built, so that failure is loud.
require_files() {
    [ "$1" -ge "$2" ] || {
        printf 'audit: internal error: expected at least %d files, got %d\n' "$2" "$1" >&2
        exit 2
    }
}

# shellcheck disable=SC2046
set -- $(git ls-files '*.rs')
require_files $# 40

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

for script in $(git ls-files '*build.rs'); do
    rendered=$(mktemp)
    build_doc_templates "$script" >"$rendered"
    findings=$(doc_shape "$rendered" | sed "s|$rendered|$script|")
    rm -f "$rendered"
    [ -z "$findings" ] || report 'Doc comments' \
        'doc-comment shape in a generated-source template' "$findings"
done

drift=$(allow_scan "$@" | diff "$ALLOWS" - || true)
if [ -n "$drift" ]; then
    report 'Warning suppressions' \
        "allow inventory drift (< committed, > working tree). A suppression needs an argument recorded in docs/CONVENTIONS.md; record it with scripts/audit.sh --update-allows" \
        "$drift"
fi

banned "$LINT_GROUPS" 'Warning suppressions' \
    'a lint group suppressed in source; name the individual lints' "$@"

findings=$(allow_argued "$@")
if [ -n "$findings" ]; then
    report 'Warning suppressions' \
        "suppression without an argument: $(printf '%s\n' "$findings" | wc -l | tr -d ' ') findings" \
        "$findings"
fi

# A module-level `#![allow(...)]` silences its lint for the whole file, including
# every item added to it later — the same unbounded budget the enumerated table
# exists to avoid, one level down. Suppressions belong on the item that triggers
# them. The files below are the recorded exceptions, and each one is a whole-file
# property rather than a property of any item in it: the reimplementation naming
# convention marks the host C++ `::` with `__`, so `non_snake_case` fires on
# essentially every name in those files, and the generated symbol table is
# machine-written throughout.
confined '^[ \t]*#!\[(allow|expect)\(' 'Warning suppressions' \
    'module-level suppression outside the recorded exception files; put it on the item' \
    "$MODULE_ALLOW_SITES" "$@"

banned 'impl( *<[^>]*>)? *(Clone|Copy) for ' 'No default Copy / Clone on aggregate structs' \
    'hand-written Clone/Copy impl — the derive inventory cannot see it' "$@"

# The two doc spellings the shape check cannot parse. Both are legal Rust and
# neither appears in the tree; banning them keeps `doc_shape` a complete rule
# rather than one with a documented way around it.
banned '/\*\*|#\[doc *=' 'Doc comments' \
    'block or attribute doc comment — use /// or //! so the shape check applies' "$@"

banned 'pub\(crate\)|pub *\(in ' 'No pub(crate) — use module hierarchy' \
    'restricted visibility' "$@"

confined 'inline\(always\)' 'Inline attributes' \
    '#[inline(always)] outside the recorded exception file' "$INLINE_ALWAYS_SITES" "$@"
confined "$ONCELOCK_PATTERN" 'LazyLock over OnceLock' \
    'OnceLock static outside the recorded exception files' "$ONCELOCK_SITES" "$@"

# --- manifests ---------------------------------------------------------------
#
# The lint tables are the tree's primary enforcement surface, and both ways they
# can be silently disabled are invisible to every other check here. `[lints]
# workspace = true` is opt-in per crate and cargo does not warn when it is
# missing, so a new member simply has pedantic and nursery switched off. And a
# lint set to `deny` aborts its crate's run at the first hit, which stops cargo
# scheduling the units that depend on it — their findings never appear at all.
# `warn` plus cargo's `build.warnings = "deny"` reports everything, then fails.
# shellcheck disable=SC2046
set -- $(git ls-files '*Cargo.toml')
require_files $# 8

for manifest in "$@"; do
    case $manifest in
    windows/Cargo.toml | unix/Cargo.toml)
        for group in nursery pedantic; do
            grep -qE "^$group = \{ level = \"warn\", priority = -1 \}" "$manifest" ||
                report 'Warning suppressions' \
                    "$manifest: $group is not warn/-1 in [workspace.lints.clippy]"
        done
        ;;
    *)
        grep -A1 '^\[lints\]' "$manifest" | grep -q 'workspace *= *true' ||
            report 'Warning suppressions' \
                "$manifest: missing [lints] workspace = true, so the workspace lint table does not apply"
        ;;
    esac
done

banned 'level *= *"(deny|forbid)"|^[a-z_0-9]+ *= *"(deny|forbid)"' 'Warning suppressions' \
    'deny/forbid lint level — warn plus build.warnings="deny" is the gate' "$@"

# shellcheck disable=SC2046
set -- $(git ls-files '*.rs')

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
banned_in_comments "[Gg]hidra|GHIDRA|[Dd]ecompil|[Dd]isassembl|[Dd]isasm|(^|[^A-Za-z])IDA([^A-Za-z]|\$)|[Rr]everse[ -]engineer|[Rr]eversed from|(^|[^A-Za-z])RE'(d|ed)([^A-Za-z]|\$)|[Ll]eaked (symbol|build|binar|client|pdb|source|debug)" \
    'Release hygiene' \
    'names a decompiler, a disassembler, or the workflow — keep the fact, drop the provenance' "$@"

# The same rule, applied to what the tool actually leaves behind. Naming the
# decompiler is the obvious form of provenance; pasting its output is the
# literal one, and it is the form that survives a search for the word. The
# address families carry a real fact and keep it — `FUN_006acdd0` becomes
# `0x6acdd0` — while the register-typed locals name a temporary that only ever
# existed in the tool's rendering, so those get a name describing what they hold.
#
# `local_[0-9a-f]+` is deliberately absent: this tree has hand-chosen names like
# `local_end` and `local_box` meaning model-local space, which match it only
# because `e` and `b` are hex digits. The families below have no such collision.
banned_anywhere '(^|[^A-Za-z0-9_])(FUN|DAT|_DAT)_[0-9a-f]{6,}|(^|[^A-Za-z0-9_])(([a-z]{1,4}Var[0-9]+)|([iu]Stack_[0-9a-f]+)|(param_[0-9]{3})|(extraout_|unaff_|in_(EAX|ECX|EDX|EBX|ESI|EDI)))' \
    'Release hygiene' \
    'a decompiler-generated identifier — keep the address, drop the tool naming' "$@"

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

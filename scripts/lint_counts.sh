#!/bin/sh
#
# Verify the finding counts annotated against each exempted lint in the
# workspace manifests, and rewrite them when they have moved.
#
#   scripts/lint_counts.sh            check the committed counts, exit 1 on drift
#   scripts/lint_counts.sh --update   rewrite the manifests with current counts
#
# docs/CONVENTIONS.md §"Warning suppressions" stakes the design on these numbers:
# the enumerated exemption is defensible only because each entry carries what it
# reports today, so the debt is something somebody can watch instead of an
# unknown behind a group name. Nothing else in the tree recomputes them, and a
# count nobody recomputes is prose. This is the recomputation.
#
# Deliberately NOT part of `make check`. Each leg force-warns the exempted lints,
# which changes the compiler flags and so cannot share `check`'s build cache —
# it is minutes, not the ~5s the rest of the gate costs. Run it when you touch a
# lint table; `make check` stays cheap enough to run on every commit.
#
# Counts come from `--force-warn`, which overrides a source-level `#![allow]` as
# well as the manifest one. That is intended: the number is what the lint finds,
# not what survives the other suppressions. `scripts/allow_inventory.txt` is
# where source-level suppressions are accounted for.

set -eu

cd "$(git rev-parse --show-toplevel)"

MODE=${1:-check}

# Each leg is a workspace, a directory, and a pinned invocation. The invocation
# lives here rather than in a manifest comment so that two people recounting
# cannot get different answers from the same table.
leg_cmd() {
    case $1 in
    windows) echo "cargo clippy -p wow-turbo-dll --target i686-pc-windows-msvc --all-targets" ;;
    unix) echo "cargo clippy --all-targets" ;;
    esac
}

# The exempted lints of a manifest: every `name = "allow"` in its lint table.
lints_of() {
    sed -n 's/^\([a-z_0-9]*\) = "allow".*/\1/p' "$1/Cargo.toml"
}

# Count findings per lint in one leg. One clippy run with every exempted lint
# force-warned, JSON output keyed by lint name — 35 separate runs would rebuild
# the crate 35 times for the same answer.
counts_of() {
    ws=$1
    flags=''
    for lint in $(lints_of "$ws"); do
        flags="$flags --force-warn clippy::$lint"
    done
    # shellcheck disable=SC2046
    (cd "$ws" && $(leg_cmd "$ws") --message-format=json -- $flags 2>/dev/null) |
        python3 -c '
import json, sys, collections

# Deduplicate on (lint, file, line, column). `--all-targets` compiles a lib
# crate twice — once as the lib, once as the test harness — so every finding in
# shared source is emitted twice. Counting raw doubles the whole table. Keying
# on the primary span collapses those without losing findings that exist only
# in a test target.
seen = set()
n = collections.Counter()
for line in sys.stdin:
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") != "compiler-message":
        continue
    msg = m.get("message") or {}
    name = (msg.get("code") or {}).get("code") or ""
    if not name.startswith("clippy::"):
        continue
    spans = [s for s in (msg.get("spans") or []) if s.get("is_primary")]
    if spans:
        s = spans[0]
        key = (name, s.get("file_name"), s.get("line_start"), s.get("column_start"))
    else:
        key = (name, msg.get("rendered"))
    if key in seen:
        continue
    seen.add(key)
    n[name[8:]] += 1
for k in sorted(n):
    print(k, n[k])
'
}

status=0

for ws in windows unix; do
    printf 'lint-counts: measuring %s (%s)\n' "$ws" "$(leg_cmd "$ws")" >&2
    counts=$(counts_of "$ws")

    for lint in $(lints_of "$ws"); do
        have=$(printf '%s\n' "$counts" | awk -v l="$lint" '$1 == l { print $2 }')
        [ -n "$have" ] || have=0
        # The committed number, if the line carries one.
        want=$(sed -n "s/^$lint = \"allow\"[ \t]*#[ \t]*\([0-9][0-9]*\).*/\1/p" "$ws/Cargo.toml")

        if [ "$MODE" = --update ]; then
            # Rewrite the trailing count, preserving any prose after it.
            python3 - "$ws/Cargo.toml" "$lint" "$have" <<'PY'
import re, sys
path, lint, n = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(path).read()
pat = re.compile(r'^(%s = "allow")([ \t]*)(#[ \t]*)(\d+)?(.*)$' % re.escape(lint), re.M)
def sub(m):
    pad = m.group(2) or "  "
    return f'{m.group(1)}{pad}{m.group(3)}{n}{m.group(5)}'
new, k = pat.subn(sub, src)
if k == 0:
    # No annotation yet: append one, aligned the way the table already is.
    new = re.sub(r'^(%s = "allow")[ \t]*$' % re.escape(lint),
                 lambda m: f'{m.group(1)}'.ljust(40) + f'# {n}', src, flags=re.M)
open(path, "w").write(new)
PY
            continue
        fi

        if [ -z "$want" ]; then
            printf '  %-34s no committed count (reports %s)\n' "$lint" "$have" >&2
            status=1
        elif [ "$want" != "$have" ]; then
            printf '  %-34s committed %s, reports %s\n' "$lint" "$want" "$have" >&2
            status=1
        fi
    done
done

if [ "$MODE" = --update ]; then
    printf 'lint-counts: manifests rewritten\n'
    exit 0
fi

if [ $status -eq 0 ]; then
    printf 'lint-counts: every annotated count matches\n'
else
    printf '\nlint-counts: annotated counts have drifted.\n' >&2
    printf 'Rewrite them with `make lint-counts-update`, then read the diff:\n' >&2
    printf 'a count that moved without the code moving means the exemption\n' >&2
    printf 'changed scope. docs/CONVENTIONS.md § Warning suppressions.\n' >&2
fi
exit $status

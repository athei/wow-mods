# Conventions

Code-organisation and style rules enforced across the codebase. New code follows these without exception; existing code is brought along on any touch.

## Mechanical audit

Most of this document is enforced by `make check` — formatting, clippy with `nursery` + `pedantic` enabled workspace-wide, the audit, the docs, the annotated lint counts, and the tests, over every target and every `cfg` (see §One gate, no lighter subset). Lint levels are `warn` in the Cargo manifests so a plain `cargo clippy` (or an editor) reports without blocking a build; the check legs pass cargo's `build.warnings = "deny"` config, which fails the run if any warning was emitted. The default-`cfg` legs change no compiler flags and so share the build cache with plain invocations; the rest keep their own.

But a lint can only express a lint-shaped rule, and the rules a lint *can't* express are exactly the ones that drift — nothing runs them, so they decay into prose nobody rereads. `make audit` (`scripts/audit.sh`) closes that gap, and `make check` runs it, so a violation fails the same gate as a clippy warning. It covers:

| Check | Section |
| --- | --- |
| Doc-block shape (title / blank / body) and the 100-byte doc line cap | §Doc comments |
| The `Clone` / `Copy` derive inventory, diffed against `scripts/derive_inventory.txt` | §No default `Copy` / `Clone` |
| The lint-suppression inventory, diffed against `scripts/allow_inventory.txt` | §Warning suppressions |
| A lint **group** suppressed in source = 0, with no exception | §Warning suppressions |
| Every suppression carries an argument | §Warning suppressions |
| Module-level `#![allow]` confined to the recorded exception files | §Warning suppressions |
| Doc-block shape in the doc comments a build script *emits* | §Doc comments |
| Every crate opts into the workspace lint table; no `deny` / `forbid` level | §Warning suppressions |
| `#[inline(always)]` confined to the recorded exception file | §Inline attributes |
| `static … : OnceLock` confined to the recorded exception files | §`LazyLock` over `OnceLock` |
| `pub(crate)` and `pub(in …)` = 0 | §No `pub(crate)` |
| Hand-written `Clone` / `Copy` impls = 0 | §No default `Copy` / `Clone` |
| Block (`/** … */`) and attribute (`#[doc = …]`) doc comments = 0 | §Doc comments |
| `mod.rs` files = 0 | §Module style |
| Release hygiene (the pattern-matchable rules) | §Release hygiene |

Every finding names the section it came from. The confined-pattern checks compare **sets of files**, not counts, so moving an exception to a new file fails even though the count is unchanged — and a recorded file that stops matching fails too, because a permission that has outlived its last use silently re-grants itself to whatever gets added next.

Lint suppressions do **not** use that shape, and the difference is the point. A set of files can only answer "may this file hold a suppression at all", which hands every file already on the list an unbounded budget — and the file holding most of them is the largest in the tree. `scripts/allow_inventory.txt` records the file, the lint **and** the count, so one more site is a diff line somebody has to look at.

`scripts/audit.sh --file <path>` runs the per-file subset — doc shape, `pub(crate)`, the lint-group ban, the suppression-argument check and the confined patterns — for an editor hook that wants feedback at edit time. It deliberately skips the release-hygiene rules, which are scoped by exclusion sets (the vendored addon, the licence texts, this document) that a lone path cannot reconstruct, and the inventory diffs, which are whole-tree by definition. It is edit-time feedback, not a substitute for `make audit`.

### One gate, no lighter subset

`make check` is everything the tree can check without a running client, and there is deliberately no faster variant to run instead.

The reason is a hole it used to have. A default build compiles the tree one way, so code behind a `cfg` was invisible to it — neither the differential harness nor the breadcrumb ring gets built, leaving around 455 lines of unsafe mmap and Win32 FFI, plus every generated `*_diff` capture function, linted by nothing. The first run with `CRUMB=1` turned up three findings that had been sitting there since the day that code was written, two of them functions carrying a panic path their own comments argued could not be reached. `PERF=1` joined them when the diagnostic layer stopped being compiled into every build: the counters, the tripwires and the script gauge are the largest body of code behind a `cfg` now, and the default legs no longer see any of it.

Those legs briefly lived in a separate, fuller target. That is the arrangement that produced the hole in the first place: the lighter gate is the one that gets run, and whatever sits outside it stops being true. So they are in `check`, and `check` is what runs before a commit.

It costs around fifty seconds against warm target dirs. Most of that is the legs that cannot share a build cache — each `cfg`, and the force-warn counts, compile under different flags, so cargo fingerprints them separately.

One of its legs is also useful alone, and one target feeds it without being part of it:

- **`make lint-counts`** re-measures the finding count annotated against every exempted lint in the workspace manifests, and fails if one has moved (it is a leg of `check`). `make lint-counts-update` rewrites them.
- **`make update-inventories`** regenerates both committed inventories after a deliberate change. It is not a leg of `check` — `check` only *diffs* the inventories; this is what you run to update them first.

**What no target covers**, and what none can: the differential harness itself. `DIFF=1 make install`, then exercise the touched arms against a live client. See §The reimplementation contract — that comparison is the definition of done, and nothing static substitutes for it.

### `make doc`

`make check` also builds the docs (with the same `build.warnings = "deny"` config, which counts rustdoc warnings too), because rustdoc holds the last set of rules nothing else sees: whether a doc link actually **resolves**. Audit gates the *shape* of a doc block and clippy gates its *prose*; only rustdoc knows that ``[`foo`]`` points at something real and that a public item isn't linking to a private one the reader can't open.

Two traps worth knowing, since both were live in the tree:

- **A public doc may not link to a private item.** `wow_turbo`'s crate doc linked to its own `math` module, which is private; the fix is backticks, not a link.
- **A link resolves against the crate's actual dependencies.** ``[`Box::from_raw`](alloc::boxed::Box::from_raw)`` fails in a crate that uses `std` and never names `alloc`.

The windows workspace is documented for the **i686 PE target**, not the host: the mods are `cdylib`s that don't build for the host, so a host-only run would silently skip them.

## The reimplementation contract

This is the rule that overrides Rust instinct, and the one most likely to be got wrong by someone arriving from ordinary application code.

`wow_turbo` replaces functions inside a running game client. A replacement is correct when it reproduces the **machine behaviour** of the code it stands in for — not when it is idiomatic, and not when it is safer. A differential harness (`DIFF=1`, then `WOW_TURBO_DIFF_ARM=all|Name` at runtime) runs the original and the replacement side by side and compares results, and that comparison is the definition of done.

What follows from that:

- **A narrowing conversion must keep narrowing silently.** The ordinary fix for `clippy::cast_possible_truncation` is `T::try_from(v).expect("…")`, turning a silent truncation into a loud panic. Here that is a bug: the original truncated, callers depend on it, and a panic on a per-frame path is a client crash. Write the `as` cast.
- **Float comparison is exact on purpose.** `clippy::float_cmp` argues for an epsilon. The harness compares bit patterns, and so does the code under test.
- **Rounding goes through `ftol`.** x87 and SSE disagree about how a float becomes an integer, and the client's behaviour depends on which one it used. Route `i64`/`f64` conversions through the shared `ftol` helper rather than inventing a cast.
- **`mul_add` changes the answer.** Fusing a multiply and an add removes an intermediate rounding, so the result stops matching the original and the harness comparison fails. On the default baselines it is also slower — nehalem for the Wine-on-macOS build and x86-64-v2 for the host kernels have no hardware FMA, so `mul_add` lowers to an `fmaf` libcall — but the AVX2 native-Windows variant *does* have FMA, so the rounding argument is the one that carries `suboptimal_flops` being allowed workspace-wide.
- **An argument list that mirrors a calling convention is not refactorable.** `clippy::too_many_arguments` on a hook is a comment on the client's ABI, not on our design.
- **NaN polarity is load-bearing.** Comparisons written `!(a >= b)` or `x < lo || x >= hi` differ from `a < b` and `Range::contains` when an operand is NaN, and they are written that way to match the original's ordered tests. `clippy::neg_cmp_op_on_partial_ord` and `clippy::manual_range_contains` are allowed for this.

None of this licenses sloppiness elsewhere. It applies to the reimplementation surface — `windows/turbo/src/math/**`, `windows/turbo/src/win/hooks.rs`, and the numeric kernels in `unix/shared` — and nowhere else. Anything outside it follows ordinary Rust.

**Verifying a change here means running the harness, not just a green build.** `DIFF=1 make install`, then exercise the touched paths with the relevant arm enabled. A build that compiles and a test that passes say nothing about whether the replacement still matches.

**But first check that the entry you touched even has a harness.** Only the entries carrying a `[diff]` table in `symbols.toml` are armable — a minority, and only those with a `*mut` out region or a scalar return can carry one at all; `build.rs` refuses the rest. A hook without a table has no diff path, so an armed run over it is *silent by construction* — it proves nothing about your change, however green it looks. If yours has no table, add one when the signature admits it; otherwise the verification is in-game observation, and say so in the change rather than implying the harness covered it.

## Release hygiene

The repository is public. Source should read as the engineering itself — not as a diary of how the engineering got written, and not as a document that assumes the reader has access to things they do not. `make audit` enforces most of the rules below by pattern — the private server's name, decompiler/disassembler words, tool-generated identifiers, dates, commit hashes, absolute paths, by-line source citations. What it cannot pattern-match is *tool names in general* and *workflow narration*: naming a build-side script the reader cannot open is against the rule but slips the grep, so those two bullets are on the author to honour.

Do not write, anywhere in first-party source:

- **The private server's name, or the game's title spelled out in full.** Say "the client" or "the launcher". The `WoW` abbreviation is fine, and so are `WoW 1.12`, `WoW 3.3.5a` and the client's virtual addresses — those are the subject matter. The vendored addon under `addon/` is exempt by scope: its glossary names servers and zones because translating them is what it does.
- **How the client was studied.** No decompiler, no disassembler, no tool names, no workflow narration. The technical claim is what belongs in the comment: "the `preserve` set is machine-checked against the binary" says everything a reader can act on; naming the script that does the checking does not, because they do not have it. (A register-stack *leak* is a real phenomenon this codebase describes, and the rule is written not to fire on it.)
- **An identifier a tool generated rather than a human chose.** Naming the decompiler is the obvious form of that; pasting its output is the literal one, and it is the form that survives a search for the tool's name. `FUN_006acdd0` and `_DAT_00801628` keep their fact and lose the naming — write `0x6acdd0`, `0x801628`, which is the house form for an address anyway. `fVar1`, `iVar4`, `param_224` and `extraout_EAX` name a temporary that only ever existed in the tool's rendering: give it a name describing what it holds, and prefer the binding the Rust reimplementation right below already uses. If a comment names a temporary with no describable role, restate the relationship without naming it.

  Two collisions the rule deliberately tolerates, both real names in this tree: `local_end` / `local_box` mean *model-local space* and match the `local_<hex>` shape only because `e` and `b` are hex digits, and `blend_sort_param_224__70c190` takes its `224` from the `fmul 224.0` it reproduces.
- **A competing mod cited as justification.** State the behaviour in its own terms. The README's compatibility chapter is a different act — users genuinely need to know which mods to remove or load first — and it is out of scope for this rule. Naming a mod for *interoperability* reasons is also fine and deliberately not banned: those notes record real constraints, and deleting them would delete safety-critical information.
- **A citation of a source file the reader cannot open** (`foo.cpp:34`). Keep the fact, drop the citation.
- **A reference to a private note**, in `[[wiki-link]]` or filename form.
- **Incident provenance**: a date, a commit hash, "until commit `abc1234`", "the bug that bit us". State the invariant and why it holds. If a comment only makes sense to someone who knows what changed, restate the change as prose.
- **A tooling signal**, or an absolute `/Users/...` path.

Several files are out of scope, and the audit excludes them by name: the two `Cargo.lock` files, `LICENSE`, `THIRD-PARTY-LICENSES.md`, the vendored addon's `LICENSE`, and — because they quote the patterns they ban in order to state the rule — `scripts/audit.sh` and this document. `THIRD-PARTY-LICENSES.md` reproduces MinHook's BSD notice, which names a disassembler engine and must keep doing so.

## Warning suppressions

Default rule: **never** `#[allow(...)]`, `#[expect(...)]`, or `#[cfg_attr(..., allow(...))]`. The lint is followed. `make check` runs clippy with `clippy::nursery` and `clippy::pedantic` enabled workspace-wide, denies every warning via `build.warnings = "deny"`, and **stays clean by fixing the code, not by silencing the lint**.

### Never a lint group

Suppressing a lint **group** in source — `clippy::pedantic`, `clippy::nursery`, `clippy::all` — is banned outright. There is no exception file and no inventory row that can legitimise one, and `make audit` fails on the pattern wherever it appears.

This is the shape of the one regression the whole layer exists to prevent. An individual lint can be argued for; a group cannot, because nobody can say what is inside it, and the set grows with every toolchain.

### The enumerated exemption

There is one large, deliberate exception. Most of it is recorded in the workspace manifests; a smaller second tier lives in the source.

Every reimplementation module used to open with a blanket `#![allow(clippy::pedantic, clippy::nursery)]` — twenty-one files carrying some variation of it. That switched off two whole lint groups, roughly four hundred lints, so that a few dozen of them would stay quiet, and it meant `cargo clippy` reported a clean tree while the largest file in the repository went unlinted.

The argument behind the blanket was sound (see §The reimplementation contract). The mechanism was not. So `[workspace.lints.clippy]` in `windows/Cargo.toml` and `unix/Cargo.toml` now names each exempt lint individually, grouped by *why* it is exempt and annotated with what it reports today. The counts are the point: they turn an unknown behind a group name into a number somebody can watch, and a lint that is not on the list now fires. `make lint-counts` is what keeps those numbers honest — a count nobody recomputes is prose, and these had already drifted once.

The second tier is the module-level `#![allow(…)]` headers the reimplementation files still carry: `non_snake_case` because the adapter names mirror the host's C++ symbols verbatim, and a handful of lints whose argument is genuinely per-file rather than workspace-wide. Every one of them is enumerated in `scripts/allow_inventory.txt` and diffed by `make audit`, so this tier is counted rather than trusted.

Adding a lint to either tier needs the same justification as a per-site allow, plus an argument that it applies across the whole reimplementation surface rather than at one site. Removing one — by fixing its findings — needs nothing but the work.

### A suppression goes on the item, not the file

`#![allow(...)]` at the top of a module silences its lint for everything in the file — including every item added to it later. That is the same unbounded budget the enumerated table exists to avoid, one level down, and it is why twenty-three reimplementation modules each carried an identical four-lint header covering functions that triggered none of them.

So a suppression belongs on the smallest item that triggers it, naming only the lints that item actually raises. `make audit` confines module-level suppressions to a recorded set of files, and the set is short because a file-level allow has to be a property of the *whole file* to earn its place:

- **The reimplementation naming convention.** Adapter and kernel names mirror the host's C++ symbols with `__` marking the `::`, so `non_snake_case` fires on essentially every name in those modules. 1,539 items would each need an attribute to say one thing about all of them. That is a convention, not an exception, and per-item annotation would obscure it rather than sharpen it.
- **Machine-written code.** The generated symbol table is non-idiomatic throughout, by construction.
- **A lint whose trigger is the file's role in code generation, not any item in it.** `build.rs` emits Rust source as string literals, so the generated code's format args read as this script's (`literal_string_with_formatting_args`). `diff.rs`'s reporting API is driven by the manifest's diff annotations, so which of its items a given build references is not something the file can state per item (`dead_code`).

Each recorded file carries its own argument in place; the list is the set the audit enforces, not an exhaustive taxonomy of what could ever qualify.

### Every suppression carries an argument

Not a formality — the audit checks it. The argument may sit inside the attribute (the shape a multi-lint module header uses, a line of prose per lint) or directly above it; an intervening `#[cfg(...)]` does not break the association.

`clippy::allow_attributes_without_reason` is deliberately **not** enabled. It demands `reason = "..."`, which rejects both shapes this codebase uses, and it is satisfied by moving the same words inside a string literal. What is worth enforcing is that an argument exists, not where it is spelled.

### Per-site allows

Every `#[allow]` is recorded in `scripts/allow_inventory.txt` as a file / lint / count row, and the audit diffs it, so a new file, a new lint in a listed file, one more site, and a row that has gone stale all fail. The recurring legitimate classes are:

1. **A lint whose suggestion changes machine behaviour.** `clippy::double_comparisons` rewriting a NaN-aware pair into `!=`; `clippy::assign_op_pattern` reassociating an accumulate that has to stay in the original's order; `clippy::manual_clamp` where `clamp`'s NaN handling differs.
2. **A dead item on one target.** Code reached only from the 32-bit hook path compiles with no caller in a host test build. Use `#[cfg_attr(not(...), allow(dead_code))]` so the allow disappears on the target that does use it, rather than a blanket one.
3. **A block of unsafe operations too dense to split.** See §One operation per unsafe block.

### Standard structural fixes

These handle most of what clippy raises outside the reimplementation surface. Apply the fix; do not allow.

| Lint | Fix |
| --- | --- |
| `cast_possible_truncation` (outside a reimpl) | `T::try_from(v).expect("<contractual bound>")` |
| `cast_sign_loss` | `.cast_unsigned()` / `.cast_signed()` — bit-identical, no panic |
| `cast_ptr_alignment` | `read_unaligned` rather than a plain deref |
| `borrow_as_ptr` | `&raw const x` / `&raw mut x` |
| `too_many_arguments` (outside a hook) | introduce a `FooParams` struct |
| `items_after_statements` | hoist `const` / `type` / `use` to the top of the function |
| `similar_names`, `many_single_char_names` | rename the binding |
| `missing_const_for_fn` | add `const fn` |
| `type_complexity` | a type alias |
| `undocumented_unsafe_blocks` | write the `// SAFETY:` comment |
| `multiple_unsafe_ops_per_block` | split the block, one operation each |
| `doc_markdown`, `too_long_first_doc_paragraph` | fix the doc (see §Doc comments) |

## Unsafe is a last resort

`unsafe` exists for FFI, hook thunks, and reading the client's memory. Anywhere else, find a safe alternative first.

The canonical pattern is a **typed boundary newtype**, not a safe-fn helper: a newtype with an `unsafe fn` constructor and safe methods type-encodes the contract once, the caller writes one `unsafe { Type::new(p) }` per entry, the `// SAFETY:` comment lives at that single assertion site, and downstream methods are safe because the invariant rides on the type. `unix/shared/src/ffi_boundary.rs` holds the set: `InPtr`, `InPtrMut`, `ValueIn`, `OutPtr`, `VtableThis`.

This mirrors the stdlib. `slice::from_raw_parts`, `Box::from_raw` and `Pin::new_unchecked` are all `unsafe fn` because the caller-upheld preconditions cannot be checked at runtime. A "safe" wrapper that internally dereferences a caller-supplied pointer with a documented contract is unsound: the signature accepts any input, but only some inputs satisfy the contract.

### Mandatory `// SAFETY:` comment

Every `unsafe {}` block carries one, directly above it:

    // SAFETY: <invariant the unsafe op relies on>; <why it holds here>.
    let g = unsafe { *((l_addr + 0x10) as *const usize) };

State the invariant being asserted and why it is true here. Bare "FFI call" or "pointer deref" is not acceptable. (The reimplementation surface carries a recorded, measured exception — see §One operation per unsafe block.)

Note that rustfmt moving code can separate a comment from its block: if a `let` gets expanded across lines, a comment above the `let` no longer precedes the `unsafe`, and the lint fires on a block that looks documented. The comment belongs next to the `unsafe`, which for a wrapped `let` means inside the expression:

```rust
let submit: extern "fastcall" fn(*mut f32, u32, *mut u32) =
    // SAFETY: 0x6a98e0 is a `.text` address in the loaded client, verified
    // non-relocated by `win`'s image-base check.
    unsafe { core::mem::transmute(BASE + 0x2a_98e0) };
```

That shape is stable under rustfmt and satisfies the lint; a comment above the `let` is neither.

### One operation per unsafe block

Each dereference, each transmute, each FFI call gets its own block with its own comment. Two reasons: a search for `SAFETY:` returns a focused answer per operation, and refactoring half the block to be safe doesn't strand the comment on the other half.

Four exceptions, all recorded as per-site allows:

- **A tight loop where every iteration performs the same operation.** One block, one comment.
- **An unrolled run of identical record writes.** Eleven consecutive stores initialising one packed record say the same thing eleven times; a comment per store is noise, and the strided field reads feeding them read as one act.
- **A long verbatim transcription.** Three bodies in `hooks.rs` carry sixty or more unsafe operations in one block — `trace_skinned_submesh` (60), `cm2_scene__trace_line__7089c0` (80) and `cm2_shadow__project_collision_quads__7137c0` (106). Each is a direct transcription of a stock routine, where one-op-per-block would mean a comment every two or three lines and the block-wide comment enumerating the offset families is genuinely more useful. Named rather than numbered by line, because line numbers drift and names do not.
- **An operand inside a short-circuit.** A change-detection chain of the form `unsafe { rd(a) } != unsafe { rd(b) } && …` cannot be split by hoisting: that evaluates reads the original skipped. It *can* be split by wrapping each operand in a block expression that keeps the laziness, but doing so turns a six-line transcribed compare chain into thirty, and the fog-state driver has several. The chain keeps one comment.

**Do not take an exception without reading the item's own argument first.** Where one of these applies, the reason is written above the `#[allow]` — `c_gx_batch__update_fog_state__70baf0` names its cohorts, copy loop and mode dispatch explicitly. A pass that splits those blocks is undoing a decision, not paying down debt.

**What remains is exception, not debt.** 85 findings sit behind 13 attributes — 12 in `hooks.rs`, one in `world.rs`. 51 are multi-operation blocks: 3 transcriptions, 11 tight loops and unrolled record-init runs, and 37 short-circuit operands. 34 are blocks whose enclosing run is one of those exceptions and carries a single block-wide comment.

Every one of the 85 sits behind an attribute whose argument names it. That is worth stating precisely, because "the attribute is argued" and "this lint is argued" are different claims: three attributes here suppressed a lint their prose never mentioned — the fog driver argued why its blocks stay joined and said nothing about why they carry one comment per run rather than per store. An argument that covers some of what it suppresses reads exactly like one that covers all of it.

It came down from 941 by writing the comments and splitting what split cleanly. What is left is the part where a machine-checkable rule and a readable transcription genuinely disagree, and each instance says which.

Splitting is **semantics-preserving and codegen-identical**: `unsafe {}` is a compile-time permission scope with no MIR or codegen representation, so re-bracketing cannot reorder evaluation. Narrow in place rather than hoisting, so a short-circuit stays one:

```rust
!best.hit.is_null()
    && unsafe { anim_u32(best.hit, 0xc) } == unsafe { anim_u32(record, 0xc) }
```

What is *not* mechanical is the comment. Each new block needs a `// SAFETY:` that says something true about that specific operation, so this is a deliberate pass, and it wants the differential harness green afterwards. Recount with `make unsafe-debt`.

Finishing an item means deleting its `#[allow]`, and that is the step that goes wrong quietly. The attribute is per-item and all-or-nothing, so a partly-finished item cannot lose it — and an item finished but left annotated stays exempt forever with nothing behind it. Removing the attribute too early fails `make check` loudly; leaving it is silent. The allow inventory catches either way, which is why the count is in it.

Adding a *new* allow for either lint needs the argument above, not just a reference to the count.

## Doc comments

- Use `///` for items, `//` for fn-body and inline notes.
- Identifiers in doc comments use backticks (`clippy::doc_markdown`).
- Default to writing no comment at all. Add one when the *why* is non-obvious: a hidden constraint, a subtle invariant, a surprise. Don't explain *what* the code does — well-named identifiers already do that.
- In this codebase the *why* is very often a fact about the original: an address, an offset, a calling convention, which register carries what. Those facts are the reason the doc exists. **Never drop one to make a sentence read better.**

### Shape: title, blank line, body

Every doc block of **two or more lines** — `///` or `//!`, on any item, including struct fields, enum variants and test functions — is shaped:

1. Line 1 is a **single-line title**: one sentence, no wrap, within the 100 columns rustfmt gives code. The budget is the *physical* line, indentation included, and the audit measures it in **bytes** — a non-ASCII character such as an em-dash costs three, so a line of 98 characters can still fail.
2. Line 2 is an **empty doc line**.
3. The body follows.

A one-line doc comment is already a title and needs nothing. A block whose content is only a `# Safety` / `# Errors` / `# Panics` section still gets a title line above it — never fold the heading into the title.

The title is what rustdoc puts in the summary column of every index, and what a reader sees before deciding to keep reading. A wrapped opening sentence gives them a fragment. `make audit` enforces the shape.

### Generated source is source

`build.rs` writes Rust as string data, so a `///` inside `out.push_str("…")` is a string literal to every tool that reads the script and a doc comment to every reader of the result.

Which half of the gate sees it depends on the half. **The lints do**: the generated file is `include!`d into the crate, so clippy and rustdoc judge it exactly as they judge hand-written code — that is how a missing pair of backticks in one of these templates surfaced. **The grep-shaped rules did not**, because the generated file is untracked and so absent from `git ls-files`; two doc blocks sat there with no title line for as long as the rule existed.

`make audit` now renders the doc templates back into the shape they take in the output and runs the doc-shape check over that, reporting the **build script's** line numbers. Fix a finding in the template, never in the generated file — the generated file is rewritten on the next build.

```rust
/// `C3Vector::Normalize` — scales a vector to unit length.
///
/// `__thiscall(ecx = this: *mut f32)`, `RET 0`, void. Reads the scale constant
/// `K` from a `.data` global at a fixed absolute address.
```

When reshaping an existing block, build the title from **verbatim substrings** of the first paragraph — cut at a clause break and re-punctuate. Paraphrasing is how a `+0x18` offset or a `RET 0x4` quietly disappears.

## Module style: `foo.rs` + `foo/`, not `foo/mod.rs`

Rust 2018+ module layout. Keeps meaningful filenames in editor tabs.

## No `pub(crate)` — use module hierarchy

Visibility via module hierarchy: private modules already restrict `pub` items to the crate.

## Name the calling convention the client actually uses

A codebase that targets both i386 and x86_64 writes `extern "system"`, which maps to stdcall on one and the x64 ABI on the other. This one does not: the client is 32-bit only, and every `extern` here describes a *specific function in a specific binary*. `extern "stdcall"`, `extern "fastcall"` and `extern "thiscall"` say which convention that function was compiled with, which is a fact worth stating precisely. `extern "system"` would abstract it away and say less.

## Data structure discipline

State on a hot path — every per-frame snapshot, every cache key — is shaped for the move/borrow semantics that keep memcpys out of the inner loop.

### No default `Copy` / `Clone` on aggregate structs

The default for a struct larger than about 16 bytes is **no derive**. `Copy` turns every accidental whole-struct read into a silent memcpy; `Clone` is the opt-in form of the same hazard. Small single-word newtypes and the bitflag types keep `Copy` because they exist to be cheap handles and the trait is structurally needed.

**Derives are never speculative.** A `Clone` or `Copy` derive needs a concrete callsite *today*. Every type deriving either is recorded in `scripts/derive_inventory.txt`, which `make audit` diffs against the tree: adding one shows up in the diff and the reviewer gets to ask for the callsite. Regenerate with `scripts/audit.sh --update-derives` once the callsite exists.

A derive can be *structurally* required with no visible `.clone()`: `vec![elem; n]` fills by cloning, and a `Clone` type containing a field by value requires that field to be `Clone`. Those are real callsites. Let the compiler arbitrate — remove the derive, build, and put back only what fails.

## Dependencies

- All third-party versions live in `[workspace.dependencies]` at the workspace root. Member crates use `{ workspace = true }`.
- Both `Cargo.lock` files are committed for reproducible releases. Bump via `make upgrade` (semver-compatible) or `make upgrade-incompat` (needs `cargo-edit`).
- `rust-version` is pinned to the current stable and tracks it. It documents what the tree is built with; it is not an MSRV promise.

## Diagnostic counters cost nothing when nobody is reading

A counter that only a log line reads is instrumentation, and instrumentation runs inside a game loop. So the whole diagnostic layer (every counter declared through `win/tally.rs`, the tripwires, and the script gauge) is a **compile-time** opt-in: `PERF=1 make` sets `cfg(wow_turbo_perf)` and the layer exists, and a default build does not have it. There is nothing to switch on at runtime. `tally::arm` answers `None` as a constant, so a counting site is not a cached load and a branch, it is nothing, and the work staged above it to feed it (a clock read, a copy into a snapshot, a scan of a result for a non-finite value) is unreachable and goes with it.

Compiling the layer in is the opt-in, which is why a `PERF=1` build is instrumented unconditionally and reports at `info`: the default filter shows it, so a capture from such a build carries the numbers by construction and there is no `RUST_LOG` recipe to get wrong. The event gauge keeps a runtime arm *on top of* the compile-time one, because it charges over a second of wall per minute to instrument script dispatch and a session that wanted the cheap counters must not buy it with them.

The token stays, and it is a token rather than a rule to remember because a counting site is added in the middle of doing something else, and a rule that has to be recalled at that moment is one a type can state instead. `tally::bump` is the form for a site whose whole bookkeeping is that one call; a site with an argument to compute or a clock to read holds the token across that work instead, so the work is skipped too. Holding it in a value is also what keeps the `cfg` in one file: the layer switches off at `arm`, at the three counter reads a report is guarded by, and nowhere else, so the sites themselves are written once and read the same in both builds.

Which atomic operation a counter compiles to is a property of its declared type — the single-writer shape is load-add-store, exact where the game thread is the only writer and free of the `lock` prefix i686 puts on a `fetch_add`; the shared shape pays that prefix. A call site cannot choose, so a counter reached from a worker thread is a decision made once, where the static is declared, and argued there.

**A tripwire is part of the layer, not an exception to it.** The rule used to be the reverse: a permanent regression check that has to fire at the default filter cannot ride an arm nobody would set, so tripwires counted unarmed on their cold branch and said at the site why they were exempt. With the layer compiled out there is no cold branch left to count on, so a default build carries no tripwire at all and `PERF=1` is the build that watches for a drift or a manufactured NaN. What survives of the rule is the shape: a tripwire still says at its site what it is watching, and one that would count on its hot path still wants a cheaper signature.

A hook placed purely to observe costs more than a counting site, a patched prologue and a trampoline on every call the target takes, so it is gated twice. Those entries carry `armed_only = true` in `symbols.toml`: without the layer `build.rs` drops them from the generated table entirely, so the build has no thunk, no install step and no `originals` accessor for them, and with the layer their install step sits behind the gauge's runtime arm, which the logger has resolved before `install_all` runs. An unarmed session leaves the function stock instead of detouring it to reach a delegate. The whole script gauge is installed this way. Such an adapter therefore has no unarmed caller and should not carry an unarmed branch: a delegate arm that cannot be reached is a claim about behaviour the code does not have.

Reporting rides one cadence, the gauge's cumulative tick, and one target. Counters are session-cumulative and are never reset, so a log's last line carries the totals. A family emits its line only when it has something in it: a feature the player never used prints nothing, which is what keeps an armed log readable. Each module reports its own counters, so a label and the counter it names sit together in the file that writes it — a positional snapshot handed to another module to format is how a column silently comes to name the wrong number.

## Imports

No glob imports. Explicit named imports only — never `use foo::*`. One exception: `use super::*` at the top of a `#[cfg(test)] mod tests` block, the standard idiom for pulling the module under test into its own tests.

`hooks.rs` writes paths fully qualified inline, with no top-level `use`, so that appended adapters compose cleanly and an import edit cannot desync them. That is deliberate and local to that file.

## Inline attributes

Default `#[inline]`, not `#[inline(always)]`. LTO inlines small functions on its own; `#[inline(always)]` is reserved for cases where measurement proves it pays, and the audit confines it to the one recorded file. The attributes there are the `cfg`-off breadcrumb shim's no-op stubs, kept so the disabled module mirrors the enabled API exactly.

## `LazyLock` over `OnceLock`

`LazyLock` is the default when the initializer is a static `fn`. Reach for `OnceLock` only when the initializer needs runtime arguments. The audit confines it to the recorded files; whether a given static actually needs the runtime argument is the author's to honour, since no grep can tell.

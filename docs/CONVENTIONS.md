# Conventions

Code-organisation and style rules enforced across the codebase. New code follows these without exception; existing code is brought along on any touch.

## Mechanical audit

Most of this document is enforced by `make check`: `cargo +nightly fmt --check`, then clippy with `nursery` + `pedantic` enabled workspace-wide, then `make audit`, then `make doc`. Lint levels are `warn` in the Cargo manifests so a plain `cargo clippy` (or an editor) reports without blocking a build; the `check` legs pass cargo's `build.warnings = "deny"` config, which fails the run if any warning was emitted — so `make check` is the hard gate, and it shares the build cache with plain invocations because no compiler flags change.

But a lint can only express a lint-shaped rule, and the rules a lint *can't* express are exactly the ones that drift — nothing runs them, so they decay into prose nobody rereads. `make audit` (`scripts/audit.sh`) closes that gap, and `make check` runs it, so a violation fails the same gate as a clippy warning. It covers:

| Check | Section |
| --- | --- |
| Doc-block shape (title / blank / body) and the 100-column doc line cap | §Doc comments |
| The `Clone` / `Copy` derive inventory, diffed against `scripts/derive_inventory.txt` | §No default `Copy` / `Clone` |
| `#[allow(...)]` confined to the recorded exception files | §Warning suppressions |
| `#[inline(always)]` confined to the one measured site | §Inline attributes |
| `static … : OnceLock` confined to the runtime-argument sites | §`LazyLock` over `OnceLock` |
| `pub(crate)` = 0 | §No `pub(crate)` |
| `extern "stdcall"` = 0 | §`extern "system"` everywhere |
| `mod.rs` files = 0 | §Module style |
| Release hygiene | §Release hygiene |

Every finding names the section it came from. The confined-pattern checks compare **sets of files**, not counts, so moving an exception to a new file fails even though the count is unchanged — which is the point: each of those files earned its exception with an argument recorded here, and a new one needs a new argument.

`scripts/audit.sh --file <path>` runs the per-file subset, for an editor hook that wants feedback at edit time.

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

## Release hygiene

The repository is public. Source should read as the engineering itself — not as a diary of how the engineering got written, and not as a document that assumes the reader has access to things they do not. `make audit` enforces the rules below.

Do not write, anywhere in first-party source:

- **The private server's name, or the game's title spelled out in full.** Say "the client" or "the launcher". The `WoW` abbreviation is fine, and so are `WoW 1.12`, `WoW 3.3.5a` and the client's virtual addresses — those are the subject matter. The vendored addon under `addon/` is exempt by scope: its glossary names servers and zones because translating them is what it does.
- **How the client was studied.** No decompiler, no disassembler, no tool names, no workflow narration. The technical claim is what belongs in the comment: "the `preserve` set is machine-checked against the binary" says everything a reader can act on; naming the script that does the checking does not, because they do not have it. (A register-stack *leak* is a real phenomenon this codebase describes, and the rule is written not to fire on it.)
- **A competing mod cited as justification.** State the behaviour in its own terms. The README's compatibility chapter is a different act — users genuinely need to know which mods to remove or load first — and it is out of scope for this rule. Naming a mod for *interoperability* reasons is also fine and deliberately not banned: those notes record real constraints, and deleting them would delete safety-critical information.
- **A citation of a source file the reader cannot open** (`foo.cpp:34`). Keep the fact, drop the citation.
- **A reference to a private note**, in `[[wiki-link]]` or filename form.
- **Incident provenance**: a date, a commit hash, "until commit `abc1234`", "the bug that bit us". State the invariant and why it holds. If a comment only makes sense to someone who knows what changed, restate the change as prose.
- **A tooling signal**, or an absolute `/Users/...` path.

The two `Cargo.lock` files and the verbatim licence texts are out of scope. `THIRD-PARTY-LICENSES.md` reproduces MinHook's BSD notice, which names a disassembler engine and must keep doing so.

## Warning suppressions

Default rule: **never** `#[allow(...)]`, `#[expect(...)]`, or `#[cfg_attr(..., allow(...))]`. The lint is followed. `make check` runs clippy with `clippy::nursery` and `clippy::pedantic` enabled workspace-wide, denies every warning via `build.warnings = "deny"`, and **stays clean by fixing the code, not by silencing the lint**.

### The enumerated exemption

There is one large, deliberate exception, and it is recorded in the workspace manifests rather than scattered through the source.

Every reimplementation module used to open with a blanket `#![allow(clippy::pedantic, clippy::nursery)]` — twenty-one files carrying some variation of it. That switched off two whole lint groups, roughly four hundred lints, so that thirty-four of them would stay quiet, and it meant `cargo clippy` reported a clean tree while the largest file in the repository went unlinted.

The argument behind the blanket was sound (see §The reimplementation contract). The mechanism was not. So `[workspace.lints.clippy]` in `windows/Cargo.toml` and `unix/Cargo.toml` now names each exempt lint individually, grouped by *why* it is exempt and annotated with what it reports today. The counts are the point: they turn an unknown behind a group name into a number somebody can watch, and a lint that is not on the list now fires.

Adding a lint to that list needs the same justification as a per-site allow, plus an argument that it applies across the whole reimplementation surface rather than at one site. Removing one — by fixing its findings — needs nothing but the work.

### Per-site allows

The files permitted to hold a per-site `#[allow]` are recorded in `scripts/audit.sh`, and the audit compares the **set**, so a new file fails. Each allow carries a comment naming why the structural fix does not apply. The recurring legitimate classes are:

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

State the invariant being asserted and why it is true here. Bare "FFI call" or "pointer deref" is not acceptable. Note that rustfmt moving code can separate a comment from its block — if a `let` gets expanded across lines, the comment belongs next to the `unsafe`, not above the `let`.

### One operation per unsafe block

Each dereference, each transmute, each FFI call gets its own block with its own comment. Two reasons: a search for `SAFETY:` returns a focused answer per operation, and refactoring half the block to be safe doesn't strand the comment on the other half.

Two exceptions, both recorded as per-site allows:

- **A tight loop where every iteration performs the same operation.** One block, one comment.
- **A long verbatim transcription.** Three bodies in `hooks.rs` carry between twenty-six and a hundred and six unsafe operations in one block — a direct transcription of a stock routine, where one-op-per-block would mean a comment every two or three lines and the block-wide comment enumerating the offset families is genuinely more useful.

**The rest is debt, and it is measured.** Forty-five `#[allow]` attributes currently suppress this pair of lints, and between them they cover 334 blocks: 314 carry two to four operations and would split cleanly, 11 carry five to twenty-five, and 3 are the transcriptions above. The small ones are worth splitting — the work is lexical and changes no code — but each needs a `// SAFETY:` comment that says something true about the specific operation, so it is a deliberate pass rather than a mechanical one, and it wants the differential harness green afterwards. Recount at any time with:

    cargo clippy -p wow-turbo-dll --target i686-pc-windows-msvc -- \
        --force-warn clippy::multiple_unsafe_ops_per_block

Adding a *new* allow for either lint needs the argument above, not just a reference to the count.

## Doc comments

- Use `///` for items, `//` for fn-body and inline notes.
- Identifiers in doc comments use backticks (`clippy::doc_markdown`).
- Default to writing no comment at all. Add one when the *why* is non-obvious: a hidden constraint, a subtle invariant, a surprise. Don't explain *what* the code does — well-named identifiers already do that.
- In this codebase the *why* is very often a fact about the original: an address, an offset, a calling convention, which register carries what. Those facts are the reason the doc exists. **Never drop one to make a sentence read better.**

### Shape: title, blank line, body

Every doc block of **two or more lines** — `///` or `//!`, on any item, including struct fields, enum variants and test functions — is shaped:

1. Line 1 is a **single-line title**: one sentence, no wrap, within the 100 columns rustfmt gives code. The budget is the *physical* line, indentation included.
2. Line 2 is an **empty doc line**.
3. The body follows.

A one-line doc comment is already a title and needs nothing. A block whose content is only a `# Safety` / `# Errors` / `# Panics` section still gets a title line above it — never fold the heading into the title.

The title is what rustdoc puts in the summary column of every index, and what a reader sees before deciding to keep reading. A wrapped opening sentence gives them a fragment. `make audit` enforces the shape.

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

## Imports

No glob imports. Explicit named imports only — never `use foo::*`. One exception: `use super::*` at the top of a `#[cfg(test)] mod tests` block, the standard idiom for pulling the module under test into its own tests.

`hooks.rs` writes paths fully qualified inline, with no top-level `use`, so that appended adapters compose cleanly and an import edit cannot desync them. That is deliberate and local to that file.

## Inline attributes

Default `#[inline]`, not `#[inline(always)]`. LTO inlines small functions on its own; `#[inline(always)]` is reserved for cases where measurement proves it pays, and the audit confines it to the one site that measured.

## `LazyLock` over `OnceLock`

`LazyLock` is the default when the initializer is a static `fn`. Reach for `OnceLock` only when the initializer needs runtime arguments — the audit confines it to those sites.

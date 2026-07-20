# Architecture

Two mods that share nothing at runtime but share a build system and a hook substrate. `wow_turbo.dll` is an incremental replace-in-place host for the 1.12 client: 335 manifest entries and as many Rust adapters, everything else runs stock. `wow_translate.dll` is a `UnitXP` extension that reaches macOS translation through Wine's unix-call boundary.

```
Wow.exe ──▶ wow_turbo.dll        MinHook detours over Wow.exe's .text
            (i686 PE, native)    and over fmod.dll (packed, runtime base)

Wow.exe ──▶ wow_translate.dll ──▶ wow_mods.dll ──▶ wow_mods.so ──▶ Translation
            (i686 PE, native)     (i686 PE,        (x86_64          .framework
                                   Wine builtin)    Mach-O)         (Swift)

            version.dll          optional loader: reads dlls.txt,
            (i686 PE, builtin)   LoadLibraryW's each line
```

For what the mods do, how to install them and which other mods they coexist with, see the [README](../README.md). This document is about how the pieces fit.

## The two workspaces

`windows/` and `unix/` are separate Cargo workspaces because they are separate linkage worlds, and a single workspace cannot hold two `[build] target` defaults or two sets of linker flags. `windows/.cargo/config.toml` pins `i686-pc-windows-msvc` with `lld-link` and the xwin include/lib paths. `unix/` deliberately declares **no** default target, so a bare `cargo test` or `cargo clippy` there runs native aarch64 with no Rosetta; the shipped `.so` is built by passing `--target x86_64-apple-darwin` explicitly from the Makefile.

Wine's unix-call boundary is what forces x86_64 Mach-O on the unix side even though the game is 32-bit.

| Crate | Workspace | Produces |
| --- | --- | --- |
| `wow-turbo-dll` | windows | `wow_turbo.dll` — the reimplementation host |
| `wow-translate-dll` | windows | `wow_translate.dll` — the `UnitXP` extension |
| `wow-mods-bridge` | windows | `wow_mods.dll` — the PE half of the bridge (Wine builtin) |
| `version-dll` | windows | `version.dll` — the optional loader |
| `wow-hook` | windows | the shared hook substrate (rlib) |
| `wow-mods-unix` | unix | `wow_mods.so` — the unix half of the bridge |
| `wow-translate-sys` | unix | the Swift static library and its bindings |
| `wow-shared` | unix | shared types, breadcrumbs, `ftol`, trig |

One crate crosses: `wow-shared` is a member of the *unix* workspace but is consumed by the *windows* workspace by relative path. It is the only crate compiled into all three linkage units — which is exactly what makes the `Thunks` enum ordering *be* the wire ABI rather than merely describe it, and exactly why changing it means rebuilding both halves. See [The wire contract](#the-wire-contract-and-the-deployment-trap).

## wow_turbo: the manifest is the architecture

`windows/turbo/symbols.toml` is the hand-edited hook manifest. `windows/turbo/build.rs` turns it into `$OUT_DIR/symbols.rs`; `windows/turbo/src/win/symbols.rs` is a short `include!` shim carrying the generated-code lint allowances.

Every entry is fully specified — `rva`, `sig`, `abi`, `ret`, `args`, `preserve` — with no half-filled entries permitted. Per entry, codegen emits the `{Name}Fn` original type, the trampoline `OnceLock`, the `extern "abi"` thunk MinHook installs, a typed `originals::<snake>()` accessor, and the install wiring.

The thunk dispatches at **compile time** to `super::hooks::<snake>`. The adapter's name is the linkage: there is no registration table and no per-call resolver, so a full replacement pays nothing for the trampoline it never calls. Names mirror the host's C++ symbols verbatim, with `__` standing in for `::` and a virtual-address suffix disambiguating overloads — `C3Vector__Scale__5f8cf0`.

### `preserve`, and the two x87 ABIs

`preserve` is orthogonal to `abi` and is the field a newcomer will not expect.

The 1.12 client's compiler does ABI-breaking interprocedural register allocation: having observed that an internal leaf never touches a volatile register, it keeps values live in EAX/ECX/EDX across the call. A Rust reimplementation uses those as scratch, and the caller reads back garbage. So for each register the original leaves intact, codegen wraps the thunk in an empty-`asm!` capture/restore pair, with the capture placed as the thunk's *first* statement — the only code the compiler emits before it is the prologue, which never writes a volatile. Under-saving corrupts the caller; over-saving is merely wasted. The sets are machine-checked against the binary.

`abi = "x87st0"` and `abi = "x87pow"` cover two CRT helpers whose arguments arrive on the x87 register stack — a contract no Rust `extern` string can express. Both get `#[unsafe(naked)]` shims with build-time asserts. The `_CIpow` case is the instructive one: its thunk left a value on the x87 stack because the shim popped one operand instead of two, and the leak compounded until the eight-slot stack jammed and every later load produced the indefinite `QNaN` — visible in-game as lighting that degraded over minutes.

### Verify before patch

The generated `install_thunk` refuses to patch unless `wow_hook::signature_matches(va, sig)` passes, logging the refusal and leaving that one function stock.

`sig` is a short entry-anchored **wildcard pattern**, not a hash of the function body, and the reason is structural: an x86 body is full of bytes that legitimately differ between build variants even when the logic is identical — rel32 call and jump displacements, absolute-address immediates. A hash is all-or-nothing and would refuse everything; a wildcard pattern pinned to the prologue's stable opcodes refuses only what actually differs.

That single mechanism is the entire compatibility story with other mods, with no per-mod special-casing: a prologue somebody else already detoured fails the check, and `wow_turbo` yields that function and keeps the rest.

`EXPECTED_IMAGE_BASE` is asserted at attach **before** any install, because reimplementations read host globals by absolute address with no per-call base lookup.

## The hook lifecycle

`wow-hook` is the shared substrate and is `#![cfg(target_arch = "x86")]` in its entirety. The path at load:

`DllMain(PROCESS_ATTACH)` → `on_dll_attach` (disable thread-library calls, init the logger) → `crumb::init` → image-base assert → `init_engine_clock(tsc_hz())` → `symbols::install_all` → `fmod::install_init_hook`.

Two rules with teeth:

- **Nothing spawns a thread from `DllMain`.** `CreateThread` under the loader lock deadlocks. The ~50 ms TSC calibration therefore blocks the loader thread on purpose rather than moving to a worker, and `wow_translate`'s queue worker is lazily spawned on first enqueue instead.
- **Create and enable are split.** Wherever a detour reads state that must exist before it can fire — a trampoline pointer, a cached original — the hook is created, the state published, and only then is the hook enabled. The combined `install` helper is for the cases without that ordering constraint.

Enables are **batched**. Every MinHook enable freezes all threads in the process to patch safely, so `install_all` does create-plus-queue per function and exactly one `apply_queued` at the end: one freeze for hundreds of hooks instead of hundreds of freezes. If that apply fails, every queued hook stays disabled and the client runs fully stock, logged at `error`.

### fmod: the one target outside `Wow.exe`

`fmod.dll` unpacks its code to a runtime base, so the manifest path — fixed RVA over `Wow.exe`'s `.text` — cannot reach it. Instead `windows/turbo/src/win/fmod.rs` hooks fmod's own `FSOUND_Init` export at attach. The detour runs the original first (it allocates the window coefficient table the mixer reads), then installs the mixer hook once, on that cold init path. Nothing polls. The per-launch base is anchored off a known export's RVA, and the mixer signature wildcards the relocated `MOV EBX,[disp32]` operand.

## Portable kernels versus host adapters

`windows/turbo/src/lib.rs` is the one place the target split is declared, and the crate splits **by portability, not by feature**:

- `math` and `storm` are `cfg(any(target_arch = "x86", test))` — pure kernels with no FFI and no host-image access.
- `win` is `cfg(target_arch = "x86")` — the generated symbol table, the FFI adapters, `DllMain`.

The consequence is worth stating loudly: `make test` builds and runs those kernels as **x86_64-apple-darwin**, deliberately not native aarch64, so the real SSE path executes under the same Rosetta translation the shipped 32-bit DLL runs under rather than a native stand-in that would hide translation-specific behaviour.

Numeric convention: kernels compute in `f32`, matching the SSE lane width, unless a function has to track an 80-bit x87 original — then it computes in `f64` and narrows through `f64_to_f32`. Every float-to-int conversion routes through `wow_shared::ftol`, because a compiler-lowered `f64 as i64` on i686 re-emits its own control-word dance and rounds differently from what the client did.

`windows/turbo/src/win/hooks.rs` is a single very large module holding all 334 adapters. Paths are written fully qualified inline with no top-level `use` block, so appended adapters compose cleanly and an import edit cannot desync them.

## The differential harness

Two switches. Compile-time `WOW_TURBO_DIFF=1` (`DIFF=1 make install`) sets `cfg(wow_turbo_diff)`; runtime `WOW_TURBO_DIFF_ARM=all|Name1,Name2` arms individual hooks at launch. Only 25 of the 335 entries carry a `diff` table, and every other entry emits a byte-identical thunk either way, so the shipped DLL pays nothing for the harness existing.

The ordering is the safety property: the compare path runs the **original** on the live arguments, so the game always proceeds on ground truth, then re-runs the reimplementation against snapshots of those same inputs and compares only the annotated region. Comparisons cover `ins`, `out`, `float` and `ulp` tolerances; an `expected` annotation demotes a known divergence from warn to debug, for the reimplementations that are deliberately more precise than the x87 original.

`double_run_safe` is a per-entry human assertion — documented like `unsafe` — needed because the compare path runs the reimplementation a second time, which is only sound when it has no side effects. It is consumed by an audit script, not by codegen.

Divergence reporting in `windows/turbo/src/win/diff.rs` is rate-limited per hook (full detail for the first few events, then a running count), and the detail string is built lazily so the matching fast path formats nothing.

## wow_translate and the bridge

`windows/translate/src/hook.rs` detours `UnitXP` and dispatches three subcommands; everything else falls through to the original.

The refusal rule here is **inverted** relative to `wow_turbo`, and the contrast is the point. `UnitXP` is the channel 1.12 mods borrow to expose native calls to Lua, so several bind it. A prologue that is already detoured (`E9` / `FF 25`) is therefore treated as resolving fine and is **chained**, never refused — chaining is sound in both directions. Only an unrecognised prologue, meaning the wrong client build, refuses, and it degrades cleanly to `dllAvailable = false` on the Lua side.

Four virtual addresses are verified though only one is patched: the three `lua_*` entries are only ever called, but they run on every pass-through, so a wrong build has to be caught before the first call rather than at it.

The queue is bounded, non-blocking, and drained by a lazily-spawned worker that coalesces requests **by language pair**, because Apple's translation session API is per-pair.

### The wire contract, and the deployment trap

`unix/shared/src/lib.rs` and `params.rs` define the boundary. The `Thunks` `#[repr(u32)]` ordering *is* the ABI. `TranslateParams` is pinned by a `const _` assert on its size, alignment, and the offset of a `u64` following a `u32` — the failure that prevents is specific: a 32-bit target aligning `u64` to 4 would shift every out-param, and the unix handler would write past the PE caller's stack-allocated struct and over its return address.

Inputs are packed, outputs land in fixed slots, all buffers are PE-allocated, and only fixed-width integers cross.

**The trap:** `wow-shared` is compiled into `wow_translate.dll` and `wow_mods.so` as two separate linkage units. Any change to `Thunks` or to a params struct is an ABI change on both sides, so both halves must be rebuilt and deployed **together**. `make install` copies all three artifacts every time, which is what makes the arrangement safe in practice — a partial deploy is the failure mode to watch for.

Every unix-call dispatch is wrapped in an autorelease pool by the `arp!` macro, because Wine's dispatcher sets none up and autoreleased framework objects would otherwise accumulate for the process lifetime.

`unix/translate-sys/build.rs` shells out to `swiftc` to build a static library, because the translation framework's only public surface is SwiftUI-bound and has no C entry point.

## The Wine builtin problem

This is why `wow_mods` exists in the shape it does, in three beats:

1. **A native game-side mod loaded from `mods/` by absolute path cannot pair a companion `.so`.** So `wow_translate.dll` stays native and raw-dylib imports `wow_mods_unix_call`, which forces Wine to load and initialise the builtin — and pair its `.so` — before `wow_translate`'s own `DllMain` runs.
2. **Wine resolves builtins by name through the prefix, not through `lib/wine`.** So `make windows` runs `winebuild --fake-module` to produce `wow_mods.fake.dll`, a placeholder that the installer copies into the prefix's `syswow64` as `wow_mods.dll`. Without it the real builtin is never found. This is the single most likely thing to break a hand install.
3. **`windows/bridge/build.rs` extracts only `unix_lib.o` out of Wine's `libwinecrt0.a`** rather than linking the archive, because the rest collides with the MSVC CRT's TLS symbols. It also has to place Wine's `libntdll.a` ahead of xwin's `ntdll.lib`.

The bridge exports both `__wine_unix_call_funcs` and `__wine_unix_call_wow64_funcs` pointing at the same table, because a 32-bit PE bridge pairs an x86_64 `.so` over Wine's wow64 path.

## Crash breadcrumbs

`unix/shared/src/crumb.rs` is a 1024-entry ring in an mmap **shared by both sides** — the same file is `/tmp/wow-crumb.bin` to the unix half and `Z:\tmp\wow-crumb.bin` to the PE half — so PE and unix events interleave in one sequence in a single dump. A 64-byte header, 48-byte entries, and a lock-free record of one fetch-add plus a volatile write.

`dump_recent` is async-signal-safe by construction: raw `write`/`WriteFile` only, no allocator, no locks, a fixed stack formatting buffer, and it tolerates a torn most-recent slot by skipping entries whose sequence disagrees with their position.

The whole module is gated on `cfg(wow_crumb)`, set from `WOW_CRUMB=1` routed through each consumer's `build.rs` rather than through `RUSTFLAGS` — cargo prefers an env `RUSTFLAGS` over config-file flags, so routing it through the build script is what keeps it composable. With the gate off the module is const no-op stubs and every probe compiles away.

Generated thunks record the hook RVA, but skip recording when the *same* hook fires consecutively, so a hook called thousands of times per frame collapses to one entry and cannot evict the distinct pre-crash sequence.

## Build profiles and the shipped artifacts

`release` is the everyday `make` build and deliberately keeps `debug-assertions` and `overflow-checks` on, with `debug = 1` for post-mortem symbolication — inline frame chains matter because the reimplementations lean on `#[inline(always)]`. `production` (`PROD=1`, forced for `make bundle`) turns both checks off and switches to fat LTO with one codegen unit.

`make bundle` produces exactly four zips, always at the production profile, and their internal layout mirrors the install destinations: `game/` merges next to `WoW.exe`, `wine/` merges into the Wine distribution.

The two `wow_turbo` builds differ **only** in ISA baseline and are built into separate target directories so they never clobber each other: nehalem for the Wine-on-macOS stack, because Rosetta's vector unit is 128-bit and AVX2 would be emulated, and haswell for native Windows. `windows/.cargo/avx.toml` documents the merge footgun — rustflags arrays from merged config sources are joined with the overlay's entries last, which is what makes the baseline override work.

Install destinations come from the environment, never from hardcoded paths. `WOW_EXE` is required and locates the game; native mods deploy into the `mods` directory beside it. `WINE_SDK`, plus an optional `WINE_INSTALL_DIR`, names the Wine trees the builtins install into.

`version.dll` is a proxy, not a reimplementation: its real APIs are forwarded to `kernelbase.dll` at link time through a `.def` file, and only the two `VerInstallFile` entries exist in the image, as logging stubs.

Each cdylib carries its own copy of the logging statics, so each initialises the logger from its own entry point — `DllMain` on the PE side, a one-shot thunk on the unix side.

## What is committed

Only addresses, signatures, names and prototypes. Never the client's own code or data, in any form. The manifest records where a function lives and what its calling convention is; the Rust beside it is written to match that behaviour, not transcribed from the original's bytes.

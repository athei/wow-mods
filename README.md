# wow-mods

This repository houses two independent mods for the WoW 1.12 client
(build 5875), written in Rust:

- **[wow_turbo](#wow_turbo)** — a performance and bug-fix mod. A single
  self-contained DLL that works anywhere the 1.12 client runs.
- **[WoWTranslate](#wowtranslate)** — in-game chat translation through
  Apple's on-device Translation framework. A DLL plus a Lua addon; requires
  the Wine-on-macOS stack.

The two share nothing at runtime — install either one, or both.

## Installing

Grab the archives you need from the [Releases](../../releases) page, or build
them yourself with `make bundle` (see [Building](#building)):

- **`wow_turbo-<version>-mac.zip`** / **`wow_turbo-<version>-windows-avx.zip`**
  — the wow_turbo DLL, one build per ISA baseline: the mac build targets
  Rosetta's 128-bit vector unit, the Windows build raises the baseline to
  AVX2 (Intel 2013+ or any AMD Ryzen) since real hardware runs 256-bit
  vectors at full width.
- **`wow_translate-<version>.zip`** — WoWTranslate (Wine-on-macOS only).
- **`version_loader-<version>.zip`** — an optional standalone loader for
  setups without one. The mods don't care how they get loaded — any mod
  loader works, and most modded clients already ship one.
- **`wow_mods-debug-<version>.zip`** — debug symbols for everything above, for
  reading a crash report. Nothing to install. Each DLL logs its version and its
  linker-assigned image ID as it loads, and the archive's `BUILD` file names the
  release it belongs to, so a captured log identifies the archive that
  symbolicates it.

Each unpacks into directories that mirror where the files go: merge `game/`
into the game folder (next to `WoW.exe`) and `wine/` into the Wine build the
game runs under.

```
wow_turbo-<version>-mac/            wow_turbo-<version>-windows-avx/
└── game/mods/wow_turbo.dll

wow_translate-<version>/
├── game/
│   ├── mods/wow_translate.dll
│   └── Interface/AddOns/WoWTranslate/
├── wine/
│   └── lib/wine/
│       ├── i386-windows/     wow_mods.dll
│       ├── x86_64-unix/      wow_mods.so
│       └── aarch64-unix/     wow_mods.so, for an arm64 Wine
└── prefix-markers/
    └── syswow64/wow_mods.dll  only for a prefix that predates the install

version_loader-<version>/
├── game/dlls.txt
└── wine/lib/wine/i386-windows/version.dll
```

1. Merge `game/` into the game folder.
2. For WoWTranslate, merge `wine/` into the Wine distribution the game runs
   under. Wine resolves builtins by name through the prefix, not `lib/wine`,
   so the real bridge is only reachable once a placeholder named
   `wow_mods.dll` sits in the prefix's `drive_c/windows/syswow64/`. `wineboot`
   writes one for every builtin it finds in `lib/wine` when it creates a
   prefix, so if the prefix comes after this merge there is nothing to do. If
   it came first, it never saw `wow_mods`: copy
   `prefix-markers/syswow64/wow_mods.dll` in yourself, or run `wineboot -u`.
   (The Makefile deliberately never writes into a Wine prefix, since the
   prefix belongs to whatever launches the game.) The bridge's unix half ships
   for both Wine host arches; Wine loads only the one matching its own build,
   so the other copy sits there inert and there is nothing to choose.
3. Load the mods with whatever mod loader you already use — **after your
   other mods**, so `wow_turbo` yields on anything they already hooked (see
   [Playing with other mods](#playing-with-other-mods)). If you don't have
   one, the `version_loader` archive provides a `version.dll` that injects
   every mod listed in `game/dlls.txt`; it ships with both mods listed, so
   drop the lines you don't want. It only works on clients that actually load
   a `version.dll` — 3.3.5a does, vanilla 1.12 does not, so on 1.12 you need
   a proper mod loader. `wow_mods.dll` is never listed in `dlls.txt` — it is
   auto-loaded as an import dependency of `wow_translate.dll`.

## wow_turbo

`wow_turbo.dll` replaces roughly 330 of the client's hottest functions with
modern reimplementations — fixing long-standing rendering bugs (the
flickering, jittering names in crowds), speeding up the addon Lua runtime, and
removing the x87 floating-point bottleneck that makes vanilla WoW crawl under
Rosetta 2 on Apple Silicon.

Nothing on disk is modified. The mod injects at launch and patches functions
in memory, and every patch is verified byte-for-byte first: if a function
doesn't look exactly as expected — because another mod already hooked it, or
the client build differs — that one function is left alone and runs stock.
That makes `wow_turbo` safe to drop into an existing mod loadout (see
[Playing with other mods](#playing-with-other-mods)).

**Highlights**

- Fixes the crowd text bugs: names that jitter sideways and glyphs that
  briefly vanish when many players are on screen.
- Ports the entire text-layout stack off x87 — in a packed capital city it
  alone was ~11.7% of the client's main-thread profile.
- Faster addon/Lua runtime: literal `string.gsub`/`string.gfind` fast paths,
  a fixed string-intern hash, and per-collection garbage-collector timing
  visibility.
- Moves MP3 music decoding — the single hottest x87 routine in a busy
  session — to SSE.
- Faster loading screens (one persistent modern zlib inflater instead of a
  fresh 2006-era one per MPQ sector, and memoized archive file-name lookups
  instead of re-hashing and re-probing every archive on every file open).
- A drop-in replacement for `libSiliconPatch.dll`, with measurably better
  code generation, that also covers everything `weirdperformance.dll` does.

### Why the 1.12 client is slow

WoW 1.12 is a 32-bit binary from the MSVC6 era: virtually all of its
floating-point math is x87, and much of it re-runs every frame. On a modern
x86 CPU x87 is merely slower than SSE. Under Rosetta 2 on Apple Silicon it is
dramatic: the JIT emits **20–21 ARM instructions for an isolated x87
operation** (long well-formed runs get down to ~4), and the client's hottest
per-frame code is exactly that kind of scattered x87.

`wow_turbo` reimplements each hot function faithfully in Rust (which compiles
to plain SSE2) and detours the original to the new code — leaving the original
in place as a fallback. **323 functions** were ported until the addressable
pool was exhausted, and the shipped DLL contains **zero x87 compute
instructions in every ported body** (stock bodies carried up to 81 each). The
work was guided by [x87sidecar](https://github.com/athei/x87sidecar), which
counts x87 blocks as Rosetta actually executes them — far better at tracking
down hot x87 code than conventional time-based sampling. What x87 remains
today is the audio thread and irreducible ABI glue, not anything that costs
frames.

### What gets patched

About 330 functions, grouped roughly like this:

| Area | ~hooks | Examples |
|---|---|---|
| Vector / matrix / quaternion math | 82 | `C3Vector`, `C44Matrix`, quaternion slerp, the CRT's `__ftol` |
| Collision, world BSP, terrain | 85 | line-of-sight traces, frustum culling, terrain height |
| Movement and orientation | 27 | ground-movement stepping, facing interpolation |
| Render device and batching | 22 | projection setup, batch sort keys, viewport |
| Particle systems | 21 | emitter update, spawn, quad building |
| Model animation (M2) | 17 | bone animation, ray-vs-model picks, track sampling |
| Font, glyph and text layout | 15 | kerning, text measurement, quad emission, glyph cache |
| Lua runtime and CRT numerics | 15 | table hashing, string interning, GC, `string.gsub` |
| Lighting, weather, sky | 13 | day/night light blending, sky dome, rain |
| UI, minimap, splines, misc | 30 | minimap blips, rect clamping, cubic splines |
| One-offs | 3 | MPQ decompression, the engine clock, the FMOD MP3 mixer |

Each hook is a faithful port. Every patch site is verified byte-for-byte
before it is applied, and the project ships a differential test mode that runs
reimplementation and original side by side on live game data and reports any
divergence — for the entries annotated with a `[diff]` table in
`windows/turbo/symbols.toml` (a minority, chosen for the arithmetic-heavy
functions where a subtle rounding difference would be hardest to spot by eye).

### Text rendering in crowds

The stock client has a real bug here, not just a slowdown. Each font gets a
glyph cache of at most eight 256×256 texture pages. A busy scene with many
distinct characters on screen (long names, several fonts, non-Latin glyphs)
overflows it, and then two things go wrong:

- Every insert into a full page evicts a least-recently-used glyph, which
  invalidates **every string on that page** — a rebuild storm that re-lays-out
  and re-uploads text over and over, each frame.
- The evict path only retries a single freed row, so glyph lookups
  intermittently fail. A failed glyph is silently dropped from the text
  *measurement* but usually still drawn — so centered text shifts sideways by
  half the missing advance. That is the nameplate jitter you see in crowds,
  and occasionally a glyph vanishes outright.

`wow_turbo` enlarges the cache pages to 256×1024 (4× the capacity) so a
crowd's working set actually fits: no evictions, no rebuild storm, no jitter.
A built-in probe watches for the failure signature and warns if it ever
returns.

The performance side: the whole text-layout stack — kerned advance, glyph
advance, run measurement, width fitting, quad emission — was x87 code running
once per visible character per string per frame. In a packed city profile it
was **~11.7% of the client's main-thread cost**, with the kerning lookup alone
called 5.9 million times in one capture window and ~80 x87 ops per glyph in
the quad emitter. All of it now runs as SSE with zero x87 operations.

### Lua / addon runtime

The 1.12 client embeds Lua 5.0, and heavy addon setups hammer it. Four
targeted fixes:

- **`string.gsub` / `string.gfind` literal fast path.** Lua 5.0 always runs
  its recursive backtracking pattern matcher, even when the pattern is a plain
  literal — the O(n·m) cost every chat-line and combat-log parser pays on
  every line. Literal patterns are detected and routed to a single-pass
  substring search; anything with magic characters still takes the original
  matcher, so behavior is identical.
- **String interning.** The stock hash only samples ~32 bytes of a string, so
  structurally similar strings (and addons generate a lot of those) collide in
  the intern table and degrade into long linked-list walks. Replaced with a
  full FNV-1a hash plus a stored-hash pre-check before any byte comparison.
- **Garbage-collector observability.** Lua 5.0's collector is stop-the-world,
  so every collection is a potential frame hitch. Each collection now logs its
  mark/sweep phase timings and heap size (`RUST_LOG=wow::gc=debug`), and
  collections long enough to drop frames emit a warning. The client's own
  pacing policy is left untouched: an earlier release widened the growth
  threshold to collect less often, which let enough garbage accumulate on
  addon-heavy raid setups to cause multi-second freezes, and was reverted.
- **Script-cost observability.** A build made with `PERF=1 make` reports, every
  second and under `RUST_LOG=wow::script=debug`, what the interface actually
  spent: the cost of each addon's scripts, of the client's own interface, and
  of the dispatch around them, plus how that cost is distributed between many
  cheap handler calls and a few expensive ones. Script time is billed to the
  addon folder that owns the file, so a ranked table answers which addon to
  look at rather than which frame happened to run. Loading screens and
  interface reloads are marked, so one-time start-up work is not mistaken for
  what a session costs while playing. The instrumentation measures its own
  overhead and reports it separately. The mod's own counters (the
  parallel-fork seams, the memo hit rates, the rest) ride the same `PERF=1`
  build and report on `wow::perf` at `info`, so they need no filter of their
  own and do not switch this gauge on with them. A release build carries
  neither: the whole layer is compiled out, which is why it costs nothing.
- **Hot VM leaves.** Table hashing and lookup, number↔string conversion and
  the constant-table path are ported to SSE.

Only functions where a port measurably pays are hooked — the bytecode
interpreter itself was measured and deliberately left alone.

### And the rest

- **Music decoding.** The FMOD MP3 synthesis filterbank was the single
  hottest x87 routine in a busy session. It is reimplemented in SSE, accurate
  to within one 16-bit-sample LSB. It runs on the audio thread, so this buys
  cooler, quieter CPUs rather than frames.
- **Loading screens.** The MPQ reader created and destroyed a fresh zlib
  stream *per sector* using a 2006-era zlib. Zlib sectors now go through one
  persistent modern inflater; everything else falls back to the stock decoder
  byte-for-byte.
- **Engine clock.** `OsGetTimeMs` is read ad hoc from ~331 call sites. The
  stock path folds the tick source through calibration doubles maintained by a
  background thread; the replacement is a pure-integer fixed-point `rdtsc`
  fold — no floats, no cross-thread calibration.

### Playing with other mods

**Install `wow_turbo` last.** Every hook verifies the target function's bytes
against a recorded signature before patching. A prologue already detoured by
another mod fails that check, and `wow_turbo` logs
`signature mismatch — refusing to patch` and leaves the function to its owner.
Loading it after SuperWoW, nampower and friends means it
automatically yields on every function they claimed and accelerates the rest.
These functions are not special-cased. SuperWoW's selection-circle hook and
the minimap-icons blip callback are two functions `wow_turbo` also knows how to
accelerate; loaded after those mods it sees their detour on the prologue, fails
the signature check, and yields — the same mechanism as every other function,
no exception.

One function is an exception, and it is an exception by having *more* verified
shapes rather than fewer. `GetName` is called tens of thousands of times a
second, and SuperWoW replaces it outright to add a second form that returns a
unit's GUID. `wow_turbo` reproduces both that form and the stock one, so it
accepts either prologue at that address and answers the calls itself, caching
results a repeat call would otherwise recompute. It still checks which module
it is standing in for: an unrecognized handler there is left to run unchanged,
and the log line names it.

- **`libSiliconPatch.dll` — replaced; remove it.** Its full hook set was the
  explicit parity target and is covered. Where the two are directly
  comparable, `wow_turbo`'s code generation matches or beats it: packed SSE on
  72/72 vectorizable kernels versus its 62/72, and its trigonometry hooks pay
  two `libm` calls per matrix where `wow_turbo` pays none — before counting
  everything it covers that `libSiliconPatch` doesn't touch (fonts, Lua,
  audio, loading, the clock).
- **`weirdperformance.dll` — replaced; remove it.** Its collision, animation
  and particle hooks and its MPQ-decompression hook are covered (the
  decompressor more thoroughly — persistent inflater state instead of a
  per-call one), and its one file-streaming optimization that actually fires
  on this client — a memo cache for archive file-name resolution — is ported,
  with the locking its version lacks. Its remaining stream hooks are no-ops
  here: pass-throughs, or fast paths for a stream shape the 1.12 client never
  creates. The only piece
  deliberately not adopted is its small-block-allocator swap, whose global
  free lists are unsynchronized.
- **UnitXP_SP3 — replaced; remove it.** `wow_turbo` serves its `UnitXP(...)`
  command set natively: line of sight (`inSight`, including the camera form),
  the five `distanceBetween` meters, `behind`, the full targeting suite, the
  nameplate filters (`modernNameplateDistance` and friends), the camera
  offsets and follow mode, weather suppression, FPS caps, script timers,
  notifications, screenshot re-encoding and the version/probe commands. Its
  companion addon (`UnitXP_SP3_Addon`) keeps working unmodified, panel and
  keybindings included. Line-of-sight answers come from a position-aware
  cache that traces the world several times less often than the original,
  which was a measurable slice of frame time in crowded scenes. The floating
  combat text overlay is served too: `combatTextSP3 enable` draws the floating
  numbers, the crit carousel and script-added lines. Its renderer is not a
  port of the original's, which rasterizes through GDI system fonts and
  `d3dx9` objects that Wine substitutes per machine; here each line is
  rasterized once from a font file into a managed-pool texture, so a device
  reset costs nothing and the face is whatever the addon's font box names,
  from the client's own `Fonts/` directory or the system's (stock `FRIZQT__`
  by default, and those edit boxes only apply on Enter). What is deliberately
  dropped: the remote Lua debugger, the TCP tweaks and the math detours, the
  math because `wow_turbo` already owns those functions natively.
  `UnitXP("version", "additionalInformation")` names
  `wow_turbo`, not the original, so a script probing for the exact original
  can tell them apart.
- **transmogfix — replaced; remove it.** Its two jobs are covered. Equipment
  appearance changes arrive from the server as a blank-then-restore pair on
  the same field, and applying each one eagerly rebuilds a model that never
  visibly changed; `wow_turbo` swallows the pair when the restore lands
  inside the window and applies the blank for real only when it does not, at
  most sixteen units per frame with one refresh each. And the client's file
  resolver only checks the file system on some lookup paths, which leaves
  loose custom art unreachable on the others: the two branches that gate the
  check are opened at load, verified before they are written and skipped if
  another mod already opened them. It also had to blank the client's
  integrity scan to keep its own patches working, which `wow_turbo` now does
  itself, so removing it changes nothing there either.
- **SuperWoW, nampower — compatible.** Load them first and `wow_turbo` stays
  out of their way. Where one of them wraps an entry `wow_turbo` also wraps
  and both only ever add behaviour around the original — the scene-end entry
  is the current example — the two stack instead: `wow_turbo` accepts the
  installed detour and chains underneath it.

If something ever misbehaves, `WOW_TURBO_SKIP=all` disables every hook and
`WOW_TURBO_SKIP=Name1,Name2` disables specific ones — no rebuild, no
reinstall.

## WoWTranslate

The second, unrelated mod in this repository: chat translation inside the
game, running entirely on your Mac through Apple's Translation framework — no
cloud service, no API keys. On macOS 26 and later the framework translates
with Apple Intelligence when it is enabled, which noticeably improves
translation quality. It comes in two halves that install together:

- **`addon/WoWTranslate/`** — a normal Lua addon providing the in-game UI and
  driving translation through the `UnitXP("WoWTranslate", …)` API it gets
  from the DLL.
- **`wow_translate.dll`** — hooks the client's `UnitXP` entry point and
  ships translation requests across the Wine boundary to a native macOS
  translation session.

A native mod can't reach the macOS side of Wine by itself, so
`wow_translate.dll` imports the **`wow_mods.dll`/`wow_mods.so`** bridge
builtin, which Wine then loads and pairs automatically. **The bridge exists
solely for WoWTranslate** — `wow_turbo` neither needs nor uses it, so if you
skip WoWTranslate you can ignore the bridge entirely. This is also why
WoWTranslate is Wine-on-macOS only while `wow_turbo` runs anywhere.

## What's in the repository

| Artifact | Belongs to | Role |
|---|---|---|
| `wow_turbo.dll` | wow_turbo | the reimplementation host; fully self-contained |
| `wow_translate.dll` | WoWTranslate | the `UnitXP` hook and translation client |
| `addon/WoWTranslate/` | WoWTranslate | the Lua addon UI |
| `wow_mods.dll` + `wow_mods.so` | WoWTranslate | the PE↔unix bridge; auto-loaded as an import of `wow_translate.dll`, needed by nothing else |
| `version.dll` | its own archive | optional standalone injector: loads every mod listed in a `dlls.txt` next to `WoW.exe`, for setups without another mod loader; only works on clients that load a `version.dll` (3.3.5a yes, vanilla 1.12 no) |

## Building

Requires the Wine-on-macOS cross toolchain (`WINE_SDK`, `xwin`/`lld-link`,
`winebuild`, `swiftc`), `cargo-nextest` (the runner both `make test` and
`make check` invoke), and a rustup toolchain: stable 1.97 or newer, per
`rust-version` in the Cargo manifests, plus nightly for `make fmt`
(`rustfmt.toml` uses nightly-only options). Two workspaces:

```
windows/     PE cross-compile workspace (i686): turbo, translate, bridge, version, hook
unix/        x86_64 Mach-O workspace: bridge (wow_mods.so), translate-sys (swiftc), shared
addon/       the WoWTranslate Lua addon
```

`make` builds, `make bundle` stages the four release archives plus their debug
symbols under `dist/` (always at the production profile — fat LTO — and
including the AVX2 native-Windows build of `wow_turbo`), and `make install`
deploys straight into a local setup, symbols included: a `.pdb` beside every PE
and a `.dSYM` beside the `.so` (on Mach-O the DWARF otherwise stays behind in
the compiler's object files, so `make` runs `dsymutil` to gather it). Destinations come from the environment, not the Makefile:
`WOW_EXE` is the path to the client's `WoW.exe` (native mods deploy next to
it) and `WINE_SDK` — plus an optional `WINE_INSTALL_DIR` — names the Wine
trees the builtins install into.

How the pieces fit — the hook manifest and its codegen, `preserve` and the x87
ABIs, the hook lifecycle, the PE↔unix bridge — is in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). The development conventions — the
reimplementation contract, doc-comment shape, unsafe discipline, warning
suppressions, release hygiene — live in
[`docs/CONVENTIONS.md`](docs/CONVENTIONS.md). Read both before a first change.

Run **`make check`** before every commit. It is the whole gate and there is
deliberately no lighter subset: `cargo fmt --check` (promoting rustfmt's silent
`Unknown configuration option` warning to a failure), the clippy sweep with
`nursery` and `pedantic` over every target *and* every `cfg` (`CRUMB=1`,
`DIFF=1`, `PERF=1` — code behind a `cfg` was once linted by nothing), `make
audit` (the rules clippy cannot express), `make doc` (rustdoc, so doc links
have to resolve), `make lint-counts` (the annotated exemption counts), and
`make test` (the unit tests, via `cargo-nextest`). The clippy and doc legs deny
every warning via cargo's `build.warnings = "deny"`; normal builds and a plain
`cargo clippy` only warn. Each audit finding names the section of
`docs/CONVENTIONS.md` it comes from. There is no CI — pushing a `v*` tag only
opens a draft release — so this is the only gate. **`make check` says nothing
about whether a reimplementation still matches the original; see the
reimplementation contract in `docs/CONVENTIONS.md`.**

Releases: pushing a `v*` tag opens a draft release through the GitHub
workflow, which first checks the tag against both workspace versions and fails
if they disagree; upload the `dist/` zips from a local `make bundle`, generate
the release notes, and publish.

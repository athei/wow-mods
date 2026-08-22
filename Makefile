export WINE_SDK

# rustfmt.toml uses unstable options, so formatting needs nightly. Override to
# pin a dated toolchain if a nightly ever regresses:  make check FMT_TOOLCHAIN=nightly-YYYY-MM-DD
FMT_TOOLCHAIN ?= nightly


# Release bundles always ship the production profile.
ifneq ($(filter bundle,$(MAKECMDGOALS)),)
PROD := 1
endif

ifeq ($(PROD),1)
PROFILE  := production
$(info ==> PROD=1: cargo profile `production` (fat LTO + codegen-units=1))
else
PROFILE  := release
endif

ifeq ($(CRUMB),1)
export WOW_CRUMB := 1
$(info ==> CRUMB=1: cfg(wow_crumb) breadcrumb ring buffer enabled)
endif

ifeq ($(DIFF),1)
export WOW_TURBO_DIFF := 1
$(info ==> DIFF=1: wow_turbo differential harness compiled in (arm at runtime via WOW_TURBO_DIFF_ARM=all|Name))
endif

ifeq ($(PERF),1)
export WOW_TURBO_PERF := 1
$(info ==> PERF=1: wow_turbo diagnostic layer compiled in: seam counters, tripwires and the script gauge, reporting at info)
endif

# Frame pointers, which shipped builds do not carry: the default hands EBP to
# the register allocator as an eighth GPR across the whole DLL. FP=1 puts the
# frames back, which is what the guest-pc sampler needs to walk the guest EBP
# chain and attribute a sample to its callers; without them a profile stops at
# the leaf. It is a config overlay rather than an env because it changes
# compiler flags on both the Rust and the C++ side (see .cargo/fp.toml), and it
# composes with the three envs above, so a profiling session is
# `FP=1 PERF=1 make install`. Only the `windows` target takes it; the AVX
# variant is a shipping artifact and stays on the default.
ifeq ($(FP),1)
FP_CONFIG := --config .cargo/fp.toml
$(info ==> FP=1: frame pointers forced on, EBP pinned for the sampler's stack walk)
endif

# WoW 1.12 is 32-bit only, so the PE side builds i686 exclusively (no x64).
PE_i386     := i686-pc-windows-msvc
# Wine picks a builtin's unix half out of `lib/wine/<cpu>-unix` by the arch of
# the Wine build that loads it, so the `.so` ships once per Wine host ISA:
# x86_64 for a Wine running under Rosetta, aarch64 for an arm64-native one. The
# PE side is unaffected, staying i686 either way, so both halves pair with the
# same 32-bit bridge DLL over Wine's wow64 path.
UNIX_TARGET_x64    := x86_64-apple-darwin
UNIX_TARGET_arm64  := aarch64-apple-darwin
UNIX_WINEDIR_x64   := x86_64-unix
UNIX_WINEDIR_arm64 := aarch64-unix
# The unix host-target legs (tests + clippy) reach the native host — aarch64 on
# Apple Silicon, no Rosetta — by omitting `--target`, per unix/.cargo/config.toml.

OUT_i386       := windows/target/$(PE_i386)/$(PROFILE)
OUT_avx        := windows/target/avx/$(PE_i386)/$(PROFILE)
OUT_unix_x64   := unix/target/$(UNIX_TARGET_x64)/$(PROFILE)
OUT_unix_arm64 := unix/target/$(UNIX_TARGET_arm64)/$(PROFILE)

# Hard-fail on any warning (cargo counts emitted warnings, including ones
# replayed from cache, and errors at the end of the run) — applied only to the
# `check` legs so normal builds and a plain `cargo clippy` stay
# warning-tolerant. Unlike `-D warnings` (via clippy args or RUSTDOCFLAGS) this
# changes no compiler flags, so check runs share the build cache with plain
# invocations.
DENY_WARNINGS := --config 'build.warnings="deny"'

INSTALL_DIRS := $(WINE_SDK) $(WINE_INSTALL_DIR)

# Deploy destinations come from the environment, never from hardcoded paths:
# WOW_EXE points at the client executable — native mods deploy into the mods/
# dir next to it. No default; pass it per invocation or export it:
#   make install WOW_EXE=/path/to/game/WoW.exe
GAME_MODS = $(dir $(WOW_EXE))mods

MAKEFLAGS += --silent

.PHONY: all windows windows-avx unix install bundle test test-isolated fmt clippy audit doc check unsafe-debt \
        lint-counts lint-counts-update update-inventories \
        upgrade upgrade-incompat clean require-wow-exe require-wine-sdk

# Scoped to the targets that actually link. `fmt`, `clippy`, `audit`, `doc` and
# `check` never reach the linker, so a contributor auditing the tree does not
# need a Wine SDK staged to run the gate — and a top-level `ifndef` would have
# stopped them at Makefile parse time, before any target was even selected.
require-wine-sdk:
	test -n "$(WINE_SDK)" || { echo "error: WINE_SDK is not set" >&2; exit 1; }

require-wow-exe:
	test -n "$(WOW_EXE)" || { echo "error: WOW_EXE is not set (path to the client's WoW.exe)" >&2; exit 1; }

all: windows unix

windows: require-wine-sdk
	cd windows && cargo build --profile $(PROFILE) --target $(PE_i386) $(FP_CONFIG)
	# Wine builtins: version.dll (the injector) and wow_mods.dll (the unixlib
	# bridge that pairs wow_mods.so). The mods themselves (wow_turbo.dll,
	# wow_translate.dll) are native and are NOT stamped.
	winebuild --builtin $(OUT_i386)/version.dll
	winebuild --builtin $(OUT_i386)/wow_mods.dll
	# Tiny "fake DLL" placeholder for the wow_mods builtin name, for placing in
	# the syswow64 of a prefix that already existed when wow_mods was installed
	# (Wine finds builtins by name in the prefix, not lib/wine). A prefix created
	# afterwards needs none: wineboot stamps a marker for every builtin it finds
	# in lib/wine. Nothing copies this automatically any more.
	winebuild --fake-module -o $(OUT_i386)/wow_mods.fake.dll -m32 --dll $(OUT_i386)/wow_mods.dll

# wow_turbo for native Windows: same i686 DLL, but with the ISA baseline raised
# to haswell (AVX2 — see .cargo/avx.toml). Built into its own target dir so it
# never clobbers the nehalem artifacts. Not a Wine builtin, so no winebuild.
windows-avx: require-wine-sdk
	cd windows && cargo build -p wow-turbo-dll --profile $(PROFILE) \
	    --target $(PE_i386) --target-dir target/avx --config .cargo/avx.toml

unix: require-wine-sdk
	cd unix && cargo build --profile $(PROFILE) --target $(UNIX_TARGET_x64)
	cd unix && cargo build --profile $(PROFILE) --target $(UNIX_TARGET_arm64)
	# On Mach-O the DWARF stays behind in the compiler's `.o` files, with only a
	# debug map in the dylib pointing at them by absolute path; `dsymutil` walks
	# that map and gathers the DWARF into a `.dSYM`, the shippable equivalent of
	# an MSVC `.pdb`. Run it on a copy already named `wow_mods.so`, because it
	# stamps the inner DWARF file after the input's basename and lldb looks it up
	# by that name — renaming the bundle afterwards produces one lldb won't find.
	for out in $(OUT_unix_x64) $(OUT_unix_arm64); do \
	    cp $$out/libwow_mods.dylib $$out/wow_mods.so ; \
	    rm -rf $$out/wow_mods.so.dSYM ; \
	    dsymutil $$out/wow_mods.so ; \
	done

install: all require-wow-exe
	# Wine builtins → the wine dirs: the i686 version.dll + wow_mods.dll and the
	# companion wow_mods.so. WoW is 32-bit, so the 32-bit bridge pairs the host
	# `.so` over Wine's wow64 path. Symbols travel with each binary: the `.pdb`
	# beside every PE, the `.dSYM` beside the `.so`, so a local crash
	# symbolicates against the installed files with no extra flags.
	#
	# Both unix arches go in, creating whichever directory the tree lacks: a
	# Wine loads only the `.so` matching its own build, so the other copy is
	# inert, and an x86_64 tree has no aarch64-unix (nor an arm64 one an
	# x86_64-unix) for the copy to land in otherwise.
	for dir in $(INSTALL_DIRS); do \
	    cp $(OUT_i386)/version.dll   $(OUT_i386)/version.pdb   $$dir/lib/wine/i386-windows/ ; \
	    cp $(OUT_i386)/wow_mods.dll  $(OUT_i386)/wow_mods.pdb  $$dir/lib/wine/i386-windows/ ; \
	    mkdir -p $$dir/lib/wine/$(UNIX_WINEDIR_x64) $$dir/lib/wine/$(UNIX_WINEDIR_arm64) ; \
	    cp $(OUT_unix_x64)/wow_mods.so       $$dir/lib/wine/$(UNIX_WINEDIR_x64)/ ; \
	    rm -rf $$dir/lib/wine/$(UNIX_WINEDIR_x64)/wow_mods.so.dSYM ; \
	    cp -R $(OUT_unix_x64)/wow_mods.so.dSYM $$dir/lib/wine/$(UNIX_WINEDIR_x64)/ ; \
	    cp $(OUT_unix_arm64)/wow_mods.so     $$dir/lib/wine/$(UNIX_WINEDIR_arm64)/ ; \
	    rm -rf $$dir/lib/wine/$(UNIX_WINEDIR_arm64)/wow_mods.so.dSYM ; \
	    cp -R $(OUT_unix_arm64)/wow_mods.so.dSYM $$dir/lib/wine/$(UNIX_WINEDIR_arm64)/ ; \
	    cp $(OUT_i386)/wow_mods.fake.dll $$dir/lib/wine/i386-windows/ ; \
	done
	# Native game-side mods → the app bundle's game mods/ dir (loaded by path via
	# dlls.txt), NOT the wine builtin dir. `DIFF=1 make install` builds wow_turbo
	# with the differential harness; `FP=1 PERF=1 make install` builds the one a
	# profiling capture wants. List both in `dlls.txt` next to WoW.exe:
	#   mods/wow_turbo.dll
	#   mods/wow_translate.dll
	mkdir -p $(GAME_MODS)
	cp $(OUT_i386)/wow_turbo.dll      $(OUT_i386)/wow_turbo.pdb      $(GAME_MODS)/
	cp $(OUT_i386)/wow_translate.dll  $(OUT_i386)/wow_translate.pdb  $(GAME_MODS)/

# Compose the user-facing release archives from the build outputs. Four zips,
# one per shippable artifact, plus a fifth holding their debug symbols; the
# layouts of the first four mirror the install destinations
# documented in the README: merge game/ into the game folder next to WoW.exe,
# merge wine/ into the Wine distribution. The loader (version.dll + dlls.txt)
# is deliberately its own artifact — the mods load with any loader, and the
# injector only works on clients that load a version.dll at all. Always built
# at the production profile (PROD is forced above when `bundle` is a goal).
BUNDLE_VERSION := $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
TURBO_MAC      := wow_turbo-$(BUNDLE_VERSION)-mac
TURBO_WIN      := wow_turbo-$(BUNDLE_VERSION)-windows-avx
TRANSLATE      := wow_translate-$(BUNDLE_VERSION)
LOADER         := version_loader-$(BUNDLE_VERSION)
# A fifth zip nobody installs: the symbols for every artifact above, so a crash
# report from a release build can be read. One archive for both ISA baselines,
# split by subdirectory because the two wow_turbo builds share a filename.
DEBUG          := wow_mods-debug-$(BUNDLE_VERSION)
BUNDLE_NAMES   := $(TURBO_MAC) $(TURBO_WIN) $(TRANSLATE) $(LOADER) $(DEBUG)
# The identity the binaries themselves log (`unix/shared/build.rs`), which drops
# the `--dirty` marker BUNDLE_VERSION carries. Written into the debug archive so
# it can be paired with a captured log without guessing.
BUILD_ID       := $(shell git describe --tags --always 2>/dev/null || \
                          sed -n 's/^version = "\(.*\)"/v\1/p' windows/Cargo.toml)

bundle: all windows-avx
	rm -rf $(addprefix dist/,$(BUNDLE_NAMES) $(addsuffix .zip,$(BUNDLE_NAMES)))
	# wow_turbo, one DLL per ISA baseline: nehalem for the Wine-on-macOS stack
	# (Rosetta's vector unit is 128-bit), haswell (AVX2) for native Windows.
	mkdir -p dist/$(TURBO_MAC)/game/mods dist/$(TURBO_WIN)/game/mods
	cp $(OUT_i386)/wow_turbo.dll  dist/$(TURBO_MAC)/game/mods/
	cp $(OUT_avx)/wow_turbo.dll   dist/$(TURBO_WIN)/game/mods/
	# WoWTranslate (Wine-on-macOS only): the mod DLL + Lua addon, and the wine/
	# half with the wow_mods unixlib bridge pair. The `.so` ships for both Wine
	# host arches so one archive drops into either build; each Wine reads only
	# the directory matching itself.
	mkdir -p dist/$(TRANSLATE)/game/mods dist/$(TRANSLATE)/game/Interface/AddOns \
	         dist/$(TRANSLATE)/wine/lib/wine/i386-windows \
	         dist/$(TRANSLATE)/wine/lib/wine/$(UNIX_WINEDIR_x64) \
	         dist/$(TRANSLATE)/wine/lib/wine/$(UNIX_WINEDIR_arm64)
	cp $(OUT_i386)/wow_translate.dll  dist/$(TRANSLATE)/game/mods/
	cp -R addon/WoWTranslate          dist/$(TRANSLATE)/game/Interface/AddOns/
	cp $(OUT_i386)/wow_mods.dll       dist/$(TRANSLATE)/wine/lib/wine/i386-windows/
	cp $(OUT_i386)/wow_mods.fake.dll  dist/$(TRANSLATE)/wine/lib/wine/i386-windows/
	cp $(OUT_unix_x64)/wow_mods.so    dist/$(TRANSLATE)/wine/lib/wine/$(UNIX_WINEDIR_x64)/
	cp $(OUT_unix_arm64)/wow_mods.so  dist/$(TRANSLATE)/wine/lib/wine/$(UNIX_WINEDIR_arm64)/
	# The standalone loader: version.dll injects every mod listed in dlls.txt
	# (ships with both mods listed — users drop the lines they don't want).
	mkdir -p dist/$(LOADER)/game dist/$(LOADER)/wine/lib/wine/i386-windows
	printf 'mods/wow_turbo.dll\nmods/wow_translate.dll\n' > dist/$(LOADER)/game/dlls.txt
	cp $(OUT_i386)/version.dll  dist/$(LOADER)/wine/lib/wine/i386-windows/
	# The symbols for exactly the binaries staged above. Grouped by ISA baseline
	# rather than by install destination: debug info has no install route, and
	# the split is what keeps the two wow_turbo.pdb files apart. The two `.so`
	# dSYMs share a name as well, so they take a subdirectory each.
	mkdir -p dist/$(DEBUG)/mac/$(UNIX_WINEDIR_x64) dist/$(DEBUG)/mac/$(UNIX_WINEDIR_arm64) \
	         dist/$(DEBUG)/windows-avx
	echo $(BUILD_ID)                     > dist/$(DEBUG)/BUILD
	cp $(OUT_i386)/version.pdb             dist/$(DEBUG)/mac/
	cp $(OUT_i386)/wow_mods.pdb            dist/$(DEBUG)/mac/
	cp $(OUT_i386)/wow_turbo.pdb           dist/$(DEBUG)/mac/
	cp $(OUT_i386)/wow_translate.pdb       dist/$(DEBUG)/mac/
	cp -R $(OUT_unix_x64)/wow_mods.so.dSYM   dist/$(DEBUG)/mac/$(UNIX_WINEDIR_x64)/
	cp -R $(OUT_unix_arm64)/wow_mods.so.dSYM dist/$(DEBUG)/mac/$(UNIX_WINEDIR_arm64)/
	cp $(OUT_avx)/wow_turbo.pdb            dist/$(DEBUG)/windows-avx/
	for name in $(BUNDLE_NAMES); do \
	    (cd dist && zip -qr $$name.zip $$name) ; \
	    echo "==> dist/$$name.zip" ; \
	done

test:
	# wow_turbo's portable numeric kernels are built+run as x86_64-apple-darwin so
	# the real SSE path executes under the same Rosetta translation the shipped
	# 32-bit DLL uses — not a native-aarch64 stand-in.
	#
	# `cargo test`, not nextest, and the reason is that same Rosetta translation.
	# nextest runs a process per test, and spawning this crate's x86_64 test
	# binary costs ~1.4s of Rosetta setup every time — a fixed toll no amount of
	# parallelism pays down. Measured over the 1846 tests: 135s under nextest,
	# 7s in one threaded process. Same tests, same target, same assertions; what
	# is given up is per-test process isolation, which `make test-isolated`
	# still buys when a test is suspected of leaking into its neighbours.
	cd windows && cargo test -p wow-turbo-dll --target $(UNIX_TARGET_x64)
	# Native aarch64, no Rosetta toll, so this leg keeps nextest.
	cd unix && cargo nextest run

# The same tests with a process per test. Slow (see `test`), and worth it only
# to pin down a test that passes alone and fails in company, or one that aborts
# the whole run rather than failing.
test-isolated:
	cd windows && cargo nextest run -p wow-turbo-dll --target $(UNIX_TARGET_x64)
	cd unix && cargo nextest run

clippy:
	# --all-targets: test code is code. Without it the intersection of cfg(test)
	# and the 32-bit target is linted by nothing, and that is where the
	# 32-bit-only test helpers live.
	cd windows && cargo clippy --target $(PE_i386) --all-targets $(DENY_WARNINGS)
	# wow_turbo's tests + portable kernels only exist off the PE target; lint them
	# on the x86_64 host target the host tests run under.
	cd windows && cargo clippy -p wow-turbo-dll --target $(UNIX_TARGET_x64) --all-targets $(DENY_WARNINGS)
	cd unix && cargo clippy --all-targets $(DENY_WARNINGS)
	# `unix/shared` ships into both worlds but is a member of only this
	# workspace, so the windows legs run it through plain rustc with no lint
	# table. The leg above already covers aarch64, one of the two arches the
	# `.so` ships as; this one takes the other, and the last takes the PE target
	# that reaches its 32-bit arms.
	cd unix && cargo clippy --all-targets --target $(UNIX_TARGET_x64) $(DENY_WARNINGS)
	cd unix && cargo clippy -p wow-shared --target $(PE_i386) $(DENY_WARNINGS)

# The conventions clippy can't express: doc-comment shape, the Clone/Copy derive
# inventory, and the patterns that are banned or confined to a known set of
# files. See docs/CONVENTIONS.md § Mechanical audit.
audit:
	./scripts/audit.sh

# The finding counts annotated against each exempted lint in the workspace
# manifests. A leg of `check`, and useful alone when you change a lint table.
# Force-warning those lints changes the compiler flags, so this leg cannot share
# the build cache with the default-cfg legs; cargo fingerprints it separately.
lint-counts:
	./scripts/lint_counts.sh

lint-counts-update:
	./scripts/lint_counts.sh --update

# The unsafe-block debt recorded in docs/CONVENTIONS.md § One operation per
# unsafe block. Both lints together — counting only one of them is how the
# undocumented-block half of the number went unrecorded.
unsafe-debt:
	cd windows && cargo clippy -p wow-turbo-dll --target $(PE_i386) --all-targets \
	    --message-format=short -- \
	    --force-warn clippy::multiple_unsafe_ops_per_block \
	    --force-warn clippy::undocumented_unsafe_blocks 2>&1 | \
	    awk '/missing a safety comment/ {u++} /expected only one/ {m++} \
	         END {printf "undocumented: %d\nmulti-operation: %d\ntotal: %d\n", u, m, u + m}'

# The two committed inventories, regenerated. Both are diffed by `make audit`,
# so a change here is a change a reviewer sees.
update-inventories:
	./scripts/audit.sh --update-derives
	./scripts/audit.sh --update-allows

# rustdoc's own lints, which no other target sees: broken and private intra-doc
# links, malformed HTML in doc comments. clippy gates a doc block's prose; only
# rustdoc knows whether its links resolve. `build.warnings` covers rustdoc
# warnings too, so no RUSTDOCFLAGS needed.
#
# The windows workspace is documented for a PE target, not the host: the mods
# are `cdylib`s that only build for *-pc-windows-msvc, so a host run would
# silently skip them.
doc:
	cd windows && cargo doc --no-deps --target $(PE_i386) $(DENY_WARNINGS)
	cd unix && cargo doc --no-deps $(DENY_WARNINGS)

fmt:
	cd windows && cargo +$(FMT_TOOLCHAIN) fmt
	cd unix && cargo +$(FMT_TOOLCHAIN) fmt

# One command, run before every commit. Everything the tree can check without a
# running client: formatting, the clippy sweep over every target and every
# `cfg`, the conventions audit, the doc build, the annotated lint counts, and
# the tests. fmt-check first — fastest leg, cheapest mistake.
#
# There is deliberately no faster subset. A second, lighter gate is the one that
# gets run, and the fuller one then rots: the `cfg` legs below were added after
# `CRUMB=1` turned up three findings that had been invisible since the day the
# breadcrumb ring was written, because nothing ever compiled it. Splitting the
# gate would recreate exactly that.
#
# It costs around fifty seconds against warm target dirs. Most of that is the
# legs that cannot share a build cache — each `cfg` and the force-warn counts
# compile under different flags, so cargo fingerprints them separately.
#
# Still NOT covered, and not coverable here: the differential harness itself.
# `DIFF=1 make install`, then exercise the touched arms against a live client.
# docs/CONVENTIONS.md § The reimplementation contract — that comparison is the
# definition of done, and no amount of static checking substitutes for it.
check:
	cd windows && cargo +$(FMT_TOOLCHAIN) fmt --check
	cd unix && cargo +$(FMT_TOOLCHAIN) fmt --check
	# rustfmt reports an unrecognised config key as a warning and still exits 0,
	# so a renamed or stabilised option would stop being applied while the tree
	# stayed green under a rule nobody was enforcing any more. Nothing else in
	# the gate can see that; promote it to a failure.
	cd windows && ! cargo +$(FMT_TOOLCHAIN) fmt --check 2>&1 | grep 'Unknown configuration option'
	$(MAKE) clippy
	$(MAKE) audit
	$(MAKE) doc
	# The code behind a `cfg`: the breadcrumb ring (~455 lines of unsafe mmap and
	# Win32 FFI), the differential harness including every generated `*_diff`
	# capture function, and the diagnostic layer (the counters, the tripwires
	# and the ~1700-line script gauge). A default build compiles none of them.
	CRUMB=1 $(MAKE) clippy
	CRUMB=1 $(MAKE) doc
	DIFF=1 $(MAKE) clippy
	DIFF=1 $(MAKE) doc
	PERF=1 $(MAKE) clippy
	PERF=1 $(MAKE) doc
	$(MAKE) lint-counts
	$(MAKE) test

# Semver-compatible bumps; `upgrade-incompat` needs cargo-edit.
upgrade:
	cd windows && cargo update
	cd unix && cargo update

upgrade-incompat:
	cd windows && cargo upgrade --incompatible && cargo update
	cd unix && cargo upgrade --incompatible && cargo update

clean:
	cd windows && cargo clean
	cd unix && cargo clean
	rm -rf dist

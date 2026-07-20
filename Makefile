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

# WoW 1.12 is 32-bit only, so the PE side builds i686 exclusively (no x64).
PE_i386     := i686-pc-windows-msvc
# The unix `.so` must be x86_64 Mach-O (Wine's unix-call boundary), so shipped
# artifacts are always built for x86_64.
UNIX_RELEASE_TARGET := x86_64-apple-darwin
# Native host target for unit tests + clippy (aarch64 on Apple Silicon) — no Rosetta.
UNIX_NATIVE_TARGET  := $(shell rustc -vV | sed -n 's/^host: //p')

OUT_i386 := windows/target/$(PE_i386)/$(PROFILE)
OUT_avx  := windows/target/avx/$(PE_i386)/$(PROFILE)
OUT_unix := unix/target/$(UNIX_RELEASE_TARGET)/$(PROFILE)

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

.PHONY: all windows windows-avx unix install bundle test fmt clippy audit doc check check-all \
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
	cd windows && cargo build --profile $(PROFILE) --target $(PE_i386)
	# Wine builtins: version.dll (the injector) and wow_mods.dll (the unixlib
	# bridge that pairs wow_mods.so). The mods themselves (wow_turbo.dll,
	# wow_translate.dll) are native and are NOT stamped.
	winebuild --builtin $(OUT_i386)/version.dll
	winebuild --builtin $(OUT_i386)/wow_mods.dll
	# Tiny "fake DLL" placeholder for the wow_mods builtin name. Shipped into
	# lib/wine; the game launcher copies it into the prefix's syswow64 on setup
	# (Wine finds builtins by name in the prefix, not lib/wine).
	winebuild --fake-module -o $(OUT_i386)/wow_mods.fake.dll -m32 --dll $(OUT_i386)/wow_mods.dll

# wow_turbo for native Windows: same i686 DLL, but with the ISA baseline raised
# to haswell (AVX2 — see .cargo/avx.toml). Built into its own target dir so it
# never clobbers the nehalem artifacts. Not a Wine builtin, so no winebuild.
windows-avx: require-wine-sdk
	cd windows && cargo build -p wow-turbo-dll --profile $(PROFILE) \
	    --target $(PE_i386) --target-dir target/avx --config .cargo/avx.toml

unix: require-wine-sdk
	cd unix && cargo build --profile $(PROFILE) --target $(UNIX_RELEASE_TARGET)

install: all require-wow-exe
	# Wine builtins → the wine dirs: the i686 version.dll + wow_mods.dll and the
	# companion wow_mods.so. WoW is 32-bit, so the 32-bit bridge pairs the x86_64
	# `.so` over Wine's wow64 path.
	for dir in $(INSTALL_DIRS); do \
	    cp $(OUT_i386)/version.dll   $(OUT_i386)/version.pdb   $$dir/lib/wine/i386-windows/ ; \
	    cp $(OUT_i386)/wow_mods.dll  $(OUT_i386)/wow_mods.pdb  $$dir/lib/wine/i386-windows/ ; \
	    cp $(OUT_unix)/libwow_mods.dylib $$dir/lib/wine/x86_64-unix/wow_mods.so ; \
	    cp $(OUT_i386)/wow_mods.fake.dll $$dir/lib/wine/i386-windows/ ; \
	done
	# Native game-side mods → the app bundle's game mods/ dir (loaded by path via
	# dlls.txt), NOT the wine builtin dir. `DIFF=1 make install` builds wow_turbo
	# with the differential harness. List both in `dlls.txt` next to WoW.exe:
	#   mods/wow_turbo.dll
	#   mods/wow_translate.dll
	mkdir -p $(GAME_MODS)
	cp $(OUT_i386)/wow_turbo.dll      $(OUT_i386)/wow_turbo.pdb      $(GAME_MODS)/
	cp $(OUT_i386)/wow_translate.dll  $(OUT_i386)/wow_translate.pdb  $(GAME_MODS)/

# Compose the user-facing release archives from the build outputs. Four zips,
# one per shippable artifact; the layouts mirror the install destinations
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
BUNDLE_NAMES   := $(TURBO_MAC) $(TURBO_WIN) $(TRANSLATE) $(LOADER)

bundle: all windows-avx
	rm -rf $(addprefix dist/,$(BUNDLE_NAMES) $(addsuffix .zip,$(BUNDLE_NAMES)))
	# wow_turbo, one DLL per ISA baseline: nehalem for the Wine-on-macOS stack
	# (Rosetta's vector unit is 128-bit), haswell (AVX2) for native Windows.
	mkdir -p dist/$(TURBO_MAC)/game/mods dist/$(TURBO_WIN)/game/mods
	cp $(OUT_i386)/wow_turbo.dll  dist/$(TURBO_MAC)/game/mods/
	cp $(OUT_avx)/wow_turbo.dll   dist/$(TURBO_WIN)/game/mods/
	# WoWTranslate (Wine-on-macOS only): the mod DLL + Lua addon, and the wine/
	# half with the wow_mods unixlib bridge pair.
	mkdir -p dist/$(TRANSLATE)/game/mods dist/$(TRANSLATE)/game/Interface/AddOns \
	         dist/$(TRANSLATE)/wine/lib/wine/i386-windows dist/$(TRANSLATE)/wine/lib/wine/x86_64-unix
	cp $(OUT_i386)/wow_translate.dll  dist/$(TRANSLATE)/game/mods/
	cp -R addon/WoWTranslate          dist/$(TRANSLATE)/game/Interface/AddOns/
	cp $(OUT_i386)/wow_mods.dll       dist/$(TRANSLATE)/wine/lib/wine/i386-windows/
	cp $(OUT_i386)/wow_mods.fake.dll  dist/$(TRANSLATE)/wine/lib/wine/i386-windows/
	cp $(OUT_unix)/libwow_mods.dylib  dist/$(TRANSLATE)/wine/lib/wine/x86_64-unix/wow_mods.so
	# The standalone loader: version.dll injects every mod listed in dlls.txt
	# (ships with both mods listed — users drop the lines they don't want).
	mkdir -p dist/$(LOADER)/game dist/$(LOADER)/wine/lib/wine/i386-windows
	printf 'mods/wow_turbo.dll\nmods/wow_translate.dll\n' > dist/$(LOADER)/game/dlls.txt
	cp $(OUT_i386)/version.dll  dist/$(LOADER)/wine/lib/wine/i386-windows/
	for name in $(BUNDLE_NAMES); do \
	    (cd dist && zip -qr $$name.zip $$name) ; \
	    echo "==> dist/$$name.zip" ; \
	done

test:
	# wow_turbo's portable numeric kernels are built+run as x86_64-apple-darwin so
	# the real SSE path executes under the same Rosetta translation the shipped
	# 32-bit DLL uses — not a native-aarch64 stand-in.
	cd windows && cargo nextest run -p wow-turbo-dll --target $(UNIX_RELEASE_TARGET)
	cd unix && cargo nextest run

clippy:
	# --all-targets: test code is code. Without it the intersection of cfg(test)
	# and the 32-bit target is linted by nothing, and that is where the
	# 32-bit-only test helpers live.
	cd windows && cargo clippy --target $(PE_i386) --all-targets $(DENY_WARNINGS)
	# wow_turbo's tests + portable kernels only exist off the PE target; lint them
	# on the x86_64 host target the host tests run under.
	cd windows && cargo clippy -p wow-turbo-dll --target $(UNIX_RELEASE_TARGET) --all-targets $(DENY_WARNINGS)
	cd unix && cargo clippy --all-targets $(DENY_WARNINGS)
	# `unix/shared` ships into both worlds but is a member of only this
	# workspace, so the windows legs run it through plain rustc with no lint
	# table. Lint it on the target it actually ships as, and again on the PE
	# target that reaches its 32-bit arms.
	cd unix && cargo clippy --all-targets --target $(UNIX_RELEASE_TARGET) $(DENY_WARNINGS)
	cd unix && cargo clippy -p wow-shared --target $(PE_i386) $(DENY_WARNINGS)

# The conventions clippy can't express: doc-comment shape, the Clone/Copy derive
# inventory, and the patterns that are banned or confined to a known set of
# files. See docs/CONVENTIONS.md § Mechanical audit.
audit:
	./scripts/audit.sh

# The finding counts annotated against each exempted lint in the workspace
# manifests. Deliberately outside `check`: force-warning those lints changes the
# compiler flags, so this cannot share check's build cache and costs minutes
# rather than seconds. Run it when you change a lint table.
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

# One command to run before every commit: formatting, the full clippy sweep, the
# conventions audit, and the doc build. fmt-check first (fast, fails early on
# the cheapest mistake).
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

# Everything the tree can check without a running client. `check` is the fast
# pre-commit gate and deliberately stays seconds; this is the one to run before
# a release, after touching a lint table, or after touching anything behind a
# `cfg`. It is minutes, because each leg below compiles under different flags
# and so cannot share check's build cache.
#
# What it adds, and why none of it belongs in `check`:
#   - the `cfg`-gated code. `check` compiles neither the differential harness
#     nor the breadcrumb ring, so ~455 lines of unsafe mmap/FFI and every
#     generated `*_diff` capture function are invisible to it. That is not
#     hypothetical: this leg was added after `CRUMB=1` turned up three findings
#     the gate had never seen.
#   - the annotated lint counts, which need a force-warn run per leg.
#   - the tests, which `check` leaves out because a green build and a green test
#     say nothing about whether a reimplementation still matches the original.
#     They are still worth running before a release.
#
# Still NOT covered, and not coverable here: the differential harness itself.
# `DIFF=1 make install`, then exercise the touched arms against a live client.
# docs/CONVENTIONS.md § The reimplementation contract — that comparison is the
# definition of done, and no amount of static checking substitutes for it.
check-all: check
	CRUMB=1 $(MAKE) clippy
	CRUMB=1 $(MAKE) doc
	DIFF=1 $(MAKE) clippy
	DIFF=1 $(MAKE) doc
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

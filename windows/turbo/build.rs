//! Generates `$OUT_DIR/symbols.rs` from `symbols.toml`.
//!
//! `symbols.toml` is the hand-edited hook manifest: one fully-specified
//! `[functions.Name]` entry per function — `rva`, `sig`, `abi`, `ret`, `args`,
//! `preserve` are all required (no half-specified entries).
//!
//! Per entry the reimpl is wired at compile time — the generated `extern "abi"`
//! thunk `MinHook` installs calls straight through to its sibling
//! `super::hooks::<snake>` adapter with only the host arguments. The unhooked
//! original (the `MinHook` trampoline) is the only runtime state; it is published
//! at install time and reached through a typed accessor, `originals::<snake>()`,
//! which returns the matching `{Name}Fn` for a reimpl that must delegate (almost
//! none do). The installer refuses to patch unless the bytes at the target match
//! the entry's `sig`. Addresses + signatures are inlined into the installer, not
//! exposed as constants. Never edit the generated file — edit the manifest.

// This script emits Rust source as string data; the brace-delimited fragments in
// those literals are the *generated* code's format args, not this script's — so
// the formatting-args lint is a false positive here.
#![allow(clippy::literal_string_with_formatting_args)]

use std::{collections::BTreeMap, env, fmt::Write as _, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    functions: BTreeMap<String, Function>,
}

#[derive(Deserialize)]
struct Function {
    rva: i64,
    sig: Signature,
    abi: String,
    ret: String,
    args: Vec<String>,
    /// Volatile GP registers the original leaves intact and our reimpl must also leave intact.
    ///
    /// Subset of `eax`/`ecx`/`edx`; empty for none. The 1.12 client keeps these
    /// caller-saved registers live across calls to these internal functions.
    /// Orthogonal to `abi`; drives the save/restore shim.
    preserve: Vec<String>,
    /// Install this hook only when the event gauge is armed.
    ///
    /// For an observation hook whose target is hot enough that even a
    /// trampoline on the unarmed path is a measurable tax — `luaD_precall` runs
    /// on every Lua-level call, not once per handler invoke. The logger is
    /// initialized before `install_all`, so the gate resolves at install time
    /// and an unarmed run keeps the function fully stock.
    #[serde(default)]
    armed_only: bool,
    /// Optional differential-mode annotation.
    ///
    /// Present only on functions the harness can validate against the live
    /// original (`wow_turbo_diff` builds); absent ⇒ the thunk is generated
    /// exactly as without the harness.
    #[serde(default)]
    diff: Option<Diff>,
}

/// The prologue byte pattern(s) an entry is willing to patch over.
///
/// One pattern is the ordinary case. A list is for a function whose entry can
/// legitimately hold more than one known prologue — a hot script method another
/// mod replaces wholesale, where our reimplementation reproduces that mod's
/// behaviour as well as stock, so either is a shape we can stand in for. Order
/// carries no meaning; a match against any listed pattern permits the patch,
/// and matching none is the refusal that guards against an unknown build.
#[derive(Deserialize)]
#[serde(untagged)]
enum Signature {
    One(String),
    Any(Vec<String>),
}

impl Signature {
    /// Every accepted pattern, whichever spelling the manifest used.
    fn patterns(&self) -> &[String] {
        match self {
            Self::One(sig) => core::slice::from_ref(sig),
            Self::Any(sigs) => sigs,
        }
    }

    /// The patterns as a generated Rust slice literal.
    fn literal(&self) -> String {
        let items = self
            .patterns()
            .iter()
            .map(|sig| format!("{sig:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("&[{items}]")
    }
}

/// A contiguous pointer-addressed memory region.
///
/// Described by which argument points at it and how many bytes it spans.
#[derive(Deserialize)]
struct Region {
    arg: usize,
    len: usize,
}

/// Differential-mode annotation (the optional `[functions.<key>.diff]` table).
///
/// `out`/`inout` run the original and the reimpl on the same inputs and compare
/// the output region; `capture` runs only the original and logs the live call.
// Each bool is an independent key in the manifest's `[functions.X.diff]` table, so
// this struct's shape is the file's shape rather than a design choice. Grouping them
// into a nested struct to satisfy the lint would change the on-disk schema of every
// annotated entry to make a deserialization type read better.
#[allow(clippy::struct_excessive_bools)]
#[derive(Deserialize)]
struct Diff {
    /// `"out"` | `"inout"` | `"capture"`.
    mode: String,
    /// The output region (required for `out`/`inout`; the reimpl writes here).
    #[serde(default)]
    out: Option<Region>,
    /// Input regions snapshotted before the original runs and replayed into the reimpl.
    ///
    /// So an in-place (`inout`) function sees the pre-call state.
    #[serde(default)]
    ins: Vec<Region>,
    /// `"f32"` (lane compare within `ulp`) | `"bytes"` (exact memcmp).
    #[serde(default = "default_float")]
    float: String,
    /// Max ULP per `f32` lane before a divergence is reported.
    #[serde(default = "default_ulp")]
    ulp: u32,
    /// Byte offset in the `out` region where the `f32` lanes begin.
    ///
    /// For a region that is not uniformly float: bytes before it are compared
    /// exactly and bytes from it on are compared as lanes within `ulp`. Only
    /// meaningful with `float = "f32"`; the default of 0 is the ordinary
    /// whole-region lane compare.
    ///
    /// This exists because the alternative for a mixed region is to compare the
    /// whole thing as lanes, which silently *loses* coverage — an arbitrary
    /// pointer can carry a NaN bit pattern, and the lane comparator treats any
    /// two NaNs as equal, so two different pointers would compare equal.
    #[serde(default)]
    float_from: usize,
    /// Bits of an integer return that are the function's contract.
    ///
    /// Defaults to every bit, which is what an integer return usually means. Set
    /// it where the original leaves a partial-register artifact above the value:
    /// a `u32` return whose callers all `TEST AL, AL` has a one-byte contract, and
    /// the upper bits are whatever happened to be in `EAX`. Comparing those
    /// reports a difference in bits nothing reads, and at a volume that buries
    /// the rest of the log.
    ///
    /// The mask has to be argued from the CALL SITES, not from the body: it is a
    /// claim that no caller reads the masked-out bits, so it is only sound with
    /// the whole caller set enumerated in the entry's comment.
    #[serde(default = "default_ret_mask")]
    ret_mask: u64,
    /// Absolute tolerance per `f32` lane, applied together with `ulp`.
    ///
    /// A lane must exceed BOTH to be reported, so neither ruler alone can hide a
    /// difference. It exists because ULP is the wrong ruler for a lane produced by
    /// cancellation: `x*x*(1 - cos) + cos` at a right angle is a tiny residue of
    /// two near-equal terms, so a fixed absolute error becomes an unbounded ULP
    /// distance as the true value approaches zero. Bounding that in ULP is not
    /// conservative, it is impossible.
    ///
    /// Default 0, which reduces to the plain ULP compare.
    #[serde(default)]
    abs: f32,
    /// Run the ORIGINAL twice and compare it against itself.
    ///
    /// A divergence is only attributable to the reimplementation if the original
    /// is a function of its declared inputs. When it is not — a value rewritten by
    /// another thread between the snapshot and the original's read, then rewritten
    /// back before the drift check can see it — the comparison is measuring the
    /// client against itself and no amount of work on the kernel will quiet it.
    /// This is the probe that tells the two cases apart, and it needs no theory
    /// about which thread.
    ///
    /// **Opt-in, because it runs the original a second time.** Only set it where
    /// the original is argued side-effect free beyond its own output region; the
    /// entry's comment has to carry that argument. The game consumes the second
    /// run's output, which for a deterministic function is the first run's.
    #[serde(default)]
    orig_twice: bool,
    /// `true` marks a function deliberately more precise than the original.
    ///
    /// (f64 intermediates vs x87/SP); divergences count and log at debug, never
    /// warn.
    #[serde(default)]
    expected: bool,
    /// `true` waives the `tools/diff_audit.py` double-run safety checks.
    ///
    /// The waived checks are call-outs via `transmute`, and writes through
    /// pointers other than the `out` arg. The waiver is for adapters whose
    /// call-outs are read-only and whose auxiliary writes are idempotent
    /// (deterministic/constant) — so the differential replay running the reimpl
    /// a second time cannot corrupt live state. A documented per-entry
    /// assertion of safety, like `unsafe`; consumed by `diff_audit.py`, not codegen.
    #[serde(default)]
    double_run_safe: bool,
    /// `true` marks an adapter that runs the original on some inputs.
    ///
    /// A partial reimplementation delegates the shapes it does not model. On
    /// those calls the compare path runs the original on both sides, so the
    /// comparison is vacuous and passes by construction — and nothing at runtime
    /// distinguishes a delegated call from a modelled one. The flag permits the
    /// `original(` the audit otherwise refuses, and makes the arming line say so,
    /// because the trap here is reading a clean result as coverage.
    ///
    /// Only sound when the delegated call cannot corrupt live state on a second
    /// run. The `out` region is redirected to scratch, so the usual case is an
    /// original that reads its receiver and writes only through `out`.
    #[serde(default)]
    delegates: bool,
}

fn default_float() -> String {
    "f32".to_owned()
}

const fn default_ulp() -> u32 {
    4
}

const fn default_ret_mask() -> u64 {
    u64::MAX
}

/// Largest snapshot region the generated diff path will stack-allocate.
const MAX_DIFF_REGION: usize = 1024;

/// Validate a `diff` annotation against its function at build time.
///
/// A malformed table fails the build rather than miscompiling the harness.
fn validate_diff(name: &str, f: &Function) {
    let Some(d) = &f.diff else { return };
    // Parsed from the manifest (the single schema source) but consumed by
    // tools/diff_audit.py, not codegen; reference it so the field stays live.
    let _ = d.double_run_safe;
    assert!(
        matches!(d.mode.as_str(), "out" | "inout"),
        "{name}: diff.mode must be out|inout, got {:?}",
        d.mode,
    );
    assert!(
        matches!(d.float.as_str(), "f32" | "bytes"),
        "{name}: diff.float must be f32|bytes, got {:?}",
        d.float,
    );
    let check_region = |r: &Region, role: &str| {
        assert!(
            r.arg < f.args.len(),
            "{name}: diff {role} arg {} out of range ({} args)",
            r.arg,
            f.args.len(),
        );
        assert!(
            f.args[r.arg].starts_with('*'),
            "{name}: diff {role} arg {} is {:?}, not a pointer",
            r.arg,
            f.args[r.arg],
        );
        assert!(
            r.len > 0 && r.len <= MAX_DIFF_REGION,
            "{name}: diff {role} len {} must be 1..={MAX_DIFF_REGION}",
            r.len,
        );
        if d.float == "f32" {
            assert!(
                r.len.is_multiple_of(4),
                "{name}: diff {role} len {} must be a multiple of 4 for f32 compare",
                r.len,
            );
        }
    };
    assert!(
        d.float_from == 0 || d.float == "f32",
        "{name}: diff float_from is only meaningful with float = \"f32\"",
    );
    if d.float_from != 0 {
        let out = d.out.as_ref().unwrap_or_else(|| {
            panic!("{name}: diff float_from needs an `out` region to offset into")
        });
        assert!(
            d.float_from.is_multiple_of(4) && d.float_from < out.len,
            "{name}: diff float_from {} must be 4-aligned and inside the {}-byte out region",
            d.float_from,
            out.len,
        );
        assert!(
            (out.len - d.float_from).is_multiple_of(4),
            "{name}: diff out region tail from {} is not a whole number of f32 lanes",
            d.float_from,
        );
    }
    // `orig_twice` re-runs the original on the same arguments and expects the same
    // answer. That only holds when the out region is pure output: an in-place
    // function reads it, so a second run would compute from the FIRST run's result
    // and report a difference that means nothing. Rejected rather than documented,
    // because a probe that reports spuriously is worse than no probe.
    assert!(
        !(d.orig_twice && d.mode == "inout"),
        "{name}: diff orig_twice cannot be used with mode = \"inout\" — the second \
         run would read the first run's output as its input",
    );
    assert!(
        !d.orig_twice || d.out.is_some(),
        "{name}: diff orig_twice needs an `out` region to compare the two runs over",
    );
    if d.mode == "inout" {
        let out = d.out.as_ref().unwrap_or_else(|| {
            panic!("{name}: diff mode \"inout\" requires an `out` region (the in-place buffer)")
        });
        assert!(
            f.args[out.arg].starts_with("*mut"),
            "{name}: diff out arg {} must be `*mut` (the reimpl writes it), got {:?}",
            out.arg,
            f.args[out.arg],
        );
        check_region(out, "out");
    } else {
        // "out": an output region, a scalar return, or both. A pure scalar reader
        // (writes nothing, returns f32/int) is diffed on its return alone.
        if let Some(out) = &d.out {
            assert!(
                f.args[out.arg].starts_with("*mut"),
                "{name}: diff out arg {} must be `*mut` (the reimpl writes it), got {:?}",
                out.arg,
                f.args[out.arg],
            );
            check_region(out, "out");
        } else {
            assert!(
                f.ret == "f32" || is_integer_ret(&f.ret),
                "{name}: diff mode \"out\" needs an `out` region or a scalar (f32/int) return, ret is {:?}",
                f.ret,
            );
        }
    }
    for r in &d.ins {
        check_region(r, "ins");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=symbols.toml");
    println!("cargo:rerun-if-changed=build.rs");

    // Diagnostic-only: `WOW_CRUMB=1` makes each generated thunk record a
    // breadcrumb (which hook ran + its output pointer) into the shared mmap ring
    // `crate::crumb` dumps on a crash. Off by default → the `record` calls are
    // const no-ops. Mirrors `windows/d3d9/build.rs`.
    println!("cargo::rustc-check-cfg=cfg(wow_crumb)");
    println!("cargo:rerun-if-env-changed=WOW_CRUMB");
    if std::env::var("WOW_CRUMB").is_ok_and(|v| !v.is_empty() && v != "0") {
        println!("cargo:rustc-cfg=wow_crumb");
    }

    // The differential harness is a COMPILE-TIME opt-in via `WOW_TURBO_DIFF=1`
    // (e.g. `DIFF=1 make install`): each function carrying a `[functions.<key>.diff]`
    // table gets a `*_diff` path that runs the live original beside the reimpl and
    // reports divergences, selected at runtime by `WOW_TURBO_DIFF_ARM`. Off by
    // default → the harness is entirely `#[cfg]`'d out and functions without a
    // `diff` table emit identical thunks either way (zero cost in the shipped DLL).
    println!("cargo::rustc-check-cfg=cfg(wow_turbo_diff)");
    println!("cargo:rerun-if-env-changed=WOW_TURBO_DIFF");
    if std::env::var("WOW_TURBO_DIFF").is_ok_and(|v| !v.is_empty() && v != "0") {
        println!("cargo:rustc-cfg=wow_turbo_diff");
    }

    // The `version coffTimeDateStamp` command answers with a build timestamp.
    // The linker's header field is a reproducibility hash, not a time, so the
    // real build time is baked here instead (fresh per release: the tag watch
    // below re-runs this script whenever the version stamp moves).
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    println!("cargo:rustc-env=WOW_TURBO_BUILD_EPOCH={epoch}");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let toml_path = manifest_dir.join("symbols.toml");
    let text = fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", toml_path.display()));
    let manifest: Manifest =
        toml::from_str(&text).unwrap_or_else(|e| panic!("parse symbols.toml: {e}"));

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let out_path = out_dir.join("symbols.rs");
    let rendered = render(&manifest);
    // Write only on a change. The ship path re-runs this script on every build
    // so the identity above cannot go stale, and rewriting an identical table
    // would make that re-stamp cost a full rebuild of the generated code.
    if fs::read_to_string(&out_path).is_ok_and(|old| old == rendered) {
        return;
    }
    fs::write(&out_path, rendered).unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));

    // Pretty-print the generated file with nightly rustfmt so it reads cleanly
    // when inspected. `--config-path` is pinned to the manifest dir so the repo's
    // `rustfmt.toml` is honored regardless of where `OUT_DIR` lives (a relocated
    // `CARGO_TARGET_DIR` would otherwise defeat rustfmt's upward config search).
    // Best-effort: the file is valid Rust regardless of layout, so a missing
    // toolchain or a rustfmt error never fails the build.
    let _ = std::process::Command::new("rustfmt")
        .args(["+nightly", "--edition", "2024", "--config-path"])
        .arg(&manifest_dir)
        .arg(&out_path)
        .status();
}

/// The caller-saved (volatile) GP registers, in capture order.
///
/// A `preserve` entry may only name one of these.
const VOL_ORDER: [&str; 3] = ["eax", "ecx", "edx"];

fn render(m: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("// @generated from symbols.toml by build.rs — do not edit.\n\n");

    // Last hook RVA recorded as a breadcrumb. The thunks skip recording when the
    // same hook fires consecutively, so a hot hook (e.g. luaH_arrayindex called
    // thousands of times) collapses to one entry and the *distinct* pre-crash
    // hook sequence stays in the ring. Crumb-only.
    out.push_str("#[cfg(wow_crumb)]\n");
    out.push_str(
        "static LAST_HOOK: ::core::sync::atomic::AtomicU32 = \
         ::core::sync::atomic::AtomicU32::new(0);\n\n",
    );

    let mut install_body = String::new();
    // Every hooked symbol, collected for the `originals` typed-accessor module
    // emitted after the loop.
    let mut all_syms: Vec<(String, String)> = Vec::new();

    for (name, f) in &m.functions {
        validate_diff(name, f);
        let snake = to_snake(name);
        let screaming = snake.to_uppercase();
        all_syms.push((name.clone(), snake.clone()));
        if f.abi == "x87st0" {
            emit_x87st0(&mut out, &mut install_body, name, &snake, &screaming, f);
            continue;
        }
        if f.abi == "x87pow" {
            emit_x87pow(&mut out, &mut install_body, name, &snake, &screaming, f);
            continue;
        }
        if f.abi == "tap" {
            emit_tap(&mut out, &mut install_body, name, &snake, &screaming, f);
            continue;
        }
        let abi = abi_str(&f.abi, name);
        // `void` → no return arrow at all (an explicit `-> ()` trips
        // `clippy::unused_unit`); any other type is emitted verbatim. A function
        // that returns an out/this/operand pointer declares that pointer type and
        // its reimpl returns it (so a chaining caller gets the right pointer).
        let ret_arrow = if f.ret == "void" {
            String::new()
        } else {
            format!(" -> {}", f.ret)
        };
        let arg_types = f.args.join(", ");
        let named_params = f
            .args
            .iter()
            .enumerate()
            .map(|(i, ty)| format!("arg{i}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let arg_names = (0..f.args.len())
            .map(|i| format!("arg{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        // Original/trampoline type + its single-write storage.
        let _ = writeln!(
            out,
            "pub type {name}Fn = extern \"{abi}\" fn({arg_types}){ret_arrow};"
        );
        let _ = writeln!(
            out,
            "static {screaming}_ORIGINAL: ::std::sync::OnceLock<{name}Fn> = ::std::sync::OnceLock::new();"
        );

        // Private thunk MinHook installs: pure ABI shim, compile-time dispatch to
        // the reimpl. The reimpl takes only the host arguments — no resolver.
        let call_args = arg_names.clone();
        // The thunk's single call into the reimpl. A hook carrying a `[diff]`
        // table gains a runtime-selectable compare path in an `wow_turbo_diff`
        // build (run the original on the live args, re-run the reimpl on input
        // snapshots, compare the annotated output region and report only on a
        // mismatch); every other hook always calls the reimpl directly. Exactly
        // one cfg arm remains after expansion, so it is the block's tail
        // expression and no path bypasses the surrounding `preserve` shim.
        let plain_call = format!("super::hooks::{snake}({call_args})");
        let dispatch_expr = if f.diff.is_some() {
            let diff_call = if arg_names.is_empty() {
                format!("{snake}_diff()")
            } else {
                format!("{snake}_diff({arg_names})")
            };
            format!(
                "{{\n        #[cfg(wow_turbo_diff)]\n        \
                 {{ if {screaming}_DIFF.load(::core::sync::atomic::Ordering::Relaxed) \
                 {{ {diff_call} }} else {{ {plain_call} }} }}\n        \
                 #[cfg(not(wow_turbo_diff))]\n        {{ {plain_call} }}\n    }}"
            )
        } else {
            plain_call.clone()
        };
        // Breadcrumb (crumb-only; `#[cfg]`'d out otherwise → zero cost). tag = hook
        // RVA, p0 = first (out/this) pointer arg. Consecutive same-hook calls are
        // collapsed via `LAST_HOOK` so hot hooks don't evict rare ones in the ring.
        let crumb_p0 = if f.args.first().is_some_and(|t| t.starts_with('*')) {
            "arg0 as usize as u64"
        } else {
            "0"
        };
        let crumb = format!(
            "    #[cfg(wow_crumb)]\n    if LAST_HOOK.swap({rva:#x}, \
             ::core::sync::atomic::Ordering::Relaxed) != {rva:#x} {{\n        \
             ::wow_shared::crumb::record(\"{rva:x}\", {crumb_p0}, 0);\n    }}\n",
            rva = f.rva
        );

        // The 1.12 client's MSVC keeps caller-saved registers live across calls to
        // these internal functions (ABI-breaking interprocedural register
        // allocation): it sees the leaf never touches a volatile and reuses it
        // across the call — the in-world crash was a caller `fmul`ing through an ECX
        // it expected to survive `Determinant`. Our Rust reimpl uses the volatiles
        // as scratch, so for each register the original leaves intact (the
        // per-function `preserve` set, machine-checked against the binary) the
        // thunk captures the incoming value, calls the reimpl, and restores it
        // before returning. The capture `asm!` is the FIRST
        // statement: the only code the compiler emits before it is the prologue
        // (`push ebp; mov ebp, esp`), which never writes a volatile, so the captured
        // value is the incoming one. Everything else — arg forwarding, the call, the
        // `abi`'s `ret` cleanup — is ordinary Rust the optimizer owns: it inlines the
        // reimpl into the thunk, so there is NO naked frame, NO manual arg re-push,
        // and NO extra call (unlike a naked shim, which forces a non-inlinable
        // `thunk -> inner -> reimpl` chain). `preserve` is independent of `abi`: it
        // can name EAX (the return register ONLY for int/ptr returns — for
        // `void`/float returns EAX is free, so a caller can keep a live value there
        // too), ECX, and/or EDX, but never an arg-carrying register (ECX for
        // thiscall, ECX/EDX for fastcall) nor the return register. EBX/ESI/EDI/EBP
        // are callee-saved by the reimpl's own prologue, and XMM is untouched by the
        // x87 originals. An empty `preserve` emits a plain thunk.
        assert!(
            f.preserve.iter().all(|p| VOL_ORDER.contains(&p.as_str())),
            "{name}: `preserve` may only list eax/ecx/edx, got {:?}",
            f.preserve
        );
        let saves: Vec<&str> = VOL_ORDER
            .into_iter()
            .filter(|&r| f.preserve.iter().any(|p| p.as_str() == r))
            .collect();
        let _ = writeln!(
            out,
            "extern \"{abi}\" fn {snake}_thunk({named_params}){ret_arrow} {{"
        );
        if saves.is_empty() {
            out.push_str(&crumb);
            let _ = writeln!(out, "    {dispatch_expr}");
        } else {
            // Empty register-operand `asm!`s read the incoming value (capture) and
            // write it back (restore). No `pure`: the value depends on register
            // state, so the blocks must not be reordered or elided.
            let cap = saves
                .iter()
                .map(|r| format!("out(\"{r}\") s_{r}"))
                .collect::<Vec<_>>()
                .join(", ");
            let res = saves
                .iter()
                .map(|r| format!("in(\"{r}\") s_{r}"))
                .collect::<Vec<_>>()
                .join(", ");
            for r in &saves {
                let _ = writeln!(out, "    let s_{r}: u32;");
            }
            let _ = writeln!(
                out,
                "    // SAFETY: empty asm capturing the incoming caller-saved {saves:?} that the\n    \
                 // 1.12 caller keeps live across this internal function (see module note)."
            );
            let _ = writeln!(
                out,
                "    unsafe {{ ::core::arch::asm!(\"\", {cap}, options(nomem, nostack, preserves_flags)) }};"
            );
            out.push_str(&crumb);
            if f.ret == "void" {
                let _ = writeln!(out, "    {dispatch_expr};");
            } else {
                let _ = writeln!(out, "    let result = {dispatch_expr};");
            }
            let _ = writeln!(
                out,
                "    // SAFETY: empty asm restoring the captured {saves:?} before returning."
            );
            let _ = writeln!(
                out,
                "    unsafe {{ ::core::arch::asm!(\"\", {res}, options(nomem, nostack, preserves_flags)) }};"
            );
            if f.ret != "void" {
                let _ = writeln!(out, "    result");
            }
        }
        // Both arms emit the thunk body up to its closing brace; close it here.
        let _ = writeln!(out, "}}");
        out.push('\n');

        // Differential-mode machinery for this hook (all `#[cfg(wow_turbo_diff)]`).
        emit_diff(&mut out, name, &snake, &screaming, f);

        // Install step: verify the signature, create the hook over the thunk,
        // store the original, queue the enable (applied in one batch at the end
        // of `install_all`). Address + signature are inlined here.
        //
        // An `armed_only` entry puts that step behind the gauge's own arming
        // check, so an unarmed run never patches the address at all. The target
        // is hot enough that the usual "unarmed cost is one trampoline" argument
        // does not carry, and the gauge resolves its filter before `install_all`.
        let ind = if f.armed_only { "    " } else { "" };
        if f.armed_only {
            let _ = writeln!(
                install_body,
                "    // Observation hook on a path too hot to detour for nothing: stock"
            );
            let _ = writeln!(install_body, "    // unless the event gauge is armed.");
            let _ = writeln!(install_body, "    if super::events::armed() {{");
        }
        let _ = writeln!(
            install_body,
            "{ind}    queued += usize::from(install_thunk("
        );
        let _ = writeln!(install_body, "{ind}        image_base,");
        let _ = writeln!(install_body, "{ind}        {:#010x},", f.rva);
        let _ = writeln!(install_body, "{ind}        {},", f.sig.literal());
        let _ = writeln!(
            install_body,
            "{ind}        {snake}_thunk as *mut ::core::ffi::c_void,"
        );
        let _ = writeln!(install_body, "{ind}        {name:?},");
        let _ = writeln!(install_body, "{ind}        |trampoline| {{");
        let _ = writeln!(
            install_body,
            "{ind}            // SAFETY: the trampoline runs the displaced prologue then continues"
        );
        let _ = writeln!(
            install_body,
            "{ind}            // into the unhooked original, carrying its ABI."
        );
        let _ = writeln!(install_body, "{ind}            let original = unsafe {{");
        let _ = writeln!(
            install_body,
            "{ind}                ::core::mem::transmute::<*mut ::core::ffi::c_void, {name}Fn>(trampoline)"
        );
        let _ = writeln!(install_body, "{ind}            }};");
        let _ = writeln!(
            install_body,
            "{ind}            let _ = {screaming}_ORIGINAL.set(original);"
        );
        let _ = writeln!(install_body, "{ind}        }},");
        let _ = writeln!(install_body, "{ind}    ));");
        if f.armed_only {
            let _ = writeln!(install_body, "    }}");
        }

        // Arm this hook's compare switch if it is selected at runtime. Only a
        // hook with a `[diff]` table has the switch (compare mode runs both
        // sides and reports mismatches); the store happens whether or not the
        // install above succeeded — a skipped hook never runs its thunk, so the
        // flag is simply inert.
        if f.diff.is_some() {
            let _ = writeln!(install_body, "    #[cfg(wow_turbo_diff)]");
            let _ = writeln!(install_body, "    if diff_enabled({name:?}) {{");
            let _ = writeln!(
                install_body,
                "        {screaming}_DIFF.store(true, ::core::sync::atomic::Ordering::Relaxed);"
            );
            let _ = writeln!(
                install_body,
                "        ::log::warn!(target: super::LOG_TARGET, \"[diff] {{}} differential mode ON\", {name:?});"
            );
            let _ = writeln!(install_body, "    }}");
        }
    }

    // Typed accessors for the unhooked original of any hooked function. The
    // reimpls no longer receive an injected resolver (almost none delegate); the
    // rare hook that must call through calls `originals::<snake>()` and invokes the
    // returned `{Name}Fn` directly — no `Original` enum, no `usize`, no transmute.
    out.push_str("/// Typed accessors for the unhooked original of every hooked function.\n");
    out.push_str("///\n");
    out.push_str("/// `originals::<snake>()` returns the matching `{Name}Fn` pointer — the\n");
    out.push_str("/// `MinHook` trampoline. A reimpl that must defer to stock code (e.g. an\n");
    out.push_str("/// unreconstructed branch) calls it and invokes the result directly; the\n");
    out.push_str("/// pointer carries the function's original ABI. Each panics if queried\n");
    out.push_str("/// before `install_all` has published that trampoline.\n");
    out.push_str("pub mod originals {\n");
    for (name, snake) in &all_syms {
        let screaming = snake.to_uppercase();
        let _ = writeln!(out, "    pub fn {snake}() -> super::{name}Fn {{");
        let _ = writeln!(
            out,
            "        super::{screaming}_ORIGINAL.get().copied().expect(\"{name} original missing\")"
        );
        let _ = writeln!(out, "    }}");
    }
    out.push_str("}\n\n");

    // The installer — patches every hooked prologue and publishes the trampolines.
    let param = if install_body.is_empty() {
        "_image_base"
    } else {
        "image_base"
    };
    out.push_str("/// Install every hooked function over the live host image.\n");
    out.push_str("///\n");
    out.push_str("/// Verify and create each hook and publish its trampoline, then apply all\n");
    out.push_str("/// queued enables in a single thread-freeze (one `MH_ApplyQueued` instead of\n");
    out.push_str("/// one freeze per hook). Called once from `DllMain`; failures are logged and\n");
    out.push_str("/// skipped.\n");
    let _ = writeln!(out, "pub fn install_all({param}: usize) {{");
    if !install_body.is_empty() {
        out.push_str("    let started = ::std::time::Instant::now();\n");
        out.push_str("    let mut queued = 0usize;\n");
        out.push_str(&install_body);
        out.push_str(
            "    // SAFETY: every queued hook pairs a sig-verified VA with its generated\n",
        );
        out.push_str("    // ABI-matching thunk; the batch goes live here.\n");
        out.push_str("    if unsafe { ::wow_hook::apply_queued(\"install_all\") } {\n");
        out.push_str("        // The patches are live now, so what the prologues hold is what\n");
        out.push_str("        // this process wrote — the baseline the overwrite check reads.\n");
        out.push_str("        ::wow_hook::snapshot_patches();\n");
        out.push_str("        ::log::info!(\n");
        out.push_str("            target: super::LOG_TARGET,\n");
        out.push_str("            \"install_all: {queued} hooks enabled in {} ms\",\n");
        out.push_str("            started.elapsed().as_millis(),\n");
        out.push_str("        );\n");
        out.push_str("    }\n");
    }
    out.push_str("}\n\n");

    out.push_str(INSTALL_THUNK);
    out.push_str(DIFF_ENABLED);
    out
}

/// Runtime selector for compare mode, appended under `#[cfg(wow_turbo_diff)]`.
///
/// The value is a comma-separated token list: an `all` token arms every
/// `[diff]`-annotated hook, a hook label arms that one. So
/// `WOW_TURBO_DIFF_ARM=all` arms all of them; `Name1,Name2` arms those two. This is
/// a runtime `WOW_TURBO_*` var: the launcher filters the PE environment down to an
/// allowlist, so it only reaches the game when the allowlist forwards it (as for
/// `WOW_TURBO_SKIP`).
const DIFF_ENABLED: &str = r#"
#[cfg(wow_turbo_diff)]
fn diff_enabled(label: &str) -> bool {
    use ::std::sync::OnceLock;
    // `WOW_TURBO_DIFF_ARM` is a launch-time selector, so parse it ONCE and cache
    // the token list — this runs on every armed hook, some of them hot, and must
    // stay off the getenv/alloc path. Unset ⇒ `None` ⇒ every hook stays dormant.
    static SEL: OnceLock<Option<Vec<String>>> = OnceLock::new();
    SEL.get_or_init(|| {
        ::std::env::var("WOW_TURBO_DIFF_ARM")
            .ok()
            .map(|s| s.split(',').map(|x| x.trim().to_owned()).collect())
    })
    .as_ref()
    .is_some_and(|tokens| tokens.iter().any(|t| t == "all" || t == label))
}
"#;

/// Emit the `#[cfg(wow_turbo_diff)]` compare machinery for one hook.
///
/// Its runtime switch, its per-hook reporting counter, and the `{snake}_diff`
/// function the thunk routes to when armed. Only a hook carrying a `[diff]`
/// `out`/`inout` table gets this — it runs the original on the live args,
/// re-runs the reimpl on input snapshots, and reports only when the annotated
/// output region diverges. Hooks without a table emit nothing here (and their
/// thunk never references a diff path), so an armed run is silent unless a
/// reimpl disagrees.
fn emit_diff(out: &mut String, name: &str, snake: &str, screaming: &str, f: &Function) {
    let Some(d) = f.diff.as_ref() else {
        return;
    };

    let ret_arrow = if f.ret == "void" {
        String::new()
    } else {
        format!(" -> {}", f.ret)
    };
    let named_params = f
        .args
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("arg{i}: {ty}"))
        .collect::<Vec<_>>()
        .join(", ");

    let _ = writeln!(out, "#[cfg(wow_turbo_diff)]");
    let _ = writeln!(
        out,
        "static {screaming}_DIFF: ::core::sync::atomic::AtomicBool = \
         ::core::sync::atomic::AtomicBool::new(false);"
    );
    let _ = writeln!(out, "#[cfg(wow_turbo_diff)]");
    let _ = writeln!(
        out,
        "static {screaming}_DIFF_STATS: super::diff::Stats = super::diff::Stats::new();"
    );
    let _ = writeln!(out, "#[cfg(wow_turbo_diff)]");
    let _ = writeln!(
        out,
        "static {screaming}_DIFF_ARMED: ::core::sync::atomic::AtomicBool = \
         ::core::sync::atomic::AtomicBool::new(false);"
    );
    let _ = writeln!(out, "#[cfg(wow_turbo_diff)]");
    let _ = writeln!(out, "fn {snake}_diff({named_params}){ret_arrow} {{");
    // First armed call logs `[diff] armed: <name>` so an armed run is visible even
    // when nothing diverges (distinguishes faithful from never-ran / not-armed).
    let _ = writeln!(
        out,
        "    super::diff::note_armed(&{screaming}_DIFF_ARMED, \"{name}\", {});",
        d.delegates
    );
    emit_diff_compare(out, name, snake, screaming, f, d);
    let _ = writeln!(out, "}}");
    out.push('\n');
}

/// `out`/`inout` body.
///
/// Snapshot inputs + the pre-call out region, run the original on the live
/// arguments (the game keeps that result), re-run the reimpl against the
/// snapshots, and compare the two outputs.
fn emit_diff_compare(
    out: &mut String,
    name: &str,
    snake: &str,
    screaming: &str,
    f: &Function,
    d: &Diff,
) {
    // Pointers that must be non-null for a meaningful comparison: the out region
    // (if any) plus every snapshotted input. A null among them ⇒ skip diffing
    // this call and just run the reimpl as the thunk normally would.
    let mut guarded: Vec<usize> = d
        .out
        .iter()
        .map(|r| r.arg)
        .chain(d.ins.iter().map(|r| r.arg))
        .collect();
    guarded.sort_unstable();
    guarded.dedup();
    let guard = guarded
        .iter()
        .map(|i| format!("arg{i}.is_null()"))
        .collect::<Vec<_>>()
        .join(" || ");
    let plain_args = (0..f.args.len())
        .map(|i| format!("arg{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    // A null among the guarded pointers ⇒ skip diffing and run the reimpl as the
    // thunk normally would. Omitted when nothing pointer-typed is guarded.
    if !guard.is_empty() {
        let _ = writeln!(out, "    if {guard} {{");
        if f.ret == "void" {
            let _ = writeln!(out, "        super::hooks::{snake}({plain_args});");
            let _ = writeln!(out, "        return;");
        } else {
            let _ = writeln!(out, "        return super::hooks::{snake}({plain_args});");
        }
        let _ = writeln!(out, "    }}");
    }

    // Snapshot each input region. `arg{i} as *const u8` reinterprets the typed
    // pointer as bytes; the copy is one unsafe op per block.
    for r in &d.ins {
        let _ = writeln!(
            out,
            "    let mut snap{} = super::diff::Buf::<{}>::zeroed();",
            r.arg, r.len
        );
        let _ = writeln!(
            out,
            "    // SAFETY: arg{0} is non-null (guarded above) and addresses at least {1} bytes\n    \
             // of the live input (manifest diff.ins region).",
            r.arg, r.len
        );
        let _ = writeln!(
            out,
            "    unsafe {{ ::core::ptr::copy_nonoverlapping(arg{0} as *const u8, snap{0}.0.as_mut_ptr(), {1}) }};",
            r.arg, r.len
        );
    }
    // Seed the scratch out buffer from the pre-call out region so lanes the
    // function legitimately never writes compare equal, and an in-place op sees
    // its prior state.
    if let Some(region_out) = &d.out {
        let _ = writeln!(
            out,
            "    let mut scratch = super::diff::Buf::<{}>::zeroed();",
            region_out.len
        );
        let _ = writeln!(
            out,
            "    // SAFETY: arg{0} is non-null (guarded above) and addresses at least {1} bytes\n    \
             // of the live output (manifest diff.out region).",
            region_out.arg, region_out.len
        );
        let _ = writeln!(
            out,
            "    unsafe {{ ::core::ptr::copy_nonoverlapping(arg{0} as *const u8, scratch.0.as_mut_ptr(), {1}) }};",
            region_out.arg, region_out.len
        );
    }

    // Run the original on the live arguments — the game consumes this result.
    let orig_call_args = (0..f.args.len())
        .map(|i| format!("arg{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let bind_orig = if f.ret == "void" {
        ""
    } else {
        "let orig_ret = "
    };
    let _ = writeln!(
        out,
        "    {bind_orig}self::originals::{snake}()({orig_call_args});"
    );

    // Determinism probe: capture the first run's output, run the original again on
    // the same arguments, and report if the two disagree.
    if d.orig_twice
        && let Some(region_out) = &d.out
    {
        {
            let (arg, len) = (region_out.arg, region_out.len);
            let _ = writeln!(
                out,
                "    let mut first_out = super::diff::Buf::<{len}>::zeroed();"
            );
            let _ = writeln!(
                out,
                "    // SAFETY: arg{arg} is non-null and addresses {len} bytes (manifest diff.out region)."
            );
            let _ = writeln!(
                out,
                "    unsafe {{ ::core::ptr::copy_nonoverlapping(arg{arg} as *const u8, first_out.0.as_mut_ptr(), {len}) }};"
            );
            let _ = writeln!(out, "    self::originals::{snake}()({orig_call_args});");
            let _ = writeln!(
                out,
                "    // SAFETY: arg{arg} still addresses the same {len} writable bytes."
            );
            let _ = writeln!(
                out,
                "    super::diff::note_nondeterministic(&{screaming}_DIFF_STATS, {name:?}, {arg}, &first_out.0, unsafe {{ ::core::slice::from_raw_parts(arg{arg} as *const u8, {len}) }});"
            );
        }
    }

    // Run the reimpl against the snapshots: out → scratch, each annotated input →
    // its snapshot, everything else passed through live (read-only by contract).
    let in_args: std::collections::BTreeSet<usize> = d.ins.iter().map(|r| r.arg).collect();
    let out_arg = d.out.as_ref().map(|r| r.arg);
    let reimpl_args = (0..f.args.len())
        .map(|i| {
            let ty = &f.args[i];
            // `as_mut_ptr()` is already `*mut u8`; a same-type `as` cast trips
            // `clippy::unnecessary_cast` in DIFF builds, so pointee-cast instead.
            let pointee = ty.trim_start_matches("*mut ").trim_start_matches("*const ");
            if Some(i) == out_arg {
                format!("scratch.0.as_mut_ptr().cast::<{pointee}>()")
            } else if in_args.contains(&i) {
                format!("snap{i}.0.as_mut_ptr().cast::<{pointee}>()")
            } else {
                format!("arg{i}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let bind_reimpl = if f.ret == "void" {
        ""
    } else {
        "let reimpl_ret = "
    };
    let _ = writeln!(
        out,
        "    {bind_reimpl}super::hooks::{snake}({reimpl_args});"
    );

    // Before trusting any comparison, check each snapshotted input still holds
    // what was snapshotted. If it moved while the original ran, the two sides
    // were handed different inputs and the divergence says nothing about the
    // reimplementation.
    for region_in in &d.ins {
        let _ = writeln!(
            out,
            "    // SAFETY: arg{0} is non-null and addresses {1} bytes (manifest diff.ins region).",
            region_in.arg, region_in.len
        );
        let _ = writeln!(
            out,
            "    super::diff::note_input_drift(&{screaming}_DIFF_STATS, {name:?}, {0}, &snap{0}.0, \
             unsafe {{ ::core::slice::from_raw_parts(arg{0} as *const u8, {1}) }});",
            region_in.arg, region_in.len
        );
    }

    // Compare the scratch (ours) against the real out region (now the original's).
    if let Some(region_out) = &d.out {
        let _ = writeln!(
            out,
            "    // SAFETY: arg{0} is non-null and addresses {1} bytes (manifest diff.out region).",
            region_out.arg, region_out.len
        );
        let _ = writeln!(
            out,
            "    let orig_out = unsafe {{ ::core::slice::from_raw_parts(arg{0} as *const u8, {1}) }};",
            region_out.arg, region_out.len
        );
        if d.float == "bytes" {
            let _ = writeln!(
                out,
                "    super::diff::region_bytes(&{screaming}_DIFF_STATS, {name:?}, {}, &scratch.0, orig_out);",
                d.expected
            );
        } else if d.float_from != 0 {
            let _ = writeln!(
                out,
                "    super::diff::region_split(&{screaming}_DIFF_STATS, {name:?}, {}, &super::diff::Tolerance {{ ulp: {}, abs: {:?} }}, {}, &scratch.0, orig_out);",
                d.expected, d.ulp, d.abs, d.float_from
            );
        } else {
            let _ = writeln!(
                out,
                "    super::diff::region_f32(&{screaming}_DIFF_STATS, {name:?}, {}, &super::diff::Tolerance {{ ulp: {}, abs: {:?} }}, &scratch.0, orig_out);",
                d.expected, d.ulp, d.abs
            );
        }
        // After the compare, so the dump describes the divergence just reported.
        // Snapshot regions only: a live pointer read here would be the post-call
        // state, which is exactly what the reimpl did NOT see.
        let ins_list = d
            .ins
            .iter()
            .map(|r| format!("({0}, &snap{0}.0[..])", r.arg))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "    super::diff::dump_case(&{screaming}_DIFF_STATS, {name:?}, {}, &[{ins_list}], &scratch.0, orig_out);",
            d.expected
        );
    }

    // Compare scalar returns; pointer/void returns carry no value to compare.
    emit_ret_compare(out, name, screaming, f, d);

    if f.ret != "void" {
        let _ = writeln!(out, "    orig_ret");
    }
}

/// Emit the scalar-return comparison and the function's tail.
///
/// `f32` returns compare within `ulp`; integer returns compare exactly (widened
/// to `u64`); pointer and `void` returns have nothing to compare.
fn emit_ret_compare(out: &mut String, name: &str, screaming: &str, f: &Function, d: &Diff) {
    if f.ret == "f32" {
        let _ = writeln!(
            out,
            "    super::diff::scalar_f32(&{screaming}_DIFF_STATS, {name:?}, {}, {}, reimpl_ret, orig_ret);",
            d.expected, d.ulp
        );
    } else if is_integer_ret(&f.ret) {
        let _ = writeln!(
            out,
            "    super::diff::scalar_int(&{screaming}_DIFF_STATS, {name:?}, {}, {:#x}, reimpl_ret as u64, orig_ret as u64);",
            d.expected, d.ret_mask
        );
    } else if f.ret != "void" {
        // Pointer return — it is the out pointer; nothing to compare. Silence the
        // unused binding for non-pointer-non-scalar exotic returns too.
        let _ = writeln!(out, "    let _ = &reimpl_ret;");
    }
}

/// Whether a manifest return type is an integer the diff path can widen to `u64`.
///
/// For an exact comparison.
fn is_integer_ret(ret: &str) -> bool {
    matches!(
        ret,
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" | "bool"
    )
}

/// The fixed `install_thunk` helper appended to the generated module.
///
/// Refuse unless the bytes at the target match one of the recorded signatures,
/// then create the hook over the generated thunk, let the caller publish the
/// original (the `store` closure runs before enabling, so the lazy resolver can
/// never see an empty slot), then queue the enable — `install_all` applies the
/// whole queue in one thread-freeze at the end.
const INSTALL_THUNK: &str = r#"
/// Install one hook; returns whether it was queued for enabling.
///
/// Patching is refused unless the bytes at the target match one of the recorded
/// `sigs` — a `0` RVA, no matching signature, or a `MinHook` failure logs and
/// skips. A hook must never crash the host, and must never patch an unverified
/// address. Most entries list one pattern; an entry lists several when its
/// reimplementation stands in for more than one known prologue at that address.
/// The enable is only queued; `install_all`'s single `apply_queued` makes the
/// batch live.
fn install_thunk(
    image_base: usize,
    rva: usize,
    sigs: &[&str],
    thunk: *mut ::core::ffi::c_void,
    label: &str,
    store: impl FnOnce(*mut ::core::ffi::c_void),
) -> bool {
    if rva == 0 {
        ::log::warn!(
            target: super::LOG_TARGET,
            "{label} rva unset in symbols.toml — skipping",
        );
        return false;
    }
    // Debug bisection: `WOW_TURBO_SKIP=all` installs nothing; a comma-separated
    // list skips those labels. Lets a crash be narrowed to a single hook without
    // a rebuild per step. This is a runtime `WOW_TURBO_*` var: the launcher filters the
    // environment passed to the PE down to an allowlist, so it only reaches the game
    // when the allowlist forwards it.
    if ::std::env::var("WOW_TURBO_SKIP")
        .is_ok_and(|s| s == "all" || s.split(',').any(|x| x.trim() == label))
    {
        ::log::warn!(target: super::LOG_TARGET, "{label} skipped (WOW_TURBO_SKIP)");
        return false;
    }
    let va = image_base + rva;
    // SAFETY: `va` is the live image base plus the function's manifest RVA, which
    // lies within the host image's mapped code.
    if !sigs.iter().any(|sig| unsafe { ::wow_hook::signature_matches(va, sig) }) {
        ::log::warn!(
            target: super::LOG_TARGET,
            "{label} signature mismatch at {va:#010x} ({}) — refusing to patch",
            ::wow_hook::prologue_owner(va),
        );
        return false;
    }
    // SAFETY: `va`'s prologue matched the recorded signature, and `thunk`'s
    // generated `extern "abi"` matches the manifest ABI.
    let Some(trampoline) = (unsafe { ::wow_hook::create_hook(va, thunk, label) }) else {
        return false;
    };
    store(trampoline);
    // SAFETY: `va` is the hook just created above.
    let queued = unsafe { ::wow_hook::queue_enable_hook(va, label) };
    if queued {
        // Registered now, snapshotted after the batch is applied: a prologue
        // another module rewrites later would leave this hook silently dead.
        ::wow_hook::watch_patch(va, label);
    }
    queued
}
"#;

/// Map a manifest ABI name to a Rust `extern` ABI string.
///
/// The generated thunks are only ever compiled for the 32-bit `i686` target
/// (the `win` module is `#[cfg(target_arch = "x86")]`), where every one of these
/// conventions — including `thiscall` (`this` in `ecx`, the remaining args on a
/// callee-cleaned stack) — lowers correctly.
fn abi_str<'a>(abi: &'a str, name: &str) -> &'a str {
    match abi {
        "cdecl" | "stdcall" | "fastcall" | "thiscall" => abi,
        other => panic!("`{name}`: unknown abi `{other}` in symbols.toml"),
    }
}

/// Emit the hook plumbing for an `abi = "x87st0"` entry.
///
/// An MSVC CRT helper that takes its argument in `ST(0)` and returns in
/// `EDX:EAX` — a register contract none of the named conventions can express,
/// so the thunk is a naked shim that spills `ST(0)` to the stack (popping it,
/// as the original does) and calls an ordinary `extern "C" fn(f64) -> i64`
/// wrapper around the reimpl. Constraints: exactly one `f64` arg, an `i64`
/// return, `preserve` limited to ECX (EAX/EDX carry the return), and no
/// differential harness (capturing would need to re-materialize `ST(0)`; these
/// entries are skipped by the `WOW_TURBO_DIFF_ARM` selector). Crumb builds skip
/// recording here for the same reason — the shim has no Rust body to host the
/// cfg'd call.
fn emit_x87st0(
    out: &mut String,
    install_body: &mut String,
    name: &str,
    snake: &str,
    screaming: &str,
    f: &Function,
) {
    assert!(
        f.args == ["f64"] && f.ret == "i64",
        "{name}: x87st0 supports exactly (f64 in ST0) -> i64, got {:?} -> {}",
        f.args,
        f.ret
    );
    assert!(
        f.preserve.iter().all(|p| p == "ecx"),
        "{name}: x87st0 may only preserve ecx (eax/edx carry the return), got {:?}",
        f.preserve
    );
    assert!(
        f.diff.is_none(),
        "{name}: x87st0 entries cannot carry a [diff] table"
    );

    // Opaque trampoline handle: the original is not callable through a Rust fn
    // type (its argument lives in ST(0)); stored only so the `originals` accessor
    // contract matches every other hook.
    let _ = writeln!(out, "pub type {name}Fn = unsafe extern \"C\" fn();");
    let _ = writeln!(
        out,
        "static {screaming}_ORIGINAL: ::std::sync::OnceLock<{name}Fn> = ::std::sync::OnceLock::new();"
    );

    // Ordinary-ABI inner the asm shim calls: its one f64 argument is the
    // spilled ST(0) already sitting on top of the stack at the `call`.
    let _ = writeln!(
        out,
        "extern \"C\" fn {snake}_st0_inner(arg0: f64) -> i64 {{"
    );
    let _ = writeln!(out, "    super::hooks::{snake}(arg0)");
    let _ = writeln!(out, "}}");

    let preserve_ecx = f.preserve.iter().any(|p| p == "ecx");
    let push = if preserve_ecx {
        "\"push ecx\",\n        "
    } else {
        ""
    };
    let pop = if preserve_ecx {
        "\"pop ecx\",\n        "
    } else {
        ""
    };
    let _ = writeln!(out, "#[unsafe(naked)]");
    let _ = writeln!(out, "extern \"C\" fn {snake}_thunk() {{");
    let _ = writeln!(
        out,
        "    // SAFETY: replicates the original's register contract: the f64 argument\n    \
         // arrives in ST(0) and is popped to the stack slot the cdecl inner reads as\n    \
         // its argument; the inner's EDX:EAX return flows through unchanged; the\n    \
         // callee-saved push/pop pair keeps ECX intact for callers that keep it live\n    \
         // across the call (the original never writes it)."
    );
    let _ = writeln!(
        out,
        "    ::core::arch::naked_asm!(\n        \
         {push}\"sub esp, 8\",\n        \
         \"fstp qword ptr [esp]\",\n        \
         \"call {{inner}}\",\n        \
         \"add esp, 8\",\n        \
         {pop}\"ret\",\n        \
         inner = sym {snake}_st0_inner,\n    \
         )"
    );
    let _ = writeln!(out, "}}");
    out.push('\n');

    // Install step: identical to the named-convention path, minus the
    // differential-mode arming (x87st0 entries have no diff machinery).
    let _ = writeln!(install_body, "    queued += usize::from(install_thunk(");
    let _ = writeln!(install_body, "        image_base,");
    let _ = writeln!(install_body, "        {:#010x},", f.rva);
    let _ = writeln!(install_body, "        {},", f.sig.literal());
    let _ = writeln!(
        install_body,
        "        {snake}_thunk as *mut ::core::ffi::c_void,"
    );
    let _ = writeln!(install_body, "        {name:?},");
    let _ = writeln!(install_body, "        |trampoline| {{");
    let _ = writeln!(
        install_body,
        "            // SAFETY: the trampoline runs the displaced prologue then continues"
    );
    let _ = writeln!(
        install_body,
        "            // into the unhooked original, carrying its ABI."
    );
    let _ = writeln!(install_body, "            let original = unsafe {{");
    let _ = writeln!(
        install_body,
        "                ::core::mem::transmute::<*mut ::core::ffi::c_void, {name}Fn>(trampoline)"
    );
    let _ = writeln!(install_body, "            }};");
    let _ = writeln!(
        install_body,
        "            let _ = {screaming}_ORIGINAL.set(original);"
    );
    let _ = writeln!(install_body, "        }},");
    let _ = writeln!(install_body, "    ));");
}

/// Emit the hook plumbing for an `abi = "x87pow"` entry, the CRT `_CIpow` intrinsic.
///
/// `_CIpow`'s two `f64` arguments arrive on the x87 register stack (`ST(1)` =
/// base, `ST(0)` = exponent) and its result returns in `ST(0)` — a register
/// contract none of the named conventions can express. The thunk is a naked
/// shim that spills BOTH arguments to the stack (popping each; the original
/// thunk's `FST` leaves the exponent live in `ST(0)` for its core to consume,
/// so a replacement that pops neither — or one — leaks x87 registers until the
/// 8-slot stack jams and every load yields the indefinite `QNaN`) and calls an
/// ordinary `extern "C" fn(f64, f64) -> f64` wrapper around the reimpl, whose
/// `f64` return lands in `ST(0)` exactly as the original leaves it.
/// Constraints mirror `x87st0`: exactly two `f64` args, an `f64` return, no
/// `preserve` (the original clobbers EAX/ECX/EDX), no differential harness,
/// and no crumb recording (the shim has no Rust body to host the cfg'd call).
fn emit_x87pow(
    out: &mut String,
    install_body: &mut String,
    name: &str,
    snake: &str,
    screaming: &str,
    f: &Function,
) {
    assert!(
        f.args == ["f64", "f64"] && f.ret == "f64",
        "{name}: x87pow supports exactly (f64 in ST1, f64 in ST0) -> f64, got {:?} -> {}",
        f.args,
        f.ret
    );
    assert!(
        f.preserve.is_empty(),
        "{name}: x87pow cannot preserve registers (the original clobbers eax/ecx/edx), got {:?}",
        f.preserve
    );
    assert!(
        f.diff.is_none(),
        "{name}: x87pow entries cannot carry a [diff] table"
    );

    // Opaque trampoline handle: the original is not callable through a Rust fn
    // type (its arguments live in x87 registers); stored only so the `originals`
    // accessor contract matches every other hook.
    let _ = writeln!(out, "pub type {name}Fn = unsafe extern \"C\" fn();");
    let _ = writeln!(
        out,
        "static {screaming}_ORIGINAL: ::std::sync::OnceLock<{name}Fn> = ::std::sync::OnceLock::new();"
    );

    // Ordinary-ABI inner the asm shim calls: its two f64 arguments are the
    // spilled ST(1)/ST(0) already sitting on the stack at the `call`.
    let _ = writeln!(
        out,
        "extern \"C\" fn {snake}_x87pow_inner(arg0: f64, arg1: f64) -> f64 {{"
    );
    let _ = writeln!(out, "    super::hooks::{snake}(arg0, arg1)");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "#[unsafe(naked)]");
    let _ = writeln!(out, "extern \"C\" fn {snake}_thunk() {{");
    let _ = writeln!(
        out,
        "    // SAFETY: replicates the original's register contract: the two f64\n    \
         // arguments arrive in ST(1) (base) and ST(0) (exponent) and are popped to\n    \
         // the stack slots the cdecl inner reads as its arguments — both popped,\n    \
         // matching the original's net effect (it consumes both and leaves only the\n    \
         // result), so the x87 stack depth is balanced at +1 on return; the inner's\n    \
         // f64 return arrives in ST(0) and flows through unchanged."
    );
    let _ = writeln!(
        out,
        "    ::core::arch::naked_asm!(\n        \
         \"sub esp, 16\",\n        \
         \"fxch\",\n        \
         \"fstp qword ptr [esp]\",\n        \
         \"fstp qword ptr [esp + 8]\",\n        \
         \"call {{inner}}\",\n        \
         \"add esp, 16\",\n        \
         \"ret\",\n        \
         inner = sym {snake}_x87pow_inner,\n    \
         )"
    );
    let _ = writeln!(out, "}}");
    out.push('\n');

    // Install step: identical to the named-convention path, minus the
    // differential-mode arming (x87pow entries have no diff machinery).
    let _ = writeln!(install_body, "    queued += usize::from(install_thunk(");
    let _ = writeln!(install_body, "        image_base,");
    let _ = writeln!(install_body, "        {:#010x},", f.rva);
    let _ = writeln!(install_body, "        {},", f.sig.literal());
    let _ = writeln!(
        install_body,
        "        {snake}_thunk as *mut ::core::ffi::c_void,"
    );
    let _ = writeln!(install_body, "        {name:?},");
    let _ = writeln!(install_body, "        |trampoline| {{");
    let _ = writeln!(
        install_body,
        "            // SAFETY: the trampoline runs the displaced prologue then continues"
    );
    let _ = writeln!(
        install_body,
        "            // into the unhooked original, carrying its ABI."
    );
    let _ = writeln!(install_body, "            let original = unsafe {{");
    let _ = writeln!(
        install_body,
        "                ::core::mem::transmute::<*mut ::core::ffi::c_void, {name}Fn>(trampoline)"
    );
    let _ = writeln!(install_body, "            }};");
    let _ = writeln!(
        install_body,
        "            let _ = {screaming}_ORIGINAL.set(original);"
    );
    let _ = writeln!(install_body, "        }},");
    let _ = writeln!(install_body, "    ));");
}

/// Emit the hook plumbing for an `abi = "tap"` entry.
///
/// An observation shim for a variadic `cdecl` function no named convention can
/// express. The naked thunk hands the adapter a pointer to the caller's
/// argument area, then tail-jumps into the original through the trampoline —
/// the original ALWAYS runs, and the stack, return address and argument area
/// reach it untouched, so the shim is transparent to both sides. The adapter
/// observes the leading (fixed) arguments and must not mutate them.
/// Constraints: exactly one `*const u32` arg (the argument-area base), a
/// `void` return, no `preserve` (the shim saves/restores `ecx`/`edx` around
/// the adapter call itself; `eax` carries no state into a function entry),
/// and no differential harness (nothing is replaced, so there is nothing to
/// compare). Crumb builds skip recording here like the x87 shims — the thunk
/// has no generated Rust body to host the cfg'd call.
fn emit_tap(
    out: &mut String,
    install_body: &mut String,
    name: &str,
    snake: &str,
    screaming: &str,
    f: &Function,
) {
    assert!(
        f.args == ["*const u32"] && f.ret == "void",
        "{name}: tap supports exactly (*const u32) -> void, got {:?} -> {}",
        f.args,
        f.ret
    );
    assert!(
        f.preserve.is_empty(),
        "{name}: tap handles register preservation itself, got {:?}",
        f.preserve
    );
    assert!(
        f.diff.is_none(),
        "{name}: tap entries cannot carry a [diff] table"
    );

    // Opaque trampoline handle for the `originals` accessor contract, plus the
    // raw cell the naked shim tail-jumps through (published before enable, so
    // the shim can never read it unset).
    let _ = writeln!(out, "pub type {name}Fn = unsafe extern \"C\" fn();");
    let _ = writeln!(
        out,
        "static {screaming}_ORIGINAL: ::std::sync::OnceLock<{name}Fn> = ::std::sync::OnceLock::new();"
    );
    let _ = writeln!(
        out,
        "static {screaming}_TRAMPOLINE: ::core::sync::atomic::AtomicUsize = \
         ::core::sync::atomic::AtomicUsize::new(0);"
    );

    // Ordinary-ABI inner the asm shim calls: its one argument is the address of
    // the hooked call's first stack argument.
    let _ = writeln!(
        out,
        "extern \"C\" fn {snake}_tap_inner(args: *const u32) {{"
    );
    let _ = writeln!(out, "    super::hooks::{snake}(args);");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "#[unsafe(naked)]");
    let _ = writeln!(out, "extern \"C\" fn {snake}_thunk() {{");
    let _ = writeln!(
        out,
        "    // SAFETY: at entry ESP points at the hooked call's return address, so\n    \
         // ESP+4 is the first stack argument; after the two saves that address is\n    \
         // ESP+12, passed to the cdecl inner and popped again. ECX/EDX are restored\n    \
         // so even a caller keeping them live across the call sees them intact, and\n    \
         // the tail-jump reaches the trampoline with the original stack layout —\n    \
         // the variadic tail is never copied, only left in place."
    );
    let _ = writeln!(
        out,
        "    ::core::arch::naked_asm!(\n        \
         \"push ecx\",\n        \
         \"push edx\",\n        \
         \"lea eax, [esp + 12]\",\n        \
         \"push eax\",\n        \
         \"call {{inner}}\",\n        \
         \"add esp, 4\",\n        \
         \"pop edx\",\n        \
         \"pop ecx\",\n        \
         \"jmp dword ptr [{{tramp}}]\",\n        \
         inner = sym {snake}_tap_inner,\n        \
         tramp = sym {screaming}_TRAMPOLINE,\n    \
         )"
    );
    let _ = writeln!(out, "}}");
    out.push('\n');

    // Install step: like the named-convention path, plus publishing the raw
    // trampoline address the naked shim jumps through.
    let _ = writeln!(install_body, "    queued += usize::from(install_thunk(");
    let _ = writeln!(install_body, "        image_base,");
    let _ = writeln!(install_body, "        {:#010x},", f.rva);
    let _ = writeln!(install_body, "        {},", f.sig.literal());
    let _ = writeln!(
        install_body,
        "        {snake}_thunk as *mut ::core::ffi::c_void,"
    );
    let _ = writeln!(install_body, "        {name:?},");
    let _ = writeln!(install_body, "        |trampoline| {{");
    let _ = writeln!(
        install_body,
        "            {screaming}_TRAMPOLINE.store(trampoline as usize, ::core::sync::atomic::Ordering::Release);"
    );
    let _ = writeln!(
        install_body,
        "            // SAFETY: the trampoline runs the displaced prologue then continues"
    );
    let _ = writeln!(
        install_body,
        "            // into the unhooked original, carrying its ABI."
    );
    let _ = writeln!(install_body, "            let original = unsafe {{");
    let _ = writeln!(
        install_body,
        "                ::core::mem::transmute::<*mut ::core::ffi::c_void, {name}Fn>(trampoline)"
    );
    let _ = writeln!(install_body, "            }};");
    let _ = writeln!(
        install_body,
        "            let _ = {screaming}_ORIGINAL.set(original);"
    );
    let _ = writeln!(install_body, "        }},");
    let _ = writeln!(install_body, "    ));");
}

/// Convert a `PascalCase`/`camelCase` manifest key to `snake_case`.
///
/// Preserving existing underscores (the `::` → `__` flattening of C++ names).
fn to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            out.push('_');
            prev = Some('_');
            continue;
        }
        if c.is_ascii_uppercase() {
            let next = chars.peek().copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

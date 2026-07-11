use std::{env, path::PathBuf, process::Command};

const SWIFT_SOURCE: &str = "swift/translate.swift";
const LIB_NAME: &str = "wow_translate_sys";
const DEPLOYMENT_TARGET: &str = "15.0";

fn main() {
    println!("cargo:rerun-if-changed={SWIFT_SOURCE}");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let target = env::var("TARGET").expect("TARGET set by cargo");
    let archive_path = out_dir.join(format!("lib{LIB_NAME}.a"));

    let swift_target = swift_target_triple(&target);

    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-emit-library",
        "-static",
        "-parse-as-library",
        "-O",
        "-target",
        &swift_target,
        "-module-name",
        LIB_NAME,
        "-o",
    ])
    .arg(&archive_path)
    .arg(SWIFT_SOURCE);

    let status = cmd
        .status()
        .expect("failed to run swiftc — install Xcode command-line tools");
    assert!(status.success(), "swiftc failed: {status}");

    let lib_dir = archive_path
        .parent()
        .expect("OUT_DIR has a parent")
        .to_string_lossy()
        .into_owned();
    println!("cargo:rustc-link-search=native={lib_dir}");
    println!("cargo:rustc-link-lib=static={LIB_NAME}");
    println!("cargo:rustc-link-lib=framework=Translation");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=SwiftUI");
    println!("cargo:rustc-link-lib=framework=AppKit");
}

fn swift_target_triple(rust_target: &str) -> String {
    // Map cargo's `*-apple-darwin` to swiftc's `*-apple-macosX.Y` form so the
    // emitted archive carries the deployment-target version min we want. We
    // only ship two unix targets (`x86_64-apple-darwin`, `aarch64-apple-darwin`).
    //
    // `MACOSX_DEPLOYMENT_TARGET` is set workspace-wide by
    // `unix/.cargo/config.toml`. Honor it so the embedded Swift archive
    // matches the surrounding dylib's load commands. `DEPLOYMENT_TARGET`
    // is only a fallback for direct `cargo` invocations from outside the
    // workspace (which wouldn't pick up the `[env]` block).
    let arch = rust_target
        .split('-')
        .next()
        .expect("non-empty target triple");
    let deployment_target =
        env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| DEPLOYMENT_TARGET.to_string());
    format!("{arch}-apple-macos{deployment_target}")
}

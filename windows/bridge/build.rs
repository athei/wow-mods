fn main() {
    let target = std::env::var("TARGET").unwrap();
    if !target.contains("windows") {
        return;
    }

    let wine_arch = if target.contains("x86_64") {
        "x86_64-windows"
    } else {
        "i386-windows"
    };

    let wine_sdk = std::env::var("WINE_SDK").expect("WINE_SDK must be set");
    let lib_dir = format!("{wine_sdk}/lib/wine/{wine_arch}");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // Extract just unix_lib.o from winecrt0.a - avoids TLS symbol conflicts
    // with MSVC's CRT (both define __tls_index, __tls_start, etc.). This object
    // publishes __wine_init_unix_call / __wine_unix_call_dispatcher /
    // __wine_unixlib_handle so this builtin pairs its companion wow_mods.so.
    let output = std::process::Command::new("/opt/homebrew/opt/llvm/bin/llvm-ar")
        .args([
            "p",
            &format!("{lib_dir}/libwinecrt0.a"),
            &format!("libs/winecrt0/{wine_arch}/unix_lib.o"),
        ])
        .output()
        .expect("ar failed");
    assert!(output.status.success(), "ar p failed");

    let unix_lib_path = format!("{out_dir}/unix_lib.o");
    std::fs::write(&unix_lib_path, &output.stdout).expect("failed to write unix_lib.o");

    // The object carries DWARF sections, whose names run past the eight
    // characters a PE section name holds, so the linker files them through a
    // string table and would say so on every link. That is the documented
    // handling for such a name, and the DLL's own debug info is a `.pdb` those
    // sections never reach, so the notice is switched off rather than the
    // sections stripped: rewriting the object would also have to rewrite its
    // address-significance table, which is more surgery than a notice merits.
    println!("cargo:rustc-link-arg-cdylib=/ignore:longsections");
    println!("cargo:rustc-link-arg-cdylib={unix_lib_path}");

    // Nothing else from the Wine tree goes on the link line: the object above
    // is the whole of what this builtin takes from it, and the SDK's import
    // library satisfies every `ntdll` reference the bridge makes.

    println!("cargo:rerun-if-env-changed=WINE_SDK");
}

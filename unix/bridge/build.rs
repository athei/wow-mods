fn main() {
    println!("cargo::rustc-check-cfg=cfg(wow_crumb)");
    println!("cargo:rerun-if-env-changed=WOW_CRUMB");
    if std::env::var("WOW_CRUMB").is_ok_and(|v| !v.is_empty() && v != "0") {
        println!("cargo:rustc-cfg=wow_crumb");
    }

    let target = std::env::var("TARGET").unwrap();
    if !target.contains("apple") {
        return;
    }

    // Wine pairs a builtin PE (`wow_mods.dll`) with the `.so` of the same base
    // name in `lib/wine/x86_64-unix/`; the install step renames this dylib to
    // `wow_mods.so`, so its install name must match.
    println!("cargo:rustc-link-arg-cdylib=-Wl,-install_name,@rpath/wow_mods.so");
}

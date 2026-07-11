fn main() {
    println!("cargo::rustc-check-cfg=cfg(wow_crumb)");
    println!("cargo:rerun-if-env-changed=WOW_CRUMB");
    if std::env::var("WOW_CRUMB").is_ok_and(|v| !v.is_empty() && v != "0") {
        println!("cargo:rustc-cfg=wow_crumb");
    }
}

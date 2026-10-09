//! Hands the path of the crate's Android module to the build scripts of the crates that depend
//! on `pushups` directly, as `DEP_PUSHUPS_ANDROID`, which `tauri-plugin-pushups` copies.

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let android = std::path::Path::new(&manifest).join("android");
    println!("cargo::metadata=android={}", android.display());
    println!("cargo::rerun-if-changed=build.rs");
}

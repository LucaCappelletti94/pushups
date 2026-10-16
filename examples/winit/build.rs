//! Writes the static service worker beside the manifest, where the Trunk hook in `Trunk.toml`
//! copies it into the site.

fn main() {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    pushups::write_service_worker(dir).expect("the worker is written");
}

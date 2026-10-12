//! Writes the static service worker beside the manifest, where the Trunk hook in `Trunk.toml`
//! copies it into the site.

fn main() {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&dir).join("pushups-sw.js");
    // Rewriting an unchanged file would wake `trunk serve`'s watcher for nothing.
    if std::fs::read_to_string(&path).ok().as_deref() != Some(pushups::SERVICE_WORKER) {
        std::fs::write(&path, pushups::SERVICE_WORKER).expect("the worker is written");
    }
}

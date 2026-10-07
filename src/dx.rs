//! What `dx` bundles from this crate for a Dioxus app, declared here so the paths stay inside the
//! crate wherever Cargo keeps it.

// `dx` copies the Gradle module into the app's Android project.
#[cfg(target_os = "android")]
#[manganis::ffi("android")]
extern "Kotlin" {
    pub type Pushups;
}

/// The static service worker, copied unchanged and unhashed into the app's assets.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) const WORKER: manganis::Asset = manganis::asset!(
    "/js/pushups-sw.js",
    manganis::AssetOptions::js()
        .with_minify(false)
        .with_hash_suffix(false)
);

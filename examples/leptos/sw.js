// The service worker under `rust_handler`. Leptos's hydration script starts the wasm only in the
// page, so this module starts it in the worker, where the library's start function serves the
// Rust handler. cargo-leptos names the wasm `<output-name>.wasm`, so its URL is passed.
import init from "/pkg/pushups-leptos-example.js";

init({ module_or_path: "/pkg/pushups-leptos-example.wasm" });

// The service worker under `rust_handler`. Trunk starts the wasm only in the page, so this module
// starts it in the worker, where the app's `main` serves its Rust handler.
import init from "./pushups-yew-example.js";

init();

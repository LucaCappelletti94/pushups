//! Macros for [`pushups`](https://crates.io/crates/pushups).
//!
//! The public entry points are the re-exports on `pushups`, and their documentation lives there.
//! The examples here are marked `ignore` because `pushups` is not a dependency of this crate.

use std::path::Path;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::parse_macro_input;

mod parse;

/// Compile the Android Firebase configuration into the calling crate.
///
/// The path is relative to the calling crate's `CARGO_MANIFEST_DIR`. The file is read and parsed
/// at expansion time. A missing file, invalid JSON, a file without clients, or a client missing a
/// required field is a compile error.
///
/// On every target the expansion includes the file bytes, so changing the file re-triggers the
/// build. On Android it additionally exports `Java_rs_pushups_FirebaseConfig_values`. The
/// `pushups` crate's `ContentProvider` calls it at process start with the app's package name and
/// receives the values of the matching client.
///
/// # Example
///
/// ```ignore
/// pushups::firebase_config!("google-services.json");
/// ```
#[proc_macro]
pub fn firebase_config(input: TokenStream) -> TokenStream {
    let literal = parse_macro_input!(input as syn::LitStr);
    let span = literal.span();
    let relative = literal.value();
    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return error("firebase_config", span, "CARGO_MANIFEST_DIR is not set");
    };
    let path = Path::new(&manifest_dir).join(relative);
    let value = match parse::read_value(&path) {
        Ok(value) => value,
        Err(message) => return error("firebase_config", span, &message),
    };
    let clients = match parse::clients_from_json(&value) {
        Ok(clients) => clients,
        Err(message) => return error("firebase_config", span, &message),
    };
    let include_path = path.to_string_lossy().into_owned();
    let entries: Vec<TokenStream2> = clients
        .iter()
        .map(|client| {
            let package_name = &client.package_name;
            let application_id = &client.application_id;
            let api_key = &client.api_key;
            let gcm_sender_id = &client.gcm_sender_id;
            let project_id = &client.project_id;
            quote! {
                ::pushups::__private::android::FirebaseClient {
                    package_name: #package_name,
                    application_id: #application_id,
                    api_key: #api_key,
                    gcm_sender_id: #gcm_sender_id,
                    project_id: #project_id,
                }
            }
        })
        .collect();
    quote! {
        const _: &[u8] = include_bytes!(#include_path);

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn Java_rs_pushups_FirebaseConfig_values(
            env: *mut ::pushups::__private::android::JNIEnv,
            _class: ::pushups::__private::android::jclass,
            package_name: ::pushups::__private::android::jstring,
        ) -> ::pushups::__private::android::jobjectArray {
            const CLIENTS: &[::pushups::__private::android::FirebaseClient] =
                &[#(#entries),*];

            // SAFETY: `env` and `package_name` are live JNI arguments of the calling frame and
            // `firebase_values` only converts them while the frame is live.
            unsafe { ::pushups::__private::android::firebase_values(env, package_name, CLIENTS) }
        }
    }
    .into()
}

/// Mark a free function as the background handler.
///
/// The function must take `pushups::Context` and `pushups::Message`, be synchronous, be
/// non-generic and have no `self`. The function is kept unchanged. On every target the expansion
/// type-checks the signature, so a wrong one fails with a clear type error. A second handler in
/// one binary is a duplicate symbol at link time.
///
/// On Android the expansion additionally exports `Java_rs_pushups_BackgroundHandler_handle`, which
/// the `pushups` messaging service calls for every push that reaches a process without a live UI.
/// On iOS and macOS it exports `__pushups_background_handler`, which the crate finds at runtime
/// and runs off the main thread for every push the system delivers, within Apple's 30 seconds.
///
/// # Example
///
/// ```ignore
/// use pushups::{Context, Message};
///
/// #[pushups::background_handler]
/// fn on_push(context: Context, message: Message) {
///     // deliver the message
/// }
/// ```
#[proc_macro_attribute]
pub fn background_handler(args: TokenStream, item: TokenStream) -> TokenStream {
    let item: TokenStream2 = item.into();
    let input: syn::ItemFn = match syn::parse2(item.clone()) {
        Ok(input) => input,
        Err(error) => return error.to_compile_error().into(),
    };
    if !args.is_empty() {
        let span = args
            .into_iter()
            .next()
            .map_or_else(|| input.sig.ident.span(), |token| token.span().into());
        return error("background_handler", span, "takes no arguments");
    }
    if let Some(message) = validate(&input) {
        return error("background_handler", input.sig.ident.span(), message);
    }
    let name = &input.sig.ident;
    quote! {
        #item
        const _: fn(::pushups::Context, ::pushups::Message) = #name;

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn Java_rs_pushups_BackgroundHandler_handle(
            env: *mut ::pushups::__private::android::JNIEnv,
            _class: ::pushups::__private::android::jclass,
            context: ::pushups::__private::android::jobject,
            payload: ::pushups::__private::android::jbyteArray,
            started_app: ::pushups::__private::android::jboolean,
        ) {
            // SAFETY: `env`, `context` and `payload` are live JNI arguments of the calling frame
            // and `run_background_handler` only converts them while the frame is live.
            unsafe {
                ::pushups::__private::android::run_background_handler(
                    env,
                    context,
                    payload,
                    started_app,
                    #name,
                )
            }
        }

        #[cfg(any(target_os = "ios", target_os = "macos"))]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn __pushups_background_handler(
            payload: *const u8,
            len: usize,
            started_app: bool,
        ) {
            // SAFETY: the Apple backend passes a payload of `len` bytes alive for the call.
            unsafe {
                ::pushups::__private::apple::run_background_handler(payload, len, started_app, #name)
            }
        }

        // Keeps the export through the linker's dead-strip, since only `dlsym` names it.
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        #[used]
        static __PUSHUPS_BACKGROUND_HANDLER_KEPT: unsafe extern "C" fn(*const u8, usize, bool) =
            __pushups_background_handler;
    }
    .into()
}

/// The rules a background handler function must satisfy.
fn validate(input: &syn::ItemFn) -> Option<&'static str> {
    if input.sig.asyncness.is_some() {
        Some("the handler must be a synchronous function")
    } else if !input.sig.generics.params.is_empty() {
        Some("the handler must not be generic")
    } else if input.sig.receiver().is_some() {
        Some("the handler must not take `self`")
    } else {
        None
    }
}

/// Builds the `compile_error!` that a macro failure expands to.
fn error(macro_name: &str, span: proc_macro2::Span, message: &str) -> TokenStream {
    let message = format!("{macro_name}: {message}");
    quote_spanned!(span => compile_error!(#message);).into()
}

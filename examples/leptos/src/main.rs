//! The server of the Leptos example. It renders the page and serves both service workers from
//! routes, the static one from `pushups::SERVICE_WORKER` and the Rust handler's entry.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use axum::http::header;
    use axum::routing::get;
    use leptos::prelude::*;
    use leptos_axum::{LeptosRoutes, generate_route_list};
    use pushups_leptos_example::{App, WORKER_ENTRY, shell};

    let javascript = |source: &'static str| ([(header::CONTENT_TYPE, "text/javascript")], source);
    let conf = get_configuration(None).expect("the cargo-leptos configuration");
    let address = conf.leptos_options.site_addr;
    let options = conf.leptos_options;
    let routes = generate_route_list(App);
    let app = Router::new()
        .route(
            "/pushups-sw.js",
            get(move || async move { javascript(pushups::SERVICE_WORKER) }),
        )
        .route(
            "/sw.js",
            get(move || async move { javascript(WORKER_ENTRY) }),
        )
        .leptos_routes(&options, routes, {
            let options = options.clone();
            move || shell(options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(options);
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .expect("the address is free");
    println!("listening on http://{address}");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("the server runs");
}

/// The wasm build is the library, which the page and the worker start.
#[cfg(not(feature = "ssr"))]
fn main() {}

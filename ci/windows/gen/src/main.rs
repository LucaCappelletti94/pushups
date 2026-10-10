//! Regenerates `src/windows/bindings.rs` from the two Windows App SDK `.winmd` files
//! named on the command line.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [push, lifecycle] = args
        .as_slice()
        else {
            eprintln!("usage: windows-bindings-gen <Microsoft.Windows.PushNotifications.winmd> <Microsoft.Windows.AppLifecycle.winmd>");
            std::process::exit(1);
        };
    let out = "src/windows/bindings.rs";
    let argv: Vec<&str> = vec![
        "--in",
        push,
        "--in",
        lifecycle,
        // The OS metadata the WinAppSDK types reference.
        "--in",
        "default",
        // OS types outside the two namespaces resolve through the `windows` crate.
        "--reference",
        "windows",
        "--out",
        out,
        "--flat",
        "--filter",
        "Microsoft.Windows.PushNotifications",
        "--filter",
        "Microsoft.Windows.AppLifecycle",
    ];
    let warnings = windows_bindgen::bindgen(&argv);
    if !warnings.is_empty() {
        eprintln!("{warnings}");
        std::process::exit(1);
    }
    relint(out);
}

// The generated `#![allow]` header predates the repository's lint rules, so it is replaced.
fn relint(out: &str) {
    // The exact lints the generated bindings trip, each named so a new one is a visible diff.
    const HEADER: &str = "#![allow(\n    non_snake_case,\n    non_upper_case_globals,\n    non_camel_case_types,\n    dead_code,\n    clippy::borrow_as_ptr,\n    clippy::missing_transmute_annotations,\n    clippy::ptr_as_ptr,\n    clippy::transmute_ptr_to_ptr,\n    clippy::useless_transmute,\n    reason = \"generated windows-bindgen output\"\n)]";
    let source = match std::fs::read_to_string(out) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("reading {out}: {error}");
            std::process::exit(1);
        }
    };
    let Some(start) = source.find("#![allow(") else {
        eprintln!("the generated allow header is missing from {out}");
        std::process::exit(1);
    };
    let Some(close) = source[start..].find(")]") else {
        eprintln!("the generated allow header is unterminated in {out}");
        std::process::exit(1);
    };
    let fixed = format!("{}{}{}", &source[..start], HEADER, &source[start + close + 2..]);
    if let Err(error) = std::fs::write(out, fixed) {
        eprintln!("writing {out}: {error}");
        std::process::exit(1);
    }
}

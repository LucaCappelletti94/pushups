//! The assets of the HTML UIs, the page's styles in the brand's colours and the mark, the event
//! fields a static page's script shows and the script a page uses for the clipboard.

use pushups::Event;

use super::{Push, push_view, shortened, shown, transport};

/// The page's styles in the brand's colours from `assets/brand`, a navy ground and the mark's
/// red-to-orange gradient.
pub const CSS: &str = "
:root { --navy: #0e1824; --surface: #172436; --text: #f5f7fa; --muted: #9aa6b8; --gradient: linear-gradient(135deg, #fa1e13, #f67017); }
html, body { margin: 0; min-height: 100%; background: var(--navy); }
.app { font-family: -apple-system, system-ui, sans-serif; color: var(--text); padding: 20px 16px; max-width: 640px; margin: 0 auto; box-sizing: border-box; }
.header { display: flex; align-items: center; gap: 12px; margin-bottom: 16px; }
.header h2 { margin: 0; font-size: 26px; font-weight: 700; }
.mark { width: 56px; flex: none; }
.mark svg { display: block; width: 100%; height: auto; }
.status { background: var(--surface); border-radius: 10px; padding: 12px 14px; margin: 0 0 12px; }
.status .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin: 4px 0; }
.status .label { color: var(--muted); }
.status button { font-size: 16px; font-weight: 600; color: #ffffff; padding: 10px 16px; border: none; border-radius: 10px; background: var(--gradient); }
.status button.secondary { background: transparent; border: 1px solid var(--muted); color: var(--text); }
.status .copy { font-size: 12px; font-weight: 600; color: var(--text); background: transparent; border: 1px solid var(--muted); border-radius: 8px; padding: 4px 10px; }
.token { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; margin: 0 0 8px; }
.actions button { font-size: 16px; font-weight: 600; color: #ffffff; padding: 10px 16px; border: none; border-radius: 10px; background: var(--gradient); }
.muted { color: var(--muted); }
.card { background: var(--surface); border-radius: 10px; padding: 12px 14px; margin: 8px 0; }
.card .title { margin: 0 0 4px; font-size: 15px; font-weight: 700; overflow-wrap: anywhere; }
.card .body { margin: 0 0 6px; overflow-wrap: anywhere; }
.card .meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; color: var(--muted); font-size: 12px; }
.badge { background: var(--gradient); color: #ffffff; font-size: 11px; font-weight: 700; padding: 2px 8px; border-radius: 10px; }
.line { font-family: ui-monospace, monospace; font-size: 12px; overflow-wrap: anywhere; background: var(--surface); padding: 8px 10px; border-radius: 8px; margin: 8px 0; }
.error { color: #fa1e13; font-size: 12px; margin: 4px 0 0; overflow-wrap: anywhere; }
";

/// The brand mark, drawn inline so every platform shows it without an asset pipeline.
pub const MARK: &str = include_str!("../../../assets/brand/mark.svg");

/// The event a static page's script shows, the card fields for a push, the token fields for a
/// registration and the line its copy log copies.
#[derive(serde::Serialize)]
pub struct PageEvent {
    /// The compact line the list shows and the copy log copies.
    pub line: String,
    /// The push the page shows as a card, for a push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<Push>,
    /// The transport the status card shows, for a token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<&'static str>,
    /// The shortened token the status card shows, for a token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The full token the copy button copies, for a token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
}

/// The page's fields for `event`, received at `at`.
#[must_use]
pub fn page_event(event: &Event, at: u128) -> PageEvent {
    match event {
        Event::Token(token) => PageEvent {
            line: shown(event),
            push: None,
            transport: Some(transport(token)),
            token: Some(shortened(token)),
            full: Some(super::token(token)),
        },
        Event::Message(message) => {
            let push = push_view(message, at);
            PageEvent {
                line: push.line(),
                push: Some(push),
                transport: None,
                token: None,
                full: None,
            }
        }
        _ => PageEvent {
            line: shown(event),
            push: None,
            transport: None,
            token: None,
            full: None,
        },
    }
}

/// The brand's assets the static page's script applies on load, and whether the platform
/// offers the notification settings the example can open.
#[must_use]
pub fn assets(settings: bool) -> String {
    serde_json::json!({ "css": CSS, "mark": MARK, "settings": settings }).to_string()
}

/// The page's fields for `event`, received at `at`, as the JSON the page's script reads.
#[must_use]
pub fn page_event_json(event: &Event, at: u128) -> String {
    serde_json::to_string(&page_event(event, at)).expect("the page's fields serialise")
}

/// The script that puts `text` on the clipboard, through `navigator.clipboard` and, when the
/// page refuses the async clipboard, through a hidden field and `document.execCommand`.
#[must_use]
pub fn clipboard_script(text: &str) -> String {
    let quoted = serde_json::to_string(text).expect("a string serialises");
    format!(
        "(function (text) {{
            var fallback = function () {{
                var field = document.createElement('textarea');
                field.value = text;
                document.body.appendChild(field);
                field.select();
                document.execCommand('copy');
                field.remove();
            }};
            if (navigator.clipboard) {{
                navigator.clipboard.writeText(text).catch(fallback);
            }} else {{
                fallback();
            }}
        }})({quoted})"
    )
}

/// Puts `text` on the clipboard, through the page's script.
#[cfg(target_arch = "wasm32")]
pub fn copy(text: &str) {
    let _ = js_sys::eval(&clipboard_script(text));
}

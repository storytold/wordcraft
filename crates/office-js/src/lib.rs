//! WordCraft's implementation of the Office JavaScript API for Office add-ins' task panes.
//!
//! An Office add-in's task pane is a web page that calls `Office.onReady`, `Word.run` and the
//! proxy objects (`context.document.body.insertParagraph("Hi", "End")`,
//! `body.paragraphs.load("items/text")`, `context.sync()`). WordCraft never ships Microsoft's
//! `office.js` (and `bootstrap.js` defuses it when a page loads it): this crate, compiled to wasm,
//! defines those globals itself, written from the public API reference (Word JavaScript API,
//! WordApi 1.1–1.3, and the common API) on learn.microsoft.com (clean room). What it covers is in
//! `docs/officejs.md`. Each `context.sync()` sends the queued actions as one JSON batch to the
//! host, which runs it as `office.sync` (one undo step) and answers with the loaded values or an
//! error:
//!
//! - request `{"id": n, "actions": [{"op": "load"|"set"|"call", "path": [...], …}]}`, posted
//!   with `globalThis.__craftOffice.post(json)`;
//! - answer `{"id": n, "results": [...]}` or `{"id": n, "error": {"code", "message", "action"}}`,
//!   delivered with `globalThis.__craftOfficeReply(json)`.
//!
//! Word's API has no ids, but the host hands out opaque ones (`"p:3"`, `"r:…"`), so an object
//! keeps naming the same thing across syncs:
//!
//! - an object an `insert*` call creates is named, until the sync answers its id, by the action
//!   that creates it: a paragraph `[{"p": "document"}, {"p": "paragraphs"}, {"m": "getById",
//!   "a": [{"result": i}]}]`, a range `[{"p": "document"}, {"m": "getRangeById", "a":
//!   [{"result": i}]}]`; after the sync, by its id (`{"m": "getById", "a": ["p:4"]}`);
//! - an object a navigation method returns (`getSelection()`, `getRange()`, `getFirst()`,
//!   `getLast()`, `getNext()`, `getPrevious()`) is pinned when a queued action first uses it: a
//!   `load` of its `id` along the method's steps goes first, and every use after it names it
//!   the same way (`{"result": i}`, then the id);
//! - a collection's loaded items are named by their ids.
//!
//! Enum values (`"End"`, `"Centered"`, `"Heading1"`) travel as the API spells them.
//!
//! Besides syncs, the library tells the host `{"event": "ready"}` once it has started, and
//! `{"event": "functionDone", "call", "error"?}` when a function command
//! (`__craftOfficeRunFunction({"call", "name", "control"})`, run on a function associated with
//! `Office.actions.associate`) completes or fails; the host calls `__craftOfficeEvent({"type":
//! "documentSelectionChanged"})` for `addHandlerAsync`'s handlers.
//!
//! [`core`] is the batching, the naming and the cache of loaded properties (plain Rust, tested
//! natively); the JavaScript side (`js`, wasm only) is the globals and proxy classes. What isn't
//! covered throws an error with code `NotImplemented`.
//!
//! # Building the library a task pane loads
//!
//! ```sh
//! cargo build --release -p wordcraft-office-js --target wasm32-unknown-unknown
//! mkdir -p "$CARGO_TARGET_DIR/office-js"
//! wasm-bindgen --target no-modules --no-typescript --out-name office_js \
//!     --out-dir "$CARGO_TARGET_DIR/office-js" \
//!     "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/wordcraft_office_js.wasm"
//! cp crates/office-js/bootstrap.js "$CARGO_TARGET_DIR/office-js/bootstrap.js"
//! ```
//!
//! (`wasm-bindgen-cli` at the `wasm-bindgen` version in `Cargo.lock`.) The output is
//! `office_js.js` (glue defining `wasm_bindgen`), `office_js_bg.wasm` and `bootstrap.js`. The
//! desktop app injects, before the page's own scripts: `bootstrap.js`, then `office_js.js`, then
//! `__craftOfficeStart(wasm_bindgen, "<office_js_bg.wasm as base64>")`.
//!
//! # Tests
//!
//! `cargo test -p wordcraft-office-js` runs the batching tests natively; the library as a page
//! sees it runs in Node against a fake host:
//!
//! ```sh
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     cargo test -p wordcraft-office-js --target wasm32-unknown-unknown
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod core;
#[cfg(target_arch = "wasm32")]
mod js;
#[cfg(target_arch = "wasm32")]
pub use js::*;

#[cfg(test)]
mod bootstrap_tests {
    /// The bootstrap turns off Microsoft's `office.js` from the CDNs sample pages load it from,
    /// and guards every global the library defines (`js::install_globals`).
    #[test]
    fn the_bootstrap_knows_the_office_js_hosts_and_guards_the_globals() {
        let js = include_str!("../bootstrap.js");
        for host in [r"appsforoffice\.microsoft\.com", r"officeapis\.public\.onecdn\.static\.microsoft", "@microsoft\\/office-js"] {
            assert!(js.contains(host), "{host}");
        }
        for name in ["Office: office", "Word: fixed", "OfficeRuntime: {}", "OfficeExtension: {}"] {
            assert!(js.contains(name), "{name}");
        }
        for hook in ["__craftOfficeAdopt", "__craftOfficeStart", "data:text/javascript,"] {
            assert!(js.contains(hook), "{hook}");
        }
    }
}

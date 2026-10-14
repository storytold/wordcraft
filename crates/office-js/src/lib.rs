//! WordCraft's implementation of the Office JavaScript API for Office add-ins' task panes.
//!
//! An Office add-in's task pane is a web page that calls `Office.onReady`, `Word.run` and the
//! proxy objects (`context.document.body.insertParagraph("Hi", "End")`,
//! `body.paragraphs.load("items/text")`, `context.sync()`). WordCraft never loads Microsoft's
//! `office.js`: this crate, compiled to wasm, defines those globals itself, written from the
//! public API reference (Word JavaScript API, WordApi 1.1–1.3) on learn.microsoft.com (clean
//! room). Each `context.sync()` sends the queued actions as one JSON batch to the host, which runs
//! it as `office.sync` (one undo step) and answers with the loaded values or an error:
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

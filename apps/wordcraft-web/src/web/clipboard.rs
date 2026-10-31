//! Copy, Cut and Paste with the browser's clipboard in its own formats. eframe's handlers carry
//! only text: a copy reaches the clipboard as plain text (written after the event, through the
//! asynchronous clipboard API) and a paste reads only its text, so formatting was lost both ways,
//! while the desktop app puts HTML beside the text and reads another program's HTML
//! (`wordcraft_ui_egui::system_clipboard`).
//!
//! A page reads and writes the clipboard's formats synchronously only in the trusted `copy`,
//! `cut` and `paste` events, while they run, and the canvas handles eframe's copy event in the
//! next frame, after it is over. So these listeners run the command then and there with the
//! event's `clipboardData`, ahead of eframe's handlers, and stop the event so the command doesn't
//! run twice. What they leave alone (the keyboard elsewhere than the document's text, an input
//! method composing, an equation, nothing selected, a paste with no HTML or no text, Paste
//! Special's modifiers) goes to eframe and the picture listener as before.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;

use super::WebShell;

/// Set while a copy or cut event runs its command: the copy's HTML and text then go in the
/// event's `clipboardData`, so egui must not write its plain text over them afterwards.
#[derive(Clone, Default)]
pub(super) struct InEvent(Rc<Cell<bool>>);

impl InEvent {
    /// The `set_clipboard_html` service: `true` (the HTML and text are on the clipboard) only
    /// during a copy or cut event, where the listener writes them.
    pub(super) fn service(&self) -> Box<dyn Fn(&str, &str, &str) -> bool> {
        let on = self.0.clone();
        Box::new(move |_, _, _| on.get())
    }
}

/// Listen for copy, cut and paste on the document, ahead of eframe (capture phase).
pub(super) fn install(runner: &eframe::WebRunner, in_event: InEvent) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let options = web_sys::AddEventListenerOptions::new();
    options.set_capture(true);
    // Alt and Shift as the browser has them now: egui's input only catches up in the next frame,
    // and a quick Alt+Mod+V can reach the paste event before that.
    let held = Rc::new(Cell::new((false, false)));
    for kind in ["keydown", "keyup"] {
        let held = held.clone();
        let on_key = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            held.set((e.alt_key(), e.shift_key()));
        });
        document
            .add_event_listener_with_callback_and_add_event_listener_options(kind, on_key.as_ref().unchecked_ref(), &options)
            .map_err(|e| format!("{e:?}"))?;
        on_key.forget();
    }
    for kind in ["copy", "cut", "paste"] {
        let runner = runner.clone();
        let in_event = in_event.clone();
        let held = held.clone();
        let on_event = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
            if !e.is_trusted() {
                return;
            }
            let Some(data) = e.clipboard_data() else { return };
            let Some(mut shell) = runner.app_mut::<WebShell>() else { return };
            let handled = if kind == "paste" { paste(&mut shell, &data, held.get()) } else { copy(&mut shell, &data, kind == "cut", &in_event) };
            if handled {
                e.prevent_default();
                e.stop_immediate_propagation();
                shell.ctx.request_repaint();
            }
        });
        document
            .add_event_listener_with_callback_and_add_event_listener_options(kind, on_event.as_ref().unchecked_ref(), &options)
            .map_err(|e| format!("{e:?}"))?;
        // The listeners live as long as the page.
        on_event.forget();
    }
    Ok(())
}

/// Whether the document's text has the keyboard, with no composition under way.
fn editing(shell: &WebShell) -> bool {
    shell.app.canvas.focused && shell.app.canvas.ime_preedit.is_empty() && shell.app.session.math.is_none()
}

/// Copy or Cut the selection into the event's clipboard: its text, and its HTML when it has
/// formatting to carry. A copy with no text (a lone picture) is left as the app puts it. `false`
/// (eframe copies) when nothing is selected in the document's text.
fn copy(shell: &mut WebShell, data: &web_sys::DataTransfer, cut: bool, in_event: &InEvent) -> bool {
    if !editing(shell) || shell.app.session.sel.is_collapsed() {
        return false;
    }
    in_event.0.set(true);
    let r = shell.app.run(if cut { "edit.cut" } else { "edit.copy" }, json!({}));
    in_event.0.set(false);
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            log::warn!("copy failed: {e}");
            // The command ran (and said why it failed in the status bar): not again in eframe's handler.
            return true;
        }
    };
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    if s("text").is_empty() {
        return true;
    }
    let mut ok = data.set_data("text/plain", s("text")).is_ok();
    if !s("html").is_empty() {
        ok &= data.set_data("text/html", s("html")).is_ok();
    }
    if !ok {
        log::warn!("the browser refused the copy's clipboard data");
    }
    true
}

/// Paste the event's HTML with its text, or WordCraft's own copy when the clipboard is empty
/// ([`crate::clipboard_input::paste_params`]). `false` (eframe and the picture listener take it)
/// for text or pictures alone, when it is too large, or with Alt or Shift held (`held`: Paste
/// Special, Keep Text Only).
fn paste(shell: &mut WebShell, data: &web_sys::DataTransfer, held: (bool, bool)) -> bool {
    if !editing(shell) || held.0 || held.1 {
        return false;
    }
    let html = data.get_data("text/html").unwrap_or_default();
    let text = data.get_data("text/plain").unwrap_or_default();
    let files = data.files().is_some_and(|f| f.length() > 0);
    let Some(params) = crate::clipboard_input::paste_params(&text, &html, files) else { return false };
    if let Err(e) = shell.app.run("edit.paste", params) {
        log::warn!("paste failed: {e}");
    }
    true
}

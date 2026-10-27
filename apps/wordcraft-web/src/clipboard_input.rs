//! What the browser's clipboard gestures hand the app (`web::clipboard`), kept apart from the
//! browser so it can be tested natively.

use serde_json::{Value, json};
use wordcraft_engine::cmd::paste::MAX_PASTE_BYTES;

/// A press of Mod+C, Mod+X or Mod+V with no Shift or Alt. The browser sends these as key presses
/// and then as its copy, cut or paste event, which is the gesture, so the press is dropped
/// before the app sees it: as a shortcut it would run the command a second time. eframe sets
/// `command` for Ctrl or Cmd; Cmd+Ctrl together is no clipboard gesture on a Mac, so it stays.
pub(crate) fn is_clipboard_key(e: &egui::Event) -> bool {
    matches!(e, egui::Event::Key { key: egui::Key::C | egui::Key::X | egui::Key::V, pressed: true, modifiers, .. }
        if modifiers.command && !modifiers.shift && !modifiers.alt && !(modifiers.mac_cmd && modifiers.ctrl))
}

/// Drop the browser's own clipboard presses ([`is_clipboard_key`]) from a frame's input.
pub(crate) fn drop_clipboard_keys(raw: &mut egui::RawInput) {
    raw.events.retain(|e| !is_clipboard_key(e));
}

/// The `edit.paste` params for a paste event's text and HTML, as the desktop app pastes another
/// program's formatted copy. An empty clipboard (no text, HTML or files) pastes WordCraft's own
/// copy, as Mod+V did as a shortcut: a copied picture alone leaves the browser's clipboard
/// empty. `None` for text alone, or when either is larger than a paste may be: the text is then
/// pasted as before. Also `None` for HTML without text: a browser's Copy Image puts the picture
/// beside HTML that only points at it, and the picture listener inserts the picture.
pub(crate) fn paste_params(text: &str, html: &str, files: bool) -> Option<Value> {
    if text.is_empty() && html.trim().is_empty() && !files {
        return Some(json!({}));
    }
    if text.is_empty() || html.trim().is_empty() || html.len() > MAX_PASTE_BYTES || text.len() > MAX_PASTE_BYTES {
        return None;
    }
    Some(json!({"text": text.replace("\r\n", "\n"), "html": html}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn only_the_plain_clipboard_presses_are_the_browsers() {
        use egui::{Key, Modifiers};
        for m in [Modifiers::COMMAND, Modifiers::CTRL | Modifiers::COMMAND, Modifiers::MAC_CMD | Modifiers::COMMAND] {
            for k in [Key::C, Key::X, Key::V] {
                assert!(is_clipboard_key(&key(k, true, m)), "{k:?} with {m:?}");
                assert!(!is_clipboard_key(&key(k, false, m)), "the release stays: {k:?}");
            }
        }
        // Copy and Paste Formatting, Paste Special, plain letters and other shortcuts stay.
        assert!(!is_clipboard_key(&key(Key::C, true, Modifiers::COMMAND | Modifiers::SHIFT)));
        assert!(!is_clipboard_key(&key(Key::V, true, Modifiers::COMMAND | Modifiers::SHIFT)));
        assert!(!is_clipboard_key(&key(Key::V, true, Modifiers::COMMAND | Modifiers::ALT)));
        assert!(!is_clipboard_key(&key(Key::V, true, Modifiers::NONE)));
        assert!(!is_clipboard_key(&key(Key::B, true, Modifiers::COMMAND)));
        // eframe's web input sets `command` for Ctrl or Cmd. Cmd+Ctrl+C/X/V on a Mac is no
        // browser copy, cut or paste, so the press stays for a shortcut bound to it.
        let mac_cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
        for k in [Key::C, Key::X, Key::V] {
            assert!(!is_clipboard_key(&key(k, true, mac_cmd | Modifiers::CTRL)), "Cmd+Ctrl+{k:?} is kept");
        }
        assert!(is_clipboard_key(&key(Key::V, true, mac_cmd)), "Cmd+V on a Mac is the browser's");
        assert!(is_clipboard_key(&key(Key::V, true, Modifiers::CTRL | Modifiers::COMMAND)), "Ctrl+V elsewhere is the browser's");
        assert!(!is_clipboard_key(&egui::Event::Paste("text".into())));
        assert!(!is_clipboard_key(&egui::Event::Copy));
    }

    #[test]
    fn a_paste_with_html_keeps_it_beside_the_text() {
        let html = "<p><b>Bold</b> plain</p>";
        assert_eq!(paste_params("Bold plain\r\nnext", html, false), Some(json!({"text": "Bold plain\nnext", "html": html})));
        assert_eq!(paste_params("", "<img src=\"https://example.com/a.png\">", false), None, "a copied picture is the picture listener's");
        assert_eq!(paste_params("plain", "", false), None, "text alone is eframe's");
        assert_eq!(paste_params("plain", " \n ", false), None);
        let big = "x".repeat(MAX_PASTE_BYTES + 1);
        assert_eq!(paste_params("plain", &big, false), None, "too large");
        assert_eq!(paste_params(&big, html, false), None, "too large");
    }

    /// One Mod+V in the browser: the key press, then the paste event with the clipboard's text,
    /// in one frame. With the presses dropped it pastes once; without, the press first pasted
    /// WordCraft's own last copy.
    #[test]
    fn one_mod_v_with_its_paste_event_pastes_once() {
        for drop_keys in [true, false] {
            let doc = wordcraft_doc::Document::from_text("ab");
            let mut app = wordcraft_ui_egui::WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
            app.run("select.all", json!({})).unwrap();
            app.run("edit.copy", json!({})).unwrap();
            app.run("caret.docEnd", json!({})).unwrap();
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
            let frame = |app: &mut wordcraft_ui_egui::WordApp, events: Vec<egui::Event>| {
                let mut raw = egui::RawInput { events, screen_rect: Some(screen), ..Default::default() };
                if drop_keys {
                    drop_clipboard_keys(&mut raw);
                }
                app.raw_input_hook(&ctx, &mut raw);
                ctx.run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                })
                .drop_without_applying_deltas();
            };
            for _ in 0..4 {
                frame(&mut app, Vec::new());
            }
            let cmd = egui::Modifiers::COMMAND;
            let v = |pressed| egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: cmd };
            frame(&mut app, vec![egui::Event::ModifiersChanged(cmd), v(true), egui::Event::Paste("Z".into()), v(false)]);
            let text = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
            let want = if drop_keys { "abZ" } else { "ababZ" };
            assert_eq!(text, want, "presses dropped: {drop_keys}");
        }
    }

    /// A picture copied alone in WordCraft leaves the browser's clipboard empty; Mod+V pastes
    /// WordCraft's own copy, as it did when the key press ran Paste.
    #[test]
    fn an_empty_clipboard_pastes_wordcrafts_own_copy() {
        assert_eq!(paste_params("", "", false), Some(json!({})));
        assert_eq!(paste_params("", " ", false), Some(json!({})));
        assert_eq!(paste_params("", "", true), None, "a pasted picture file is the picture listener's");
        assert_eq!(paste_params("plain", "", false), None, "text is eframe's");
    }
}

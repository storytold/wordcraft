//! What the browser's clipboard gestures hand the app (`web::clipboard`), kept apart from the
//! browser so it can be tested natively.

use serde_json::{Value, json};
use wordcraft_engine::cmd::paste::MAX_PASTE_BYTES;

/// A press of Mod+C, Mod+X or Mod+V with no Shift or Alt. The browser sends these as key presses
/// and then as its copy, cut or paste event, which is the gesture, so the press is dropped
/// before the app sees it: as a shortcut it would run the command a second time.
pub(crate) fn is_clipboard_key(e: &egui::Event) -> bool {
    matches!(e, egui::Event::Key { key: egui::Key::C | egui::Key::X | egui::Key::V, pressed: true, modifiers, .. }
        if modifiers.command && !modifiers.shift && !modifiers.alt)
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

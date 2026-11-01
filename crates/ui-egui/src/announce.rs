//! Screen readers: what a formatting shortcut did, said as Word says it ("Bold On", "Center Off",
//! "Font 14pt"). Word raises these itself; here they go through a polite live region, which
//! screen readers read when it appears.

use egui::accesskit::{Live, Role};
use serde_json::Value;
use wordcraft_doc::Align;

use crate::WordApp;

/// How long an announcement stays in the tree: a screen reader reads its name after the
/// live-region event, so it must still be there then.
const SHOWN_MS: f64 = 3000.0;

/// An announcement for screen readers. Each one gets its own node, numbered: a live region is
/// read when it appears or its text changes, so the same text twice needs a new node.
#[derive(Clone, Debug)]
pub(crate) struct Announcement {
    text: String,
    serial: u64,
    at: f64,
}

/// Announces what the shortcut for command `id` did, once it has run; nothing for a command Word
/// doesn't announce.
pub(crate) fn after_shortcut(app: &mut WordApp, id: &str) {
    let Some(text) = describe(app, id) else { return };
    let serial = app.announcement.as_ref().map_or(0, |a| a.serial.wrapping_add(1));
    app.announcement = Some(Announcement { text, serial, at: crate::now_ms() });
}

/// The words for what command `id` did: the command's name and whether its formatting is now on,
/// or the font size the selection now has.
fn describe(app: &WordApp, id: &str) -> Option<String> {
    let on_off = |name: &str, on: bool| format!("{name} {}", if on { tl!("On") } else { tl!("Off") });
    let state = || wordcraft_engine::cmd::format::state(&app.session);
    let character = match id {
        "format.bold" => Some(("bold", tl!("Bold"))),
        "format.italic" => Some(("italic", tl!("Italic"))),
        "format.underline" => Some(("underline", tl!("Underline"))),
        "format.subscript" => Some(("subscript", tl!("Subscript"))),
        "format.superscript" => Some(("superscript", tl!("Superscript"))),
        _ => None,
    };
    if let Some((key, name)) = character {
        return Some(on_off(name, state().get(key).and_then(Value::as_bool).unwrap_or(false)));
    }
    let alignment = match id {
        "para.alignLeft" => Some((Align::Left, tl!("Align Left"))),
        "para.alignCenter" => Some((Align::Center, tl!("Center"))),
        "para.alignRight" => Some((Align::Right, tl!("Align Right"))),
        "para.justify" => Some((Align::Justify, tl!("Justify"))),
        _ => None,
    };
    if let Some((want, name)) = alignment {
        // As the ribbon's buttons show it: Center twice centres, then goes back to the left. Read
        // in a paragraph the command formatted: a selection ending at the next paragraph's start
        // leaves that one out (#418).
        let s = &app.session;
        let formatted = wordcraft_engine::cmd::selected_paths(s);
        let path = if formatted.contains(&s.sel.focus.path) { Some(s.sel.focus.path.clone()) } else { formatted.last().cloned() };
        let at = path.map(|path| wordcraft_doc::Pos { path, off: 0, ..s.sel.focus.clone() });
        let align = at.and_then(|at| s.doc.para_at(&at)).map(|p| s.doc.styles.resolve_para(&p.props)).map(|r| r.align.visual(r.bidi));
        return Some(on_off(name, align == Some(want)));
    }
    if matches!(id, "format.growFont" | "format.shrinkFont" | "format.growFont1" | "format.shrinkFont1") {
        // A selection of mixed sizes has no one size to say.
        let size = state().get("size").and_then(Value::as_f64)?;
        return Some(format!("{} {}", tl!("Font"), crate::options::length_in(wordcraft_geom::Unit::Points, size as f32)));
    }
    None
}

/// Puts the latest announcement in the tree, as a polite live region, while it is fresh.
pub(crate) fn expose(app: &WordApp, ctx: &egui::Context) {
    let Some(a) = &app.announcement else { return };
    let age = crate::now_ms() - a.at;
    if !(0.0..SHOWN_MS).contains(&age) {
        return;
    }
    ctx.accesskit_node_builder(egui::Id::unique(("announcement", a.serial)), |node| {
        // A label's text is its value: that is the name screen readers read (UI Automation's
        // live-region event goes only to a node with a name).
        node.set_role(Role::Label);
        node.set_live(Live::Polite);
        node.set_value(a.text.clone());
    });
    // A frame when it's due to go, so it leaves the tree even while nothing else happens.
    ctx.request_repaint_after(std::time::Duration::from_secs_f64((SHOWN_MS - age) / 1000.0));
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Live;
    use egui::{Key, Modifiers};
    use egui_kittest::kittest::{NodeT, Queryable};
    use serde_json::json;

    use crate::WordApp;

    fn harness(text: &str) -> egui_kittest::Harness<'static, WordApp> {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(text)), crate::Services::default());
        app.canvas.want_focus = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(6);
        assert!(h.state().canvas.focused, "the page has the keyboard");
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 8) };
        h.run_steps(2);
        h
    }

    /// The live region's text and node (its id, written out), if one is in the tree. The text is
    /// the label's value, which is its name to screen readers.
    fn heard(h: &egui_kittest::Harness<'_, WordApp>) -> Option<(String, String)> {
        let node = h.query_by(|n| n.live() == Live::Polite)?;
        let n = node.accesskit_node();
        assert!(n.label_comes_from_value(), "a label, named by its value");
        Some((n.value().unwrap_or_default(), format!("{:?}", n.id())))
    }

    fn press(h: &mut egui_kittest::Harness<'_, WordApp>, m: Modifiers, key: Key) -> Option<(String, String)> {
        h.key_press_modifiers(m, key);
        h.run_steps(2);
        heard(h)
    }

    /// Ctrl+B, Ctrl+I, Ctrl+U and the alignment keys say what they did, as Word does ("Bold On",
    /// then "Bold Off"), each time in a new live region, so the same words twice are heard twice.
    #[test]
    fn formatting_shortcuts_say_what_they_did() {
        let mut h = harness("Lanterns glow");
        assert_eq!(heard(&h), None, "nothing to say before a shortcut");
        let (text, first) = press(&mut h, Modifiers::COMMAND, Key::B).expect("Ctrl+B is announced");
        assert_eq!(text, "Bold On");
        let (text, second) = press(&mut h, Modifiers::COMMAND, Key::B).expect("again");
        assert_eq!(text, "Bold Off");
        assert_ne!(first, second, "each announcement is a new live region");
        for (key, said) in [
            (Key::I, "Italic On"),
            (Key::U, "Underline On"),
            (Key::E, "Center On"),
            (Key::E, "Center Off"),
            (Key::R, "Align Right On"),
            (Key::J, "Justify On"),
            (Key::L, "Align Left On"),
        ] {
            assert_eq!(press(&mut h, Modifiers::COMMAND, key).map(|(t, _)| t).as_deref(), Some(said), "{key:?}");
        }
        let (_, before) = heard(&h).expect("the last one");
        let (text, again) = press(&mut h, Modifiers::COMMAND, Key::L).expect("Ctrl+L twice");
        assert_eq!(text, "Align Left On", "left alignment stays");
        assert_ne!(before, again, "the same words twice are a new live region, so they are heard again");
        let grown = press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Period).map(|(t, _)| t).expect("Ctrl+Shift+. is announced");
        assert!(grown.starts_with("Font ") && grown.ends_with(" pt"), "{grown}");
    }

    /// Only shortcuts are announced: a ribbon button says its own state, and a command that isn't
    /// formatting says nothing. An announcement leaves the tree after three seconds.
    #[test]
    fn only_formatting_shortcuts_are_announced_for_a_while() {
        let mut h = harness("Lanterns glow");
        h.state_mut().run("format.bold", json!({})).unwrap();
        h.run_steps(2);
        assert_eq!(heard(&h), None, "a command run another way isn't announced");
        assert_eq!(press(&mut h, Modifiers::COMMAND, Key::Z), None, "Undo isn't announced");
        assert!(press(&mut h, Modifiers::COMMAND, Key::I).is_some());
        if let Some(a) = &mut h.state_mut().announcement {
            a.at -= super::SHOWN_MS;
        }
        h.run_steps(2);
        assert_eq!(heard(&h), None, "gone after three seconds");
    }

    /// A selection ending at the next paragraph's start (Shift+Down from a paragraph's start)
    /// formats only the paragraph it covers, as in Word: that is the one the announcement says.
    #[test]
    fn alignment_is_said_for_the_paragraph_the_shortcut_formatted() {
        let mut h = harness("Lanterns glow\nRivers run");
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(1, 0) };
        h.run_steps(2);
        assert_eq!(press(&mut h, Modifiers::COMMAND, Key::E).map(|(t, _)| t).as_deref(), Some("Center On"));
        let align =
            |h: &egui_kittest::Harness<'_, WordApp>, i| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(i, 0)).and_then(|p| p.props.align);
        assert_eq!((align(&h, 0), align(&h, 1)), (Some(wordcraft_doc::Align::Center), None), "only the first paragraph is centred");
        assert_eq!(press(&mut h, Modifiers::COMMAND, Key::E).map(|(t, _)| t).as_deref(), Some("Center Off"));
    }

    /// The shortcut runs after the announcement's place in the frame, so the announcement goes in
    /// the next frame's tree: a key asks egui for that frame itself.
    #[test]
    fn the_announcement_is_in_the_frame_after_the_key() {
        let mut h = harness("Lanterns glow");
        // The key down alone, in one frame (`key_press_modifiers` gives each event a frame).
        h.input_mut().events.push(egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
        h.step();
        assert_eq!(heard(&h), None, "the key's own frame built its tree before the shortcut ran");
        assert!(h.ctx.has_requested_repaint(), "the key asked for another frame");
        h.step();
        assert_eq!(heard(&h).map(|(t, _)| t).as_deref(), Some("Bold On"));
    }
}

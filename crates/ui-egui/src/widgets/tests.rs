use super::*;

/// #653: a large button's label line is no taller than `LABEL_LINE_H` in the interface font, so
/// two lines (checked at compile time) end above the group title row.
#[test]
fn big_button_label_lines_fit_their_slot() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::font_definitions(false, false));
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    let h = ctx.fonts_mut(|f| f.layout_no_wrap("Page Number Mail Merge gjpqy".into(), crate::theme::regular(11.5), Color32::WHITE).size().y);
    assert!(h <= LABEL_LINE_H, "label line is {h} px tall");
}

/// #1945: a ribbon button's right-click menu offers Show Quick Access Toolbar only while the
/// toolbar is hidden, and it brings the toolbar back.
#[test]
fn ribbon_menu_offers_show_quick_access_toolbar_only_while_hidden() {
    use egui_kittest::kittest::Queryable;
    let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    // The ribbon's Bold is the topmost one (the mini toolbar's, when up, sits lower).
    let ribbon_bold = |h: &egui_kittest::Harness<'static, WordApp>| {
        h.get_all_by_label("Bold").map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).expect("the ribbon has Bold")
    };
    let right_click = |h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2| {
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        }
        h.run_steps(2);
    };
    let at = ribbon_bold(&h);
    right_click(&mut h, at);
    assert!(h.query_by_label("Add to Quick Access Toolbar").is_some(), "the button's menu opened");
    assert!(h.query_by_label("Show Quick Access Toolbar").is_none(), "nothing extra while the toolbar shows");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().run("ui.quickAccessToolbar", serde_json::json!({"value": false})).unwrap();
    h.run_steps(2);
    let at = ribbon_bold(&h);
    right_click(&mut h, at);
    h.get_by_label("Show Quick Access Toolbar").click();
    h.run_steps(2);
    assert!(h.state().ui.quick_access_toolbar, "Show Quick Access Toolbar brings it back");
    assert!(h.query_by_label("Show Quick Access Toolbar").is_none(), "and closes the menu");
}

fn hint_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::font_definitions(false, false));
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    ctx
}

/// Runs `frames` frames of `build` from time `t0` on 10 fps, with `events` in the first; returns
/// whether a tooltip layer is showing after the last.
fn hint_showing(ctx: &egui::Context, t0: f64, frames: u32, events: Vec<egui::Event>, build: &mut dyn FnMut(&mut Ui)) -> bool {
    let mut shown = false;
    for k in 0..frames {
        let input = egui::RawInput {
            time: Some(t0 + f64::from(k) * 0.1),
            predicted_dt: 0.1,
            events: if k == 0 { events.clone() } else { vec![] },
            ..Default::default()
        };
        ctx.run_ui(input, |ui| build(ui)).drop_without_applying_deltas();
        shown = ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|l| l.order == egui::Order::Tooltip));
    }
    shown
}

#[test]
fn a_combo_shows_its_hint_on_hover_and_not_before() {
    let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
    let ctx = hint_ctx();
    let field = std::cell::Cell::new(Rect::NOTHING);
    let mut build = |ui: &mut Ui| {
        combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
        field.set(ui.min_rect());
    };
    assert!(!hint_showing(&ctx, 1.0, 3, vec![], &mut build));
    let over = field.get().left_center() + vec2(20.0, 0.0);
    assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(over)], &mut build), "no hint over the field");
    // Over the arrow too.
    let ctx = hint_ctx();
    let mut build2 = |ui: &mut Ui| {
        combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
    };
    hint_showing(&ctx, 1.0, 3, vec![], &mut build2);
    let arrow = field.get().right_center() - vec2(6.0, 0.0);
    assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(arrow)], &mut build2), "no hint over the arrow");
}

#[test]
fn a_disabled_control_still_shows_its_hint_on_hover() {
    let ctx = hint_ctx();
    let rect = std::cell::Cell::new(Rect::NOTHING);
    let mut build = |ui: &mut Ui| {
        ui.add_enabled_ui(false, |ui| {
            let resp = ui.allocate_response(vec2(60.0, 22.0), Sense::click());
            show_hint(&resp, "Bold", &["Select text first".to_string()]);
            rect.set(resp.rect);
        });
    };
    assert!(!hint_showing(&ctx, 1.0, 3, vec![], &mut build));
    let over = rect.get().center();
    assert!(hint_showing(&ctx, 2.0, 12, vec![egui::Event::PointerMoved(over)], &mut build), "no hint over a disabled control");
}

#[test]
fn a_focus_hint_gives_way_to_a_disabled_control_under_the_pointer() {
    let ctx = hint_ctx();
    let off = std::cell::Cell::new(Rect::NOTHING);
    let mut build = |ui: &mut Ui| {
        ui.horizontal(|ui| {
            let (_, a) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
            show_hint(&a, "Bold", &[]);
            ui.add_enabled_ui(false, |ui| {
                let b = ui.allocate_response(vec2(40.0, 20.0), Sense::click());
                show_hint(&b, "Italic", &["Select text first".to_string()]);
                off.set(b.rect);
            });
        });
    };
    let tips = |ctx: &egui::Context| ctx.memory(|m| m.areas().visible_layer_ids().iter().filter(|l| l.order == egui::Order::Tooltip).count());
    hint_showing(&ctx, 1.0, 2, vec![], &mut build);
    let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let over = egui::Event::PointerMoved(off.get().center());
    // Focus the first control, rest the pointer on the disabled one: past the idle time only its reason shows.
    hint_showing(&ctx, 2.0, 2, vec![tab, over], &mut build);
    assert!(hint_showing(&ctx, 2.2, 40, vec![], &mut build), "the disabled control shows no hint");
    assert_eq!(tips(&ctx), 1, "the focused control's hint is drawn beside the disabled control's");
}

#[test]
fn a_combo_field_reached_with_tab_shows_its_hint_after_a_delay() {
    let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
    let ctx = hint_ctx();
    let focused = std::cell::Cell::new(false);
    let mut build = |ui: &mut Ui| {
        combo_hint(ui, "f", 150.0, "12", &sizes, None, "Font Size");
        focused.set(ui.memory(|m| m.focused()).is_some());
    };
    hint_showing(&ctx, 1.0, 2, vec![], &mut build);
    let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    assert!(!hint_showing(&ctx, 2.0, 2, vec![tab], &mut build));
    assert!(focused.get(), "Tab did not focus the field");
    assert!(hint_showing(&ctx, 2.2, 10, vec![], &mut build), "focused field shows no hint");
}

#[test]
fn keyboard_focus_shows_the_hint_after_a_delay_but_not_over_a_menu() {
    let ctx = hint_ctx();
    let btn_id = std::cell::Cell::new(None);
    let menu = std::cell::Cell::new(false);
    let mut build = |ui: &mut Ui| {
        let (_, resp) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
        btn_id.set(Some(resp.id));
        if menu.get() {
            egui::Popup::menu(&resp).open_memory(egui::SetOpenCommand::Bool(true)).show(|ui| ui.label("menu"));
        }
        show_hint(&resp, "Bold", &[]);
    };
    hint_showing(&ctx, 1.0, 2, vec![], &mut build);
    let Some(id) = btn_id.get() else { return };
    // Tab moves the keyboard focus onto the button, as it does for a user.
    let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // Just focused: no hint yet; after the tooltip delay: shown.
    assert!(!hint_showing(&ctx, 2.0, 2, vec![tab], &mut build));
    assert!(ctx.memory(|m| m.has_focus(id)), "Tab did not focus the button");
    assert!(hint_showing(&ctx, 2.2, 10, vec![], &mut build), "focused control shows no hint");
    // A menu open anywhere hides it.
    menu.set(true);
    assert!(!hint_showing(&ctx, 4.0, 3, vec![], &mut build), "hint drawn over an open menu");
}

#[test]
fn tooltip_lines_do_not_repeat_a_shortcut_already_in_the_label() {
    assert_eq!(tooltip_lines("Paste (⌘V)", "⌘V", "", None).0, "Paste (⌘V)");
    assert_eq!(tooltip_lines("Paste (Ctrl+V)", "Ctrl+V", "", None).0, "Paste (Ctrl+V)");
    assert_eq!(tooltip_lines("Font (dialog)", "Ctrl+D", "", None).0, "Font (dialog) (Ctrl+D)");
}

#[test]
fn tooltip_lines_name_shortcut_group_and_reason() {
    let (t, n) = tooltip_lines("Bold", "Ctrl+B", "Home > Font", None);
    assert_eq!(t, "Bold (Ctrl+B)");
    assert_eq!(n, vec!["Home > Font".to_string()]);
    let (t, n) = tooltip_lines("Header", "", "", Some("Open a document first"));
    assert_eq!(t, "Header");
    assert_eq!(n, vec!["Open a document first".to_string()]);
    let (_, n) = tooltip_lines("Header", "", "", Some("nothing is selected"));
    assert_eq!(n, vec!["Nothing is selected".to_string()]);
    // No reason given: the generic line.
    let (_, n) = tooltip_lines("Header", "", "", Some(""));
    assert_eq!(n, vec!["Not available right now".to_string()]);
}

#[test]
fn drawn_buttons_fade_in_on_hover_and_darken_while_pressed() {
    // #305's drawn replacements for ✕, 💬, ✎ and ➕ must react like egui's buttons.
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx, crate::theme::Appearance::Light);
    let clock = std::cell::Cell::new(0u32);
    let seen = std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING, Color32::PLACEHOLDER, Color32::PLACEHOLDER));
    // One 60 fps frame; returns both buttons' rects and fills (framed, frameless).
    let frame = |events: Vec<egui::Event>| {
        clock.set(clock.get() + 1);
        let input = egui::RawInput { time: Some(f64::from(clock.get()) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
        ctx.run_ui(input, |ui| {
            let framed = icon_text_button(ui, "comment", "Comments", regular(12.0));
            let frameless = icon_button(ui, "close", "Close");
            let (a, b) = (drawn_state(ui, &framed, true).fill, drawn_state(ui, &frameless, false).fill);
            seen.set((framed.rect, frameless.rect, a, b));
        })
        .drop_without_applying_deltas();
        seen.get()
    };
    // Half a second of frames, so animations finish; returns the last one.
    let settle = || {
        for _ in 0..29 {
            frame(vec![]);
        }
        frame(vec![])
    };
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let w = ctx.global_style().visuals.widgets.clone();
    let t = Tokens::light();

    let (framed, frameless, idle, idle_bare) = settle();
    assert_eq!(idle, w.inactive.weak_bg_fill);
    assert_eq!(idle_bare, Color32::TRANSPARENT);

    // Hover fades in over a few frames rather than jumping, then settles on egui's hovered fill.
    let first = frame(vec![egui::Event::PointerMoved(framed.center())]).2;
    let between = |c: Color32, a: Color32, b: Color32| a.r().min(b.r()) < c.r() && c.r() < a.r().max(b.r());
    assert!(between(first, idle, w.hovered.weak_bg_fill), "hover should animate: {idle:?} → {first:?} → {:?}", w.hovered.weak_bg_fill);
    assert_eq!(settle().2, w.hovered.weak_bg_fill);
    // Held down: blends to the pressed (active) fill.
    frame(vec![press(framed.center(), true)]);
    assert_eq!(settle().2, w.active.weak_bg_fill);
    frame(vec![press(framed.center(), false)]);

    // The frameless close button uses the ribbon's hover and pressed tokens; the other fades out.
    frame(vec![egui::Event::PointerMoved(frameless.center())]);
    let after = settle();
    assert_eq!(after.3, t.hover);
    assert_eq!(after.2, w.inactive.weak_bg_fill);
    frame(vec![press(frameless.center(), true)]);
    assert_eq!(settle().3, t.pressed);
}

#[test]
fn font_matches_puts_exact_and_prefix_before_substring() {
    let items: Vec<String> = ["Arial Black", "Arial", "Courier New", "Times New Roman", "Roman Serif"].map(String::from).to_vec();
    // Case-insensitive; the exact name first, then prefixes, then other substrings.
    assert_eq!(font_matches("ARIAL", &items), vec![1, 0]);
    assert_eq!(font_matches("roman", &items), vec![4, 3]);
    assert_eq!(font_matches(" new ", &items), vec![2, 3]);
    assert!(font_matches("zapfino", &items).is_empty());
    assert_eq!(font_matches("", &items), vec![0, 1, 2, 3, 4]);
}

/// Focuses a fresh combo, types `typed`, presses `keys` (one per frame) and returns its value.
fn type_into_combo(search: bool, items: &[String], typed: &str, keys: &[egui::Key]) -> Option<String> {
    type_into(search, items, &[], typed, keys)
}

/// [`type_into_combo`] for a font box headed by the `recent` families.
fn type_into(search: bool, items: &[String], recent: &[String], typed: &str, keys: &[egui::Key]) -> Option<String> {
    let ctx = egui::Context::default();
    let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let mut frames = vec![vec![], vec![egui::Event::Text(typed.into())]];
    frames.extend(keys.iter().map(|&k| vec![key(k)]));
    let mut out = None;
    for (n, events) in frames.into_iter().enumerate() {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if n == 0 {
                ui.memory_mut(|m| m.request_focus(ui.scope_id().with(("combo_edit", "f"))));
            }
            let v = if search { font_combo_with_recent(ui, "f", 150.0, "", items, recent, None) } else { combo(ui, "f", 150.0, "", items, None) };
            out = out.take().or(v);
        })
        .drop_without_applying_deltas();
    }
    out
}

#[test]
fn typing_in_the_font_box_finds_the_family() {
    // #359: typing "arial" or "roman" picks the installed family; the size box stays literal.
    use egui::Key::{ArrowDown, Enter, Escape};
    let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
    assert_eq!(type_into_combo(true, &fonts, "arial", &[Enter]).as_deref(), Some("Arial"));
    assert_eq!(type_into_combo(true, &fonts, "roman", &[Enter]).as_deref(), Some("Times New Roman"));
    assert_eq!(type_into_combo(true, &fonts, "arial", &[ArrowDown, Enter]).as_deref(), Some("Arial Black"));
    // Not installed: applied as typed, as Word allows.
    assert_eq!(type_into_combo(true, &fonts, "Zapfino", &[Enter]).as_deref(), Some("Zapfino"));
    assert_eq!(type_into_combo(true, &fonts, "arial", &[Escape]), None);
    let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
    assert_eq!(type_into_combo(false, &sizes, "1", &[Enter]).as_deref(), Some("1"));
}

/// #2068: with nothing typed, the recent families head the list and Enter applies the one
/// highlighted; Down walks past them into All Fonts; a query lists its matches only.
#[test]
fn recent_fonts_head_the_typed_list() {
    use egui::Key::{ArrowDown, ArrowUp, Backspace, Enter};
    let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
    let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
    // Typing, then clearing the box, leaves the list open on the recent fonts. Enter alone
    // applies nothing the user didn't pick; Down picks the second recent font.
    assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, Enter]).as_deref(), Some(""));
    assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, ArrowUp, Enter]).as_deref(), Some("Georgia"));
    assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, Enter]).as_deref(), Some("Courier New"));
    // Past the recent entries comes All Fonts; the headings are skipped.
    assert_eq!(type_into(true, &fonts, &recent, "x", &[Backspace, ArrowDown, ArrowDown, Enter]).as_deref(), Some("Arial"));
    // A query lists the matching installed families only.
    assert_eq!(type_into(true, &fonts, &recent, "cour", &[Enter]).as_deref(), Some("Courier New"));
    assert_eq!(type_into(true, &fonts, &recent, "arial", &[ArrowDown, Enter]).as_deref(), Some("Arial Black"));
    // Nothing recent, nothing typed and no key moved the highlight: the text is committed as before.
    assert_eq!(type_into(true, &fonts, &[], "x", &[Backspace, Enter]).as_deref(), Some(""));
}

/// The typed list once held one match and stayed one row tall when the query then matched
/// many: an area keeps its last size unless a changed query sizes it again.
#[test]
fn the_typed_list_grows_when_the_query_matches_more() {
    let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
    let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
    let ctx = egui::Context::default();
    let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let erase = || vec![key(egui::Key::Backspace)];
    let frames = vec![vec![], vec![egui::Event::Text("cour".into())], vec![], erase(), erase(), erase(), erase(), vec![]];
    let mut area = None;
    let mut heights = Vec::new();
    for (n, events) in frames.into_iter().enumerate() {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if n == 0 {
                ui.memory_mut(|m| m.request_focus(ui.scope_id().with(("combo_edit", "f"))));
            }
            area = Some(ui.scope_id().with(("combo_search", "f")).with("list"));
            font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
        })
        .drop_without_applying_deltas();
        heights.push(area.and_then(|id| egui::AreaState::load(&ctx, id)).and_then(|s| s.size).map(|s| s.y));
    }
    let one = heights[2].expect("one match listed");
    let all = heights[7].expect("every font listed");
    assert!(all > 4.0 * one, "one match: {one} tall; two headings, two recent and four fonts: {all}");
}

/// The typed list was built invisibly (a sizing pass) on each keystroke that changed its
/// length, so it blinked out for a frame. It is drawn on the keystroke's own frame.
#[test]
fn the_typed_list_shows_on_the_frame_of_the_keystroke() {
    fn has_text(shape: &egui::Shape, s: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text() == s,
            egui::Shape::Vec(v) => v.iter().any(|x| has_text(x, s)),
            _ => false,
        }
    }
    let fonts: Vec<String> = ["Arial", "Arial Black", "Courier New", "Times New Roman"].map(String::from).to_vec();
    let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
    let ctx = egui::Context::default();
    let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let erase = || vec![key(egui::Key::Backspace)];
    // Each keystroke and a row the list must draw on that frame: one match, the four
    // queries on the way back to a cleared box (whose list shows the recent fonts), then two
    // matches. The field itself shows only the typed text.
    let frames = [
        (vec![egui::Event::Text("cour".into())], "Courier New"),
        (erase(), "Courier New"),
        (erase(), "Courier New"),
        (erase(), "Arial Black"),
        (erase(), "Recently Used Fonts"),
        (vec![egui::Event::Text("ar".into())], "Arial Black"),
    ];
    ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.memory_mut(|m| m.request_focus(ui.scope_id().with(("combo_edit", "f"))));
        font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
    })
    .drop_without_applying_deltas();
    for (n, (events, row)) in frames.into_iter().enumerate() {
        let out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            font_combo_with_recent(ui, "f", 150.0, "", &fonts, &recent, None);
        });
        let drawn = out.shapes.iter().any(|c| has_text(&c.shape, row));
        out.drop_without_applying_deltas();
        assert!(drawn, "keystroke {n}: {row} is drawn");
    }
}

/// Clearing a query with no matches must reveal the recent section, even before the list
/// has ever had a visible viewport. A matching query is the control for a measured viewport.
#[test]
fn clearing_a_font_query_keeps_the_recent_heading_visible() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{By, Queryable};
    for query in ["a", "x"] {
        // A short list clamps the bad offset to zero and would hide this regression.
        let fonts: Vec<String> = (0..24).map(|n| format!("Family {n:02}")).collect();
        let recent: Vec<String> = ["Georgia", "Courier New"].map(String::from).to_vec();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 600.0)).build_ui(move |ui| {
            font_combo_with_recent(ui, "recent_ci_reduced", 150.0, "", &fonts, &recent, None);
        });
        h.run_steps(2);
        h.get(By::new().role(Role::TextInput)).focus();
        h.step();
        h.event(egui::Event::Text(query.into()));
        h.run_steps(2);
        assert!(h.query(By::new().role(Role::TextInput).value(query)).is_some());
        assert!(h.query_by_label("Recently Used Fonts").is_none());
        h.key_press(egui::Key::Backspace);
        h.run_steps(2);
        assert!(h.query(By::new().role(Role::TextInput).value("")).is_some());
        assert!(h.query_by_label("Recently Used Fonts").is_some(), "cleared query {query:?}");
        assert!(h.query_by_label("All Fonts").is_some());
    }
}

/// The rows of a sectioned font list, and where the keyboard highlight lands among them.
#[test]
fn font_list_sections_skip_their_headings() {
    let items: Vec<String> = ["Arial", "Courier New"].map(String::from).to_vec();
    let recent: Vec<String> = ["Georgia"].map(String::from).to_vec();
    fn names<'a>(items: &'a [String], recent: &'a [String], n: usize) -> Vec<String> {
        let item = |k: usize| items.get(k);
        (0..n)
            .map(|k| match section_row(k, recent, &item) {
                Some(Row::Heading(h)) => format!("[{h}]"),
                Some(Row::Item(i)) => i.clone(),
                None => "-".into(),
            })
            .collect()
    }
    assert_eq!(section_rows(1, 2), 5);
    assert_eq!(names(&items, &recent, 6), ["[Recently Used Fonts]", "Georgia", "[All Fonts]", "Arial", "Courier New", "-"]);
    assert_eq!([highlight_row(0, 1), highlight_row(1, 1), highlight_row(2, 1)], [1, 3, 4]);
    // Nothing recent: no headings, the rows are the items.
    assert_eq!(section_rows(0, 2), 2);
    assert_eq!(names(&items, &[], 3), ["Arial", "Courier New", "-"]);
    assert_eq!(highlight_row(1, 0), 1);
}

/// The field was rounded on all four corners and the arrow on none, so the two looked apart,
/// and their borders stood side by side as a double-width divider. The field is rounded only
/// on the left, the arrow only on the right, and the arrow's box lies under the field by one
/// border width, so a single line divides them; focused or not.
#[test]
fn combo_field_and_arrow_share_one_outline() {
    let sizes: Vec<String> = ["10", "11", "12"].map(String::from).to_vec();
    let ctx = egui::Context::default();
    let cases = [crate::theme::Appearance::Light, crate::theme::Appearance::Dark].into_iter().flat_map(|a| [(a, false), (a, true)]);
    for (appearance, focused) in cases {
        crate::theme::apply(&ctx, appearance);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if focused {
                ui.memory_mut(|m| m.request_focus(ui.scope_id().with(("combo_edit", "f"))));
            }
            combo(ui, "f", 150.0, "12", &sizes, None);
        });
        // Painted boxes in paint order: (order, rect, corners, border width).
        let boxes: Vec<(usize, Rect, CornerRadius, f32)> = std::mem::take(&mut out.shapes)
            .into_iter()
            .filter_map(|c| match c.shape {
                egui::Shape::Rect(r) => Some((r.rect, r.corner_radius, r.stroke.width)),
                _ => None,
            })
            .enumerate()
            .map(|(i, (rect, c, w))| (i, rect, c, w))
            .collect();
        out.drop_without_applying_deltas();
        let corners: Vec<CornerRadius> = boxes.iter().map(|b| b.2).collect();
        let field = boxes.iter().find(|b| b.2.nw > 0 && b.2.sw == b.2.nw && b.2.ne == 0 && b.2.se == 0);
        let field = field.unwrap_or_else(|| panic!("focused {focused}: no field rounded on the left only in {corners:?}"));
        let arrow_corners = CornerRadius { nw: 0, sw: 0, ne: field.2.nw, se: field.2.sw };
        let arrow = boxes.iter().find(|b| b.2 == arrow_corners);
        let arrow = arrow.unwrap_or_else(|| panic!("focused {focused}: no arrow rounded on the right only in {corners:?}"));
        assert!(!corners.iter().any(|c| c.nw > 0 && c.ne > 0), "focused {focused}: a part is still rounded all round: {corners:?}");
        // One divider as wide as the other borders: the arrow's left border lies exactly under
        // the field's right border, and the field is painted over it.
        assert!(field.3 > 0.0 && arrow.3 == field.3, "focused {focused}: borders {} and {} differ", field.3, arrow.3);
        assert!(
            (arrow.1.min.x - (field.1.max.x - field.3)).abs() < 0.01,
            "focused {focused}: arrow {:?} and field {:?} borders stand apart",
            arrow.1,
            field.1
        );
        assert!(arrow.0 < field.0, "focused {focused}: the arrow's box must be painted under the field");
        assert!(
            (arrow.1.min.y - field.1.min.y).abs() < 0.01 && (arrow.1.max.y - field.1.max.y).abs() < 0.01,
            "focused {focused}: the outlines' tops and bottoms differ"
        );
    }
}

#[test]
fn a_long_combo_list_builds_only_the_rows_in_view() {
    // #121: the font menu built (and so loaded the font of) every installed family.
    let items: Vec<String> = (0..5000).map(|i| format!("Family {i}")).collect();
    let ctx = egui::Context::default();
    let built = std::cell::RefCell::new(Vec::new());
    let row = |ui: &mut Ui, name: &str| {
        built.borrow_mut().push(name.to_string());
        ui.allocate_exact_size(vec2(260.0, COMBO_PREVIEW_ROW_H), Sense::click()).1
    };
    for _ in 0..3 {
        built.borrow_mut().clear();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(combo_list(ui, 150.0, "", &items, Some(&row)), None);
        })
        .drop_without_applying_deltas();
        let b = built.borrow();
        assert!(!b.is_empty() && b.len() <= 420 / COMBO_PREVIEW_ROW_H as usize + 2, "built {} rows", b.len());
        assert_eq!(b[0], "Family 0");
    }
    // Without previews too.
    ctx.run_ui(egui::RawInput::default(), |ui| {
        assert_eq!(combo_list(ui, 52.0, "8", &items, None), None);
    })
    .drop_without_applying_deltas();
}

/// Clicks the main part of a split button for `id` (one press and release) and returns whether
/// the click ran the command (a ribbon run asks for the canvas focus).
fn click_split(id: &str) -> bool {
    let session = wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one two"));
    let mut app = WordApp::new(session, crate::Services::default());
    let ctx = egui::Context::default();
    let at = std::cell::Cell::new(Rect::NOTHING);
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let frame = |events: Vec<egui::Event>, app: &mut WordApp| {
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            at.set(Rect::from_min_size(ui.cursor().min, vec2(24.0, 22.0)));
            split(ui, app, "cut", "Split", ButtonAction::new(id, Value::Object(Default::default()), false), None, |_, _| {});
        })
        .drop_without_applying_deltas();
    };
    frame(vec![], &mut app);
    let c = at.get().center();
    app.canvas.want_focus = false;
    frame(vec![egui::Event::PointerMoved(c)], &mut app);
    frame(vec![press(c, true)], &mut app);
    frame(vec![press(c, false)], &mut app);
    frame(vec![], &mut app);
    app.canvas.want_focus
}

/// A disabled split button ignores clicks; an enabled one runs its command.
#[test]
fn disabled_split_button_does_not_run_its_command() {
    assert!(click_split("format.bold"), "an enabled split button runs its command");
    // Nothing is selected, so Cut is disabled.
    assert!(!click_split("edit.cut"), "a disabled split button ran its command");
}

/// Colour split buttons draw their bar in the colour they will apply (the last one used);
/// disabled, everything follows the disabled ink.
#[test]
fn split_colour_bar_follows_the_swatch() {
    let t = Tokens::light();
    let red = Color32::from_rgb(200, 0, 0);
    assert_eq!(split_colors(true, Some(red), &t), (t.icon, red));
    assert_eq!(split_colors(true, None, &t), (t.icon, t.accent));
    assert_eq!(split_colors(false, Some(red), &t), (t.text_disabled, t.text_disabled));
    // The Shading button's swatch is the last shading colour.
    assert_eq!(crate::ribbon::shading_swatch("FF0000"), Some(Color32::from_rgb(255, 0, 0)));
    assert_eq!(crate::ribbon::shading_swatch("not a colour"), None);
}

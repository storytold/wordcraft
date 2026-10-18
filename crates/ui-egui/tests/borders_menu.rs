//! The ribbon's Borders menu (#2718): every entry carries a picture of the border it applies,
//! and the split button applies the kind last chosen from the menu (Bottom until one is), as
//! the Shading button beside it remembers its colour.

use egui_kittest::kittest::{NodeT, Queryable};
use wordcraft_doc::props::Border;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

/// A WordCraft window on the Home tab, ready for ribbon clicks.
fn home_tab() -> egui_kittest::Harness<'static, WordApp> {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.step();
    }
    h
}

fn settle(h: &mut egui_kittest::Harness<'static, WordApp>) {
    for _ in 0..3 {
        h.step();
    }
}

/// Open the Borders menu (a click on the button's arrow) and click the entry labelled `item`.
fn choose(h: &mut egui_kittest::Harness<'static, WordApp>, item: &str) {
    click_arrow(h);
    h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
        .find(|n| n.accesskit_node().label().as_deref().map(str::trim) == Some(item))
        .unwrap_or_else(|| panic!("the {item} entry is in the Borders menu"))
        .click();
    settle(h);
}

/// Click the Borders split button's arrow: the 11 pt strip at its right (widgets::split),
/// one item spacing (6 pt, theme) past the main part.
fn click_arrow(h: &mut egui_kittest::Harness<'static, WordApp>) {
    let r = h.get_by_role_and_label(egui::accesskit::Role::Button, "Borders").rect();
    let pos = egui::pos2(r.right() + 6.0 + 5.5, r.center().y);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    settle(h);
}

/// A plain click on the Borders split button's main part, which runs the command.
fn click_main(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Borders").click();
    settle(h);
}

/// Whether the paragraph at the caret has a visible border on that side.
fn side_on(h: &egui_kittest::Harness<'static, WordApp>, side: &str) -> bool {
    let app = h.state();
    let p = app.session.doc.para_at(&app.session.sel.focus).expect("a paragraph at the caret");
    let Some(b) = &p.props.borders else { return false };
    let side = match side {
        "top" => &b.top,
        "left" => &b.left,
        "right" => &b.right,
        "between" => &b.between,
        _ => &b.bottom,
    };
    side.as_ref().is_some_and(Border::is_visible)
}

#[test]
fn the_menu_lists_every_border_choice() {
    let mut h = home_tab();
    click_arrow(&mut h);
    for item in [
        "Bottom Border",
        "Top Border",
        "Left Border",
        "Right Border",
        "No Border",
        "All Borders",
        "Outside Borders",
        "Inside Borders",
        "Horizontal Line",
        "Borders and Shading…",
    ] {
        assert!(
            h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button)).any(|n| n
                .accesskit_node()
                .label()
                .as_deref()
                .map(str::trim)
                == Some(item)),
            "the {item} entry is in the Borders menu"
        );
    }
}

/// The choice pictograms are drawn in code (icons::paint), not the letter fallback, and each
/// paints exactly the accent sides its name asks for. Asserting the segment geometry catches a
/// swapped side, which shape counts cannot: Bottom, Top, Left and Right all draw 17 shapes
/// (PR #2861 review).
#[test]
fn every_border_choice_paints_its_own_sides() {
    let top = vec![((3, 3), (17, 3))];
    let bottom = vec![((3, 17), (17, 17))];
    let left = vec![((3, 3), (3, 17))];
    let right = vec![((17, 3), (17, 17))];
    let inside = vec![((3, 10), (17, 10)), ((10, 3), (10, 17))];
    let mut outside = [top.as_slice(), right.as_slice(), bottom.as_slice(), left.as_slice()].concat();
    outside.sort();
    let mut all = [outside.as_slice(), inside.as_slice()].concat();
    all.sort();
    for (name, want) in [
        ("borders_bottom", bottom.clone()),
        ("borders_top", top.clone()),
        ("borders_left", left.clone()),
        ("borders_right", right.clone()),
        ("borders_none", vec![]),
        ("borders_outside", outside.clone()),
        ("borders_inside", inside.clone()),
        ("borders_all", all),
    ] {
        assert_eq!(accent_segments(name), want, "{name} paints the wrong accent sides");
    }
    // None has no accent, so the segments alone cannot tell it from the letter fallback: its
    // pictogram is the full dotted grid.
    assert_eq!(drawn_shapes("borders_none"), 21, "the None pictogram is the dotted grid");
    // The control: an unknown name draws the fallback only (a tile and a letter).
    assert!(drawn_shapes("borders_nope") <= 2, "unknown icons use the letter fallback");
}

/// The accent line segments an icon paints, normalized: each segment's endpoints sorted and the
/// list sorted, drawn headless into a bare egui context.
fn accent_segments(icon: &str) -> Vec<((i32, i32), (i32, i32))> {
    let ctx = egui::Context::default();
    let out = ctx.run_ui(egui::RawInput::default(), |ui| {
        wordcraft_ui_egui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 20.0)),
            icon,
            egui::Color32::WHITE,
            egui::Color32::BLUE,
        );
    });
    let mut segs: Vec<((i32, i32), (i32, i32))> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(path)
                if !path.closed && path.stroke.color == egui::epaint::ColorMode::Solid(egui::Color32::BLUE) && path.points.len() == 2 =>
            {
                let (a, b) = ((path.points[0].x as i32, path.points[0].y as i32), (path.points[1].x as i32, path.points[1].y as i32));
                Some(if a <= b { (a, b) } else { (b, a) })
            }
            _ => None,
        })
        .collect();
    // The frame's texture deltas were never handed to a renderer; drop them on purpose.
    out.drop_without_applying_deltas();
    segs.sort();
    segs
}

/// How many shapes an icon name paints, drawn headless into a bare egui context.
fn drawn_shapes(icon: &str) -> usize {
    let ctx = egui::Context::default();
    let out = ctx.run_ui(egui::RawInput::default(), |ui| {
        wordcraft_ui_egui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 20.0)),
            icon,
            egui::Color32::WHITE,
            egui::Color32::BLUE,
        );
    });
    let n = out.shapes.len();
    // The frame's texture deltas were never handed to a renderer; drop them on purpose.
    out.drop_without_applying_deltas();
    n
}

#[test]
fn the_button_applies_the_kind_last_chosen_from_the_menu() {
    let mut h = home_tab();
    choose(&mut h, "Top Border");
    assert!(side_on(&h, "top"), "the menu put a top border on the paragraph");
    // The plain click repeats the last choice, so it toggles the top border back off — as in
    // Word. It must not fall back to a bottom border (#2718).
    click_main(&mut h);
    assert!(!side_on(&h, "top"), "the plain click applied the last chosen kind (top)");
    assert!(!side_on(&h, "bottom"), "the plain click did not fall back to a bottom border");
}

#[test]
fn a_fresh_button_applies_the_bottom_border() {
    let mut h = home_tab();
    click_main(&mut h);
    assert!(side_on(&h, "bottom"), "the default kind is Bottom");
}

#[test]
fn borders_and_shading_still_opens_the_dialog() {
    let mut h = home_tab();
    choose(&mut h, "Borders and Shading…");
    assert!(matches!(h.state().dialog, Some(wordcraft_ui_egui::dialogs::Dialog::Borders { .. })), "the menu item still opens Borders and Shading");
}

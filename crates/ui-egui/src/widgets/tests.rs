use craft::font_manager::Catalog;
use craft::font_manager::craft_font_catalog::{Limits, Source};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _};
use egui::Key;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use wordcraft_engine::Session;

use crate::{Services, WordApp};

fn ensure_catalog_ready() {
    let catalog = Catalog::global();
    let count = catalog.families().iter().filter(|f| !f.starts_with('.')).count();
    if count < 2 {
        for bytes in [
            include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        ] {
            let _ = catalog.add_bytes(bytes.to_vec(), Source::Bundled, &Limits::default());
        }
    }
}

fn open_ribbon_font_box(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.get_by_role_and_label(Role::ComboBox, "Font").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::ALT, Key::ArrowDown);
    h.run_steps(3);
}

fn listed_families(h: &egui_kittest::Harness<'static, WordApp>, box_rect: egui::Rect) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> = h
        .query_all_by_role(Role::TreeItem)
        .filter(|n| n.rect().min.y >= box_rect.max.y)
        .filter_map(|n| {
            let label = n.accesskit_node().label()?;
            (!label.ends_with('…')).then_some((label, n.rect().min.y))
        })
        .collect();
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    rows
}

fn current_font(h: &mut egui_kittest::Harness<'static, WordApp>) -> Option<String> {
    h.state_mut().session.run("format.state", &json!({})).ok()?.get("font").and_then(Value::as_str).map(str::to_owned)
}

/// #7317: the Home ribbon's font box shows "Recently Used Fonts" and "All Fonts" headings only
/// when recent fonts exist, and Arrow-Down skips section headings rather than landing on them.
#[test]
fn font_list_sections_skip_their_headings() {
    ensure_catalog_ready();

    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Sample")), Services::default());
    app.ui.language = "en".into();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app);
    h.run_steps(4);

    let box_rect = h.get_by_role_and_label(Role::ComboBox, "Font").rect();

    // Without recent fonts: no "Recently Used Fonts" heading, only "All Fonts".
    assert!(h.state().ui.font_picker.recent.is_empty());
    open_ribbon_font_box(&mut h);
    assert!(h.query_by_label("Recently Used Fonts").is_none(), "no recent heading without recents");
    assert!(h.query_by_label("All Fonts").is_some(), "All Fonts heading is shown");

    let all_rows = listed_families(&h, box_rect);
    assert!(all_rows.len() >= 2, "need at least two families to test section skipping: {all_rows:?}");
    let first_all = all_rows[0].0.clone();
    let chosen_recent = all_rows[1].0.clone();
    assert_ne!(chosen_recent, first_all);

    h.key_press(Key::Escape);
    h.run_steps(2);

    // Record `chosen_recent` as a recent font through a fresh app window (keeping `chosen_recent`
    // as the initial document font too, distinct from `first_all`). Opening the list starts the
    // lead past "Recently Used Fonts" on `chosen_recent`; one Arrow-Down must skip the "All Fonts"
    // heading and land on `first_all`, so Enter switches the document font to `first_all`.
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Sample")), Services::default());
    app.ui.language = "en".into();
    app.run("format.font", json!({"name": chosen_recent})).unwrap();
    assert_eq!(app.ui.font_picker.recent, [chosen_recent.as_str()]);
    assert_eq!(app.session.run("format.state", &json!({})).unwrap().get("font").and_then(Value::as_str), Some(chosen_recent.as_str()));
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app);
    h.run_steps(4);

    // With a recent font: both headings appear in order above their sections.
    let box_rect = h.get_by_role_and_label(Role::ComboBox, "Font").rect();
    open_ribbon_font_box(&mut h);
    let recent_heading_y = h.query_by_label("Recently Used Fonts").expect("Recently Used Fonts heading").rect().min.y;
    let all_heading_y = h.query_by_label("All Fonts").expect("All Fonts heading").rect().min.y;
    let sectioned_rows = listed_families(&h, box_rect);
    assert!(recent_heading_y < sectioned_rows[0].1, "Recently Used Fonts sits above the recent row");
    assert!(sectioned_rows[0].1 < all_heading_y, "recent row ({}) sits above All Fonts", sectioned_rows[0].0);
    assert!(all_heading_y < sectioned_rows[1].1, "All Fonts sits above {}", sectioned_rows[1].0);
    assert_eq!(sectioned_rows[0].0, chosen_recent);
    assert_eq!(sectioned_rows[1].0, first_all);

    // Arrow-Down from the recent entry skips "All Fonts" and lands on `first_all`.
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(current_font(&mut h).as_deref(), Some(first_all.as_str()));
}

fn to_color32(rgba: craft::theme::Rgba) -> egui::Color32 {
    egui::Color32::from(craft::theme::Srgba::from(rgba))
}

fn collect_shapes<'a>(shapes: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
    for clipped in shapes {
        fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        walk(child, out);
                    }
                }
                other => out.push(other),
            }
        }
        walk(&clipped.shape, out);
    }
}

fn find_button_fill(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> Option<(egui::Color32, egui::CornerRadius)> {
    let mut flattened = Vec::new();
    collect_shapes(shapes, &mut flattened);
    flattened.into_iter().find_map(|s| match s {
        egui::Shape::Rect(r)
            if (r.rect.min.x - rect.min.x).abs() <= 1.5
                && (r.rect.max.x - rect.max.x).abs() <= 1.5
                && (r.rect.min.y - rect.min.y).abs() <= 1.5
                && (r.rect.max.y - rect.max.y).abs() <= 1.5
                && r.fill != egui::Color32::TRANSPARENT
                && r.corner_radius != egui::CornerRadius::ZERO =>
        {
            Some((r.fill, r.corner_radius))
        }
        _ => None,
    })
}

/// #305: drawn buttons must react like standard buttons — resting at idle without a fill plate,
/// fading in through an intermediate fill on pointer hover, settling on the theme's hover state
/// token, darkening/transitioning to the pressed state token while held down, and clearing when
/// unhovered.
#[test]
fn drawn_buttons_fade_in_on_hover_and_darken_while_pressed() {
    use egui::accesskit::Role;

    let mut h = crate::test_harness::builder().with_size(egui::vec2(200.0, 100.0)).with_step_dt(0.04).build_ui(|ui| {
        ui.horizontal(|ui| {
            let _ = IconButton::new(crate::icons::id("save")).label("Save").show(ui);
        });
    });
    h.ctx.all_styles_mut(|s| s.animation_time = egui::Style::default().animation_time);
    h.step();

    let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
    let expected_hover = to_color32(env.tokens.color.state.hover);
    let expected_pressed = to_color32(env.tokens.color.state.pressed);

    let node = h.get_by_role_and_label(Role::Button, "Save");
    let rect = node.rect();
    assert!(rect.is_positive(), "Save button must have positive rect");

    // 1. Idle state: no hover or pressed fill plate
    let fill_before = find_button_fill(&h.output().shapes, rect);
    assert_eq!(fill_before, None, "Save button must have no hover fill before hover");

    // 2. Hover state: fades in through an intermediate fill, then settles on the hover token
    node.hover();
    h.run_steps(2);
    let fill_mid = find_button_fill(&h.output().shapes, rect);
    assert!(fill_mid.is_some(), "Save button must paint an intermediate fill while fading in");
    let (mid_fill, _) = fill_mid.unwrap();
    assert!(
        mid_fill != expected_hover && mid_fill.a() > 0 && mid_fill.a() < expected_hover.a(),
        "Save button hover should fade in through an intermediate alpha: got {mid_fill:?}, expected settled {expected_hover:?}"
    );

    h.run_steps(3);
    let fill_hovered = find_button_fill(&h.output().shapes, rect);
    assert!(fill_hovered.is_some(), "Save button must produce settled hover shape");
    let (hover_fill, radius) = fill_hovered.unwrap();
    assert_eq!(hover_fill, expected_hover, "Save button settled hover fill must match theme hover token");
    assert!(
        radius.nw > 0 && radius.nw == radius.ne && radius.nw == radius.sw && radius.nw == radius.se,
        "Save button corner radius must be uniformly rounded: {:?}",
        radius
    );

    // 3. Pressed state: pressing pointer button darkens/transitions to theme's pressed state token
    h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    let fill_pressed = find_button_fill(&h.output().shapes, rect);
    assert!(fill_pressed.is_some(), "Save button must have fill while pressed");
    let (pressed_fill, _) = fill_pressed.unwrap();
    assert_eq!(pressed_fill, expected_pressed, "Save button pressed fill must match theme pressed token");

    // 4. Release: releasing pointer button transitions back to hover state
    h.event(egui::Event::PointerButton {
        pos: rect.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(4);
    let fill_released = find_button_fill(&h.output().shapes, rect);
    assert!(fill_released.is_some(), "Save button must remain filled upon release");
    let (released_fill, _) = fill_released.unwrap();
    assert_eq!(released_fill, expected_hover, "Save button fill after release should return to hover token");

    // 5. Unhover: pointer leaving the button clears the fill plate
    h.event(egui::Event::PointerMoved(egui::pos2(0.0, 0.0)));
    h.run_steps(4);
    let fill_after = find_button_fill(&h.output().shapes, rect);
    assert_eq!(fill_after, None, "Save button must clear hover fill after unhovering");
}

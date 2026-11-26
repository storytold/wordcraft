use craft::font_manager::Catalog;
use craft::font_manager::craft_font_catalog::{Limits, Source};
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

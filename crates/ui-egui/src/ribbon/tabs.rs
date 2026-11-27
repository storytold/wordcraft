use craft::keymap::CommandRef;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::Button;
use craft::ui::contract::{Entry, Show as _, ShowContainer as _, Trigger, clicked};
use craft::ui::display::{Divider, Text};
use craft::ui::menu::Menu;
use craft::ui::style::TypeRole;
use egui::Ui;
use serde_json::{Value, json};

use super::WordApp;

pub const TABS: [&str; 13] = wordcraft_engine::ribbon::TABS;

/// True when the caret/selection touches a picture (#147).
pub fn has_picture_selected(s: &wordcraft_engine::Session) -> bool {
    matches!(wordcraft_engine::cmd::objects::selected(s), Some((_, wordcraft_doc::para::InlineObject::Image { .. })))
}

/// Whether the selection is a shape, text box or Drawing Canvas (so Shape Format is shown).
pub fn has_shape_selected(s: &wordcraft_engine::Session) -> bool {
    match wordcraft_engine::cmd::objects::selected(s) {
        Some((_, o)) => matches!(o, wordcraft_doc::para::InlineObject::Shape { .. }) || o.is_canvas(),
        None => false,
    }
}

/// One picker of the reading toolbar: a button that opens a value menu of `entries` and returns the command of
/// the one picked.
fn picker(ui: &mut Ui, label: String, entries: &[Entry<CommandRef>]) -> Option<CommandRef> {
    let button = Button::new(label.clone()).show(ui);
    Menu::new(label).values().opened_by(&button.response, Trigger::Press).pick(ui, entries).picked
}

/// An entry of a picker that runs `id` with `{"value": value}`.
fn choice(id: &str, value: Value, label: String, checked: bool) -> Entry<CommandRef> {
    Entry::new(CommandRef::with_params(id, json!({ "value": value })), label).checked(checked)
}

/// Reading controls replace editing tabs entirely. All settings are transient view controls.
pub(super) fn reading_toolbar(app: &mut WordApp, ui: &mut Ui) {
    let view = &app.session.view;
    let column = match view.read_column_width {
        None => "Auto",
        Some(v) if v <= 180.0 => "Narrow",
        Some(v) if v <= 240.0 => "Medium",
        Some(v) if v <= 350.0 => "Wide",
        _ => "Page",
    };
    let columns: Vec<Entry<CommandRef>> = [("Auto", "auto"), ("Narrow", "narrow"), ("Medium", "medium"), ("Wide", "wide"), ("Page", "page")]
        .into_iter()
        .map(|(name, value)| choice("view.readColumnWidth", json!(value), tl!(name).to_string(), column == name))
        .collect();
    let column_label = format!("{}: {}", tl!("Column width"), tl!(column));

    let pct = (view.read_zoom * 100.0).round() as u32;
    let sizes: Vec<Entry<CommandRef>> =
        [75, 90, 100, 110, 125, 150].into_iter().map(|p| choice("view.readTextSize", json!(p), format!("{p}%"), pct == p)).collect();
    let size_label = format!("{}: {pct}%", tl!("Text size"));

    let spacing = view.read_line_spacing;
    let mut spacings = vec![choice("view.readLineSpacing", json!("document"), tl!("Document").to_string(), spacing.is_none())];
    spacings.extend([1.0, 1.25, 1.5, 2.0].into_iter().map(|v| choice("view.readLineSpacing", json!(v), format!("{v}×"), spacing == Some(v))));
    let spacing_text = spacing.map_or_else(|| tl!("Document").to_owned(), |v| format!("{v}×"));
    let spacing_label = format!("{}: {}", tl!("Line spacing"), spacing_text);

    use wordcraft_engine::cmd::view::ReadPageColor;
    let current = view.read_page_color;
    let color_names = [
        (ReadPageColor::None, tl!("None"), "none"),
        (ReadPageColor::Sepia, tl!("Sepia"), "sepia"),
        (ReadPageColor::Inverse, tl!("Inverse"), "inverse"),
    ];
    let color_name = color_names.iter().find(|c| c.0 == current).map_or_else(|| tl!("None"), |c| c.1);
    let colors: Vec<Entry<CommandRef>> =
        color_names.iter().map(|(color, name, value)| choice("view.readPageColor", json!(value), name.to_string(), current == *color)).collect();
    let color_label = format!("{}: {}", tl!("Page color"), color_name);

    // The bar's closures only record what was picked: the commands run after the bar, which holds no `app`.
    let mut picked: Vec<CommandRef> = Vec::new();
    let mut navigation = false;
    let mut edit = false;
    Bar::new(tl!("Reading Mode").to_string(), BarKind::Options)
        .trailing(|ui, _| {
            edit = clicked(&Button::new(tl!("Edit Document").to_string()).show(ui).response);
        })
        .show(ui, |ui, _| {
            // PLAN: assumed `craft::ui::theme::TypeRole` is the re-export path of the theme's type roles.
            Text::new(tl!("Reading Mode").to_string()).role(TypeRole::Heading).show(ui);
            Divider::vertical().show(ui);
            picked.extend(picker(ui, column_label.clone(), &columns));
            Divider::vertical().show(ui);
            picked.extend(picker(ui, size_label.clone(), &sizes));
            Divider::vertical().show(ui);
            picked.extend(picker(ui, spacing_label.clone(), &spacings));
            Divider::vertical().show(ui);
            picked.extend(picker(ui, color_label.clone(), &colors));
            Divider::vertical().show(ui);
            // Headings to jump to, in the Navigation pane beside the columns (Outline view
            // is an editing layout, so it isn't offered here).
            navigation = clicked(&Button::new(tl!("Navigation Pane").to_string()).show(ui).response);
        });
    for command in picked {
        let _ = app.run(&command.id, command.params);
    }
    if navigation {
        let _ = app.run("view.navigationPane", json!({"value": true}));
        app.ui.nav_tab = "headings".into();
    }
    if edit {
        let _ = app.run("view.printLayout", json!({}));
    }
}

/// Whether the selection is a SmartArt graphic WordCraft can edit (so SmartArt Design is shown).
pub fn has_smart_art_selected(s: &wordcraft_engine::Session) -> bool {
    wordcraft_engine::cmd::smart_art::selected_smart_art(s).is_some()
}

/// Whether the selection is a chart WordCraft can edit (so Chart Design is shown).
pub fn has_chart_selected(s: &wordcraft_engine::Session) -> bool {
    wordcraft_engine::cmd::chart::selected_chart(s).is_some()
}

/// Contextual tabs for the current selection (pure, tested).
pub fn contextual_tabs(s: &wordcraft_engine::Session) -> Vec<&'static str> {
    let mut tabs = Vec::new();
    if s.sel.focus.path.cell().is_some() {
        tabs.push("Table Design");
        tabs.push("Table Layout");
    }
    if has_picture_selected(s) {
        tabs.push("Picture Format");
    }
    if has_shape_selected(s) {
        tabs.push("Shape Format");
    }
    if has_smart_art_selected(s) {
        tabs.push("SmartArt Design");
    }
    if has_chart_selected(s) {
        tabs.push("Chart Design");
    }
    // Editing an equation.
    if s.math.is_some() {
        tabs.push("Equation");
    }
    // Outline view (drawn first, next to File, as Word does).
    if s.view.mode == wordcraft_layout::ViewMode::Outline {
        tabs.push("Outlining");
    }
    tabs
}

/// Tab to show when the stored tab is no longer applicable (pure, tested).
pub fn resolve_tab<'a>(current: &'a str, available: &[&str]) -> &'a str {
    if available.contains(&current) { current } else { crate::DEFAULT_TAB }
}

/// Whether the caret is inside a table (so the contextual tabs and their badges are shown).
pub fn in_table_public(app: &WordApp) -> bool {
    app.session.sel.focus.path.cell().is_some()
}

#[cfg(test)]
mod tests {
    use crate::WordApp;

    fn collect_rects(shape: &egui::Shape, out: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.clone()),
            egui::Shape::Vec(v) => {
                for s in v {
                    collect_rects(s, out);
                }
            }
            _ => {}
        }
    }

    fn underline_x(output: egui::FullOutput, tab_bottom: f32) -> f32 {
        let mut rects = Vec::new();
        for cs in &output.shapes {
            collect_rects(&cs.shape, &mut rects);
        }
        output.drop_without_applying_deltas();
        rects
            .into_iter()
            .filter(|r| {
                r.fill != egui::Color32::TRANSPARENT
                    && (r.rect.height() - 3.0).abs() < 0.6
                    && r.rect.width() >= 15.0
                    && r.rect.width() <= 150.0
                    && r.rect.bottom() <= tab_bottom + 6.0
            })
            .map(|r| r.rect.center().x)
            .next()
            .expect("ribbon 3pt tab underline rect")
    }

    fn run_pass(ctx: &egui::Context, app: &mut WordApp, now: f64) -> egui::FullOutput {
        craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
        let input = egui::RawInput {
            time: Some(now),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 400.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            crate::test_harness::logic(ui.ctx(), app);
            app.ui(ui);
        })
    }

    /// Switching tabs slides the real ribbon's 3pt underline between tabs and reverses mid-motion.
    #[test]
    fn tab_underline_slides_and_can_reverse_mid_motion() {
        let ctx = crate::test_harness::context();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.ui.animations = crate::theme::Animations::On;

        let mut now = 1.0;
        for _ in 0..5 {
            now += 1.0 / 60.0;
            run_pass(&ctx, &mut app, now).drop_without_applying_deltas();
        }
        now += 1.0 / 60.0;
        let x0 = underline_x(run_pass(&ctx, &mut app, now), 80.0);

        app.ui.tab = "Layout".into();
        run_pass(&ctx, &mut app, now).drop_without_applying_deltas();
        now += 0.05;
        let x_mid = underline_x(run_pass(&ctx, &mut app, now), 80.0);
        assert!(x_mid > x0 + 5.0, "underline starts sliding right toward Layout: x0={x0}, x_mid={x_mid}");

        now += 0.05;
        let x_mid2 = underline_x(run_pass(&ctx, &mut app, now), 80.0);
        assert!(x_mid2 > x_mid, "underline continues sliding right: x_mid={x_mid}, x_mid2={x_mid2}");

        app.ui.tab = "Home".into();
        run_pass(&ctx, &mut app, now).drop_without_applying_deltas();
        now += 0.05;
        let x_rev = underline_x(run_pass(&ctx, &mut app, now), 80.0);
        assert!(x_rev < x_mid2, "reversing mid-motion moves underline back toward Home: x_rev={x_rev}, x_mid2={x_mid2}");

        now += 1.0;
        let x_end = underline_x(run_pass(&ctx, &mut app, now), 80.0);
        assert!((x_end - x0).abs() < 2.0, "underline settles back under Home: x0={x0}, x_end={x_end}");
    }
}

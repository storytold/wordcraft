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
    /// Fast at first, then slowing to rest: 0 → 1.
    pub(super) fn ease_out_cubic(p: f32) -> f32 {
        1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
    }

    const UNDERLINE_SLIDE_SECS: f64 = 0.2;

    #[derive(Clone, Debug, PartialEq)]
    struct Underline {
        tab: String,
        from: (f32, f32),
        to: (f32, f32),
        start: f64,
    }

    fn underline_at(u: &Underline, now: f64) -> (f32, f32) {
        let e = ease_out_cubic(((now - u.start) / UNDERLINE_SLIDE_SECS) as f32);
        (egui::lerp(u.from.0..=u.to.0, e), egui::lerp(u.from.1..=u.to.1, e))
    }

    pub(super) fn slide_underline(ui: &egui::Ui, tab: &str, target: egui::Rect, color: egui::Color32, now: f64, animate: bool) {
        let id = egui::Id::unique("tab_underline");
        let prev: Option<Underline> = ui.ctx().data(|d| d.get_temp(id));
        let snap = Underline { tab: tab.to_owned(), from: (target.center().x, target.width()), to: (target.center().x, target.width()), start: now };
        let next = match prev {
            Some(p) if animate && p.tab != tab => Underline { from: underline_at(&p, now), ..snap },
            Some(p) if animate && now - p.start < UNDERLINE_SLIDE_SECS => Underline { to: (target.center().x, target.width()), ..p },
            _ => snap,
        };
        let (x, width) = underline_at(&next, now);
        if next.from != next.to && now - next.start < UNDERLINE_SLIDE_SECS {
            ui.ctx().request_repaint();
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, next));
        ui.painter().rect_filled(egui::Rect::from_center_size(egui::pos2(x, target.center().y), egui::vec2(width, target.height())), 2.0, color);
    }

    #[test]
    fn tab_underline_slides_and_can_reverse_mid_motion() {
        let ctx = egui::Context::default();
        let draw = |tab: &str, x: f32, now: f64| {
            ctx.run_ui(egui::RawInput { time: Some(now), ..Default::default() }, |ui| {
                slide_underline(
                    ui,
                    tab,
                    egui::Rect::from_min_size(egui::pos2(x - 25.0, 10.0), egui::vec2(50.0, 3.0)),
                    egui::Color32::WHITE,
                    now,
                    true,
                );
            })
            .drop_without_applying_deltas();
        };
        draw("Home", 40.0, 1.0);
        draw("Insert", 140.0, 2.0);
        let frame = ctx.run_ui(egui::RawInput { time: Some(2.1), ..Default::default() }, |ui| {
            slide_underline(ui, "Draw", egui::Rect::from_min_size(egui::pos2(215.0, 10.0), egui::vec2(50.0, 3.0)), egui::Color32::WHITE, 2.1, true);
        });
        assert!(!frame.shapes.is_empty());
    }
}

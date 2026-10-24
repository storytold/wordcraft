//! Design › Colors › Customize Colors… and Fonts › Customize Fonts… (#1869): name a set of the
//! 12 theme colours or a heading and body font pair. Save ends in one command
//! (`design.customColors`, `design.customFonts`; a rename passes `renameFrom`), so agents get the
//! same result without the dialogs. Shown through
//! [`crate::dialogs::Dialog::Themes`].

use egui::{Sense, Ui, vec2};
use serde::Serialize;
use serde_json::json;

use crate::WordApp;
use crate::dialogs::{buttons, color_field};
use crate::theme::{Tokens, semibold};

/// The 12 theme colours' labels, in `Settings::theme_colors` order.
pub const COLOR_LABELS: [&str; 12] = [
    "Text/Background - Dark 1",
    "Text/Background - Light 1",
    "Text/Background - Dark 2",
    "Text/Background - Light 2",
    "Accent 1",
    "Accent 2",
    "Accent 3",
    "Accent 4",
    "Accent 5",
    "Accent 6",
    "Hyperlink",
    "Followed Hyperlink",
];

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum ThemeDialog {
    Colors(ColorsForm),
    Fonts(FontsForm),
}

impl ThemeDialog {
    pub fn name(&self) -> &'static str {
        match self {
            ThemeDialog::Colors(_) => "customizeColors",
            ThemeDialog::Fonts(_) => "customizeFonts",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            ThemeDialog::Colors(_) => "Customize Colors",
            ThemeDialog::Fonts(_) => "Customize Fonts",
        }
    }
}

/// The dialog `ui.dialog` opens by `name`.
pub fn open(name: &str, app: &mut WordApp) -> Option<ThemeDialog> {
    Some(match name {
        "customizeColors" => ThemeDialog::Colors(ColorsForm::read(app)),
        "customizeFonts" => ThemeDialog::Fonts(FontsForm::read(app)),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut ThemeDialog) -> bool {
    match d {
        ThemeDialog::Colors(f) => colors_ui(app, ui, f),
        ThemeDialog::Fonts(f) => fonts_ui(app, ui, f),
    }
}

/// "Custom 1", "Custom 2"… (translated): the first free one.
fn free_name(taken: impl Fn(&str) -> bool) -> String {
    (1..1000u32)
        .map(|n| crate::i18n::fmt(tl!("Custom {n}"), &[("n", &n.to_string())]))
        .find(|n| !taken(n))
        .unwrap_or_else(|| tl!("Custom").to_string())
}

/// Save: create (or, editing, update) the set. A renamed set replaces the old one in the same
/// command, so a failed save can't leave both.
fn save(app: &mut WordApp, id: &str, mut params: serde_json::Value, name: &str, editing: Option<&str>) -> Result<(), String> {
    let same = editing.is_some_and(|e| e.eq_ignore_ascii_case(name.trim()));
    params["name"] = json!(name);
    params["replace"] = json!(same);
    if let Some(old) = editing.filter(|_| !same) {
        params["renameFrom"] = json!(old);
    }
    app.run(id, params)?;
    Ok(())
}

/// Customize Colors: a name and the 12 colours (hex), with a preview.
#[derive(Clone, Debug, Serialize)]
pub struct ColorsForm {
    pub name: String,
    pub colors: Vec<String>,
    /// The custom set being edited (None: a new one).
    pub editing: Option<String>,
    /// The colours it opened with, for Reset.
    start: Vec<String>,
    pub message: String,
}

impl ColorsForm {
    /// A new set from the document's colours.
    pub fn read(app: &WordApp) -> ColorsForm {
        let colors: Vec<String> = (0..12)
            .map(|i| app.session.doc.settings.theme_colors.get(i).or_else(|| wordcraft_doc::THEME_COLORS.get(i)).copied().unwrap_or_default().hex())
            .collect();
        let c = &app.session.custom_themes;
        ColorsForm { name: free_name(|n| c.find_colors(n).is_some()), start: colors.clone(), colors, editing: None, message: String::new() }
    }

    /// Edit the custom set `name`.
    pub fn edit(app: &WordApp, name: &str) -> Option<ColorsForm> {
        let set = app.session.custom_themes.find_colors(name)?;
        let colors: Vec<String> = set.colors.iter().map(|c| c.hex()).collect();
        Some(ColorsForm { name: set.name.clone(), start: colors.clone(), colors, editing: Some(set.name.clone()), message: String::new() })
    }

    pub fn apply(&self, app: &mut WordApp) -> Result<(), String> {
        save(app, "design.customColors", json!({"colors": self.colors}), &self.name, self.editing.as_deref())
    }
}

fn swatch(ui: &mut Ui, hex: &str, size: egui::Vec2) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let t = Tokens::get(ui.ctx());
    if let Some(c) = wordcraft_doc::props::Rgb::parse(hex) {
        ui.painter().rect_filled(r, 1.0, crate::theme::c32(c));
    }
    ui.painter().rect_stroke(r, 1.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
}

/// Returns true to close.
fn colors_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ColorsForm) -> bool {
    let theme = app.session.doc.settings.theme_colors.clone();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Theme colors")).font(semibold(12.5)));
            egui::Grid::new("customize_colors").num_columns(2).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                for (i, label) in COLOR_LABELS.iter().enumerate() {
                    let text = match label.strip_prefix("Accent ") {
                        Some(n) => crate::i18n::fmt(tl!("Accent {n}"), &[("n", n)]),
                        None => tl!(label).to_string(),
                    };
                    ui.label(text);
                    if let Some(v) = f.colors.get_mut(i) {
                        ui.push_id(i, |ui| color_field(ui, &theme, v, "", ""));
                    }
                    ui.end_row();
                }
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Sample")).font(semibold(12.5)));
            colors_preview(ui, &f.colors);
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Name:"));
        ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(220.0));
        if ui.button(tl!("Reset")).clicked() {
            f.colors = f.start.clone();
        }
    });
    if !f.message.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(f.message.as_str()).small().color(ui.visuals().error_fg_color)).wrap());
    }
    let (ok, cancel) = buttons(ui, tl!("Save"));
    if ok && let Err(e) = f.apply(app) {
        f.message = e;
        return false;
    }
    ok || cancel
}

/// The preview: a page in Light 1 with a heading in Dark 2, text in Dark 1, the six accents as
/// bars and both hyperlink colours, and the 12 colours as a swatch row.
fn colors_preview(ui: &mut Ui, colors: &[String]) {
    let rgb = |i: usize| colors.get(i).and_then(|h| wordcraft_doc::props::Rgb::parse(h)).map(crate::theme::c32).unwrap_or(egui::Color32::GRAY);
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(200.0, 150.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, rgb(1));
    p.rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(13.0);
    p.text(r.min + vec2(10.0, 8.0), egui::Align2::LEFT_TOP, tl!("Heading"), egui::FontId::proportional(16.0), rgb(2));
    p.text(r.min + vec2(10.0, 32.0), egui::Align2::LEFT_TOP, tl!("Text"), font.clone(), rgb(0));
    p.text(r.min + vec2(60.0, 32.0), egui::Align2::LEFT_TOP, tl!("Hyperlink"), font.clone(), rgb(10));
    p.text(r.min + vec2(10.0, 52.0), egui::Align2::LEFT_TOP, tl!("Followed Hyperlink"), font, rgb(11));
    // The accents as a little bar chart.
    let base = r.min.y + 140.0;
    for i in 0..6 {
        let h = 20.0 + 9.0 * (i as f32 % 3.0) + 4.0 * i as f32;
        let x = r.min.x + 12.0 + i as f32 * 30.0;
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, base - h), egui::pos2(x + 22.0, base)), 1.0, rgb(4 + i));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for c in colors {
            swatch(ui, c, vec2(14.0, 14.0));
        }
    });
}

/// Customize Fonts: a name and the heading and body fonts.
#[derive(Clone, Debug, Serialize)]
pub struct FontsForm {
    pub name: String,
    pub heading: String,
    pub body: String,
    pub editing: Option<String>,
    pub message: String,
}

impl FontsForm {
    /// A new set from the document's fonts.
    pub fn read(app: &WordApp) -> FontsForm {
        let s = &app.session.doc.settings;
        let c = &app.session.custom_themes;
        FontsForm {
            name: free_name(|n| c.find_fonts(n).is_some()),
            heading: s.major_font.clone(),
            body: s.minor_font.clone(),
            editing: None,
            message: String::new(),
        }
    }

    pub fn edit(app: &WordApp, name: &str) -> Option<FontsForm> {
        let set = app.session.custom_themes.find_fonts(name)?;
        Some(FontsForm {
            name: set.name.clone(),
            heading: set.heading.clone(),
            body: set.body.clone(),
            editing: Some(set.name.clone()),
            message: String::new(),
        })
    }

    pub fn apply(&self, app: &mut WordApp) -> Result<(), String> {
        save(app, "design.customFonts", json!({"heading": self.heading, "body": self.body}), &self.name, self.editing.as_deref())
    }
}

/// Returns true to close.
fn fonts_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FontsForm) -> bool {
    let fams = app.previews.families();
    let prev = app.previews.font_preview_fn();
    egui::Grid::new("customize_fonts").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for (label, id, value) in [(tl!("Heading font:"), "theme_heading_font", &mut f.heading), (tl!("Body font:"), "theme_body_font", &mut f.body)]
        {
            ui.label(label);
            if let Some(v) = crate::widgets::font_combo(ui, id, 220.0, value, &fams, Some(&*prev)) {
                if v == crate::widgets::ADD_FONT {
                    let _ = app.run("font.add", json!({}));
                } else if v == crate::widgets::LOAD_SYSTEM_FONTS {
                    let _ = app.run("font.loadSystem", json!({}));
                } else {
                    *value = v;
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Sample")).font(semibold(12.5)));
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
        ui.set_width(300.0);
        prev(ui, &f.heading);
        prev(ui, &f.body);
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Name:"));
        ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(220.0));
    });
    if !f.message.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(f.message.as_str()).small().color(ui.visuals().error_fg_color)).wrap());
    }
    let (ok, cancel) = buttons(ui, tl!("Save"));
    if ok && let Err(e) = f.apply(app) {
        f.message = e;
        return false;
    }
    ok || cancel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use crate::dialogs::Dialog;
    use serde_json::json;
    use wordcraft_engine::Session;

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// The ribbon's Customize Colors… opens the dialog with the document's colours; Save creates
    /// the named set and applies all 12 colours.
    #[test]
    fn customize_colors_dialog_creates_and_applies() {
        let mut a = app();
        assert_eq!(a.run("design.customColors", json!({})).unwrap(), json!({"pending": "customizeColors"}));
        let Some(Dialog::Themes(d)) = &mut a.dialog else { panic!("dialog: {:?}", a.dialog.as_ref().map(Dialog::name)) };
        let ThemeDialog::Colors(form) = d.as_mut() else { panic!("colors form") };
        assert_eq!(form.colors.len(), 12);
        assert_eq!(form.colors[4], a.session.doc.settings.theme_colors[4].hex());
        form.name = "Harbour".into();
        form.colors[4] = "123456".into();
        form.colors[11] = "ABCDEF".into();
        let form = form.clone();
        form.apply(&mut a).unwrap();
        assert_eq!(a.session.doc.settings.theme_colors[4], wordcraft_doc::props::Rgb(0x12, 0x34, 0x56));
        assert_eq!(a.session.doc.settings.theme_colors[11], wordcraft_doc::props::Rgb(0xAB, 0xCD, 0xEF));
        assert!(a.session.custom_themes.find_colors("Harbour").is_some());

        // Edit… opens it filled in; renaming replaces the old set.
        let mut f = ColorsForm::edit(&a, "Harbour").unwrap();
        assert_eq!(f.colors[4], "123456");
        f.name = "Harbour 2".into();
        f.apply(&mut a).unwrap();
        assert!(a.session.custom_themes.find_colors("Harbour").is_none());
        assert!(a.session.custom_themes.find_colors("Harbour 2").is_some());
        // A bad colour is the dialog's message, not a change.
        let mut bad = ColorsForm::edit(&a, "Harbour 2").unwrap();
        bad.colors[0] = "nope".into();
        assert!(bad.apply(&mut a).is_err());
    }

    #[test]
    fn customize_fonts_dialog_creates_and_applies() {
        let mut a = app();
        assert_eq!(a.run("design.customFonts", json!({})).unwrap(), json!({"pending": "customizeFonts"}));
        let Some(Dialog::Themes(d)) = &mut a.dialog else { panic!("dialog") };
        let ThemeDialog::Fonts(form) = d.as_mut() else { panic!("fonts form") };
        assert_eq!(form.heading, a.session.doc.settings.major_font);
        form.name = "Bookish".into();
        form.heading = "Palatino".into();
        form.body = "Optima".into();
        let form = form.clone();
        form.apply(&mut a).unwrap();
        assert_eq!(a.session.doc.settings.major_font, "Palatino");
        assert_eq!(a.session.doc.settings.minor_font, "Optima");
        assert!(a.session.custom_themes.find_fonts("Bookish").is_some());
    }

    /// Save Current Theme… asks where and writes the .thmx; Browse for Themes… asks for one and
    /// applies it.
    #[test]
    fn save_and_browse_for_themes_go_through_the_file_picker() {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Picked.thmx").to_string_lossy().to_string();
        let (p1, p2) = (path.clone(), path.clone());
        let services = Services {
            pick_save: Some(Box::new(move |_| Some(p1.clone()))),
            pick_open: Some(Box::new(move |_| Some(p2.clone()))),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("design.customFonts", json!({"name": "Bookish", "heading": "Palatino", "body": "Optima"})).unwrap();
        a.run("design.saveTheme", json!({})).unwrap();
        assert!(std::path::Path::new(&path).is_file(), "Save Current Theme wrote the picked file");
        a.run("design.theme", json!({"name": "Studio"})).unwrap();
        assert_eq!(a.session.doc.settings.major_font, "Georgia");
        a.run("design.openTheme", json!({})).unwrap();
        assert_eq!(a.session.doc.settings.major_font, "Palatino");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A theme colour is always a colour: the slots' menus offer no "Automatic" (an empty slot
    /// would make Save fail).
    #[test]
    fn colour_slots_offer_no_automatic() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("design.customColors", json!({})).unwrap();
        let dark1 = a.session.doc.settings.theme_colors[0].hex();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_all_by_label(&dark1).next().unwrap().click();
        h.run_steps(4);
        assert!(h.query_all_by_value("Theme Colors").next().is_some(), "the slot's menu is open");
        assert!(h.query_by_label("Automatic").is_none(), "no Automatic in a theme colour's menu");
    }
}

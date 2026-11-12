//! Define New List Style (Home › Paragraph › Multilevel List, #3805): a named list style whose
//! levels every list using it follows. The form reuses Define New Multilevel List's levels, preview
//! and fields (Format › Numbering…), and the number's Font dialog from Define New Number Format
//! (Format › Font…). OK runs one `list.defineStyle`, so agents get the same style without it.

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::dialogs::{Dialog, buttons};
use wordcraft_engine::cmd::lists::LEVELS;

use crate::dialogs_lists::{ListForm, STYLES, list_fields, preview_width, style_label};
use crate::theme::semibold;

/// A size as the ribbon's size box writes it: 11, 10.5.
fn size_text(s: f32) -> String {
    if s.fract() == 0.0 { format!("{s:.0}") } else { format!("{s}") }
}

/// The Define New List Style form: the style's name and its nine levels.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListStyleForm {
    pub name: String,
    /// The levels and the one formatting applies to (Define New Multilevel List's form).
    pub list: ListForm,
    /// Format › Font… open over this dialog: the app's Font dialog for the level's number.
    #[serde(skip)]
    pub font_dialog: Option<Box<Dialog>>,
    /// Format › Numbering… open over this dialog: the levels in Define New Multilevel List's form,
    /// a copy taken back on OK.
    #[serde(skip)]
    pub numbering: Option<Box<ListForm>>,
}

impl ListStyleForm {
    /// A new style: the first free name of Style1, Style2… and a new list style's levels.
    pub fn new(app: &WordApp) -> ListStyleForm {
        let styles = &app.session.doc.styles;
        let name = (1..=9999).map(|n| format!("Style{n}")).find(|n| styles.find(n).is_none()).unwrap_or_default();
        let list = ListForm::with_levels(app, &wordcraft_engine::cmd::lists::list_style_levels());
        ListStyleForm { name, list, font_dialog: None, numbering: None }
    }

    /// `list.defineStyle` parameters.
    pub fn params(&self) -> Value {
        json!({"name": self.name.trim(), "levels": self.list.params()["levels"], "apply": true})
    }

    /// The level formatting applies to.
    fn level(&mut self) -> Option<&mut crate::dialogs_lists::LevelForm> {
        let i = self.list.level.min(LEVELS - 1);
        self.list.levels.get_mut(i)
    }

    /// The level's number formatting in `format.set`'s shape, for the Font dialog.
    fn number_props(&self) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        let Some(l) = self.list.levels.get(self.list.level.min(LEVELS - 1)) else { return m };
        if !l.font.trim().is_empty() {
            m.insert("font".into(), json!(l.font));
        }
        if l.size > 0.0 {
            m.insert("size".into(), json!(l.size));
        }
        m.insert("bold".into(), json!(l.bold));
        m.insert("italic".into(), json!(l.italic));
        m.insert("underline".into(), json!(if l.underline { "single" } else { "none" }));
        if let Some(c) = wordcraft_doc::Rgb::parse(&l.color) {
            m.insert("color".into(), json!({"Rgb": [c.0, c.1, c.2]}));
        }
        m
    }

    /// Take what the Font dialog changed (font, size, bold, italic, underline, colour) into the level.
    fn take_number_props(&mut self, changed: &serde_json::Map<String, Value>) {
        let Some(l) = self.level() else { return };
        if let Some(f) = changed.get("font").and_then(Value::as_str) {
            l.font = f.to_string();
        }
        if let Some(s) = changed.get("size").and_then(Value::as_f64).filter(|s| s.is_finite()) {
            l.size = (s as f32).clamp(1.0, 1638.0);
        }
        if let Some(b) = changed.get("bold").and_then(Value::as_bool) {
            l.bold = b;
        }
        if let Some(b) = changed.get("italic").and_then(Value::as_bool) {
            l.italic = b;
        }
        if let Some(u) = changed.get("underline").and_then(Value::as_str) {
            l.underline = u != "none";
        }
        match changed.get("color") {
            Some(Value::Object(c)) => {
                if let Some([r, g, b]) =
                    c.get("Rgb").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect::<Vec<_>>()).as_deref()
                {
                    l.color = wordcraft_doc::Rgb(*r.min(&255) as u8, *g.min(&255) as u8, *b.min(&255) as u8).hex();
                }
            }
            // The Font dialog sends Automatic as "Auto".
            Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => l.color.clear(),
            _ => {}
        }
    }

    /// The level's settings in words, under the preview.
    pub fn description(&self) -> String {
        let i = self.list.level.min(LEVELS - 1);
        let Some(l) = self.list.levels.get(i) else { return String::new() };
        let unit = crate::options::unit();
        let k = unit.pt_per_unit();
        let style = STYLES.iter().find(|(f, _)| *f == l.format).map(|(_, s)| style_label(s)).unwrap_or_default();
        [
            crate::i18n::fmt(tl!("Level {n}"), &[("n", &(i + 1).to_string())]),
            style,
            format!("{} {}", tl!("Start at:"), l.start),
            format!("{} {}", tl!("Aligned at:"), unit.format(l.aligned_at * k)),
            format!("{} {}", tl!("Text indent at:"), unit.format(l.indent_at * k)),
        ]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

/// Make `l` a bullet level showing `c`, in the font Home › Bullets gives that bullet.
fn set_bullet(l: &mut crate::dialogs_lists::LevelForm, c: &str) {
    l.format = "bullet".into();
    l.text = c.to_string();
    let kind = wordcraft_doc::ListKind::parse(c).unwrap_or(wordcraft_doc::ListKind::Bullet);
    l.font = wordcraft_doc::numbering::levels_for(kind).first().and_then(|x| x.chr.font.clone()).unwrap_or_default();
}

/// Define New List Style. Returns true to close.
pub fn body(app: &mut WordApp, ui: &mut Ui, f: &mut ListStyleForm) -> bool {
    // A dialog open over this one (Format › Font… or Numbering…): this one waits, as in Word.
    if f.font_dialog.is_some() || f.numbering.is_some() {
        ui.disable();
    }
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    heading(ui, "Properties");
    egui::Grid::new("dls_properties").num_columns(2).spacing(vec2(12.0, 6.0)).min_col_width(140.0).show(ui, |ui| {
        let label = ui.label(tl!("Name:"));
        ui.add(egui::TextEdit::singleline(&mut f.name).char_limit(253).desired_width(300.0)).labelled_by(label.id);
        ui.end_row();
        let label = ui.label(tl!("Style type:"));
        // Always a list style here.
        ui.add_enabled_ui(false, |ui| {
            craft_ui::ComboBox::from_id_salt("dls_type").width(300.0).selected_text(tl!("List")).show_ui(ui, |_| {}).response.labelled_by(label.id);
        });
        ui.end_row();
    });
    heading(ui, "Formatting");
    let unit = crate::options::unit();
    let quarter = 18.0 / unit.pt_per_unit();
    egui::Grid::new("dls_formatting").num_columns(2).spacing(vec2(12.0, 6.0)).min_col_width(140.0).show(ui, |ui| {
        let label = ui.label(tl!("Start at:"));
        if let Some(l) = f.level() {
            ui.add(egui::DragValue::new(&mut l.start).range(0..=32_767)).labelled_by(label.id);
        }
        ui.end_row();
        let label = ui.label(tl!("Apply formatting to:"));
        let level_name = |i: usize| crate::i18n::fmt(tl!("Level {n}"), &[("n", &(i + 1).to_string())]);
        let combo = craft_ui::ComboBox::from_id_salt("dls_level").width(300.0).selected_text(level_name(f.list.level)).show_ui(ui, |ui| {
            for i in 0..LEVELS {
                ui.selectable_value(&mut f.list.level, i, level_name(i));
            }
        });
        combo.response.labelled_by(label.id);
        ui.end_row();
    });
    ui.add_space(4.0);
    // The level's number: font, size, bold, italic, underline and colour, with the ribbon's own
    // font and size boxes and buttons, and the Font dialog's colour field.
    let theme = app.session.doc.settings.theme_colors.clone();
    let shown = f.list.levels.get(f.list.level.min(LEVELS - 1)).map(|l| app.previews.font_shown(&l.font)).unwrap_or_default();
    let fams = app.previews.families();
    let recent: Vec<String> = app.ui.recent_fonts.iter().map(|x| app.previews.font_shown(x)).collect();
    let prev = app.previews.font_preview_fn();
    let sizes: Vec<String> = wordcraft_engine::cmd::format::SIZES.iter().map(|s| size_text(*s)).collect();
    let mut picked = None;
    crate::widgets::row(ui, |ui| {
        picked = crate::widgets::font_combo_with_recent_hint(
            ui,
            crate::widgets::FontComboOptions {
                id: "dls_font",
                width: 150.0,
                current: &shown,
                items: &fams,
                recent: &recent,
                preview: Some(&*prev),
                tip: tl!("Font"),
                shown_as: None,
            },
        );
        let Some(l) = f.level() else { return };
        let size = if l.size > 0.0 { size_text(l.size) } else { String::new() };
        if let Some(v) = crate::widgets::combo_hint(ui, "dls_size", 52.0, &size, &sizes, None, tl!("Font Size")) {
            // Empty: the paragraph's size; anything else that isn't a size is ignored.
            match v.trim() {
                "" => l.size = 0.0,
                v => {
                    if let Some(x) = v.parse::<f32>().ok().filter(|x| x.is_finite() && *x >= 1.0) {
                        l.size = x.min(1638.0);
                    }
                }
            }
        }
        ui.add_space(3.0);
        for (icon, tip, on) in [("bold", "Bold", &mut l.bold), ("italic", "Italic", &mut l.italic), ("underline", "Underline", &mut l.underline)] {
            if crate::widgets::small_toggle(ui, icon, tl!(tip), *on, true).clicked() {
                *on = !*on;
            }
        }
        ui.add_space(3.0);
        crate::dialogs::color_field(ui, &theme, &mut l.color, "Automatic", "");
    });
    if let Some(p) = picked {
        if p == crate::widgets::ADD_FONT {
            let _ = app.run("font.add", json!({}));
        } else if p == crate::widgets::LOAD_SYSTEM_FONTS {
            let _ = app.run("font.loadSystem", json!({}));
        } else {
            let stored = app.previews.font_stored(&p);
            if let Some(l) = f.level() {
                l.font = stored;
            }
        }
    }
    // The level's number style, a bullet's symbol and picture, and its indents.
    let i = f.list.level.min(LEVELS - 1);
    crate::widgets::row(ui, |ui| {
        let Some(l) = f.level() else { return };
        let bullet = l.format == "bullet";
        if crate::widgets::small_toggle(ui, "numbering", tl!("Numbering"), !bullet, true).clicked() && bullet {
            l.format = "decimal".into();
            l.text = format!("%{})", i + 1);
        }
        if crate::widgets::small_toggle(ui, "bullets", tl!("Bullets"), bullet, true).clicked() && !bullet {
            set_bullet(l, "•");
        }
        ui.add_space(3.0);
        // The number styles, then the Bullet Library as Home › Bullets shows it.
        let current = if bullet {
            // The interface fonts have no ➢ (the tile draws it).
            let c = if l.text == crate::ribbon::ARROW_BULLET { "" } else { l.text.as_str() };
            format!("{c} {}", tl!("Bullet")).trim().to_string()
        } else {
            style_label(STYLES.iter().find(|(k, _)| *k == l.format).map(|(_, s)| *s).unwrap_or(""))
        };
        let combo = craft_ui::ComboBox::from_id_salt("dls_style").width(170.0).height(480.0).selected_text(current).show_ui(ui, |ui| {
            for (k, s) in STYLES.into_iter().filter(|(k, _)| *k != "bullet") {
                if ui.selectable_label(l.format == k, style_label(s)).clicked() {
                    if l.format == "bullet" || !l.text.contains('%') {
                        l.text = format!("%{})", i + 1);
                    }
                    l.format = k.to_string();
                }
            }
            ui.separator();
            ui.label(egui::RichText::new(tl!("Bullet Library")).small().weak());
            ui.horizontal(|ui| {
                for c in crate::ribbon::BULLET_LIBRARY {
                    if crate::ribbon::bullet_tile(ui, c).clicked() {
                        set_bullet(l, c);
                        ui.close();
                    }
                }
            });
        });
        // No label beside it: the box is named for screen readers as Word's is.
        combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("Number style for this level:")));
        ui.add_space(3.0);
        // A bullet's symbol (the Symbol dialog picking one, #2860) and picture bullets are later.
        crate::widgets::small_toggle(ui, "symbol", tl!("Symbol"), false, false);
        crate::widgets::small_toggle(ui, "picture", tl!("Picture"), false, false);
        ui.add_space(3.0);
        if crate::widgets::small_toggle(ui, "outdent", tl!("Decrease Indent"), false, true).clicked() {
            let by = quarter.min(l.aligned_at.max(0.0));
            l.aligned_at -= by;
            l.indent_at = (l.indent_at - by).max(0.0);
        }
        if crate::widgets::small_toggle(ui, "indent", tl!("Increase Indent"), false, true).clicked() {
            l.aligned_at += quarter;
            l.indent_at += quarter;
        }
    });
    ui.add_space(4.0);
    preview_width(ui, &f.list, ui.available_width());
    // The level in words, as Manage Styles describes a style.
    ui.add_space(4.0);
    ui.add(egui::Label::new(egui::RichText::new(f.description()).small()).wrap());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut here = true;
        ui.radio_value(&mut here, true, tl!("Only in this document"));
        ui.add_enabled_ui(false, |ui| ui.radio_value(&mut here, false, tl!("New documents based on this template")));
    });
    ui.add_space(4.0);
    let (ok, cancel) = ui
        .horizontal(|ui| {
            ui.menu_button(format!("{} ▾", tl!("Format")), |ui| {
                if ui.button(tl!("Font…")).clicked() {
                    let props = f.number_props();
                    f.font_dialog = crate::dialogs::number_font_dialog(app, &props).map(Box::new);
                    ui.close();
                }
                // What a list style doesn't have, greyed out as in Word.
                for s in ["Paragraph…", "Tabs…", "Border…", "Language…", "Frame…"] {
                    ui.add_enabled(false, egui::Button::new(tl!(s)));
                }
                if ui.button(tl!("Numbering…")).clicked() {
                    f.numbering = Some(Box::new(f.list.clone()));
                    ui.close();
                }
                // Keys for a style (#3910) and text effects for a list style: not yet.
                for s in ["Shortcut key…", "Text Effects…"] {
                    ui.add_enabled(false, egui::Button::new(tl!(s)));
                }
            });
            buttons(ui, tl!("OK"))
        })
        .inner;
    // Format › Font… or Numbering… over this dialog: Enter and Escape are theirs.
    if let Some(mut d) = f.font_dialog.take() {
        match crate::dialogs::show_number_font(app, ui.ctx(), &mut d) {
            None => f.font_dialog = Some(d),
            Some(true) => {
                if let Some(changed) = crate::dialogs::number_font_result(app, &d) {
                    f.take_number_props(&changed);
                }
            }
            Some(false) => {}
        }
        return false;
    }
    if let Some(mut list) = f.numbering.take() {
        match show_numbering(ui.ctx(), &mut list) {
            None => f.numbering = Some(list),
            Some(true) => f.list = *list,
            Some(false) => {}
        }
        return false;
    }
    if ok {
        if f.name.trim().is_empty() {
            app.status(tl!("Type a name for the style."));
            return false;
        }
        // list.defineStyle checks the name: a refused one keeps the dialog open, with the reason
        // in the status bar.
        if let Err(e) = app.run("list.defineStyle", f.params()) {
            app.status(e);
            return false;
        }
        return true;
    }
    cancel
}

/// Format › Numbering…: Define New Multilevel List's fields for the style's levels, over the list
/// style dialog. `Some(true)` on OK, `Some(false)` on Cancel or its close button, `None` while open.
fn show_numbering(ctx: &egui::Context, f: &mut ListForm) -> Option<bool> {
    let (mut open, mut done) = (true, None);
    egui::Window::new(tl!("Define New Multilevel List"))
        .id(egui::Id::unique(("dialog", "Define New List Style", "numbering")))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -20.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            list_fields(ui, f);
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok || cancel {
                done = Some(ok);
            }
        });
    if !open { Some(false) } else { done }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// The dialog on its own: state = (app, form, what it returned once drawn).
    fn harness(a: WordApp, form: super::ListStyleForm) -> egui_kittest::Harness<'static, (WordApp, super::ListStyleForm, Option<bool>)> {
        egui_kittest::Harness::builder().with_size(egui::vec2(620.0, 760.0)).build_ui_state(
            |ui, st: &mut (WordApp, super::ListStyleForm, Option<bool>)| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    crate::theme::install_fonts(ui.ctx());
                    ui.ctx().request_repaint();
                    return;
                }
                if st.2 != Some(true) {
                    st.2 = Some(super::body(&mut st.0, ui, &mut st.1));
                }
            },
            (a, form, None),
        )
    }

    /// #3805: Define New List Style opens from its command without settings, named Style1 with the
    /// levels `1)`, `a)`, `i)`; the formatting row changes the level chosen in Apply formatting to;
    /// OK makes the style and numbers the paragraph with it; the next dialog takes the first free
    /// StyleN name (still Style1, the style made being named Steps).
    #[test]
    fn define_new_list_style_dialog_makes_a_style_and_applies_it() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("text.insert", json!({"text": "Mix the clay"})).unwrap();
        assert_eq!(a.run("list.defineStyle", json!({})).unwrap(), json!({"pending": "defineListStyle"}));
        let Some(Dialog::DefineListStyle { form }) = &a.dialog else { panic!("dialog") };
        assert_eq!(form.name, "Style1");
        assert_eq!((form.list.sample(0), form.list.sample(1), form.list.sample(3)), ("1)".to_string(), "a)".to_string(), "(1)".to_string()));
        let mut form = (**form).clone();
        a.dialog = None;
        form.name = "Steps".into();
        let mut h = harness(a, form);
        h.run();
        // Bold, then Bullets, on level 1.
        h.get_by_label("Bold").click();
        h.run();
        assert!(h.state().1.list.levels[0].bold);
        h.get_by_label("Bullets").click();
        h.run();
        assert_eq!((h.state().1.list.levels[0].format.as_str(), h.state().1.list.levels[0].text.as_str()), ("bullet", "•"));
        h.get_by_label("Numbering").click();
        h.run();
        assert_eq!(h.state().1.list.sample(0), "1)");
        // Increase Indent moves the level a quarter inch (18 pt) in.
        let before = h.state().1.list.levels[0].indent_at;
        h.get_by_label("Increase Indent").click();
        h.run();
        let k = crate::options::unit().pt_per_unit();
        assert!(((h.state().1.list.levels[0].indent_at - before) * k - 18.0).abs() < 0.01);
        assert!(h.state().1.description().contains("Level 1"));
        assert_eq!(h.state().2, Some(false), "still open");
        h.get_by_label("OK").click();
        h.run();
        assert_eq!(h.state().2, Some(true), "OK closes");
        let a = &mut h.state_mut().0;
        let id = a.session.doc.styles.find("Steps").map(|st| st.id.clone()).expect("the style");
        let def = a.session.doc.numbering.style_definition(&id).expect("its definition");
        assert!(def.levels[0].chr.bold == Some(true));
        let num = a.session.doc.body.iter().filter_map(|b| b.as_para()).next().and_then(|p| p.props.numbering).map(|n| n.num).unwrap();
        assert_eq!(a.session.doc.numbering.abstract_of(num).and_then(|x| x.num_style_link.clone()), Some(id));
        // The next one is named after the first free StyleN.
        a.run("ui.dialog", json!({"name": "defineListStyle"})).unwrap();
        assert!(matches!(&a.dialog, Some(Dialog::DefineListStyle { form }) if form.name == "Style1"));
    }

    /// #3805: OK with a name in use or no name keeps the dialog open and adds nothing; Cancel adds
    /// nothing.
    #[test]
    fn define_new_list_style_dialog_refuses_a_taken_name() {
        use egui_kittest::kittest::Queryable;
        let a = app();
        let mut form = super::ListStyleForm::new(&a);
        form.name = "Heading 1".into();
        let styles = a.session.doc.styles.styles.len();
        let mut h = harness(a, form);
        h.run();
        h.get_by_label("OK").click();
        h.run();
        assert_eq!(h.state().2, Some(false), "still open");
        h.state_mut().1.name = "  ".into();
        h.get_by_label("OK").click();
        h.run();
        assert_eq!(h.state().2, Some(false), "still open");
        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().2, Some(true));
        assert_eq!(h.state().0.session.doc.styles.styles.len(), styles);
    }

    /// #3805: Format › Numbering… edits the levels in Define New Multilevel List's form over the
    /// dialog; its OK takes them back, its Cancel doesn't.
    #[test]
    fn format_numbering_edits_the_style_levels() {
        let a = app();
        let mut form = super::ListStyleForm::new(&a);
        let mut copy = form.list.clone();
        copy.levels[0].text = "Step %1:".into();
        form.numbering = Some(Box::new(copy));
        let mut h = harness(a, form);
        h.run();
        assert!(h.state().1.numbering.is_some(), "open over the dialog");
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(h.state().1.numbering.is_none());
        assert_eq!(h.state().1.list.sample(0), "Step 1:");
    }

    /// #3805: the number style list ends with the Bullet Library of Home › Bullets; a bullet from
    /// it takes the font that menu gives it, and a number style after it puts the number back.
    #[test]
    fn a_bullet_from_the_library_takes_its_font() {
        let a = app();
        let mut form = super::ListStyleForm::new(&a);
        let l = &mut form.list.levels[0];
        for c in crate::ribbon::BULLET_LIBRARY {
            super::set_bullet(l, c);
            let want = wordcraft_doc::numbering::levels_for(wordcraft_doc::ListKind::parse(c).unwrap())[0].chr.font.clone().unwrap_or_default();
            assert_eq!((l.format.as_str(), l.text.as_str(), l.font.as_str()), ("bullet", c, want.as_str()));
        }
        assert_eq!(form.list.sample(0), "–");
        let params = form.params();
        assert_eq!((params["levels"][0]["format"].as_str(), params["levels"][0]["text"].as_str()), (Some("bullet"), Some("–")));
    }

    /// #3805: the level's number formatting from Format › Font… goes into the level and back.
    #[test]
    fn format_font_sets_the_level_number_formatting() {
        let a = app();
        let mut form = super::ListStyleForm::new(&a);
        let mut changed = serde_json::Map::new();
        changed.insert("size".into(), json!(14.0));
        changed.insert("italic".into(), json!(true));
        changed.insert("underline".into(), json!("single"));
        changed.insert("color".into(), json!({"Rgb": [200, 30, 40]}));
        form.take_number_props(&changed);
        let l = &form.list.levels[0];
        assert_eq!((l.size, l.italic, l.underline, l.color.as_str()), (14.0, true, true, "C81E28"));
        let back = form.number_props();
        assert_eq!((back["size"].as_f64(), back["underline"].as_str()), (Some(14.0), Some("single")));
        changed.clear();
        changed.insert("color".into(), json!("Auto"));
        form.take_number_props(&changed);
        assert!(form.list.levels[0].color.is_empty());
    }
}

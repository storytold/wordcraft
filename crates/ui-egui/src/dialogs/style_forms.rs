//! New Style and Modify Style. Each ends in one command (`styles.create`, `styles.modify`), held until the
//! command answers so a refusal shows in the dialog.

use std::sync::Arc;

use craft::behavior::{SelectionChange, keyed};
use craft::font_manager::Catalog;
use craft::font_manager::picker::FontPickerSettings;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, FormDialog, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::Section;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::{NumberField, TextField};
use egui::{Context, Ui};
use serde_json::{Value, json};
use wordcraft_geom::Unit;

use super::host::{DialogHooks, Settle, open_with};
use super::styles::{ManageStylesDialog, label};
use crate::WordApp;

const NEW_KEY: &str = "newStyle";
const MODIFY_KEY: &str = "modifyStyle";

/// The values of Modify Style.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModifyStyleForm {
    id: String,
    name: String,
    font: String,
    size: f64,
    bold: bool,
    italic: bool,
    /// `RRGGBB`, or "auto".
    color: String,
    before: f64,
    after: f64,
    /// Visual alignment (`left`, `center`, `right`, `justify`, or `distribute`).
    align: String,
    /// Visual left indent in points.
    left: f64,
    /// Visual right indent in points.
    right: f64,
    /// First-line indent in points (negative = hanging).
    first: f64,
    /// Multiple line spacing (`None` while an untouched non-multiple rule such as `Exactly` or `AtLeast` is kept).
    line: Option<f64>,
    /// Non-multiple line spacing (`Exactly` or `AtLeast`) if the style had one when opened (#5833).
    non_multiple_line: Option<wordcraft_doc::props::LineSpacing>,
    /// True if this is a character style (paragraph formatting hidden).
    is_char: bool,
    /// Right-to-left paragraph text direction.
    bidi: bool,
    /// Current tab index: 0 Font, 1 Paragraph.
    tab: u8,
    /// Interface measurement unit for length fields.
    unit: Unit,
}

impl ModifyStyleForm {
    /// What the style resolves to now. `None` for an unknown style.
    fn read(app: &WordApp, id: &str) -> Option<ModifyStyleForm> {
        let st = app.session.doc.styles.get(id)?;
        let rc = app.session.doc.styles.resolve_char(
            if st.kind == wordcraft_doc::StyleKind::Paragraph { Some(id) } else { None },
            &wordcraft_doc::CharProps {
                style: if st.kind == wordcraft_doc::StyleKind::Character { Some(id.into()) } else { None },
                ..Default::default()
            },
        );
        let rp = app.session.doc.styles.resolve_para(&wordcraft_doc::ParaProps { style: Some(id.into()), ..Default::default() });
        let is_char = st.kind == wordcraft_doc::StyleKind::Character;
        let (line, non_multiple_line) = match rp.line_spacing {
            wordcraft_doc::props::LineSpacing::Multiple(m) => (Some(f64::from(m)), None),
            other => (None, Some(other)),
        };
        let (vl, vr) = rp.visual_indents();
        Some(ModifyStyleForm {
            id: id.into(),
            name: st.name.clone(),
            font: rc.font,
            size: f64::from(rc.size),
            bold: rc.bold,
            italic: rc.italic,
            color: match rc.color {
                wordcraft_doc::TextColor::Rgb(c) => c.hex(),
                wordcraft_doc::TextColor::Auto => "auto".into(),
            },
            before: f64::from(rp.space_before),
            after: f64::from(rp.space_after),
            align: format!("{:?}", rp.align.visual(rp.bidi)).to_lowercase(),
            left: f64::from(vl),
            right: f64::from(vr),
            first: f64::from(rp.indent_first),
            line,
            non_multiple_line,
            is_char,
            bidi: rp.bidi,
            tab: 0,
            unit: app.session.units(),
        })
    }

    /// The `styles.modify` parameters for fields that changed since `start`; `font` and `start_font` are the family
    /// names as the document stores them. Returns `None` when nothing changed so an untouched OK leaves the style's
    /// own properties and base-style inheritance intact (#5833).
    fn params(&self, start: &ModifyStyleForm, font: &str, start_font: &str) -> Option<Value> {
        let mut params = serde_json::Map::new();
        params.insert("style".into(), json!(self.id));
        if self.name != start.name {
            params.insert("name".into(), json!(self.name));
        }
        let mut chr = serde_json::Map::new();
        if font != start_font {
            chr.insert("font".into(), json!(font));
        }
        if (self.size - start.size).abs() > 1e-3 {
            chr.insert("size".into(), json!(self.size));
        }
        if self.bold != start.bold {
            chr.insert("bold".into(), json!(self.bold));
        }
        if self.italic != start.italic {
            chr.insert("italic".into(), json!(self.italic));
        }
        if self.color != start.color {
            if let Some(c) = wordcraft_doc::Rgb::parse(&self.color) {
                chr.insert("color".into(), json!({"Rgb": [c.0, c.1, c.2]}));
            } else if self.color.eq_ignore_ascii_case("auto") {
                chr.insert("color".into(), json!("Auto"));
            }
        }
        if !chr.is_empty() {
            params.insert("chr".into(), Value::Object(chr));
        }
        if !self.is_char {
            let mut para = serde_json::Map::new();
            if self.align != start.align {
                let logical_align = if self.bidi {
                    match self.align.as_str() {
                        "left" => "right",
                        "right" => "left",
                        other => other,
                    }
                } else {
                    self.align.as_str()
                };
                para.insert("align".into(), json!(logical_align));
            }
            let left_changed = (self.left - start.left).abs() > 1e-3;
            let right_changed = (self.right - start.right).abs() > 1e-3;
            let first_changed = (self.first - start.first).abs() > 1e-3;
            if self.bidi {
                if left_changed {
                    para.insert("indentRight".into(), json!(self.left));
                }
                if right_changed {
                    para.insert("indentLeft".into(), json!(self.right));
                }
            } else {
                if left_changed {
                    para.insert("indentLeft".into(), json!(self.left));
                }
                if right_changed {
                    para.insert("indentRight".into(), json!(self.right));
                }
            }
            if first_changed {
                para.insert("indentFirst".into(), json!(self.first));
            }
            if (self.before - start.before).abs() > 1e-3 {
                para.insert("spaceBefore".into(), json!(self.before));
            }
            if (self.after - start.after).abs() > 1e-3 {
                para.insert("spaceAfter".into(), json!(self.after));
            }
            let line_changed = match (self.line, start.line) {
                (Some(now), Some(was)) => (now - was).abs() > 1e-3,
                (Some(_), None) => true,
                _ => false,
            };
            if line_changed && let Some(line) = self.line {
                para.insert("lineSpacing".into(), json!({"rule": "multiple", "value": line}));
            }
            if !para.is_empty() {
                params.insert("para".into(), Value::Object(para));
            }
        }
        (params.len() > 1).then_some(Value::Object(params))
    }
}

/// What the Modify Style body asks of the app.
enum Request {
    /// The font box's Add Font… entry.
    AddFont,
}

/// Modify Style on the dialog stack, over a font box that needs the app's catalog and recent fonts.
pub(crate) struct ModifyStyleDialog {
    id: DialogId,
    form: ModifyStyleForm,
    /// Snapshot of the form as opened, used to send only edited properties on OK (#5833).
    start: ModifyStyleForm,
    held: HeldAnswer<ModifyStyleForm>,
    catalog: Arc<Catalog>,
    settings: FontPickerSettings,
    requests: Vec<Request>,
}

/// Opens New Style (`newStyle`) or Modify Style (`modifyStyle`, for `params["style"]`) over `parent`; `None` for another
/// name and for an unknown style (TOC1–TOC9 may be absent).
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        NEW_KEY => open_new_with(app, "Normal", parent),
        MODIFY_KEY => {
            let id = params.get("style")?.as_str()?;
            let dialog = modify_dialog(app, id)?;
            Some(open_with(app, parent, dialog, modify_hooks()))
        }
        _ => None,
    }
}

/// Opens Modify Style for the style `id`, or nothing for an unknown style.
pub(crate) fn modify_style(app: &mut WordApp, id: &str) -> Option<DialogId> {
    let dialog = modify_dialog(app, id)?;
    Some(open_with(app, None, dialog, modify_hooks()))
}

/// Opens Modify Style over `parent`, which stays open under it; nothing for an unknown style (TOC1–TOC9 may be absent).
pub(crate) fn open_modify_child(app: &mut WordApp, id: &str, parent: DialogId) -> Option<DialogId> {
    let dialog = modify_dialog(app, id)?;
    Some(open_with(app, Some(parent), dialog, modify_hooks()))
}

/// Builds the [`ModifyStyleDialog`] state for `id`, or `None` for an unknown style.
fn modify_dialog(app: &mut WordApp, id: &str) -> Option<ModifyStyleDialog> {
    let form = ModifyStyleForm::read(app, id)?;
    // PLAN: assumed `WordApp::font_catalog(&self) -> Arc<Catalog>` and `WordApp::font_picker_settings(&self) -> FontPickerSettings` (ribbon-w11, rb.A5).
    Some(ModifyStyleDialog {
        id: dialog_id(MODIFY_KEY),
        start: form.clone(),
        form,
        held: HeldAnswer::default(),
        catalog: app.font_catalog(),
        settings: app.font_picker_settings(),
        requests: Vec::new(),
    })
}

/// Hooks for driving [`ModifyStyleDialog`] on the dialog stack.
fn modify_hooks() -> DialogHooks {
    DialogHooks { prepare: Some(modify_prepare), settle: modify_settle }
}

/// Draws the Modify Style dialog body: Name, Font/Paragraph tabs for paragraph styles, and formatting fields.
fn body(ui: &mut Ui, form: &mut ModifyStyleForm, catalog: &Arc<Catalog>, settings: &mut FontPickerSettings, requests: &mut Vec<Request>) {
    Field::new(label(tl!("Name:"))).show(ui, |ui, _| TextField::new(label(tl!("Name:")), Bind::bound(&mut form.name)).show(ui));
    if !form.is_char {
        let pages = [0u8, 1u8];
        let out = TabStrip::new(label(tl!("Modify Style"))).kind(TabKind::Text).shown(form.tab).natural_height(true).show_items(
            ui,
            &keyed(&pages, |t: &u8| *t),
            &mut |t, _| Tab::new(label(if *t == 0 { tl!("Font") } else { tl!("Paragraph") })),
        );
        for event in out.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(t) = keys.last()
            {
                form.tab = *t;
            }
        }
    }
    match if form.is_char { 0 } else { form.tab } {
        0 => {
            let actions = [label(tl!("Add Font…"))];
            Field::new(label(tl!("Font:"))).show(ui, |ui, _| {
                let out =
                    crate::widgets::font_picker(label(tl!("Font:")), Bind::bound(&mut form.font), catalog, Bind::bound(settings), &actions, None)
                        .show(ui);
                if out.action == Some(0) {
                    requests.push(Request::AddFont);
                }
            });
            // The form keeps a number; a cleared box leaves the value as it was.
            let mut size = Some(form.size);
            Field::new(label(tl!("Size:")))
                .show(ui, |ui, _| NumberField::new(label(tl!("Size:")), Bind::bound(&mut size)).range(1.0..=1638.0).step(0.5).show(ui));
            Field::new(label(tl!("Color:"))).show(ui, |ui, _| TextField::new(label(tl!("Color:")), Bind::bound(&mut form.color)).show(ui));
            form.size = size.unwrap_or(form.size);
            Row::new().show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut form.bold)).label(label(tl!("Bold"))).id_salt("bold").show(ui);
                Checkbox::new(Bind::bound(&mut form.italic)).label(label(tl!("Italic"))).id_salt("italic").show(ui);
            });
        }
        1 => {
            Field::new(label(tl!("Alignment:"))).show(ui, |ui, _| {
                let aligns = [("left", tl!("Left")), ("center", tl!("Center")), ("right", tl!("Right")), ("justify", tl!("Justify"))];
                let mut entries: Vec<Entry<String>> = aligns.iter().map(|(k, l)| Entry::new((*k).to_string(), label(l))).collect();
                if !aligns.iter().any(|(k, _)| *k == form.align) && !form.align.is_empty() {
                    entries.push(Entry::new(form.align.clone(), label(&form.align)));
                }
                if let Some(a) = Select::new(label(tl!("Alignment:")), Bind::given(form.align.clone()), &entries).show(ui).value().cloned() {
                    form.align = a;
                }
            });
            let lengths = crate::options::lengths(form.unit);
            let (mut left, mut right, mut first) = (Some(form.left), Some(form.right), Some(form.first));
            Section::fixed(label(tl!("Indentation"))).show(ui, |ui, _| {
                Field::new(label(tl!("Left:"))).show(ui, |ui, _| {
                    NumberField::length(label(tl!("Left:")), Bind::bound(&mut left), lengths).range(-792.0..=1584.0).id_salt("left").show(ui);
                });
                Field::new(label(tl!("Right:"))).show(ui, |ui, _| {
                    NumberField::length(label(tl!("Right:")), Bind::bound(&mut right), lengths).range(-792.0..=1584.0).id_salt("right").show(ui);
                });
                Field::new(label(tl!("First line:"))).show(ui, |ui, _| {
                    NumberField::length(label(tl!("First line:")), Bind::bound(&mut first), lengths).range(-792.0..=1584.0).id_salt("first").show(ui);
                });
            });
            form.left = left.unwrap_or(form.left);
            form.right = right.unwrap_or(form.right);
            form.first = first.unwrap_or(form.first);
            let (mut before, mut after, mut line) = (Some(form.before), Some(form.after), form.line);
            Section::fixed(label(tl!("Spacing"))).show(ui, |ui, _| {
                Field::new(label(tl!("Before:"))).show(ui, |ui, _| {
                    NumberField::new(label(tl!("Before:")), Bind::bound(&mut before))
                        .unit_kind(UnitKind::Points)
                        .range(0.0..=1584.0)
                        .id_salt("before")
                        .show(ui);
                });
                Field::new(label(tl!("After:"))).show(ui, |ui, _| {
                    NumberField::new(label(tl!("After:")), Bind::bound(&mut after))
                        .unit_kind(UnitKind::Points)
                        .range(0.0..=1584.0)
                        .id_salt("after")
                        .show(ui);
                });
                Field::new(label(tl!("Line spacing:"))).show(ui, |ui, _| {
                    NumberField::new(label(tl!("Line spacing:")), Bind::bound(&mut line)).range(0.5..=5.0).step(0.05).id_salt("line").show(ui);
                });
            });
            form.before = before.unwrap_or(form.before);
            form.after = after.unwrap_or(form.after);
            if let Some(v) = line {
                form.line = Some(v);
                form.non_multiple_line = None;
            }
        }
        _ => {}
    }
}

impl StackDialog for ModifyStyleDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(MODIFY_KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let ModifyStyleDialog { form, held, catalog, settings, requests, .. } = self;
        let shown = FormDialog::new(label(tl!("Modify Style")), Bind::bound(form)).held(held).show(ui, |ui, _, form| {
            body(ui, form, catalog, settings, requests);
            true
        });
        shown.closed.map(|_| if shown.accepted.is_some() { DialogResult::Ok(()) } else { DialogResult::Cancel })
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn confirm(&mut self, _: &Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            self.held.file(self.form.clone());
        }
        None
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let wrong = || format!("wrong value for {field}");
        let f = &mut self.form;
        match field {
            "name" => f.name = value.as_str().ok_or_else(wrong)?.to_string(),
            "font" => f.font = value.as_str().ok_or_else(wrong)?.to_string(),
            "color" => f.color = value.as_str().ok_or_else(wrong)?.to_string(),
            "size" => f.size = value.as_f64().ok_or_else(wrong)?,
            "before" => f.before = value.as_f64().ok_or_else(wrong)?,
            "after" => f.after = value.as_f64().ok_or_else(wrong)?,
            "bold" => f.bold = value.as_bool().ok_or_else(wrong)?,
            "italic" => f.italic = value.as_bool().ok_or_else(wrong)?,
            "align" => f.align = value.as_str().ok_or_else(wrong)?.to_string(),
            "left" => f.left = value.as_f64().ok_or_else(wrong)?,
            "right" => f.right = value.as_f64().ok_or_else(wrong)?,
            "first" => f.first = value.as_f64().ok_or_else(wrong)?,
            "line" => {
                f.line = Some(value.as_f64().ok_or_else(wrong)?);
                f.non_multiple_line = None;
            }
            "tab" => {
                // Only the tabs the dialog draws: Font (0), and Paragraph (1) for paragraph styles.
                let tab = value.as_u64().and_then(|t| u8::try_from(t).ok()).filter(|&t| t == 0 || (t == 1 && !f.is_char)).ok_or_else(wrong)?;
                f.tab = tab;
            }
            _ => return Err(format!("unknown field {field}")),
        }
        Ok(())
    }

    fn json_fields(&self) -> Option<Value> {
        let f = &self.form;
        Some(json!({
            "name": f.name,
            "font": f.font,
            "size": f.size,
            "color": f.color,
            "bold": f.bold,
            "italic": f.italic,
            "before": f.before,
            "after": f.after,
            "align": f.align,
            "left": f.left,
            "right": f.right,
            "first": f.first,
            "line": f.line,
            "isChar": f.is_char,
            "bidi": f.bidi,
            "tab": f.tab,
        }))
    }
}

/// Brings the recent fonts in for the font box.
fn modify_prepare(app: &mut WordApp, id: DialogId) {
    let settings = app.font_picker_settings();
    if let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) {
        d.settings = settings;
    }
}

/// Runs the held OK's command and answers it; hands the recent fonts back. True once the dialog is gone.
fn modify_settle(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<ModifyStyleDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) else { return Settle::Done };
    let requests = std::mem::take(&mut d.requests);
    let settings = d.settings.clone();
    let start = d.start.clone();
    let filed = d.held.take();
    for Request::AddFont in requests {
        let _ = app.run("font.add", json!({}));
    }
    app.set_font_picker_settings(settings);
    if let Some(form) = filed {
        let font = app.previews.font_stored(&form.font);
        let start_font = app.previews.font_stored(&start.font);
        let answer = match form.params(&start, &font, &start_font) {
            Some(params) => app.run("styles.modify", params).map(|_| ()).map_err(label),
            None => Ok(()),
        };
        if answer.is_ok()
            && let Some(parent) = app.dialog_host.stack().parent(id)
            && let Some(manage) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(parent)
        {
            manage.select(form.id.clone());
        }
        if let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) {
            d.held.answer(answer);
        }
    }
    Settle::Keep
}

/// Opens New Style based on the style named `based_on`, over `parent` (Manage Styles).
pub(crate) fn open_new_child(app: &mut WordApp, based_on: &str, parent: DialogId) -> Option<DialogId> {
    open_new_with(app, based_on, Some(parent))
}

/// `None` before the first frame: the dialog reads its remembered values through the egui context.
fn open_new_with(app: &mut WordApp, based_on: &str, parent: Option<DialogId>) -> Option<DialogId> {
    let ctx = app.ctx.clone()?;
    let fields = DialogFields::from_pairs([("name", "Style1"), ("basedOn", based_on)]);
    let (name_default, based_default) = ("Style1".to_string(), based_on.to_string());
    let dialog = HostedParamDialog::command(&ctx, NEW_KEY, fields, move |_| {
        let params = vec![
            Param::new("name", label(tl!("Name:")), ParamKind::text(name_default.clone())),
            Param::new("basedOn", label(tl!("Style based on:")), ParamKind::text(based_default.clone())),
        ];
        ParamDialog::new(NEW_KEY, label(tl!("Create New Style")), params)
            .footer_note(label(tl!("The new style takes the formatting of the current paragraph.")))
    })
    .held_ok();
    Some(open_with(app, parent, dialog, DialogHooks { prepare: None, settle: new_settle }))
}

/// Runs `styles.create` for the held OK, answers it, and selects the created style in Manage Styles when that is the parent.
fn new_settle(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<HostedParamDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) else { return Settle::Done };
    let Some(ok) = d.take_ok() else { return Settle::Keep };
    let text = |k: &str| ok.fields.text(k).unwrap_or_default().to_string();
    let result = app.run("styles.create", json!({"name": text("name"), "basedOn": text("basedOn")}));
    let answer = match result {
        Ok(v) => {
            let created = v.get("id").and_then(Value::as_str).map(str::to_string);
            if let (Some(created), Some(parent)) = (created, app.dialog_host.stack().parent(id))
                && let Some(manage) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(parent)
            {
                manage.select(created);
            }
            Ok(())
        }
        Err(e) => Err(label(e)),
    };
    if let Some(d) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) {
        d.answer_ok(answer);
    }
    Settle::Keep
}

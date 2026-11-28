//! Home › Font › Enclose Characters: the selected text in a circle, square, triangle or diamond
//! (`format.enclose`).

use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::Picture;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::primitives::{ContentPainterCx, HAlign, PaintShape, StrokeColor, StrokeWidth, TextRun};
use craft::ui::runtime::env::Label;
use craft::ui::style::{DataColor, TypeRole};
use craft::ui::text::TextField;
use egui::{CornerRadius, Rect, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::host::{Asks, WordForm, open_form};
use crate::WordApp;

const PREVIEW_W: f32 = 110.0;
const PREVIEW_H: f32 = 64.0;

/// Enclose Characters' fields.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct EncloseForm {
    /// `none`, `shrink` or `enlarge`.
    pub style: String,
    /// `circle`, `square`, `triangle` or `diamond`.
    pub shape: String,
    /// The enclosed characters: one or two.
    pub text: String,
}

impl EncloseForm {
    /// The selection's text and its enclosure, if it has one.
    pub(crate) fn read(app: &WordApp) -> EncloseForm {
        let t = wordcraft_engine::cmd::format::enclose_target(&app.session).ok();
        let cur = t.as_ref().and_then(|t| t.current.clone());
        EncloseForm {
            style: cur.as_ref().map_or("shrink", |e| e.style.name()).into(),
            shape: cur.as_ref().map_or("circle", |e| e.shape.name()).into(),
            text: t.map(|t| t.text).unwrap_or_default(),
        }
    }

    /// The preview: the text in the chosen shape, shrunk or with the shape enlarged (drawn, so it doesn't
    /// depend on the interface font having ○ □ △ ◇).
    fn preview(&self) -> Picture<'static> {
        let (style, shape, text) = (self.style.clone(), self.shape.clone(), self.text.clone());
        Picture::painter(Label::from(tl!("Preview")), ThumbAspect::new(PREVIEW_W as u16, PREVIEW_H as u16), move |cx: &ContentPainterCx, r: Rect| {
            let ink = StrokeColor::Data(DataColor::new(cx.ink()));
            let c = r.center();
            // The enclosure's height as a share of the preview's: none, enlarged, or shrunk around the text.
            let box_h = match style.as_str() {
                "none" => 0.0,
                "enlarge" => r.height() * 0.75,
                _ => r.height() * 0.53,
            };
            let h = box_h / 2.0;
            let role = if style == "shrink" { TypeRole::Label } else { TypeRole::Heading };
            let dy = if shape == "triangle" && box_h > 0.0 { h * 0.3 } else { 0.0 };
            cx.with_draw(|d| {
                d.stroke(&PaintShape::Rect(r, CornerRadius::same(2)), StrokeWidth::Thin, ink);
                let outline = match (shape.as_str(), box_h > 0.0) {
                    (_, false) => None,
                    ("square", _) => Some(PaintShape::Rect(Rect::from_center_size(c, vec2(box_h, box_h)), CornerRadius::ZERO)),
                    ("triangle", _) => {
                        Some(PaintShape::Polygon(vec![pos2(c.x, c.y - h), pos2(c.x + h * 1.1, c.y + h), pos2(c.x - h * 1.1, c.y + h)]))
                    }
                    ("diamond", _) => Some(PaintShape::Polygon(vec![pos2(c.x, c.y - h), pos2(c.x + h, c.y), pos2(c.x, c.y + h), pos2(c.x - h, c.y)])),
                    _ => Some(PaintShape::Circle(c, h)),
                };
                if let Some(outline) = outline {
                    d.stroke(&outline, StrokeWidth::Medium, ink);
                }
                let line = Rect::from_center_size(c + vec2(0.0, dy), vec2(r.width(), r.height() * 0.5));
                d.text(line, &TextRun::new(text.as_str()).role(role).align(HAlign::Center));
            });
        })
    }
}

/// A radio group over `value` whose options are `(id, label)`.
fn radios(ui: &mut Ui, name: &'static str, value: &mut String, options: &[(&'static str, &'static str)], enabled: bool, horizontal: bool) {
    let group = RadioGroup::new(Label::from(name), Bind::given(value.clone())).enabled(enabled).show(ui, |ui, _| {
        let items = |ui: &mut Ui| {
            for (id, label) in options {
                Radio::new((*id).to_string()).label(Label::from(tl!(*label))).enabled(enabled).show(ui);
            }
        };
        if horizontal {
            Row::new().show(ui, |ui, _| items(ui));
        } else {
            Stack::new().show(ui, |ui, _| items(ui));
        }
    });
    if let Some(v) = group.output.value().cloned() {
        *value = v;
    }
}

impl WordForm for EncloseForm {
    fn key(&self) -> &'static str {
        "enclose"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Enclose Characters"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        FieldGroup::new(Label::from(tl!("Style"))).show(ui, |ui, _| {
            radios(ui, tl!("Style"), &mut self.style, &[("none", "None"), ("shrink", "Shrink text"), ("enlarge", "Enlarge symbol")], true, true);
        });
        Field::new(Label::from(tl!("Text:"))).show(ui, |ui, _| {
            TextField::new(Label::from(tl!("Text:")), Bind::bound(&mut self.text)).limit(Some(2)).show(ui);
        });
        self.preview().show(ui);
        let shapes = [("circle", "Circle"), ("square", "Square"), ("triangle", "Triangle"), ("diamond", "Diamond")];
        let enabled = self.style != "none";
        FieldGroup::new(Label::from(tl!("Enclosure"))).show(ui, |ui, _| {
            radios(ui, tl!("Enclosure"), &mut self.shape, &shapes, enabled, false);
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let params = if self.style == "none" { json!({"shape": null}) } else { json!({"shape": self.shape, "style": self.style, "text": self.text}) };
        vec![("format.enclose".into(), params)]
    }
}

/// Opens Enclose Characters (name `enclose`).
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let form = EncloseForm::read(app);
    Some(open_form(app, parent, form))
}

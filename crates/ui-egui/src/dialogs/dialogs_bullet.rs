//! Define New Bullet (Home › Paragraph › Bullets, #2786): the bullet character (picked in the
//! Symbol dialog), its font (in the app's Font dialog) and its alignment, with a preview. It
//! ends in one command, `para.defineBullet`, so agents get the same result without the dialog.

use craft::ui::buttons::Button;
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::display::{Picture, Text};
use craft::ui::form::FieldGroup;
use craft::ui::layout_components::Row;
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::primitives::{ContentPainterCx, TextureCache};
use craft::ui::runtime::env::Label;
use egui::{Color32, Rect, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::props::Align;

use super::dialogs_insert::symbol::SymbolRequest;
use super::font::FontRequest;
use super::host::{Asks, ChildAnswer, WordForm, open_form};
use crate::WordApp;

/// The alignments the dialog offers: `para.defineBullet`'s `align` and their names.
pub const ALIGNMENTS: [(&str, &str); 3] = [("left", "Left"), ("center", "Center"), ("right", "Right")];

/// The preview's size in points, as the old dialog drew it.
const PREVIEW_W: f32 = 340.0;
const PREVIEW_H: f32 = 17.0 * 7.0 + 10.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefineBulletForm {
    /// The bullet character.
    pub ch: String,
    /// The bullet's own font; empty = the text's.
    pub font: String,
    /// The bullet's size in points; empty = the text's.
    pub size: String,
    pub bold: bool,
    pub italic: bool,
    /// `RRGGBB`; empty or `auto` = the text's colour.
    pub color: String,
    /// `left`, `center` or `right`.
    pub align: String,
}

impl DefineBulletForm {
    /// Starts from the bullet the caret's paragraph has (its level's character, font and
    /// alignment), or a plain • when it has none.
    pub fn new(app: &WordApp) -> DefineBulletForm {
        let doc = &app.session.doc;
        let level = doc
            .para_at(&app.session.sel.focus)
            .and_then(|p| doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| doc.numbering.level(n.num, n.level))
            .filter(|l| l.format == wordcraft_doc::section::NumFormat::Bullet && l.text.chars().count() == 1);
        match level {
            Some(l) => DefineBulletForm {
                ch: l.text.clone(),
                font: l.chr.font.clone().unwrap_or_default(),
                size: l.chr.size.map(|s| s.to_string()).unwrap_or_default(),
                bold: l.chr.bold.unwrap_or(false),
                italic: l.chr.italic.unwrap_or(false),
                color: match l.chr.color {
                    Some(wordcraft_doc::props::TextColor::Rgb(c)) => c.hex(),
                    _ => String::new(),
                },
                align: match l.align {
                    Align::Center => "center",
                    Align::Right => "right",
                    _ => "left",
                }
                .into(),
            },
            None => DefineBulletForm::plain("•"),
        }
    }

    /// A bullet `ch` in the text's own formatting, left aligned.
    pub fn plain(ch: &str) -> DefineBulletForm {
        DefineBulletForm {
            ch: ch.into(),
            font: String::new(),
            size: String::new(),
            bold: false,
            italic: false,
            color: String::new(),
            align: "left".into(),
        }
    }

    /// The bullet's formatting as the preview draws it, at [`Self::preview_pt`].
    pub fn chr(&self) -> wordcraft_doc::props::CharProps {
        wordcraft_doc::props::CharProps {
            size: Some(self.preview_pt()),
            font: (!self.font.is_empty()).then(|| self.font.clone()),
            bold: self.bold.then_some(true),
            italic: self.italic.then_some(true),
            color: wordcraft_doc::Rgb::parse(&self.color).map(wordcraft_doc::props::TextColor::Rgb),
            ..Default::default()
        }
    }

    /// The bullet's size in the preview, in points: its own size, or the text's (11 pt) when it
    /// has none, kept to what the preview's rows can show (4–20 pt).
    pub fn preview_pt(&self) -> f32 {
        let pt = self.size.trim().parse::<f32>().ok().filter(|pt| pt.is_finite() && (1.0..=1638.0).contains(pt)).unwrap_or(11.0);
        pt.clamp(4.0, 20.0)
    }

    /// The `para.defineBullet` params this form stands for.
    pub fn params(&self) -> serde_json::Value {
        let mut v = json!({"char": self.ch, "font": self.font, "align": self.align, "bold": self.bold, "italic": self.italic});
        // No size is the text's size; anything else goes to the command, which rejects a bad one.
        let size = self.size.trim();
        if !size.is_empty() {
            v["size"] = match size.parse::<f64>() {
                Ok(pt) if pt.is_finite() => json!(pt),
                _ => json!(size),
            };
        }
        v["color"] = json!(if wordcraft_doc::Rgb::parse(&self.color).is_some() { self.color.as_str() } else { "auto" });
        v
    }

    /// The preview, drawn like Define New Multilevel List's: two plain lines of text, three bulleted
    /// ones and two plain lines again, as grey bars on a white page. The bullet sits at the first
    /// level's aligned-at position (18 pt), aligned there as chosen, and its text starts at the text
    /// indent (36 pt); the bullet itself is laid out by the layout engine, so a symbol font shows the
    /// character the page will.
    fn preview(&self) -> Picture<'static> {
        let (ch, align, preview_pt, chr) = (self.ch.clone(), self.align.clone(), self.preview_pt(), self.chr());
        Picture::painter(
            Label::from(tl!("Preview")),
            ThumbAspect::new(PREVIEW_W as u16, PREVIEW_H as u16),
            move |cx: &ContentPainterCx, rect: Rect| {
                let k = rect.height() / PREVIEW_H;
                let row = 17.0 * k;
                let mut shapes = vec![
                    egui::Shape::rect_filled(rect, 2.0, Color32::WHITE),
                    egui::Shape::rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Inside),
                ];
                // Positions in points, scaled so 4.5 inches fit, as in Define New Multilevel List.
                let scale = (rect.width() - 16.0) / 324.0;
                let left = rect.left() + 8.0;
                let (aligned_at, text_at) = (left + 18.0 * scale, left + 36.0 * scale);
                let bar = |shapes: &mut Vec<egui::Shape>, y: f32, from: f32, gray: u8| {
                    let r = Rect::from_min_max(pos2(from, y - 2.0), pos2(rect.right() - 8.0, y + 2.0));
                    shapes.push(egui::Shape::rect_filled(r, 1.0, Color32::from_gray(gray)));
                };
                let glyph = preview_pt * scale / 0.7;
                let sized = wordcraft_doc::props::CharProps { size: Some(preview_pt * scale), ..chr.clone() };
                let ppp = cx.pixels_per_point();
                let mut images = Vec::new();
                for i in 0..7 {
                    let y = rect.top() + 5.0 * k + row * i as f32 + row / 2.0;
                    if !(2..5).contains(&i) {
                        bar(&mut shapes, y, left, 205);
                        continue;
                    }
                    let x = match align.as_str() {
                        "center" => aligned_at - glyph / 2.0,
                        "right" => aligned_at - glyph,
                        _ => aligned_at,
                    };
                    let r = Rect::from_min_size(pos2(x, y - glyph / 2.0), vec2(glyph, glyph));
                    images.push(r);
                    bar(&mut shapes, y, text_at.max(r.right() + 2.0), 120);
                }
                cx.data_shapes(shapes);
                let key = format!("bullet-glyph:{ch}:{sized:?}:{glyph}:{ppp}");
                let tex = TextureCache::texture(cx.painter.ctx(), key, || {
                    crate::previews::glyph_image(&ch, &sized, (glyph, glyph, ppp), false)
                        .unwrap_or_else(|| egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]))
                });
                let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                for r in images {
                    cx.picture(tex, r, full, 0.0);
                }
            },
        )
    }
}

impl WordForm for DefineBulletForm {
    fn key(&self) -> &'static str {
        "defineBullet"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Define New Bullet"))
    }

    /// OK needs exactly one character.
    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        Text::new(Label::from(tl!("Bullet character"))).show(ui);
        Row::new().show(ui, |ui, _| {
            if clicked(&Button::new(Label::from(tl!("Symbol…"))).show(ui)) {
                let ch = self.ch.chars().next().unwrap_or('•');
                // Picking the bullet blocks the window as Define New Bullet does (#3859).
                asks.push(Asks::Symbol(SymbolRequest { ch, font: self.font.clone(), modal: true }));
            }
            if clicked(&Button::new(Label::from(tl!("Font…"))).show(ui)) {
                asks.push(Asks::Font(FontRequest::bullet(self.clone())));
            }
        });
        let entries: Vec<Entry<String>> = ALIGNMENTS.iter().map(|(k, n)| Entry::new((*k).to_string(), Label::from(tl!(n)))).collect();
        let current = if self.align.is_empty() { "left".to_string() } else { self.align.clone() };
        FieldGroup::new(Label::from(tl!("Alignment:"))).show(ui, |ui, _| {
            let group = RadioGroup::new(Label::from(tl!("Alignment:")), Bind::given(current.clone())).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    for e in &entries {
                        if let (Some(id), label) = (e.value(), e.label()) {
                            Radio::new(id.clone()).label(label.clone()).show(ui);
                        }
                    }
                });
            });
            if let Some(a) = group.output.value().cloned() {
                self.align = a;
            }
        });
        FieldGroup::new(Label::from(tl!("Preview"))).show(ui, |ui, _| {
            self.preview().show(ui);
        });
        self.ch.chars().count() == 1
    }

    fn commands(&self) -> Vec<(String, Value)> {
        // A bad size is the command's error: the dialog's held refusal shows it and stays open.
        vec![("para.defineBullet".into(), self.params())]
    }

    fn answer(&mut self, answer: ChildAnswer) {
        match answer {
            ChildAnswer::Symbol(pick) => {
                self.ch = pick.ch.to_string();
                if !pick.font.is_empty() {
                    self.font = pick.font;
                }
            }
            ChildAnswer::Font(font) => {
                if let Some(name) = font.bullet_font() {
                    self.font = name;
                }
                self.size = font.size.map(|s| s.to_string()).unwrap_or_default();
                self.bold = font.bold;
                self.italic = font.italic;
                self.color = font.color.clone();
            }
            _ => {}
        }
    }
}

/// Opens Define New Bullet (name `defineBullet`) on the caret paragraph's bullet.
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let form = DefineBulletForm::new(app);
    Some(open_form(app, parent, form))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::host::FormHost;
    use crate::dialogs::host::testing::{confirm, harness};
    use egui_kittest::kittest::Queryable;
    use wordcraft_engine::Session;

    fn app_with_bullet() -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("One\nTwo")), crate::Services::default());
        a.run("para.bullets", json!({"kind": "▪"})).unwrap();
        a
    }

    fn level_at(app: &WordApp, i: usize) -> wordcraft_doc::numbering::Level {
        let doc = &app.session.doc;
        let n = doc.para_at(&wordcraft_doc::Pos::body(i, 0)).unwrap().props.numbering.unwrap();
        doc.numbering.level(n.num, n.level).unwrap().clone()
    }

    fn form_of(h: &egui_kittest::Harness<'static, WordApp>, id: DialogId) -> Option<DefineBulletForm> {
        h.state().dialog_host.get_by_id::<FormHost<DefineBulletForm>>(id).map(|f| f.form().clone())
    }

    fn set(h: &mut egui_kittest::Harness<'static, WordApp>, id: DialogId, field: &str, value: Value) {
        let ctx = h.ctx.clone();
        h.state_mut().dialog_host.set_json_field(&ctx, id, field, value).unwrap();
    }

    /// The form starts from the caret's bullet; OK ends in `para.defineBullet` with the
    /// character, its font and the alignment.
    #[test]
    fn ok_defines_the_bullet_the_form_shows() {
        let app = app_with_bullet();
        let font = level_at(&app, 0).chr.font.unwrap_or_default();
        let mut h = harness(app);
        h.run_steps(3);
        let id = crate::dialogs::open(h.state_mut(), "defineBullet", &json!({})).unwrap();
        h.run_steps(3);
        let f = form_of(&h, id).unwrap();
        assert_eq!((f.ch.as_str(), f.font.as_str(), f.align.as_str()), ("▪", font.as_str(), "left"));
        set(&mut h, id, "ch", json!("†"));
        set(&mut h, id, "font", json!("Segoe UI Symbol"));
        set(&mut h, id, "align", json!("right"));
        confirm(&mut h, id);
        let l = level_at(h.state(), 0);
        assert_eq!((l.text.as_str(), l.chr.font.as_deref(), l.align), ("†", Some("Segoe UI Symbol"), Align::Right));
    }

    /// #2786 review: a size the command rejects (not a number, out of range) is an error OK
    /// reports, keeping the dialog open and the list as it was; no size is the text's size.
    #[test]
    fn ok_with_a_bad_size_keeps_the_dialog_open() {
        assert!(DefineBulletForm::plain("†").params().get("size").is_none(), "no size: the text's size");
        for bad in ["abc", "5000", "0"] {
            let app = app_with_bullet();
            let before = level_at(&app, 0);
            let mut h = harness(app);
            h.run_steps(3);
            let id = crate::dialogs::open(h.state_mut(), "defineBullet", &json!({})).unwrap();
            h.run_steps(3);
            set(&mut h, id, "ch", json!("†"));
            set(&mut h, id, "size", json!(bad));
            confirm(&mut h, id);
            assert!(form_of(&h, id).is_some(), "{bad}: the dialog stays open");
            assert_eq!(level_at(h.state(), 0).text, before.text, "{bad}: the list is unchanged");
        }
    }

    /// Symbol… asks the Symbol dialog, whose OK brings the character back to the form; nothing is
    /// inserted into the text.
    #[test]
    fn symbol_picks_the_character_and_returns() {
        let mut h = harness(app_with_bullet());
        h.run_steps(3);
        let id = crate::dialogs::open(h.state_mut(), "defineBullet", &json!({})).unwrap();
        h.run_steps(3);
        set(&mut h, id, "align", json!("center"));
        h.run_steps(2);
        h.get_by_label("Symbol…").click();
        h.run_steps(3);
        assert!(h.state().dialog_host.shows_modal(), "the bullet's Symbol dialog is modal");
        let open = h.state().dialog_host.open_dialogs();
        // PLAN: assumed the Symbol dialog's field `selected` (dialogs-w16)
        let symbol = open.last().unwrap().id;
        assert_ne!(symbol, id, "a child dialog is open over the form");
        set(&mut h, symbol, "selected", json!("†"));
        // The first confirm after a shown pass files the OK; a later one finishes the dialog (`confirm`'s doc).
        confirm(&mut h, symbol);
        confirm(&mut h, symbol);
        h.run_steps(3);
        let back = form_of(&h, id).unwrap();
        assert_eq!((back.ch.as_str(), back.align.as_str()), ("†", "center"));
        assert_eq!(h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().text, "One");
    }

    /// Closing the Symbol dialog leaves the bullet form open and unchanged.
    #[test]
    fn closing_the_symbol_dialog_returns_to_the_bullet() {
        let mut h = harness(app_with_bullet());
        h.run_steps(3);
        let id = crate::dialogs::open(h.state_mut(), "defineBullet", &json!({})).unwrap();
        h.run_steps(3);
        set(&mut h, id, "align", json!("center"));
        h.run_steps(2);
        h.get_by_label("Symbol…").click();
        h.run_steps(3);
        let symbol = h.state().dialog_host.open_dialogs().last().unwrap().id;
        let ctx = h.ctx.clone();
        assert!(h.state_mut().dialog_host.cancel_by_id(&ctx, symbol));
        h.run_steps(3);
        let open = h.state().dialog_host.open_dialogs();
        assert_eq!(open.len(), 1, "only the bullet form is left");
        assert_eq!(open.last().unwrap().id, id);
        let back = form_of(&h, id).unwrap();
        assert_eq!((back.ch.as_str(), back.align.as_str()), ("▪", "center"));
    }
}

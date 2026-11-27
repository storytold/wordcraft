use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::sections::Section;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use wordcraft_engine::cmd::objects::fit_size;

use super::{DialogId, WordApp, length_field, percent_field, points};
use crate::dialogs::host::{Asks, FormHost, WordForm};

// ---------------------------------------------------------------------------------------------
// Size

/// The Size dialog's fields (Picture Format › Size launcher, #2573) for the selected picture or
/// shape. Lengths are in points; the fields show them in the interface unit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SizeForm {
    /// Where the object sized is (the selected one when the dialog opened).
    #[serde(skip_deserializing)]
    pub at: wordcraft_doc::Pos,
    pub width: f32,
    pub height: f32,
    /// Changing one side changes the other in proportion (on for pictures, as in Word).
    pub lock: bool,
    /// The scale counts from the original size; otherwise from the size the dialog opened with.
    pub relative: bool,
    /// The size the dialog opened with.
    pub start: (f32, f32),
    /// What the crop leaves of the picture's original size: Reset's size. None for a shape, or a
    /// picture whose image can't be read.
    pub original: Option<(f32, f32)>,
    /// The original size before the crop, and a bitmap's pixels, as shown.
    pub uncropped: Option<(f32, f32)>,
    pub pixels: Option<(u32, u32)>,
    /// What the form was read from (see [`seen`]).
    #[serde(skip)]
    seen: Seen,
    /// The user's length unit, which only the app knows; read with the form.
    #[serde(skip)]
    lengths: Option<LengthUnits>,
}

/// An object's picture, crop and size, bit for bit: what [`SizeForm::follow`] compares.
type Seen = (String, [u32; 4], [u32; 2]);

fn seen(o: &wordcraft_doc::para::InlineObject) -> Seen {
    let (w, h) = o.frame().map_or((0.0, 0.0), |(w, h, _)| (w, h));
    match o {
        wordcraft_doc::para::InlineObject::Image { media, crop, .. } => (media.clone(), crop.map(f32::to_bits), [w.to_bits(), h.to_bits()]),
        _ => (String::new(), [0; 4], [w.to_bits(), h.to_bits()]),
    }
}

impl SizeForm {
    /// The selected picture's or shape's size; None when nothing that has one is selected.
    pub fn read(app: &WordApp) -> Option<SizeForm> {
        use wordcraft_doc::para::InlineObject;
        let (at, o) = wordcraft_engine::cmd::objects::selected(&app.session)?;
        let (w, h, _) = o.frame()?;
        let full = match &o {
            InlineObject::Image { media, .. } => wordcraft_engine::cmd::objects::original_size(&app.session, media),
            _ => None,
        };
        let original = wordcraft_engine::cmd::objects::cropped_original_size(&app.session, &o);
        Some(SizeForm {
            at,
            width: w,
            height: h,
            lock: matches!(o, InlineObject::Image { .. }),
            relative: original.is_some(),
            start: (w, h),
            original,
            uncropped: full.map(|f| (f.w, f.h)),
            pixels: full.and_then(|f| f.pixels),
            seen: seen(&o),
            lengths: Some(crate::options::lengths(app.session.units())),
        })
    }

    /// Follow the selection: dialogs aren't modal, so a click on the page may pick another object,
    /// and the ribbon or the handles may crop, change or resize this one. The form then starts over
    /// on what is selected, or there is nothing to size (false: close the dialog). OK and Reset
    /// never work from numbers read from another object or an older state of this one.
    pub fn follow(&mut self, app: &WordApp) -> bool {
        match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((at, o)) if at == self.at && seen(&o) == self.seen => true,
            Some(_) => SizeForm::read(app).map(|f| *self = f).is_some(),
            None => false,
        }
    }

    /// What the scale counts from.
    fn base(&self) -> (f32, f32) {
        match self.original {
            Some(o) if self.relative => o,
            _ => self.start,
        }
    }

    /// The width and height as percentages of [`Self::base`].
    pub fn scale(&self) -> (f32, f32) {
        let (bw, bh) = self.base();
        (self.width / bw.max(0.01) * 100.0, self.height / bh.max(0.01) * 100.0)
    }

    /// A new width; locked, the height follows in proportion, both kept within 1–4000 pt by one
    /// factor (as `picture.size` does), so the limit never changes the shape.
    pub fn set_width(&mut self, w: f32) {
        if self.lock && self.width > 0.0 {
            (self.width, self.height) = fit_size(w, self.height * w / self.width, true);
        } else {
            self.width = fit_size(w, self.height, false).0;
        }
    }

    /// A new height; locked, the width follows (see [`Self::set_width`]).
    pub fn set_height(&mut self, h: f32) {
        if self.lock && self.height > 0.0 {
            (self.width, self.height) = fit_size(self.width * h / self.height, h, true);
        } else {
            self.height = fit_size(self.width, h, false).1;
        }
    }

    /// The width as a percentage of [`Self::base`].
    pub fn set_scale_width(&mut self, percent: f32) {
        self.set_width(self.base().0 * percent / 100.0);
    }

    /// The height as a percentage of [`Self::base`].
    pub fn set_scale_height(&mut self, percent: f32) {
        self.set_height(self.base().1 * percent / 100.0);
    }

    /// Reset: the picture's original size, less what is cropped; one past 4000 pt (a large photo)
    /// shrinks as a whole.
    pub fn reset(&mut self) {
        if let Some((w, h)) = self.original {
            (self.width, self.height) = fit_size(w, h, true);
        }
    }

    /// `picture.size` parameters: both sides, as set here.
    pub fn params(&self) -> Value {
        json!({"width": self.width, "height": self.height, "lockAspect": false})
    }
}

impl WordForm for SizeForm {
    fn key(&self) -> &'static str {
        "pictureSize"
    }

    fn title(&self) -> Label {
        tl!("Size").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let lengths = self.lengths.unwrap_or_else(points);
        // A value already outside the range (a 1 px picture is 0.75 pt) is shown as it is; only a typed
        // change is applied.
        Section::new(tl!("Size").to_string()).show(ui, |ui, _| {
            let mut h = self.height;
            if length_field(ui, tl!("Height:"), &mut h, lengths, 1.0..=4000.0, true) {
                self.set_height(h);
            }
            let mut w = self.width;
            if length_field(ui, tl!("Width:"), &mut w, lengths, 1.0..=4000.0, true) {
                self.set_width(w);
            }
        });
        Section::new(tl!("Scale").to_string()).show(ui, |ui, _| {
            let (mut sw, mut sh) = self.scale();
            if percent_field(ui, tl!("Height:"), &mut sh, 1.0..=10_000.0) {
                self.set_scale_height(sh);
            }
            if percent_field(ui, tl!("Width:"), &mut sw, 1.0..=10_000.0) {
                self.set_scale_width(sw);
            }
            let original = self.original.is_some();
            Checkbox::new(Bind::bound(&mut self.lock)).label(tl!("Lock aspect ratio").to_string()).show(ui);
            Checkbox::new(Bind::bound(&mut self.relative)).label(tl!("Relative to original picture size").to_string()).enabled(original).show(ui);
        });
        if let Some((mut w, mut h)) = self.uncropped {
            Section::new(tl!("Original size").to_string()).show(ui, |ui, _| {
                length_field(ui, tl!("Height:"), &mut h, lengths, 0.0..=f64::MAX, false);
                length_field(ui, tl!("Width:"), &mut w, lengths, 0.0..=f64::MAX, false);
                if let Some((pw, ph)) = self.pixels {
                    let (pw, ph) = (pw.to_string(), ph.to_string());
                    Text::new(crate::i18n::fmt(tl!("{width} × {height} pixels"), &[("width", &pw), ("height", &ph)])).show(ui);
                }
                if Button::new(tl!("Reset").to_string()).show(ui).clicked() {
                    self.reset();
                }
            });
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("picture.size".to_string(), self.params())]
    }
}

/// Runs `follow` on the form of the open dialog `id`, writing it back only when it changed, so what the user is
/// typing survives a pass that follows nothing. False when the dialog has no form or `follow` says to close.
fn follow_form<F: WordForm>(app: &mut WordApp, id: DialogId, follow: impl FnOnce(&mut F, &WordApp) -> bool) -> bool {
    let Some(mut form) = app.dialog_host.get_by_id::<FormHost<F>>(id).map(|h| h.form().clone()) else {
        return false;
    };
    let keep = follow(&mut form, app);
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id)
        && *host.form() != form
    {
        *host.form_mut() = form;
    }
    keep
}

/// The Size dialog's per-pass hook: follows the selection and closes the dialog when nothing is left to size.
pub(super) fn prepare_size(app: &mut WordApp, id: DialogId) {
    if !follow_form::<SizeForm>(app, id, |f, app| f.follow(app)) {
        // The hook has no `Context`, so the host closes the dialog at its next `run`.
        app.dialog_host.close_by_id(id);
    }
}

/// The Page Number Format dialog's per-pass hook: follows the caret to its section.
pub(super) fn prepare_page_numbers(app: &mut WordApp, id: DialogId) {
    follow_form::<PageNumbersForm>(app, id, |f, app| {
        f.follow(app);
        true
    });
}

// ---------------------------------------------------------------------------------------------
// Format Page Numbers

/// The Format Page Numbers dialog's fields (#1113): the number format, and where numbering
/// starts — `cont` numbers on from the section before (no `w:start`), else the pages start at
/// `start`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageNumbersForm {
    /// The section the fields were read from (see [`Self::follow`]).
    pub section: usize,
    /// OOXML number format (`decimal`, `upperRoman`, …).
    pub format: String,
    pub cont: bool,
    pub start: u32,
}

impl PageNumbersForm {
    /// The section's numbering as it is at the caret.
    pub fn read(app: &WordApp) -> PageNumbersForm {
        let sp = wordcraft_engine::cmd::page::sect(&app.session);
        PageNumbersForm {
            section: wordcraft_engine::cmd::page::section_index_of_focus(&app.session),
            format: sp.page_num_format.ooxml().to_string(),
            cont: sp.page_num_start.is_none(),
            start: sp.page_num_start.unwrap_or(1).min(100_000),
        }
    }

    /// Follow the caret: dialogs aren't modal, so a click on the page may move it to another
    /// section. The form then shows that section's numbering, so OK never writes numbers read
    /// from another one (the Size dialog follows its object the same way).
    pub fn follow(&mut self, app: &WordApp) {
        if self.section != wordcraft_engine::cmd::page::section_index_of_focus(&app.session) {
            *self = PageNumbersForm::read(app);
        }
    }

    /// `layout.pageNumberFormat` parameters: no `start` (null) when the pages number on from the
    /// section before.
    pub fn params(&self) -> Value {
        let start = if self.cont { Value::Null } else { json!(self.start) };
        json!({"format": self.format, "start": start})
    }
}

impl WordForm for PageNumbersForm {
    fn key(&self) -> &'static str {
        "pageNumbers"
    }

    fn title(&self) -> Label {
        tl!("Page Number Format").to_string().into()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let formats: Vec<Entry<String>> =
            crate::dialogs_notes::FORMATS.iter().map(|(v, l)| Entry::new((*v).to_string(), tl!(l).to_string())).collect();
        Select::new(tl!("Number format").to_string(), Bind::bound(&mut self.format), &formats).show(ui);
        RadioGroup::new(tl!("Page numbering").to_string(), Bind::bound(&mut self.cont)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, cx| {
                cx.labelled(ui, 0, Label::from(tl!("Continue from previous section")), SideLabel::new().layout(FieldLayout::After), Radio::new(true));
                cx.labelled(ui, 1, Label::from(tl!("Start at:")), SideLabel::new().layout(FieldLayout::After), Radio::new(false));
            });
        });
        super::count_field(ui, tl!("Start at:"), &mut self.start, 0..=100_000, !self.cont);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("layout.pageNumberFormat".to_string(), self.params())]
    }
}

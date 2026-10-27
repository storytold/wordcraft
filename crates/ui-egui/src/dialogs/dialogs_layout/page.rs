use super::*;

// ---------------------------------------------------------------------------------------------
// Size

/// The Size dialog's fields (Picture Format › Size launcher, #2573) for the selected picture or
/// shape. Lengths are in points; the fields show them in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeForm {
    /// Where the object sized is (the selected one when the dialog opened).
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

/// The Size dialog; returns true to close it.
pub(super) fn size_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SizeForm) -> bool {
    if !f.follow(app) {
        return true;
    }
    let unit = unit();
    let k = unit.pt_per_unit();
    // A length in the interface unit that takes any typed unit (`300 px` too), applied once typed
    // (Enter, Tab or a click elsewhere, OK included) so a half-typed number never resizes the other
    // side; `None` until its value changes (while
    // typing, the field reports each keystroke as a change). The range bounds what is typed or
    // dragged only: a value already outside it (a 1 px picture is 0.75 pt) is shown as it is, not
    // clamped, which egui would report as a change and so apply unasked.
    let length = |ui: &mut Ui, label: &str, pt: f32| {
        let name = ui.label(tl!(label));
        let was = pt / k;
        let mut v = was;
        let field =
            egui::DragValue::new(&mut v).speed(1.0 / k).range(1.0 / k..=4000.0 / k).clamp_existing_to_range(false).measure(unit).max_decimals(2);
        let changed = ui.add(field.update_while_editing(false)).labelled_by(name.id).changed();
        (changed && v != was).then_some(v * k)
    };
    let percent = |ui: &mut Ui, label: &str, was: f32| {
        let name = ui.label(tl!(label));
        let mut v = was;
        let field = egui::DragValue::new(&mut v).speed(1.0).range(1.0..=10_000.0).clamp_existing_to_range(false).suffix(" %").max_decimals(1);
        let changed = ui.add(field.update_while_editing(false)).labelled_by(name.id).changed();
        (changed && v != was).then_some(v)
    };
    heading(ui, "Size");
    egui::Grid::new("size_lengths").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        if let Some(h) = length(ui, "Height:", f.height) {
            f.set_height(h);
        }
        if let Some(w) = length(ui, "Width:", f.width) {
            f.set_width(w);
        }
        ui.end_row();
    });
    heading(ui, "Scale");
    let (sw, sh) = f.scale();
    egui::Grid::new("size_scale").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        if let Some(p) = percent(ui, "Height:", sh) {
            f.set_scale_height(p);
        }
        if let Some(p) = percent(ui, "Width:", sw) {
            f.set_scale_width(p);
        }
        ui.end_row();
    });
    ui.checkbox(&mut f.lock, tl!("Lock aspect ratio"));
    ui.add_enabled(f.original.is_some(), egui::Checkbox::new(&mut f.relative, tl!("Relative to original picture size")));
    if let Some((w, h)) = f.uncropped {
        heading(ui, "Original size");
        ui.horizontal(|ui| {
            ui.label(format!("{} {}", tl!("Height:"), crate::options::length(h)));
            ui.add_space(12.0);
            ui.label(format!("{} {}", tl!("Width:"), crate::options::length(w)));
            if let Some((pw, ph)) = f.pixels {
                ui.add_space(12.0);
                let (pw, ph) = (pw.to_string(), ph.to_string());
                ui.weak(crate::i18n::fmt(tl!("{width} × {height} pixels"), &[("width", &pw), ("height", &ph)]));
            }
            ui.add_space(12.0);
            if ui.button(tl!("Reset")).clicked() {
                f.reset();
            }
        });
    }
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "picture.size", f.params());
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// Format Page Numbers

/// The Format Page Numbers dialog's fields (#1113): the number format, and where numbering
/// starts — `cont` numbers on from the section before (no `w:start`), else the pages start at
/// `start`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
            start: sp.page_num_start.unwrap_or(1).clamp(1, 100_000),
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

pub(super) fn page_numbers_ui(app: &mut WordApp, ui: &mut Ui, f: &mut PageNumbersForm) -> bool {
    f.follow(app);
    ui.add_space(4.0);
    ui.label(tl!("Number format"));
    let label = crate::dialogs_notes::FORMATS.iter().find(|(v, _)| *v == f.format.as_str()).map(|(_, l)| *l).unwrap_or_default();
    craft_ui::ComboBox::from_id_salt("pnum_format").selected_text(label).width(150.0).show_ui(ui, |ui| {
        for (v, l) in crate::dialogs_notes::FORMATS {
            ui.selectable_value(&mut f.format, v.to_string(), l);
        }
    });
    heading(ui, "Page numbering");
    ui.radio_value(&mut f.cont, true, tl!("Continue from previous section"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.cont, false, tl!("Start at:"));
        ui.add_enabled(!f.cont, egui::DragValue::new(&mut f.start).range(1..=100_000));
    });
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        return run(app, "layout.pageNumberFormat", f.params());
    }
    cancel
}

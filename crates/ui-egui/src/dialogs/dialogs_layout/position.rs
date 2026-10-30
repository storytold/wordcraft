use super::{MeasureExt, Ui, Value, WordApp, buttons, json, run, semibold, unit, vec2};
use serde::Serialize;

/// What a horizontal position can be relative to: `arrange.positionOptions` name, label.
const ACROSS_TO: [(&str, &str); 7] = [
    ("margin", "Margin"),
    ("page", "Page"),
    ("column", "Column"),
    ("leftMargin", "Left Margin"),
    ("rightMargin", "Right Margin"),
    ("insideMargin", "Inside Margin"),
    ("outsideMargin", "Outside Margin"),
];
/// What a vertical position can be relative to; a paragraph takes an offset only.
const DOWN_TO: [(&str, &str); 7] = [
    ("margin", "Margin"),
    ("page", "Page"),
    ("paragraph", "Paragraph"),
    ("topMargin", "Top Margin"),
    ("bottomMargin", "Bottom Margin"),
    ("insideMargin", "Inside Margin"),
    ("outsideMargin", "Outside Margin"),
];
const ACROSS_ALIGN: [(&str, &str); 5] = [("left", "Left"), ("center", "Centered"), ("right", "Right"), ("inside", "Inside"), ("outside", "Outside")];
const DOWN_ALIGN: [(&str, &str); 5] = [("top", "Top"), ("center", "Centered"), ("bottom", "Bottom"), ("inside", "Inside"), ("outside", "Outside")];

/// One axis of the Position tab: an alignment relative to `align_to`, or `offset` points from
/// `offset_to` (names as `arrange.positionOptions` takes them).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisForm {
    pub aligned: bool,
    pub align: String,
    pub align_to: String,
    pub offset: f32,
    pub offset_to: String,
}

impl AxisForm {
    /// The axis as the object has it. Anchors the dialog doesn't list are shown as layout places
    /// them: across, as the column; down, as the paragraph.
    fn read(across: bool, rel: wordcraft_doc::para::Anchor, align: Option<wordcraft_doc::para::FloatAlign>, offset: f32) -> AxisForm {
        use wordcraft_doc::para::{Anchor, FloatAlign};
        let to = match (rel, across) {
            (Anchor::Margin, _) => "margin",
            (Anchor::Page, _) => "page",
            (Anchor::InsideMargin, _) => "insideMargin",
            (Anchor::OutsideMargin, _) => "outsideMargin",
            (Anchor::LeftMargin, true) => "leftMargin",
            (Anchor::RightMargin, true) => "rightMargin",
            (_, true) => "column",
            (Anchor::TopMargin, false) => "topMargin",
            (Anchor::BottomMargin, false) => "bottomMargin",
            (_, false) => "paragraph",
        };
        let name = match (align, across) {
            (Some(FloatAlign::Start) | None, true) => "left",
            (Some(FloatAlign::Start) | None, false) => "top",
            (Some(FloatAlign::Center), _) => "center",
            (Some(FloatAlign::End), true) => "right",
            (Some(FloatAlign::End), false) => "bottom",
            (Some(FloatAlign::Inside), _) => "inside",
            (Some(FloatAlign::Outside), _) => "outside",
        };
        AxisForm {
            aligned: align.is_some(),
            align: name.into(),
            align_to: if to == "paragraph" { "margin".into() } else { to.into() },
            offset: if offset.is_finite() { offset } else { 0.0 },
            offset_to: to.into(),
        }
    }

    /// This axis's part of `arrange.positionOptions`.
    fn params(&self, across: bool) -> Value {
        if self.aligned {
            json!({"align": self.align, "relativeTo": self.align_to})
        } else {
            json!({(if across { "x" } else { "y" }): self.offset, "relativeTo": self.offset_to})
        }
    }
}

/// What [`PositionForm::follow`] compares: the object's wrapping and position, bit for bit.
type Placed = (wordcraft_doc::para::Wrap, String);

fn placed(o: &wordcraft_doc::para::InlineObject) -> Placed {
    o.frame().map_or((wordcraft_doc::para::Wrap::Inline, String::new()), |(_, _, f)| {
        (f.wrap, format!("{:?} {:?} {} {:?} {:?} {}", f.h_rel, f.h_align, f.x.to_bits(), f.v_rel, f.v_align, f.y.to_bits()))
    })
}

/// The Layout dialog's Position tab for the selected picture or shape.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionForm {
    pub at: wordcraft_doc::Pos,
    /// In Line with Text: OK floats it, so both axes are sent, from where it is on the page.
    pub inline: bool,
    pub horizontal: AxisForm,
    pub vertical: AxisForm,
    #[serde(skip)]
    basis: (AxisForm, AxisForm),
    #[serde(skip)]
    seen: Placed,
}

impl PositionForm {
    /// The selected object's position; None when nothing movable is selected.
    pub fn read(app: &mut WordApp) -> Option<PositionForm> {
        use wordcraft_doc::para::{Anchor, InlineObject, Wrap};
        let (at, o) = wordcraft_engine::cmd::objects::selected(&app.session)?;
        if matches!(o, InlineObject::Graphic { .. }) {
            return None;
        }
        let (_, _, f) = o.frame()?;
        let inline = f.wrap == Wrap::Inline;
        let (horizontal, vertical) = if inline {
            let r = app.session.layout().object(&at, app.session.page_hint)?.rect;
            (AxisForm::read(true, Anchor::Page, None, r.x), AxisForm::read(false, Anchor::Page, None, r.y))
        } else {
            (AxisForm::read(true, f.h_rel, f.h_align, f.x), AxisForm::read(false, f.v_rel, f.v_align, f.y))
        };
        Some(PositionForm { at, inline, basis: (horizontal.clone(), vertical.clone()), horizontal, vertical, seen: placed(&o) })
    }

    /// Follow the selection, as the Size dialog does: another object, or this one moved by a drag,
    /// starts the form over; nothing selected closes it (false).
    pub fn follow(&mut self, app: &mut WordApp) -> bool {
        match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((at, o)) if at == self.at && placed(&o) == self.seen => true,
            Some(_) => PositionForm::read(app).map(|f| *self = f).is_some(),
            None => false,
        }
    }

    /// `arrange.positionOptions` parameters: the axes changed (both for an inline object), or
    /// `{}` when nothing changed.
    pub fn params(&self) -> Value {
        let (h, v) = (self.horizontal != self.basis.0, self.vertical != self.basis.1);
        let mut out = serde_json::Map::new();
        if h || (self.inline && v) {
            out.insert("horizontal".into(), self.horizontal.params(true));
        }
        if v || (self.inline && h) {
            out.insert("vertical".into(), self.vertical.params(false));
        }
        Value::Object(out)
    }
}

/// A dropdown of `items` (name, label) setting `value` to a name.
fn pick(ui: &mut Ui, salt: &str, value: &mut String, items: &[(&str, &str)]) {
    let shown = items.iter().find(|(n, _)| *n == value.as_str()).map_or("", |(_, l)| *l);
    craft_ui::ComboBox::from_id_salt(salt).selected_text(tl!(shown)).width(130.0).show_ui(ui, |ui| {
        for (n, l) in items {
            ui.selectable_value(value, (*n).to_string(), tl!(l));
        }
    });
}

/// One axis as two rows of the dialog's grid: Alignment … relative to …, or Absolute position …
/// to the right of (below) ….
fn axis_rows(ui: &mut Ui, a: &mut AxisForm, across: bool) {
    let unit = unit();
    let k = unit.pt_per_unit();
    let (key, aligns, tos) = if across { ("h", &ACROSS_ALIGN, &ACROSS_TO[..]) } else { ("v", &DOWN_ALIGN, &DOWN_TO[..]) };
    let align_tos: Vec<(&str, &str)> = tos.iter().copied().filter(|(n, _)| *n != "paragraph").collect();
    ui.radio_value(&mut a.aligned, true, tl!("Alignment"));
    ui.add_enabled_ui(a.aligned, |ui| pick(ui, &format!("{key}_align"), &mut a.align, aligns));
    ui.label(tl!("relative to"));
    ui.add_enabled_ui(a.aligned, |ui| pick(ui, &format!("{key}_align_to"), &mut a.align_to, &align_tos));
    ui.end_row();
    ui.radio_value(&mut a.aligned, false, tl!("Absolute position"));
    ui.add_enabled_ui(!a.aligned, |ui| {
        let mut v = a.offset / k;
        let field =
            egui::DragValue::new(&mut v).speed(1.0 / k).range(-4000.0 / k..=4000.0 / k).clamp_existing_to_range(false).measure(unit).max_decimals(2);
        if ui.add(field.update_while_editing(false)).changed() && v.is_finite() {
            a.offset = (v * k).clamp(-4000.0, 4000.0);
        }
    });
    ui.label(if across { tl!("to the right of") } else { tl!("below") });
    ui.add_enabled_ui(!a.aligned, |ui| pick(ui, &format!("{key}_offset_to"), &mut a.offset_to, tos));
    ui.end_row();
}

/// The Layout dialog's Position tab; returns true to close it.
pub(super) fn position_ui(app: &mut WordApp, ui: &mut Ui, f: &mut PositionForm) -> bool {
    if !f.follow(app) {
        return true;
    }
    // One grid for both axes, so their fields line up; `heading` adds space, which a grid can't.
    let title = |ui: &mut Ui, s: &str| ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    egui::Grid::new("position").num_columns(4).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        title(ui, "Horizontal");
        ui.end_row();
        axis_rows(ui, &mut f.horizontal, true);
        title(ui, "Vertical");
        ui.end_row();
        axis_rows(ui, &mut f.vertical, false);
    });
    if f.inline {
        ui.add_space(4.0);
        ui.weak(tl!("The object is In Line with Text: OK floats it with Square wrapping."));
    }
    let (ok, cancel) = buttons(ui, "OK", true);
    if ok {
        let params = f.params();
        return params.as_object().is_some_and(|m| m.is_empty()) || run(app, "arrange.positionOptions", params);
    }
    cancel
}

use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton, Section};
use craft::ui::text::{NumberField, TextField};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::cmd::mailings::{ENVELOPE_SIZES, LABEL_PRODUCTS};
use wordcraft_geom::Region;

use crate::WordApp;
use crate::dialogs::host::{Asks, WordForm, open_form};

// ---------------------------------------------------------------------------------------------
// Envelopes

/// The Envelopes dialog's fields. `size` indexes [`ENVELOPE_SIZES`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeForm {
    pub delivery: String,
    pub ret: String,
    pub omit_return: bool,
    pub size: usize,
    /// The document's region, whose decimal separator words the size labels.
    #[serde(skip)]
    pub region: Region,
}

impl EnvelopeForm {
    /// The delivery address starts as the selected text (an address in the letter).
    pub fn read(app: &WordApp) -> EnvelopeForm {
        let sel = if app.session.sel.is_collapsed() { String::new() } else { app.session.selected_text() };
        EnvelopeForm {
            delivery: sel.trim().replace("\r\n", "\n").replace('\r', "\n"),
            ret: String::new(),
            omit_return: false,
            size: 0,
            // PLAN: assumed `Session::region()` (`engine/src/session/query.rs:164`) gives a `Region` that is `Clone`.
            region: app.session.region().clone(),
        }
    }

    /// `mailings.envelopes` parameters; `add` puts the envelope at the start of this document.
    pub fn params(&self, add: bool) -> Value {
        let size = ENVELOPE_SIZES.get(self.size).map_or("Envelope #10", |s| s.0);
        let ret = if self.omit_return { "" } else { self.ret.as_str() };
        json!({"delivery": self.delivery, "return": ret, "size": size, "add": add})
    }
}

impl EnvelopeForm {
    /// Both buttons need a delivery address.
    fn ready(&self) -> bool {
        !self.delivery.trim().is_empty()
    }
}

impl WordForm for EnvelopeForm {
    fn key(&self) -> &'static str {
        "envelopes"
    }
    fn title(&self) -> Label {
        tl!("Envelopes").into()
    }
    fn ok_label(&self) -> Option<Label> {
        Some(tl!("Add to Document").into())
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let sizes: Vec<Entry<usize>> =
            ENVELOPE_SIZES.iter().enumerate().map(|(i, (n, w, h))| Entry::new(i, envelope_label(n, *w, *h, &self.region))).collect();
        TextField::new(tl!("Delivery address:"), Bind::bound(&mut self.delivery)).lines(4).show(ui);
        Checkbox::new(Bind::bound(&mut self.omit_return)).label(tl!("Omit")).show(ui);
        TextField::new(tl!("Return address:"), Bind::bound(&mut self.ret)).lines(3).enabled(!self.omit_return).show(ui);
        Select::new(tl!("Envelope size:"), Bind::bound(&mut self.size), &sizes).show(ui);
        self.ready()
    }
    fn commands(&self) -> Vec<(String, Value)> {
        vec![("mailings.envelopes".into(), self.params(true))]
    }
    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(tl!("New Document"), DialogRole::Other).enabled(self.ready())]
    }
    fn extra(&mut self, _at: usize) -> Vec<(String, Value)> {
        vec![("mailings.envelopes".into(), self.params(false))]
    }
}

/// Opens Envelopes on the selected address.
pub(crate) fn open_envelopes(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    let form = EnvelopeForm::read(app);
    open_form(app, parent, form)
}

/// `DL   110 × 220 mm` / `#10   4.13 × 9.5 in`: metric sizes in millimetres, the others in
/// inches. The pattern is translated (the unit's name, the order) and the numbers use the
/// region's decimal separator.
pub(crate) fn envelope_label(name: &str, w: f32, h: f32, region: &Region) -> String {
    let short = name.strip_prefix("Envelope ").unwrap_or(name);
    let (template, w, h) = if short.starts_with('#') || short == "Monarch" {
        let i = |pt: f32| ((pt / 72.0) * 100.0).round() / 100.0;
        (tl!("{name}   {height} × {width} in"), i(w), i(h))
    } else {
        let mm = |pt: f32| (pt * 25.4 / 72.0).round();
        (tl!("{name}   {height} × {width} mm"), mm(w), mm(h))
    };
    crate::i18n::fmt(template, &[("name", short), ("height", &with_decimal(h.to_string(), region)), ("width", &with_decimal(w.to_string(), region))])
}

/// A number written with `.` rewritten with the region's decimal separator (`2,625`).
fn with_decimal(n: String, region: &Region) -> String {
    let sep = region.decimal();
    if sep == '.' { n } else { n.replace('.', sep.encode_utf8(&mut [0; 4])) }
}

// ---------------------------------------------------------------------------------------------
// Labels

/// The Labels dialog's fields. `product` indexes [`LABEL_PRODUCTS`]; `row`/`col` count from 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelsForm {
    pub text: String,
    pub product: usize,
    pub single: bool,
    pub row: usize,
    pub col: usize,
    /// One label per recipient (when a recipient list is loaded).
    pub recipients: bool,
    /// The document's region, whose decimal separator words the product labels.
    #[serde(skip)]
    pub region: Region,
    /// Whether a recipient list is loaded, so the per-recipient box shows.
    #[serde(skip)]
    pub has_recipients: bool,
}

impl LabelsForm {
    pub fn read(app: &WordApp) -> LabelsForm {
        let sel = if app.session.sel.is_collapsed() { String::new() } else { app.session.selected_text() };
        LabelsForm {
            text: sel.trim().replace("\r\n", "\n").replace('\r', "\n"),
            product: 0,
            single: false,
            row: 1,
            col: 1,
            recipients: !app.session.merge.rows.is_empty(),
            region: app.session.region().clone(),
            has_recipients: !app.session.merge.rows.is_empty(),
        }
    }

    /// `mailings.labels` parameters.
    pub fn params(&self) -> Value {
        let id = LABEL_PRODUCTS.get(self.product).map_or("letter-3x10", |p| p.id);
        let mut v = json!({"text": self.text, "product": id, "fromRecipients": self.recipients && !self.single});
        if self.single {
            v["single"] = json!({"row": self.row, "col": self.col});
        }
        v
    }
}

/// `Address labels, 3 × 10, 2.625" × 1" (Letter)`.
pub(super) fn product_label(i: usize, region: &Region) -> String {
    let Some(p) = LABEL_PRODUCTS.get(i) else { return String::new() };
    let kind = match p.kind {
        "shipping" => tl!("Shipping labels"),
        "return" => tl!("Return address labels"),
        _ => tl!("Address labels"),
    };
    // Inches on Letter, millimetres on A4, as the sheets are sold.
    let size = if p.paper == "A4" {
        let mm = |pt: f32| with_decimal(format!("{:.1}", pt * 25.4 / 72.0), region);
        crate::i18n::fmt(tl!("{width} × {height} mm"), &[("width", &mm(p.width)), ("height", &mm(p.height))])
    } else {
        let i = |pt: f32| with_decimal((((pt / 72.0) * 1000.0).round() / 1000.0).to_string(), region);
        format!("{}\" × {}\"", i(p.width), i(p.height))
    };
    let (columns, rows) = (p.cols.to_string(), p.rows.to_string());
    crate::i18n::fmt(
        tl!("{kind}, {columns} × {rows}, {size} ({paper})"),
        &[("kind", kind), ("columns", &columns), ("rows", &rows), ("size", &size), ("paper", tl!(p.paper))],
    )
}

impl WordForm for LabelsForm {
    fn key(&self) -> &'static str {
        "labels"
    }
    fn title(&self) -> Label {
        tl!("Labels").into()
    }
    fn ok_label(&self) -> Option<Label> {
        Some(tl!("New Document").into())
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        if self.has_recipients {
            Checkbox::new(Bind::bound(&mut self.recipients)).label(tl!("One label per recipient")).show(ui);
        }
        let per_recipient = self.has_recipients && self.recipients && !self.single;
        TextField::new(tl!("Address:"), Bind::bound(&mut self.text)).lines(4).enabled(!per_recipient).show(ui);
        let products: Vec<Entry<usize>> = (0..LABEL_PRODUCTS.len()).map(|i| Entry::new(i, product_label(i, &self.region))).collect();
        Select::new(tl!("Label:"), Bind::bound(&mut self.product), &products).show(ui);
        let (rows, cols) = LABEL_PRODUCTS.get(self.product).map_or((1, 1), |p| (p.rows, p.cols));
        let mut single = self.single;
        let (mut row, mut col) = (self.row, self.col);
        let single_on = self.single;
        Section::fixed(tl!("Print")).show(ui, |ui, _| {
            let mut mode = u8::from(single);
            RadioGroup::new(tl!("Print"), Bind::bound(&mut mode)).show(ui, |ui, _| {
                Radio::new(0u8).label(tl!("Full page of the same label")).show(ui);
                Radio::new(1u8).label(tl!("Single label")).show(ui);
            });
            single = mode == 1;
            if let Some(Some(v)) = NumberField::new(tl!("Row:"), Bind::given(Some(row as f64)))
                .range(1.0..=rows.max(1) as f64)
                .enabled(single_on)
                .show(ui)
                .value()
                .copied()
            {
                row = v as usize;
            }
            if let Some(Some(v)) = NumberField::new(tl!("Column:"), Bind::given(Some(col as f64)))
                .range(1.0..=cols.max(1) as f64)
                .enabled(single_on)
                .show(ui)
                .value()
                .copied()
            {
                col = v as usize;
            }
        });
        self.single = single;
        self.row = row.clamp(1, rows.max(1));
        self.col = col.clamp(1, cols.max(1));
        (self.has_recipients && self.recipients && !self.single) || !self.text.trim().is_empty()
    }
    fn commands(&self) -> Vec<(String, Value)> {
        vec![("mailings.labels".into(), self.params())]
    }
}

/// Opens the Envelopes or Labels dialog by `name`; `None` for any other name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "envelopes" => Some(open_envelopes(app, parent)),
        "labels" => Some(open_labels(app, parent)),
        _ => None,
    }
}

/// Opens Labels on the selected address.
pub(crate) fn open_labels(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    let form = LabelsForm::read(app);
    open_form(app, parent, form)
}

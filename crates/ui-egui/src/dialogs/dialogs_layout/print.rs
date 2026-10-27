use super::*;

// ---------------------------------------------------------------------------------------------
// Envelopes

/// The Envelopes dialog's fields. `size` indexes [`ENVELOPE_SIZES`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeForm {
    pub delivery: String,
    pub ret: String,
    pub omit_return: bool,
    pub size: usize,
}

impl EnvelopeForm {
    /// The delivery address starts as the selected text (an address in the letter).
    pub fn read(app: &WordApp) -> EnvelopeForm {
        let sel = if app.session.sel.is_collapsed() { String::new() } else { app.session.selected_text() };
        EnvelopeForm { delivery: sel.trim().replace("\r\n", "\n").replace('\r', "\n"), ret: String::new(), omit_return: false, size: 0 }
    }

    /// `mailings.envelopes` parameters; `add` puts the envelope at the start of this document.
    pub fn params(&self, add: bool) -> Value {
        let size = ENVELOPE_SIZES.get(self.size).map_or("Envelope #10", |s| s.0);
        let ret = if self.omit_return { "" } else { self.ret.as_str() };
        json!({"delivery": self.delivery, "return": ret, "size": size, "add": add})
    }
}

pub(super) fn envelopes_ui(app: &mut WordApp, ui: &mut Ui, f: &mut EnvelopeForm) -> bool {
    ui.label(tl!("Delivery address:"));
    ui.add(egui::TextEdit::multiline(&mut f.delivery).desired_rows(4).desired_width(340.0));
    ui.horizontal(|ui| {
        ui.label(tl!("Return address:"));
        ui.checkbox(&mut f.omit_return, tl!("Omit"));
    });
    ui.add_enabled(!f.omit_return, egui::TextEdit::multiline(&mut f.ret).desired_rows(3).desired_width(340.0));
    ui.horizontal(|ui| {
        ui.label(tl!("Envelope size:"));
        let label = |i: usize| ENVELOPE_SIZES.get(i).map(|(n, w, h)| envelope_label(n, *w, *h)).unwrap_or_default();
        craft_ui::ComboBox::from_id_salt("env_size").selected_text(label(f.size)).width(220.0).show_ui(ui, |ui| {
            for i in 0..ENVELOPE_SIZES.len() {
                ui.selectable_value(&mut f.size, i, label(i));
            }
        });
    });
    let ready = !f.delivery.trim().is_empty();
    ui.add_space(8.0);
    let (mut add, mut new, mut cancel) = (false, false, false);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            new = ui.add_enabled(ready, egui::Button::new(tl!("New Document"))).clicked();
            add = ui.add_enabled(ready, crate::widgets::primary_button(ui, tl!("Add to Document"))).clicked();
        });
    });
    if add || new {
        return run(app, "mailings.envelopes", f.params(add));
    }
    cancel
}

/// `DL   110 × 220 mm` / `#10   4.13 × 9.5 in`: metric sizes in millimetres, the others in
/// inches. The pattern is translated (the unit's name, the order) and the numbers use the
/// region's decimal separator.
pub(crate) fn envelope_label(name: &str, w: f32, h: f32) -> String {
    let short = name.strip_prefix("Envelope ").unwrap_or(name);
    let (template, w, h) = if short.starts_with('#') || short == "Monarch" {
        let i = |pt: f32| ((pt / 72.0) * 100.0).round() / 100.0;
        (tl!("{name}   {height} × {width} in"), i(w), i(h))
    } else {
        let mm = |pt: f32| (pt * 25.4 / 72.0).round();
        (tl!("{name}   {height} × {width} mm"), mm(w), mm(h))
    };
    crate::i18n::fmt(template, &[("name", short), ("height", &with_decimal(h.to_string())), ("width", &with_decimal(w.to_string()))])
}

/// A number written with `.` rewritten with the region's decimal separator (`2,625`).
fn with_decimal(n: String) -> String {
    let sep = crate::options::decimal();
    if sep == '.' { n } else { n.replace('.', sep.encode_utf8(&mut [0; 4])) }
}

// ---------------------------------------------------------------------------------------------
// Labels

/// The Labels dialog's fields. `product` indexes [`LABEL_PRODUCTS`]; `row`/`col` count from 1.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelsForm {
    pub text: String,
    pub product: usize,
    pub single: bool,
    pub row: usize,
    pub col: usize,
    /// One label per recipient (when a recipient list is loaded).
    pub recipients: bool,
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
pub(super) fn product_label(i: usize) -> String {
    let Some(p) = LABEL_PRODUCTS.get(i) else { return String::new() };
    let kind = match p.kind {
        "shipping" => tl!("Shipping labels"),
        "return" => tl!("Return address labels"),
        _ => tl!("Address labels"),
    };
    // Inches on Letter, millimetres on A4, as the sheets are sold.
    let size = if p.paper == "A4" {
        let mm = |pt: f32| with_decimal(format!("{:.1}", pt * 25.4 / 72.0));
        crate::i18n::fmt(tl!("{width} × {height} mm"), &[("width", &mm(p.width)), ("height", &mm(p.height))])
    } else {
        let i = |pt: f32| with_decimal((((pt / 72.0) * 1000.0).round() / 1000.0).to_string());
        format!("{}\" × {}\"", i(p.width), i(p.height))
    };
    let (columns, rows) = (p.cols.to_string(), p.rows.to_string());
    crate::i18n::fmt(
        tl!("{kind}, {columns} × {rows}, {size} ({paper})"),
        &[("kind", kind), ("columns", &columns), ("rows", &rows), ("size", &size), ("paper", tl!(p.paper))],
    )
}

pub(super) fn labels_ui(app: &mut WordApp, ui: &mut Ui, f: &mut LabelsForm) -> bool {
    let has_recipients = !app.session.merge.rows.is_empty();
    if has_recipients {
        ui.checkbox(&mut f.recipients, tl!("One label per recipient"));
    }
    ui.add_enabled_ui(!(has_recipients && f.recipients && !f.single), |ui| {
        ui.label(tl!("Address:"));
        ui.add(egui::TextEdit::multiline(&mut f.text).desired_rows(4).desired_width(340.0));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Label:"));
        craft_ui::ComboBox::from_id_salt("label_product").selected_text(product_label(f.product)).width(300.0).show_ui(ui, |ui| {
            for i in 0..LABEL_PRODUCTS.len() {
                ui.selectable_value(&mut f.product, i, product_label(i));
            }
        });
    });
    let (rows, cols) = LABEL_PRODUCTS.get(f.product).map_or((1, 1), |p| (p.rows, p.cols));
    heading(ui, "Print");
    ui.radio_value(&mut f.single, false, tl!("Full page of the same label"));
    ui.horizontal(|ui| {
        ui.radio_value(&mut f.single, true, tl!("Single label"));
        ui.add_enabled_ui(f.single, |ui| {
            ui.label(tl!("Row:"));
            ui.add(egui::DragValue::new(&mut f.row).range(1..=rows));
            ui.label(tl!("Column:"));
            ui.add(egui::DragValue::new(&mut f.col).range(1..=cols));
        });
    });
    f.row = f.row.clamp(1, rows.max(1));
    f.col = f.col.clamp(1, cols.max(1));
    let ready = (has_recipients && f.recipients && !f.single) || !f.text.trim().is_empty();
    let (ok, cancel) = buttons(ui, "New Document", ready);
    if ok {
        return run(app, "mailings.labels", f.params());
    }
    cancel
}

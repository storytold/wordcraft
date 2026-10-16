//! The New Pleading window (`law.pleadings.new` with no params asks for it) and the firm details
//! it shares with File › Options › LawCraft. English only, like the options page.

use egui::{RichText, Ui};
use serde_json::{Map, Value, json};
use wordcraft_ui_egui::theme::semibold;

use crate::Ctx;

/// The error `law.pleadings.new` gives on a document that isn't blank: the window then runs File ›
/// New first, as the File menu does.
const NOT_BLANK: &str = "New Pleading writes into a blank document";

/// The firm block's fields: (key, label, required).
pub const FIRM_FIELDS: [(&str, &str, bool); 11] = [
    ("firmName", "Firm name", true),
    ("address1", "Address", true),
    ("address2", "Address line 2", false),
    ("city", "City", true),
    ("state", "State", true),
    ("zip", "ZIP code", true),
    ("phone", "Phone", true),
    ("fax", "Fax", false),
    ("email", "Email", true),
    ("attorneyName", "Attorney name", true),
    ("barNumber", "MO Bar No.", true),
];

/// Firm details as typed, one value per [`FIRM_FIELDS`] entry.
pub type Firm = [String; 11];

/// Firm details from a saved firm block (`law.firm.get`); missing fields are empty.
pub fn firm_from(v: &Value) -> Firm {
    let mut f = Firm::default();
    for ((key, _, _), value) in FIRM_FIELDS.iter().zip(f.iter_mut()) {
        *value = v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    }
    f
}

/// The firm block `law.firm.set` and `law.pleadings.new` take; empty fields are left out.
pub fn firm_json(f: &Firm) -> Value {
    let mut m = Map::new();
    for ((key, _, _), value) in FIRM_FIELDS.iter().zip(f) {
        if !value.trim().is_empty() {
            m.insert((*key).to_string(), json!(value.trim()));
        }
    }
    Value::Object(m)
}

/// The first required firm field that is empty.
fn firm_missing(f: &Firm) -> Option<&'static str> {
    FIRM_FIELDS.iter().zip(f).find(|((_, _, required), value)| *required && value.trim().is_empty()).map(|((_, label, _), _)| *label)
}

/// The firm fields as a form; whether any changed.
fn firm_fields(ui: &mut Ui, f: &mut Firm, salt: &str) -> bool {
    let mut changed = false;
    egui::Grid::new(salt).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for ((_, label, _), value) in FIRM_FIELDS.iter().zip(f.iter_mut()) {
            let l = ui.label(*label);
            changed |= ui.add(egui::TextEdit::singleline(value).desired_width(260.0)).labelled_by(l.id).changed();
            ui.end_row();
        }
    });
    changed
}

/// The kinds of pleading: (type, button label, title the form starts with).
const TYPES: [(&str, &str, &str); 4] =
    [("motion", "Motion", "Motion"), ("notice", "Notice", "Notice of Hearing"), ("answer", "Answer", "Answer"), ("petition", "Petition", "Petition")];

/// What the New Pleading window holds while it is open.
#[derive(Clone, Debug)]
pub struct PleadingForm {
    /// `motion`, `notice`, `answer` or `petition`.
    pub kind: &'static str,
    pub title: String,
    pub court_type: String,
    pub county: String,
    pub case_number: String,
    /// One name per line.
    pub first_names: String,
    pub first_role: String,
    /// One name per line.
    pub second_names: String,
    pub second_role: String,
    pub represent_first: bool,
    /// Service by e-file and USPS to a party (else e-file to counsel of record).
    pub usps: bool,
    pub served_party: String,
    pub served_address: String,
    /// `YYYY-MM-DD`, prefilled with today so the person sees it and can correct it.
    pub filing_date: String,
    /// Petition only.
    pub verified: bool,
    pub firm: Firm,
    pub remember_firm: bool,
    /// Firm details were saved already (their section starts closed).
    pub firm_saved: bool,
    /// Why the last Create didn't go through.
    pub error: Option<String>,
    /// File › New is waiting on its Save Changes prompt: the document that was open then.
    pub pending: Option<u64>,
    /// The saved firm details have been read in.
    loaded: bool,
}

impl Default for PleadingForm {
    fn default() -> Self {
        Self {
            kind: "motion",
            title: "Motion".into(),
            court_type: "CIRCUIT".into(),
            county: String::new(),
            case_number: String::new(),
            first_names: String::new(),
            first_role: "Petitioner".into(),
            second_names: String::new(),
            second_role: "Respondent".into(),
            represent_first: true,
            usps: false,
            served_party: String::new(),
            served_address: String::new(),
            filing_date: today(),
            verified: false,
            firm: Firm::default(),
            remember_firm: true,
            firm_saved: false,
            error: None,
            pending: None,
            loaded: false,
        }
    }
}

/// Today as `YYYY-MM-DD`.
// ponytail: UTC, the same clock insert.dateTime uses; nothing in the workspace gives the local offset
// without a new dependency or unsafe. The field is editable, so a person late in the evening corrects it.
pub fn today() -> String {
    wordcraft_engine::cmd::now_iso().chars().take(10).collect()
}

fn names(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

impl PleadingForm {
    fn petition(&self) -> bool {
        self.kind == "petition"
    }

    /// The first required field that is empty (Create waits for it).
    pub fn missing(&self) -> Option<&'static str> {
        let fields = [
            (&self.title, "Title"),
            (&self.court_type, "Court type"),
            (&self.county, "County"),
            (&self.first_names, "First side names"),
            (&self.first_role, "First side role"),
            (&self.second_names, "Second side names"),
            (&self.second_role, "Second side role"),
        ];
        if let Some((_, label)) = fields.iter().find(|(v, _)| v.trim().is_empty()) {
            return Some(label);
        }
        if self.filing_date.trim().is_empty() {
            return Some("Filing date");
        }
        if !self.petition() && self.case_number.trim().is_empty() {
            return Some("Case No.");
        }
        if !self.petition() && self.usps {
            if self.served_party.trim().is_empty() {
                return Some("Party name");
            }
            if self.served_address.trim().is_empty() {
                return Some("Address");
            }
        }
        firm_missing(&self.firm)
    }

    /// The params of `law.pleadings.new`.
    pub fn params(&self) -> Value {
        let mut p = json!({
            "type": self.kind,
            "title": self.title.trim(),
            "courtType": self.court_type.trim(),
            "county": self.county.trim(),
            "caseNumber": self.case_number.trim(),
            "firstSide": {"names": names(&self.first_names), "role": self.first_role.trim()},
            "secondSide": {"names": names(&self.second_names), "role": self.second_role.trim()},
            "weRepresent": if self.represent_first { "first" } else { "second" },
            "service": if self.usps && !self.petition() { "efileUsps" } else { "efileCounsel" },
            "filingDate": self.filing_date.trim(),
            "firm": firm_json(&self.firm),
        });
        if let Some(o) = p.as_object_mut() {
            if self.usps && !self.petition() {
                o.insert("servedParty".into(), json!(self.served_party.trim()));
                o.insert("servedAddress".into(), json!(self.served_address.trim()));
            }
            if self.petition() {
                o.insert("verified".into(), json!(self.verified));
            }
        }
        p
    }
}

/// `law.pleadings.new` asked for the window.
pub(crate) fn open(law: &mut crate::LawUi, what: &str) {
    if what == "newPleading" && law.pleading.is_none() {
        law.pleading = Some(PleadingForm::default());
    }
}

/// Draw the window while it is open, and finish a Create that waited on File › New.
pub(crate) fn show(app: &mut Ctx<'_>, ctx: &egui::Context) {
    let Some(mut form) = app.law.pleading.take() else { return };
    if !form.loaded {
        form.loaded = true;
        let saved = app.run("law.firm.get", json!({})).unwrap_or_default();
        if saved.as_object().is_some_and(|o| !o.is_empty()) {
            form.firm = firm_from(&saved);
            form.firm_saved = true;
        }
    }
    let mut keep = true;
    if let Some(old) = form.pending {
        if app.session.document_id() != old {
            // File › New went through: the blank document is in place.
            form.pending = None;
            keep = !create(app, &mut form);
        } else if app.dialog.is_none() && !app.file_dialog_open() {
            // The Save Changes prompt (or its Save As) was cancelled: nothing is created and the
            // window stays.
            form.pending = None;
        }
    }
    // While File › New's prompt is up the window steps aside for it.
    if keep && form.pending.is_none() {
        let mut open = true;
        let mut close = false;
        egui::Window::new("New Pleading").id(egui::Id::new("law_new_pleading")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| form_ui(ui, &mut form));
            if let Some(e) = &form.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            let missing = form.missing();
            if let Some(label) = missing {
                ui.label(RichText::new(format!("Fill in {label} to create.")).small().weak());
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(missing.is_none(), egui::Button::new("Create")).clicked() && submit(app, &mut form) {
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        keep = open && !close && keep;
    }
    if keep {
        app.law.pleading = Some(form);
    }
}

fn form_ui(ui: &mut Ui, form: &mut PleadingForm) {
    let field = |ui: &mut Ui, label: &str, value: &mut String| {
        let l = ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(260.0)).labelled_by(l.id);
        ui.end_row();
    };
    let lines = |ui: &mut Ui, label: &str, value: &mut String| {
        let l = ui.label(label);
        ui.add(egui::TextEdit::multiline(value).desired_rows(2).desired_width(260.0)).labelled_by(l.id);
        ui.end_row();
    };
    egui::Grid::new("law_pleading_form").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Type");
        ui.horizontal(|ui| {
            for (kind, label, title) in TYPES {
                if ui.selectable_label(form.kind == kind, label).clicked() && form.kind != kind {
                    form.kind = kind;
                    form.title = title.to_string();
                }
            }
        });
        ui.end_row();
        field(ui, "Title", &mut form.title);
        field(ui, "Court type", &mut form.court_type);
        field(ui, "County", &mut form.county);
        field(ui, "Case No.", &mut form.case_number);
        lines(ui, "First side names (one per line)", &mut form.first_names);
        field(ui, "First side role", &mut form.first_role);
        lines(ui, "Second side names (one per line)", &mut form.second_names);
        field(ui, "Second side role", &mut form.second_role);
        ui.label("We represent");
        ui.horizontal(|ui| {
            ui.radio_value(&mut form.represent_first, true, "First side");
            ui.radio_value(&mut form.represent_first, false, "Second side");
        });
        ui.end_row();
        if form.kind != "petition" {
            ui.label("Service");
            ui.vertical(|ui| {
                ui.radio_value(&mut form.usps, false, "E-file to counsel of record");
                ui.radio_value(&mut form.usps, true, "E-file and USPS to a party");
            });
            ui.end_row();
            if form.usps {
                field(ui, "Party name", &mut form.served_party);
                lines(ui, "Address", &mut form.served_address);
            }
        }
        let l = ui.label("Filing date");
        ui.add(egui::TextEdit::singleline(&mut form.filing_date).hint_text("YYYY-MM-DD").desired_width(260.0)).labelled_by(l.id);
        ui.end_row();
        if form.kind == "petition" {
            ui.label("");
            ui.checkbox(&mut form.verified, "Verified");
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Firm details").default_open(!form.firm_saved).show(ui, |ui| {
        firm_fields(ui, &mut form.firm, "law_pleading_firm");
    });
    ui.checkbox(&mut form.remember_firm, "Remember firm details");
}

/// Create: save the firm details if asked, make sure the document is blank (File › New, with its
/// Save Changes prompt), then write the pleading. `true`: done, close the window.
fn submit(app: &mut Ctx<'_>, form: &mut PleadingForm) -> bool {
    form.error = None;
    if form.remember_firm {
        if let Err(e) = app.run("law.firm.set", firm_json(&form.firm)) {
            form.error = Some(e);
            return false;
        }
        form.firm_saved = true;
        // The options page reads them again.
        app.law.firm_draft = None;
    }
    let old = app.session.document_id();
    match app.run("law.pleadings.new", form.params()) {
        Ok(_) => {
            place_caret(app);
            true
        }
        Err(e) if e.contains(NOT_BLANK) => match app.run("file.new", json!({})) {
            Ok(v) if v.get("pending").is_some() => {
                form.pending = Some(old);
                false
            }
            Ok(_) => create(app, form),
            Err(e) => {
                form.error = Some(e);
                false
            }
        },
        Err(e) => {
            form.error = Some(e);
            false
        }
    }
}

/// Write the pleading into the (new, blank) document. `true`: done.
fn create(app: &mut Ctx<'_>, form: &mut PleadingForm) -> bool {
    match app.run("law.pleadings.new", form.params()) {
        Ok(_) => {
            place_caret(app);
            true
        }
        Err(e) => {
            form.error = Some(e);
            false
        }
    }
}

/// The caret goes to the first placeholder (`[`) for the person to fill in.
fn place_caret(app: &mut Ctx<'_>) {
    if app.run("select.text", json!({"text": "["})).is_ok() {
        app.session.sel = wordcraft_engine::Selection::caret(app.session.sel.anchor.clone());
    }
    app.canvas.want_focus = true;
}

/// File › Options › LawCraft: the firm details New Pleading signs with.
pub(crate) fn firm_section(app: &mut Ctx<'_>, ui: &mut Ui) {
    ui.label(RichText::new("Firm details").font(semibold(14.0)));
    ui.label(RichText::new("New Pleading signs with these. They stay on this computer.").small().weak());
    let mut draft = match app.law.firm_draft.take() {
        Some(d) => d,
        None => firm_from(&app.run("law.firm.get", json!({})).unwrap_or_default()),
    };
    if firm_fields(ui, &mut draft, "law_options_firm") {
        app.law.firm_dirty = true;
        app.law.firm_message = None;
    }
    app.law.firm_draft = Some(draft);
    if ui.add_enabled(app.law.firm_dirty, egui::Button::new("Save firm details")).clicked() {
        save_firm(app);
    }
    if let Some(m) = &app.law.firm_message {
        ui.colored_label(ui.visuals().error_fg_color, m);
    }
}

/// Save the options page's firm details if they changed; `false` (with the reason shown) when
/// they can't be.
pub(crate) fn save_firm(app: &mut Ctx<'_>) -> bool {
    if !app.law.firm_dirty {
        return true;
    }
    let firm = app.law.firm_draft.as_ref().map(firm_json).unwrap_or_default();
    match app.run("law.firm.set", firm) {
        Ok(_) => {
            app.law.firm_dirty = false;
            app.law.firm_message = None;
            true
        }
        Err(e) => {
            app.law.firm_message = Some(e);
            false
        }
    }
}

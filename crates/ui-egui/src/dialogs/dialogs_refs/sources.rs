use super::shell::{Next, go};
#[cfg(not(target_arch = "wasm32"))]
use super::{AfterPick, Asked, FileDialogRequest};
use super::{RefsDialog, Serialize, Source, Tokens, Ui, Value, WordApp, buttons, json, vec2};

// ---------------------------------------------------------------------------------------------
// Sources

/// Source types with their fields: (kind, label, recommended fields). A field is (key, label).
const SOURCE_TYPES: [(&str, &str, &[(&str, &str)]); 9] = [
    ("book", "Book", &[("author", "Author"), ("title", "Title"), ("year", "Year"), ("city", "City"), ("publisher", "Publisher")]),
    (
        "bookSection",
        "Chapter in a book",
        &[
            ("author", "Author"),
            ("title", "Title"),
            ("journal", "Book title"),
            ("year", "Year"),
            ("pages", "Pages"),
            ("city", "City"),
            ("publisher", "Publisher"),
        ],
    ),
    (
        "article",
        "Journal article",
        &[("author", "Author"), ("title", "Title"), ("journal", "Journal"), ("year", "Year"), ("volume", "Volume"), ("pages", "Pages")],
    ),
    (
        "periodical",
        "Magazine or newspaper article",
        &[
            ("author", "Author"),
            ("title", "Title"),
            ("journal", "Periodical"),
            ("year", "Year"),
            ("month", "Month"),
            ("day", "Day"),
            ("pages", "Pages"),
        ],
    ),
    (
        "conference",
        "Conference paper",
        &[
            ("author", "Author"),
            ("title", "Title"),
            ("journal", "Conference"),
            ("year", "Year"),
            ("pages", "Pages"),
            ("city", "City"),
            ("publisher", "Publisher"),
        ],
    ),
    ("report", "Report", &[("author", "Author"), ("title", "Title"), ("year", "Year"), ("publisher", "Institution"), ("city", "City")]),
    (
        "website",
        "Website",
        &[
            ("author", "Author"),
            ("title", "Page title"),
            ("journal", "Site name"),
            ("year", "Year"),
            ("month", "Month"),
            ("day", "Day"),
            ("url", "URL"),
        ],
    ),
    ("film", "Film", &[("author", "Author"), ("title", "Title"), ("publisher", "Studio"), ("year", "Year")]),
    ("other", "Other", &[("author", "Author"), ("title", "Title"), ("year", "Year"), ("city", "City"), ("publisher", "Publisher")]),
];

/// Every field "show all bibliography fields" adds, after the type's own.
const ALL_FIELDS: [(&str, &str); 17] = [
    ("author", "Author"),
    ("editor", "Editor"),
    ("title", "Title"),
    ("journal", "Container Title"),
    ("year", "Year"),
    ("month", "Month"),
    ("day", "Day"),
    ("city", "City"),
    ("publisher", "Publisher"),
    ("edition", "Edition"),
    ("volume", "Volume"),
    ("issue", "Issue"),
    ("pages", "Pages"),
    ("url", "URL"),
    ("doi", "DOI"),
    ("accessed", "Accessed on"),
    ("tag", "Tag"),
];

pub(super) fn field_mut<'a>(s: &'a mut Source, key: &str) -> Option<&'a mut String> {
    Some(match key {
        "author" => &mut s.author,
        "editor" => &mut s.editor,
        "title" => &mut s.title,
        "journal" => &mut s.journal,
        "year" => &mut s.year,
        "month" => &mut s.month,
        "day" => &mut s.day,
        "city" => &mut s.city,
        "publisher" => &mut s.publisher,
        "edition" => &mut s.edition,
        "volume" => &mut s.volume,
        "issue" => &mut s.issue,
        "pages" => &mut s.pages,
        "url" => &mut s.url,
        "doi" => &mut s.doi,
        "accessed" => &mut s.accessed,
        "tag" => &mut s.tag,
        _ => return None,
    })
}

/// Create Source / Edit Source.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceForm {
    pub source: Source,
    pub show_all: bool,
    /// Editing a source that is there (its tag stays: citations point at it).
    pub edit: bool,
    /// Insert a citation of the source on OK (Insert Citation › Add New Source…).
    pub cite: bool,
    /// Opened from the Source Manager, which comes back when this closes.
    pub back: bool,
}

impl SourceForm {
    pub fn new(cite: bool, back: bool) -> SourceForm {
        SourceForm { source: Source { kind: "book".into(), ..Default::default() }, show_all: false, edit: false, cite, back }
    }

    /// The fields shown: the type's own, or every field.
    pub fn fields(&self) -> Vec<(&'static str, &'static str)> {
        let own = SOURCE_TYPES.iter().find(|(k, ..)| *k == self.source.kind).map(|(_, _, f)| *f).unwrap_or(SOURCE_TYPES[0].2);
        let mut v: Vec<(&str, &str)> = own.to_vec();
        if self.show_all {
            v.extend(ALL_FIELDS.iter().filter(|(k, _)| !own.iter().any(|(o, _)| o == k) && *k != "tag").copied());
        }
        v.push(("tag", "Tag"));
        v
    }
}

/// Draws a source's type and fields, optionally every bibliography field, and adds it as a
/// bibliography source or inserts a citation on OK.
pub(super) fn source_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SourceForm) -> Next {
    egui::Grid::new("src").num_columns(2).spacing(vec2(10.0, 5.0)).show(ui, |ui| {
        let kind_label = ui.label(tl!("Source type:"));
        let shown = SOURCE_TYPES.iter().find(|(k, ..)| *k == f.source.kind).map(|(_, l, _)| tl!(l).to_string()).unwrap_or_default();
        let combo = craft_ui::ComboBox::from_id_salt("src_kind").selected_text(shown).width(200.0).show_ui(ui, |ui| {
            for (k, l, _) in SOURCE_TYPES {
                ui.selectable_value(&mut f.source.kind, k.to_string(), tl!(l));
            }
        });
        combo.response.labelled_by(kind_label.id);
        ui.end_row();
        let edit = f.edit;
        for (key, label) in f.fields() {
            let field_label = ui.label(format!("{}:", tl!(label)));
            if let Some(v) = field_mut(&mut f.source, key) {
                let hint = match key {
                    "author" => tl!("Last, First; Last, First"),
                    "tag" => tl!("Made from the author and year when empty"),
                    _ => "",
                };
                ui.add_enabled(!(edit && key == "tag"), egui::TextEdit::singleline(v).desired_width(260.0).hint_text(hint))
                    .labelled_by(field_label.id);
            }
            ui.end_row();
        }
    });
    ui.checkbox(&mut f.show_all, tl!("Show every bibliography field"));
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let src = serde_json::to_value(&f.source).unwrap_or(Value::Null);
        let r = if f.cite { app.run("references.citation", json!({"source": src})) } else { app.run("references.sources", json!({"add": src})) };
        if let Err(e) = r {
            app.status(e);
        }
    }
    if (ok || cancel) && f.back {
        return go(RefsDialog::Sources { selected: None });
    }
    if ok || cancel { Next::Close } else { Next::Stay }
}

/// The Source Manager: the document's sources with New…, Import…, Edit…, Delete and Insert
/// Citation; each button runs the command an agent would.
pub(super) fn sources_ui(app: &mut WordApp, ui: &mut Ui, selected: &mut Option<usize>) -> Next {
    let t = Tokens::get(ui.ctx());
    let list: Vec<String> = app
        .session
        .doc
        .sources
        .iter()
        .map(|s| {
            let parts: Vec<&str> = [s.author.as_str(), s.title.as_str(), s.year.as_str()].into_iter().filter(|x| !x.is_empty()).collect();
            format!("{} — {}", s.tag, parts.join("; "))
        })
        .collect();
    ui.label(tl!("Sources in this document:"));
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border_strong)).inner_margin(4.0).show(ui, |ui| {
        ui.set_min_size(vec2(380.0, 160.0));
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            if list.is_empty() {
                ui.label(egui::RichText::new(tl!("No sources yet.")).color(t.text_dim));
            }
            for (i, l) in list.iter().enumerate() {
                if ui.selectable_label(*selected == Some(i), l).clicked() {
                    *selected = Some(i);
                }
            }
        });
    });
    let src = selected.and_then(|i| app.session.doc.sources.get(i)).cloned();
    let mut next = Next::Stay;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("New…")).clicked() {
            next = go(RefsDialog::Source(Box::new(SourceForm::new(false, true))));
        }
        // Import… (desktop): a Word bibliography file (`b:Sources`, Word's own Sources.xml) adds
        // its sources to this document (#2640); the list above follows on the next frame.
        #[cfg(not(target_arch = "wasm32"))]
        if ui.button(tl!("Import…")).clicked()
            && let Asked::Done(Err(e)) = app.ask_file(FileDialogRequest::Open { purpose: "sources".into() }, AfterPick::Sources)
        {
            app.status(e);
        }
        if ui.add_enabled(src.is_some(), egui::Button::new(tl!("Edit…"))).clicked()
            && let Some(s) = &src
        {
            next = go(RefsDialog::Source(Box::new(SourceForm { source: s.clone(), show_all: false, edit: true, cite: false, back: true })));
        }
        if ui.add_enabled(src.is_some(), egui::Button::new(tl!("Delete"))).clicked()
            && let Some(s) = &src
        {
            if let Err(e) = app.run("references.sources", json!({"remove": s.tag})) {
                app.status(e);
            }
            *selected = None;
        }
        if ui.add_enabled(src.is_some(), egui::Button::new(tl!("Insert Citation"))).clicked()
            && let Some(s) = &src
        {
            if let Err(e) = app.run("references.citation", json!({"tag": s.tag})) {
                app.status(e);
            }
            next = Next::Close;
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                next = Next::Close;
            }
        });
    });
    next
}

use craft::behavior::{Mixed, Selection, SelectionChange, keyed};
#[cfg(not(target_arch = "wasm32"))]
use craft::host::PickRequest;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow};
// PLAN: assumed FormButtons (dialogs-w5)
use craft::ui::patterns::form_dialogs::{FormButtons, FormCx};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::Source;

use crate::WordApp;
use crate::dialogs::host::{Asks, DialogHooks, Filed, FormHost, WordForm, open_form, open_form_with};
#[cfg(not(target_arch = "wasm32"))]
use crate::file_dialogs::{AfterPick, Asked};

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
        "corporate" => &mut s.corporate,
        _ => return None,
    })
}

/// Create Source / Edit Source.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceForm {
    pub source: Source,
    pub show_all: bool,
    /// Editing a source that is there (its tag stays: citations point at it).
    pub edit: bool,
    /// Insert a citation of the source on OK (Insert Citation › Add New Source…).
    pub cite: bool,
    /// Corporate Author is checked: the Author box holds one organisation's name (#4848).
    pub corporate: bool,
}

impl SourceForm {
    pub fn new(cite: bool) -> SourceForm {
        SourceForm::of(Source { kind: "book".into(), ..Default::default() }, false, cite)
    }

    /// The form for `source`; Corporate Author starts checked when the source has one.
    pub fn of(source: Source, edit: bool, cite: bool) -> SourceForm {
        let corporate = !source.corporate.trim().is_empty();
        SourceForm { source, show_all: false, edit, cite, corporate }
    }

    /// The source as OK adds it: the person authors or the corporate author, whichever box is
    /// shown, never both.
    pub fn finished(&self) -> Source {
        let mut src = self.source.clone();
        if self.corporate {
            src.author.clear();
        } else {
            src.corporate.clear();
        }
        src
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

impl WordForm for SourceForm {
    fn key(&self) -> &'static str {
        if self.edit { "editSource" } else { "createSource" }
    }

    fn title(&self) -> Label {
        Label::from(if self.edit { tl!("Edit Source") } else { tl!("Create Source") }.to_string())
    }

    /// A refusal from the command is shown by the held OK.
    fn commands(&self) -> Vec<(String, Value)> {
        let src = serde_json::to_value(self.finished()).unwrap_or(Value::Null);
        if self.cite {
            vec![("references.citation".to_string(), json!({"source": src}))]
        } else {
            vec![("references.sources".to_string(), json!({"add": src}))]
        }
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let types: Vec<Entry<String>> = SOURCE_TYPES.iter().map(|(k, l, _)| Entry::new(k.to_string(), Label::from(tl!(l).to_string()))).collect();
        let kind_title = Label::from(tl!("Source type:").to_string());
        let kind = Field::new(kind_title.clone())
            .show(ui, |ui, _| Select::new(kind_title.clone(), Bind::given(Mixed::Same(self.source.kind.clone())), &types).show(ui).value().cloned())
            .inner
            .flatten();
        if let Some(kind) = kind {
            self.source.kind = kind;
        }
        for (key, label) in self.fields() {
            // Corporate Author swaps the Author box for one organisation's name (#4848).
            let (key, label) = if key == "author" && self.corporate { ("corporate", "Corporate Author") } else { (key, label) };
            let current = field_mut(&mut self.source, key).map(|v| v.clone()).unwrap_or_default();
            let hint = match key {
                "author" => tl!("Last, First; Last, First"),
                "tag" => tl!("Made from the author and year when empty"),
                _ => "",
            };
            let enabled = !(self.edit && key == "tag");
            let title = Label::from(format!("{}:", tl!(label)));
            let typed = Field::new(title.clone())
                .show(ui, |ui, _| {
                    let mut field = TextField::new(title.clone(), Bind::given(current.clone())).enabled(enabled);
                    if !hint.is_empty() {
                        field = field.placeholder(Label::from(hint.to_string()));
                    }
                    field.show(ui).value().cloned()
                })
                .inner
                .flatten();
            if let Some(text) = typed
                && let Some(field) = field_mut(&mut self.source, key)
            {
                *field = text;
            }
            if matches!(key, "author" | "corporate") {
                Checkbox::new(Bind::bound(&mut self.corporate)).label(Label::from(tl!("Corporate Author").to_string())).show(ui);
            }
        }
        Checkbox::new(Bind::bound(&mut self.show_all)).label(Label::from(tl!("Show every bibliography field").to_string())).show(ui);
        true
    }
}

/// The body action that asks for a Word bibliography file to import.
const IMPORT: u32 = 1;

/// The Source Manager: the document's sources with New…, Import…, Edit…, Delete and Insert Citation; each button runs
/// the command an agent would.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceManagerForm {
    /// The picked row.
    pub selected: Option<usize>,
    /// The row texts, read from the document's sources before every run.
    pub rows: Vec<String>,
    /// The tag of each row.
    pub tags: Vec<String>,
}

impl SourceManagerForm {
    /// The tag of the picked row.
    fn picked(&self) -> Option<&String> {
        self.tags.get(self.selected?)
    }
}

impl WordForm for SourceManagerForm {
    fn key(&self) -> &'static str {
        "sourceManager"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Source Manager").to_string())
    }

    /// Insert Citation is the Accept-role extra; there is no OK, only Close.
    fn buttons(&self) -> FormButtons {
        FormButtons::Close
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(Label::from(tl!("Insert Citation").to_string()), DialogRole::Accept).enabled(self.picked().is_some())]
    }

    fn extra(&mut self, _at: usize) -> Vec<(String, Value)> {
        self.picked().map(|tag| ("references.citation".to_string(), json!({"tag": tag}))).into_iter().collect()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let which: Vec<usize> = (0..self.rows.len()).collect();
        let source = keyed(which.as_slice(), |i: &usize| *i);
        let selection = self.selected.map_or_else(|| Selection::new(Vec::new(), None, None), Selection::single);
        let rows = &self.rows;
        let shown = List::<usize>::new(Label::from(tl!("Sources in this document:").to_string()))
            .select(SelectMode::Single)
            // The dialog body scrolls, and a collection owns its own scroll: the list shows this many rows
            // and scrolls the rest itself.
            .max_rows(10)
            .empty(Label::from(tl!("No sources yet.").to_string()))
            .selection(Bind::given(selection))
            .show_items(ui, &source, &mut |i, _| ListRow::new(Label::from(rows.get(*i).cloned().unwrap_or_default())));
        for event in shown.events {
            match event {
                CollectionEvent::Selection(SelectionChange::Set(keys)) => self.selected = keys.last().copied(),
                CollectionEvent::Selection(SelectionChange::Toggle(key)) => {
                    self.selected = if self.selected == Some(key) { None } else { Some(key) };
                }
                CollectionEvent::Selection(SelectionChange::Clear) => self.selected = None,
                _ => {}
            }
        }
        let tag = self.picked().cloned();
        // Importing reads a path, which the web has no way to hand over.
        let can_import = cfg!(not(target_arch = "wasm32")) && Env::of(ui).view().host().is_some_and(|h| h.files().can_pick());
        if Button::new(Label::from(tl!("New…").to_string())).show(ui).clicked() {
            asks.push(Asks::Open("createSource".to_string(), Value::Null));
        }
        if Button::new(Label::from(tl!("Import…").to_string())).enabled(can_import).show(ui).clicked() {
            asks.push(Asks::Act(IMPORT));
        }
        if Button::new(Label::from(tl!("Edit…").to_string())).enabled(tag.is_some()).show(ui).clicked()
            && let Some(tag) = &tag
        {
            asks.push(Asks::Open("editSource".to_string(), json!({"tag": tag})));
        }
        if Button::new(Label::from(tl!("Delete").to_string())).enabled(tag.is_some()).show(ui).clicked()
            && let Some(tag) = &tag
        {
            asks.push(Asks::Run("references.sources".to_string(), json!({"remove": tag})));
            self.selected = None;
        }
        true
    }

    /// Import…: a Word bibliography file (`b:Sources`, Word's own Sources.xml) adds its sources to this document
    /// (#2640); the list follows on the next run.
    fn applied(&mut self, app: &mut WordApp, filed: Filed, _outcome: Result<&[Value], &str>) {
        #[cfg(not(target_arch = "wasm32"))]
        if matches!(filed, Filed::Act(IMPORT))
            && let Asked::Done(Err(e)) = app.ask_file(PickRequest::open("sources"), AfterPick::Sources)
        {
            app.status(e);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (app, filed);
    }
}

/// The row text of a source: `tag — who; title; year`, `who` the corporate author when there is one.
fn row_text(s: &Source) -> String {
    let who = if s.corporate.trim().is_empty() { s.author.as_str() } else { s.corporate.trim() };
    let parts: Vec<&str> = [who, s.title.as_str(), s.year.as_str()].into_iter().filter(|x| !x.is_empty()).collect();
    format!("{} — {}", s.tag, parts.join("; "))
}

/// Copies the document's sources into the manager before every run.
fn prepare_manager(app: &mut WordApp, id: DialogId) {
    let rows: Vec<String> = app.session.doc.sources.iter().map(row_text).collect();
    let tags: Vec<String> = app.session.doc.sources.iter().map(|s| s.tag.clone()).collect();
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<SourceManagerForm>>(id) {
        let form = host.form_mut();
        form.rows = rows;
        form.tags = tags;
        if form.selected.is_some_and(|at| at >= form.rows.len()) {
            form.selected = None;
        }
    }
}

/// Opens the Source Manager, Create Source or Edit Source dialog (`name`), over `parent` when it is a child. Create
/// Source opened over nothing inserts a citation of the new source; over the manager it only adds the source. Edit
/// Source takes the source's `tag` from `params`; an unknown tag opens nothing.
pub(super) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        // PLAN: assumed DialogHooks::prepare_only(prepare: fn(&mut WordApp, DialogId)) -> DialogHooks (dialogs-w13)
        "sourceManager" => Some(open_form_with(app, parent, SourceManagerForm::default(), DialogHooks::prepare_only(prepare_manager))),
        "createSource" => Some(open_form(app, parent, SourceForm::new(parent.is_none()))),
        "editSource" => {
            let tag = params.get("tag").and_then(Value::as_str)?;
            let source = app.session.doc.sources.iter().find(|s| s.tag == tag)?.clone();
            Some(open_form(app, parent, SourceForm::of(source, true, false)))
        }
        _ => None,
    }
}

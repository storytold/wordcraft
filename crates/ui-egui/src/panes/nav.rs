use std::borrow::Cow;

use craft::behavior::{DisplayOrder, EditEvent, ItemSource, Selection, SelectionChange, keyed};
use craft::ui::contract::{Bind, CollectionEvent, Props, SelectMode, Show, ShowCollection};
use craft::ui::display::Text;
use craft::ui::list::{List, ListRow, RowEvent};
use craft::ui::primitives::Span;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;
use wordcraft_doc::numbering::{Counters, LevelSuffix};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Document, Path, Pos, StoryRef};

use super::WordApp;

/// The Navigation pane's tabs: the `nav_tab` value and the label's catalog key, in order.
const TABS: [(&str, &str); 3] = [("headings", "Headings"), ("pages", "Pages"), ("results", "Results")];

/// A row's place and its text, as the keyed source of a flat list reads them.
type Place = (usize, String);

/// A slice of rows as a collection source: the key of a row is its place.
fn places(rows: &[Place]) -> impl ItemSource<usize> + '_ {
    keyed(rows, |row: &Place| row.0).with_text(|row: &Place| Cow::Borrowed(row.1.as_str()))
}

/// Draws the Navigation pane: search box and tabs, page list, find results and heading outline.
pub(super) fn nav(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    search(app, ui, &ctx);
    tabs(app, ui);
    match app.ui.nav_tab.as_str() {
        "pages" => pages(app, ui),
        "results" => results(app, ui),
        _ => headings(app, ui, &ctx),
    }
}

/// The search field. Ctrl+F with text selected searches for it, as Word does (a selection within one line, at
/// most 255 characters): the text is the field's and the field takes focus with it selected. The search runs as
/// the user types; an edit event that carries the text already searched for runs nothing.
fn search(app: &mut WordApp, ui: &mut Ui, ctx: &egui::Context) {
    let qid = app.window_id("nav_query");
    let field = app.window_id("nav_query_field");
    let stored = ui.data(|d| d.get_temp::<String>(qid)).unwrap_or_default();
    // Kept until a pass takes the focus request: a measuring run moves no focus (`focus::request` is `false` there),
    // and the pane's first drawing after Ctrl+F opened it may be one.
    let focus = app.nav_search_focus;
    let picked = focus.then(|| app.session.selected_text()).filter(|t| !t.is_empty() && !t.contains(['\n', '\r']) && t.chars().count() <= 255);
    let shown = picked.clone().unwrap_or_else(|| stored.clone());
    let mut text_field = TextField::search(tl!("Search document").to_owned(), Bind::given(shown.clone())).id(field);
    if focus {
        text_field = text_field.select_on_focus();
    }
    let out = text_field.show(ui);
    if focus && focus::request(ctx, FocusTarget::Id(field)) {
        app.nav_search_focus = false;
    }
    let mut q = shown;
    for event in out.events.iter() {
        match event {
            EditEvent::Update { value, .. } => q = value.clone(),
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => q = after.clone(),
            _ => {}
        }
    }
    if picked.is_some() || q != stored {
        app.ui.nav_tab = "results".into();
        ui.data_mut(|d| d.insert_temp(qid, q.clone()));
        if !q.is_empty() {
            let _ = app.session.run("edit.find", &json!({"text": q}));
        } else {
            app.session.find.results.clear();
        }
    }
}

fn tabs(app: &mut WordApp, ui: &mut Ui) {
    let labels: Vec<Place> = TABS.iter().enumerate().map(|(at, (_, label))| (at, tl!(label).to_owned())).collect();
    let shown = TABS.iter().position(|(key, _)| *key == app.ui.nav_tab).unwrap_or(0);
    let out = TabStrip::new(tl!("Navigation views").to_owned())
        .kind(TabKind::Panel)
        .shown(shown)
        .show_items(ui, &places(&labels), &mut |key, _| Tab::new(labels.get(*key).map(|label| label.1.clone()).unwrap_or_default()));

    if let Some((key, _)) = out.picked.and_then(|k| TABS.get(k)) {
        app.ui.nav_tab = (*key).into();
    }
}

/// The key a selection event picked, if it picked one row.
fn picked_row(event: &CollectionEvent<usize, RowEvent>) -> Option<usize> {
    match event {
        CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.first().copied(),
        CollectionEvent::Activate(key) => Some(*key),
        _ => None,
    }
}

fn pages(app: &mut WordApp, ui: &mut Ui) {
    let labels: Vec<Place> = app
        .session
        .layout()
        .pages
        .iter()
        .enumerate()
        .map(|(at, p)| (at, crate::i18n::fmt(tl!("Page {page}"), &[("page", &p.number.to_string())])))
        .collect();
    let out = List::<usize>::new(tl!("Pages").to_owned())
        .natural_height(true)
        .select(SelectMode::Single)
        .selection(Bind::given(Selection::single(app.session.page_hint)))
        .show_items(ui, &places(&labels), &mut |key, _| ListRow::new(labels.get(*key).map(|label| label.1.clone()).unwrap_or_default()));
    for page in out.events.iter().filter_map(picked_row) {
        let _ = app.run("edit.goto", json!({"page": page + 1}));
    }
}

/// One find result: the row's text with the hit marked, as the spans of its label.
struct Hit {
    text: String,
    spans: Vec<Span<'static>>,
}

fn results(app: &mut WordApp, ui: &mut Ui) {
    let results = app.session.find.results.clone();
    Text::new(format!("{} results", results.len())).show(ui);
    // Each hit with its surrounding text, the hit itself marked.
    let mut hits: Vec<Hit> = Vec::new();
    for (a, b) in results.iter().take(500) {
        let Some(p) = app.session.doc.para_at(a) else { continue };
        let from = p.clamp(a.off.saturating_sub(30));
        let to = p.clamp((b.off + 40).min(p.len())).max(from);
        let start = p.clamp(a.off).clamp(from, to);
        let end = p.clamp(b.off).clamp(start, to);
        let part = |range: std::ops::Range<usize>| p.text.get(range).unwrap_or("").to_owned();
        let (before, hit, after) = (part(from..start), part(start..end), part(end..to));
        let text = format!("{before}{hit}{after}");
        // PLAN: assumed `Span::new(impl Into<Cow<'a, str>>)` and `ListRow::label_spans(Vec<Span<'static>>)` (chrome-w5, S-L267)
        let spans = vec![Span::new(before), Span::new(hit).highlight(), Span::new(after)];
        hits.push(Hit { text, spans });
    }
    let rows: Vec<Place> = hits.iter().enumerate().map(|(at, hit)| (at, hit.text.clone())).collect();
    let out = List::<usize>::new(tl!("Results").to_owned()).natural_height(true).select(SelectMode::Single).show_items(
        ui,
        &places(&rows),
        &mut |key, _| match hits.get(*key) {
            Some(hit) => ListRow::new(hit.text.clone()).label_spans(hit.spans.clone()),
            None => ListRow::new(String::new()),
        },
    );
    for at in out.events.iter().filter_map(picked_row) {
        if let Some((a, b)) = results.get(at) {
            let _ = app.run("select.range", json!({"anchor": a, "focus": b}));
        }
    }
}

/// The outline as a tree source: the headings that show (none under a collapsed one), each keyed by its place in
/// the list of all headings.
struct Outline<'a> {
    items: &'a [(Path, u8, String)],
    shown: Vec<usize>,
    has_children: Vec<bool>,
    open: Vec<bool>,
}

impl<'a> Outline<'a> {
    fn new(app: &WordApp, items: &'a [(Path, u8, String)]) -> Self {
        // A heading has children when the next heading is a deeper level (#1997).
        let has_children: Vec<bool> = items.iter().enumerate().map(|(i, (_, lv, _))| items.get(i + 1).is_some_and(|n| n.1 > *lv)).collect();
        let open: Vec<bool> = items.iter().enumerate().map(|(i, (p, _, _))| !(has_children[i] && app.ui.nav_collapsed.contains(p))).collect();
        // Everything of a deeper level under a collapsed heading is hidden.
        let mut hide_below: Option<u8> = None;
        let mut shown = Vec::new();
        for (i, (_, lv, _)) in items.iter().enumerate() {
            if hide_below.is_some_and(|h| *lv > h) {
                continue;
            }
            hide_below = (!open[i]).then_some(*lv);
            shown.push(i);
        }
        Outline { items, shown, has_children, open }
    }
}

impl DisplayOrder<usize> for Outline<'_> {
    fn len(&self) -> usize {
        self.shown.len()
    }

    fn key(&self, at: usize) -> usize {
        self.shown.get(at).copied().unwrap_or(usize::MAX)
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.shown.iter().position(|i| i == key)
    }

    fn depth(&self, at: usize) -> usize {
        self.shown.get(at).and_then(|i| self.items.get(*i)).map_or(0, |item| usize::from(item.1))
    }

    fn has_children(&self, at: usize) -> bool {
        self.shown.get(at).and_then(|i| self.has_children.get(*i)).copied().unwrap_or(false)
    }

    fn open(&self, at: usize) -> bool {
        self.shown.get(at).and_then(|i| self.open.get(*i)).copied().unwrap_or(true)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.shown.get(at).and_then(|i| self.items.get(*i)).map(|item| Cow::Borrowed(item.2.as_str()))
    }
}

impl ItemSource<usize> for Outline<'_> {}

fn headings(app: &mut WordApp, ui: &mut Ui, ctx: &egui::Context) {
    let items = nav_headings(&app.session.doc);
    if items.is_empty() {
        Text::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.").to_owned()).show(ui);
        return;
    }
    let caret = app.session.sel.focus.clone();
    let current = items.iter().rposition(|(path, ..)| *path <= caret.path);
    let outline = Outline::new(app, &items);
    let out = List::<usize>::new(tl!("Headings").to_owned())
        .natural_height(true)
        .tree(true)
        .select(SelectMode::Single)
        .selection(Bind::given(current.map_or_else(Selection::default, Selection::single)))
        .show_items(ui, &outline, &mut |key, _| {
            let Some((_, level, text)) = items.get(*key) else { return ListRow::new(String::new()) };
            let row = ListRow::new(text.clone()).depth(usize::from(*level));
            if outline.has_children.get(*key).copied().unwrap_or(false) { row.group(outline.open.get(*key).copied().unwrap_or(true)) } else { row }
        });
    let groups: Vec<usize> = (0..items.len()).filter(|i| outline.has_children.get(*i).copied().unwrap_or(false)).collect();
    let mut go = None;
    for event in &out.events {
        if let CollectionEvent::Open(change) = event {
            let targets = if change.all { groups.clone() } else { vec![change.key] };
            for i in targets {
                let Some((path, ..)) = items.get(i) else { continue };
                if change.open {
                    let _ = app.ui.nav_collapsed.remove(path);
                } else {
                    let _ = app.ui.nav_collapsed.insert(path.clone());
                }
            }
        } else if let Some(i) = picked_row(event) {
            go = Some(i);
        }
    }
    if let Some((path, ..)) = go.and_then(|i| items.get(i)) {
        let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path: path.clone(), off: 0 }}));
        // Align the chosen heading's page immediately against this pane, rather
        // than the indeterminate minimum movement from scroll_to_rect.
        app.canvas.nav_to_caret = true;
        app.canvas.scroll_to_caret = false;
        app.canvas.focus_page(ctx);
    }
}

/// The body's headings (paragraphs with an outline level) in document order, as (path, level,
/// text). A numbered heading starts with the number it shows (`2.1 Method`), as in Word.
pub(super) fn nav_headings(doc: &Document) -> Vec<(Path, u8, String)> {
    // Every numbered paragraph advances the list counters, headings or not.
    let mut counters = Counters::default();
    let mut items = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let rp = doc.styles.resolve_para(&p.props);
        let number = p
            .props
            .numbering
            .or(rp.numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None));
        if let Some(lv) = rp.outline_level {
            let txt = p.plain_text().trim().to_string();
            if txt.is_empty() || rp.style.starts_with("TOC") {
                continue;
            }
            let txt = match number {
                Some((label, def)) if def.suffix == LevelSuffix::Nothing => format!("{label}{txt}"),
                Some((label, _)) => format!("{label} {txt}"),
                None => txt,
            };
            items.push((path, lv, txt));
        }
    }
    items
}

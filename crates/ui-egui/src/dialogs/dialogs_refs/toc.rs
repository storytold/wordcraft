use craft::behavior::{Mixed, Selection, SelectionChange, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::StyleKind;
use wordcraft_engine::cmd::references::FieldCode;

use super::index::{body_field, leaders};
use super::labels::caption_labels;
use super::sample::{leader_char, sample_card};
use crate::WordApp;
use crate::dialogs::host::{Asks, ChildAnswer, DialogHooks, FormHost, WordForm, open_form, open_form_with};

// ---------------------------------------------------------------------------------------------
// Table of Contents

/// The Custom Table of Contents dialog's fields (with its Options).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TocForm {
    pub page_numbers: bool,
    pub right_align: bool,
    pub leader: String,
    pub hyperlinks: bool,
    pub levels: u8,
    /// Options: heading styles (`\o`), outline levels (`\u`), TC fields (`\f`).
    pub heading_styles: bool,
    pub outline_levels: bool,
    pub tc_fields: bool,
    /// Options: other paragraph styles with a TOC level (0 = not in the table), by style name.
    pub style_levels: Vec<(String, u8)>,
}

impl TocForm {
    /// The table in the document, or the defaults for a new one.
    pub fn read(app: &WordApp) -> TocForm {
        let f = body_field(app, "TOC").unwrap_or_else(|| FieldCode::parse(r#"TOC \o "1-3" \h \z \u"#));
        let levels = f.arg('o').and_then(wordcraft_engine::cmd::references::level_range).map_or(3, |r| r.1);
        let listed: Vec<(String, u8)> = f
            .arg('t')
            .map(|t| {
                let parts: Vec<&str> = t.split(',').map(str::trim).collect();
                parts.chunks(2).filter_map(|c| Some((c.first()?.to_string(), c.get(1)?.parse::<u8>().ok()?.clamp(1, 9)))).collect()
            })
            .unwrap_or_default();
        let style_levels = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == StyleKind::Paragraph && !s.hidden && !s.id.starts_with("Heading") && !s.id.starts_with("TOC"))
            .map(|s| (s.name.clone(), listed.iter().find(|(n, _)| n.eq_ignore_ascii_case(&s.name)).map_or(0, |(_, l)| *l)))
            .collect();
        // Right-aligned page numbers: no `\p` separator, or a tab one.
        let leader = app
            .session
            .doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .find(|p| p.props.style.as_deref().is_some_and(|s| s.starts_with("TOC") && s != "TOCHeading"))
            .and_then(|p| p.props.tabs.as_ref()?.last().map(|t| t.leader));
        TocForm {
            page_numbers: !f.has('n'),
            right_align: f.arg('p').is_none_or(|p| p.contains('\t')),
            leader: match leader {
                Some(wordcraft_doc::props::TabLeader::None) => "none",
                Some(wordcraft_doc::props::TabLeader::Hyphen) => "hyphen",
                Some(wordcraft_doc::props::TabLeader::Underscore) => "underscore",
                _ => "dot",
            }
            .into(),
            hyperlinks: f.has('h'),
            levels: levels.clamp(1, 9),
            heading_styles: f.has('o'),
            outline_levels: f.has('u'),
            tc_fields: f.has('f'),
            style_levels,
        }
    }

    /// `references.toc` parameters.
    pub fn params(&self) -> Value {
        let styles: serde_json::Map<String, Value> = self.style_levels.iter().filter(|(_, l)| *l > 0).map(|(n, l)| (n.clone(), json!(l))).collect();
        json!({
            "levels": self.levels,
            "pageNumbers": self.page_numbers,
            "rightAlign": self.right_align,
            "tabLeader": self.leader,
            "hyperlinks": self.hyperlinks,
            "headingStyles": self.heading_styles,
            "outlineLevels": self.outline_levels,
            "tcFields": self.tc_fields,
            "styleLevels": styles,
        })
    }

    /// The sample card's lines: (indent level, text, page or empty).
    fn sample_lines(&self) -> Vec<(usize, String, String)> {
        (1..=self.levels.min(3))
            .map(|l| {
                let name = crate::i18n::fmt(tl!("Heading {n}"), &[("n", &l.to_string())]);
                (l as usize - 1, name, if self.page_numbers { (l * 2 - 1).to_string() } else { String::new() })
            })
            .collect()
    }
}

impl WordForm for TocForm {
    fn key(&self) -> &'static str {
        "toc"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Table of Contents").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("references.toc".to_string(), self.params())]
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        sample_card(tl!("Preview"), self.sample_lines(), self.right_align, leader_char(&self.leader)).show(ui);
        Checkbox::new(Bind::bound(&mut self.page_numbers)).label(Label::from(tl!("Include page numbers").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.right_align))
            .label(Label::from(tl!("Page numbers at the right margin").to_string()))
            .enabled(self.page_numbers)
            .show(ui);
        let title = Label::from(tl!("Tab leader:").to_string());
        let entries = leaders();
        let leader = Field::new(title.clone())
            .show(ui, |ui, _| {
                Select::new(title.clone(), Bind::given(Mixed::Same(self.leader.clone())), &entries)
                    .enabled(self.page_numbers && self.right_align)
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten();
        if let Some(leader) = leader {
            self.leader = leader;
        }
        Checkbox::new(Bind::bound(&mut self.hyperlinks)).label(Label::from(tl!("Link entries to their headings").to_string())).show(ui);
        // "Levels:" beside a field as wide as its number, as the old dialog's label and spin box.
        let title = Label::from(tl!("Levels:").to_string());
        let levels = Field::new(title.clone())
            .sizing(Sizing::Content)
            .show(ui, |ui, _| {
                NumberField::new(title.clone(), Bind::given(Some(f64::from(self.levels))))
                    .range(1.0..=9.0)
                    .sizing(Sizing::Content)
                    .show(ui)
                    .edit
                    .value()
                    .cloned()
            })
            .inner
            .flatten();
        if let Some(Some(n)) = levels {
            self.levels = n.round().clamp(1.0, 9.0) as u8;
        }
        // Options… and Modify… side by side, as in the old dialog.
        let (options, modify) = Row::new()
            .show(ui, |ui, _| {
                let options = Button::new(Label::from(tl!("Options…").to_string())).show(ui).clicked();
                let modify = Button::new(Label::from(tl!("Modify…").to_string())).show(ui).clicked();
                (options, modify)
            })
            .inner
            .unwrap_or_default();
        if options {
            let options = serde_json::to_value(TocOptionsForm::of(self)).unwrap_or(Value::Null);
            asks.push(Asks::Open("tocOptions".to_string(), options));
        }
        if modify {
            asks.push(Asks::Open("tocStyles".to_string(), Value::Null));
        }
        true
    }

    /// Options returns its four option fields on OK; Cancel sends nothing, so the form stays as it was. TOC Styles
    /// returns the form unchanged (its Modify Style edits the document's styles, not this form).
    fn answer(&mut self, answer: ChildAnswer) {
        let ChildAnswer::Form { key, value } = answer else { return };
        if key != "tocOptions" {
            return;
        }
        if let Ok(options) = serde_json::from_value::<TocOptionsForm>(value) {
            self.heading_styles = options.heading_styles;
            self.outline_levels = options.outline_levels;
            self.tc_fields = options.tc_fields;
            self.style_levels = options.style_levels;
        }
    }
}

/// Table of Contents › Options…: where the table collects its entries from. Edited on a copy of the TOC form's option
/// fields, which the TOC form takes back on OK.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TocOptionsForm {
    pub heading_styles: bool,
    pub outline_levels: bool,
    pub tc_fields: bool,
    /// Other paragraph styles with a TOC level (0 = not in the table), by style name.
    pub style_levels: Vec<(String, u8)>,
    /// The TOC form's level count, for the "Heading styles (levels 1–n)" text.
    pub levels: u8,
    /// The names `style_levels` show, in the interface language; the document keeps the English name.
    pub shown: Vec<String>,
}

impl TocOptionsForm {
    /// The option fields of `toc`.
    fn of(toc: &TocForm) -> TocOptionsForm {
        TocOptionsForm {
            heading_styles: toc.heading_styles,
            outline_levels: toc.outline_levels,
            tc_fields: toc.tc_fields,
            style_levels: toc.style_levels.clone(),
            levels: toc.levels,
            shown: Vec::new(),
        }
    }
}

impl WordForm for TocOptionsForm {
    fn key(&self) -> &'static str {
        "tocOptions"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Table of Contents Options").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Text::new(Label::from(tl!("Collect entries from:").to_string())).show(ui);
        let heading_styles = crate::i18n::fmt(tl!("Heading styles (levels 1–{n})"), &[("n", &self.levels.to_string())]);
        Checkbox::new(Bind::bound(&mut self.heading_styles)).label(Label::from(heading_styles)).show(ui);
        Text::new(Label::from(tl!("Other styles and their TOC level:").to_string())).show(ui);
        let TocOptionsForm { style_levels, shown, .. } = self;
        // PLAN: `Table` has no editable-cell API at fw-freeze-1, so the style rows scroll in a bounded area
        // with each level field labelled by its style name.
        egui::ScrollArea::vertical().max_height(ui.spacing().interact_size.y * 8.0).show(ui, |ui| {
            for (at, (_, level)) in style_levels.iter_mut().enumerate() {
                let name = Label::from(shown.get(at).cloned().unwrap_or_default());
                // Level 0 (not in the table) is the empty box.
                let value = (*level > 0).then(|| f64::from(*level));
                let typed = Field::new(name.clone())
                    .show(ui, |ui, _| NumberField::new(name.clone(), Bind::given(value)).range(1.0..=9.0).show(ui).edit.value().cloned())
                    .inner
                    .flatten();
                if let Some(typed) = typed {
                    *level = typed.map_or(0, |n| n.round().clamp(1.0, 9.0) as u8);
                }
            }
        });
        Checkbox::new(Bind::bound(&mut self.outline_levels)).label(Label::from(tl!("Outline levels").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.tc_fields)).label(Label::from(tl!("TC entry fields").to_string())).show(ui);
        if Button::new(Label::from(tl!("Reset").to_string())).show(ui).clicked() {
            self.heading_styles = true;
            self.outline_levels = true;
            self.tc_fields = false;
            for (_, level) in &mut self.style_levels {
                *level = 0;
            }
        }
        true
    }
}

/// Table of Contents › Modify…: the TOC 1–9 styles, one picked to edit in Modify Style.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TocStylesForm {
    /// The picked level, 1–9.
    pub level: u8,
    /// The style names of levels 1–9, read from the document's styles before every run.
    pub names: Vec<String>,
}

impl WordForm for TocStylesForm {
    fn key(&self) -> &'static str {
        "tocStyles"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Table of Contents Styles").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let which: Vec<usize> = (0..self.names.len()).collect();
        let source = keyed(which.as_slice(), |i: &usize| *i);
        let selection = Selection::single(usize::from(self.level.clamp(1, 9)) - 1);
        let names = &self.names;
        let shown = List::<usize>::new(Label::from(tl!("Styles:").to_string()))
            .select(SelectMode::Single)
            .selection(Bind::given(selection))
            .show_items(ui, &source, &mut |i, _| ListRow::new(Label::from(names.get(*i).cloned().unwrap_or_default())));
        // A level is always picked: Clear and a Toggle off keep the current one.
        for event in shown.events {
            let picked = match event {
                CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.last().copied(),
                CollectionEvent::Selection { change: SelectionChange::Toggle(key), .. } => Some(key),
                _ => None,
            };
            if let Some(at) = picked {
                self.level = u8::try_from(at.saturating_add(1)).unwrap_or(1).clamp(1, 9);
            }
        }
        if Button::new(Label::from(tl!("Modify…").to_string())).show(ui).clicked() {
            asks.push(Asks::Open("modifyStyle".to_string(), json!({"style": format!("TOC{}", self.level.max(1))})));
        }
        true
    }
}

/// Copies the TOC styles' current names into the form before every run.
fn prepare_styles(app: &mut WordApp, id: DialogId) {
    let names: Vec<String> =
        (1..=9u8).map(|l| app.session.doc.styles.get(&format!("TOC{l}")).map(|s| s.name.clone()).unwrap_or_else(|| format!("TOC {l}"))).collect();
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<TocStylesForm>>(id) {
        host.form_mut().names = names;
    }
}

/// The names `style_levels` show: the interface-language name of a built-in style, else the document's own.
fn shown_names(app: &WordApp, style_levels: &[(String, u8)]) -> Vec<String> {
    style_levels
        .iter()
        .map(|(name, _)| app.session.doc.styles.styles.iter().find(|st| st.name == *name).map_or_else(|| name.clone(), crate::i18n::style_name))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Table of Figures

/// The Table of Figures dialog's fields (#3013).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FiguresForm {
    /// The caption label whose captions are listed.
    pub label: String,
    /// Built-in labels, the user's own and any the document's captions use.
    pub labels: Vec<String>,
    pub include_label: bool,
    pub page_numbers: bool,
    pub right_align: bool,
    pub leader: String,
}

impl FiguresForm {
    /// Word's defaults for a new table of figures, listing every caption label the user can pick.
    pub fn read(app: &WordApp) -> FiguresForm {
        FiguresForm {
            label: "Figure".into(),
            labels: caption_labels(app),
            include_label: true,
            page_numbers: true,
            right_align: true,
            leader: "dot".into(),
        }
    }

    /// `references.tableOfFigures` parameters.
    pub fn params(&self) -> Value {
        json!({
            "label": self.label,
            "includeLabel": self.include_label,
            "pageNumbers": self.page_numbers,
            "rightAlign": self.right_align,
            "tabLeader": self.leader,
        })
    }

    /// The sample card's lines: (indent level, text, page or empty).
    fn sample_lines(&self) -> Vec<(usize, String, String)> {
        [(1, "1"), (2, "3")]
            .iter()
            .map(|(n, page)| {
                let text = if self.include_label { format!("{} {n}: {}", tl!(&self.label), tl!("Text")) } else { tl!("Text").to_string() };
                (0, text, if self.page_numbers { page.to_string() } else { String::new() })
            })
            .collect()
    }
}

impl WordForm for FiguresForm {
    fn key(&self) -> &'static str {
        "tableOfFigures"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Table of Figures").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("references.tableOfFigures".to_string(), self.params())]
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        sample_card(tl!("Preview"), self.sample_lines(), self.right_align, leader_char(&self.leader)).show(ui);
        let labels: Vec<Entry<String>> = self.labels.iter().map(|l| Entry::new(l.clone(), Label::from(tl!(l).to_string()))).collect();
        let title = Label::from(tl!("Label:").to_string());
        let label = Field::new(title.clone())
            .show(ui, |ui, _| Select::new(title.clone(), Bind::given(Mixed::Same(self.label.clone())), &labels).show(ui).value().cloned())
            .inner
            .flatten();
        if let Some(label) = label {
            self.label = label;
        }
        Checkbox::new(Bind::bound(&mut self.include_label)).label(Label::from(tl!("Include label and number").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.page_numbers)).label(Label::from(tl!("Include page numbers").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.right_align))
            .label(Label::from(tl!("Page numbers at the right margin").to_string()))
            .enabled(self.page_numbers)
            .show(ui);
        let title = Label::from(tl!("Tab leader:").to_string());
        let entries = leaders();
        let leader = Field::new(title.clone())
            .show(ui, |ui, _| {
                Select::new(title.clone(), Bind::given(Mixed::Same(self.leader.clone())), &entries)
                    .enabled(self.page_numbers && self.right_align)
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten();
        if let Some(leader) = leader {
            self.leader = leader;
        }
        true
    }
}

/// Opens the Table of Contents dialog, its Options or Styles child, or the Table of Figures dialog (`name`), over
/// `parent` when it is a child. Options opened by name builds from the document's table.
pub(super) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "toc" => {
            let form = TocForm::read(app);
            Some(open_form(app, parent, form))
        }
        "tocOptions" => {
            // The TOC form sends its option fields to its child; opened by name, the options are the document's table's.
            let sent = parent.and_then(|_| serde_json::from_value::<TocOptionsForm>(params.clone()).ok());
            let mut form = sent.unwrap_or_else(|| TocOptionsForm::of(&TocForm::read(app)));
            form.shown = shown_names(app, &form.style_levels);
            Some(open_form(app, parent, form))
        }
        "tocStyles" => {
            let form = TocStylesForm { level: 1, names: Vec::new() };
            // PLAN: assumed DialogHooks::prepare_only(prepare: fn(&mut WordApp, DialogId)) -> DialogHooks (dialogs-w13)
            Some(open_form_with(app, parent, form, DialogHooks::prepare_only(prepare_styles)))
        }
        "tableOfFigures" => {
            let form = FiguresForm::read(app);
            Some(open_form(app, parent, form))
        }
        _ => None,
    }
}

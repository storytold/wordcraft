//! Form controls (ODF 1.3 §13: `office:forms` at the start of the text, each control placed by
//! a `draw:control` in a paragraph) read as WordCraft content controls and written back:
//! text fields (`form:text`, `form:textarea`) as plain text controls, check boxes and radio
//! buttons as check boxes (a radio button with round symbols), list boxes as drop-down lists,
//! combo boxes as combo boxes, date fields as date controls. A control's name is its tag, a
//! check box's label its title. Buttons, images and other controls aren't read.

use std::collections::HashMap;

use wordcraft_doc::control::{ContentControl, ControlKind, ListItem};

use super::writer::x;
use super::xml::{Attrs, get};

/// Most controls read, list entries in one, and characters of a value.
const MAX_CONTROLS: usize = 10_000;
const MAX_ITEMS: usize = 1000;
const MAX_VALUE: usize = 10_000;

/// A radio button's symbols: a selected and an empty circle.
pub(in crate::odt) const RADIO_ON: char = '\u{25C9}';
pub(in crate::odt) const RADIO_OFF: char = '\u{25CB}';

/// Form controls WordCraft has no kind for, kept verbatim (see [`ContentControl::form_xml`]):
/// every control of ODF 1.3 §13.5 but those it reads (text fields and areas, check boxes, option
/// buttons, list and combo boxes, date fields) and hidden ones (kept apart).
const KEPT: [&str; 12] = [
    "form:button",
    "form:image",
    "form:fixed-text",
    "form:file",
    "form:value-range",
    "form:frame",
    "form:image-frame",
    "form:generic-control",
    "form:password",
    "form:number",
    "form:time",
    "form:grid",
];
/// Most bytes of a kept control's XML.
const MAX_KEPT: usize = 64 * 1024;
/// The namespace prefixes a kept control may use: those the content WordCraft writes declares.
const PREFIXES: [&str; 19] = [
    "office", "style", "text", "table", "draw", "fo", "xlink", "dc", "meta", "svg", "number", "form", "ooo", "ooow", "oooc", "script", "dom",
    "loext", "xml",
];

/// The `text:placeholder-type` (§19.851) of a placeholder: what it stands for.
pub(in crate::odt) fn placeholder_type(t: Option<&str>) -> &'static str {
    match t.map(str::trim) {
        Some("table") => "table",
        Some("text-box") => "text-box",
        Some("image") => "image",
        Some("object") => "object",
        _ => "text",
    }
}

/// A placeholder's start tag, which a content control read from one keeps (in
/// [`ContentControl::form_xml`]) to be written back as one.
pub(in crate::odt) fn placeholder_start(kind: &str) -> String {
    format!("<text:placeholder text:placeholder-type=\"{kind}\">")
}

/// Whether control `c` was read from a placeholder.
pub(in crate::odt) fn is_placeholder(c: &ContentControl) -> bool {
    c.form_xml.starts_with("<text:placeholder ")
}

/// A control read: its content control and the text it holds.
pub(in crate::odt) type Read = (ContentControl, String);

fn capped(s: &str) -> String {
    s.chars().take(MAX_VALUE).collect()
}

/// `office:forms`, read as it streams past.
#[derive(Default)]
pub(in crate::odt) struct FormsReader {
    /// How deep inside `office:forms` the reader is (0: outside).
    depth: usize,
    /// The control being read: its id, control, selected entries.
    open: Option<(String, ContentControl, Vec<String>)>,
    /// The controls read, by `form:id` (or `xml:id`).
    pub(in crate::odt) controls: HashMap<String, Read>,
    /// Hidden controls (§13.5.15 `form:hidden`: a name and a value the form sends, not shown),
    /// kept verbatim without their ids.
    pub(in crate::odt) hidden: Vec<String>,
    /// A control WordCraft has no kind for being kept: its id, name, label, its XML so far (its
    /// id left out) and how many of its elements are open.
    kept: Option<Kept>,
}

struct Kept {
    id: String,
    name: String,
    label: String,
    xml: String,
    open: usize,
    /// It uses a prefix the content WordCraft writes doesn't declare: not kept.
    foreign: bool,
}

/// Whether name `q` has a prefix the content WordCraft writes declares.
fn known(q: &str) -> bool {
    q.split_once(':').is_some_and(|(p, _)| PREFIXES.contains(&p))
}

/// Whether element `q` with attributes `a` uses only declared prefixes.
fn declared(q: &str, a: &Attrs) -> bool {
    known(q) && a.iter().all(|(k, _)| known(k))
}

/// Element `q` with attributes `a` as XML, its ids (`form:id`, `xml:id`) left out.
fn element_xml(q: &str, a: &Attrs) -> String {
    let mut s = format!("<{q}");
    for (k, v) in a.iter().filter(|(k, _)| k != "form:id" && k != "xml:id") {
        s.push_str(&format!(" {k}=\"{}\"", x(v)));
    }
    s.push('>');
    s
}

impl FormsReader {
    /// Element `q` starts: whether it is part of `office:forms` (and so read here).
    pub(in crate::odt) fn start(&mut self, q: &str, a: &Attrs) -> bool {
        if self.depth == 0 {
            if q != "office:forms" {
                return false;
            }
            self.depth = 1;
            return true;
        }
        self.depth += 1;
        if let Some(k) = &mut self.kept {
            if k.xml.len() < MAX_KEPT {
                k.xml.push_str(&element_xml(q, a));
            }
            k.foreign |= !declared(q, a);
            k.open += 1;
            return true;
        }
        let id = get(a, "form:id").or_else(|| get(a, "xml:id")).unwrap_or_default().to_string();
        // A hidden control: kept as it is, its properties with it (its id is given anew).
        if q == "form:hidden" && self.hidden.len() < MAX_CONTROLS {
            self.kept = Some(Kept {
                id: String::new(),
                name: String::new(),
                label: String::new(),
                xml: element_xml(q, a),
                open: 1,
                foreign: !declared(q, a),
            });
            return true;
        }
        if KEPT.contains(&q) && !id.is_empty() && self.controls.len() < MAX_CONTROLS {
            let name = get(a, "form:name").map(capped).unwrap_or_default();
            let label = get(a, "form:label").map(capped).filter(|l| !l.is_empty()).unwrap_or_else(|| name.clone());
            self.kept = Some(Kept { id, name, label, xml: element_xml(q, a), open: 1, foreign: !declared(q, a) });
            return true;
        }
        let name = get(a, "form:name").map(capped).unwrap_or_default();
        let value = || get(a, "form:current-value").or_else(|| get(a, "form:value")).map(capped).unwrap_or_default();
        let kind = match q {
            "form:text" | "form:formatted-text" => Some((ControlKind::Text { multi_line: false }, value())),
            "form:textarea" => Some((ControlKind::Text { multi_line: true }, value())),
            "form:checkbox" | "form:radio" => {
                let radio = q == "form:radio";
                let on = if radio {
                    get(a, "form:current-selected").or_else(|| get(a, "form:selected")) == Some("true")
                } else {
                    get(a, "form:current-state").or_else(|| get(a, "form:state")) == Some("checked")
                };
                let (checked_char, unchecked_char) =
                    if radio { (RADIO_ON, RADIO_OFF) } else { (wordcraft_doc::control::CHECKED, wordcraft_doc::control::UNCHECKED) };
                let symbol = if on { checked_char } else { unchecked_char };
                let kind =
                    ControlKind::CheckBox { checked: on, checked_char, checked_font: String::new(), unchecked_char, unchecked_font: String::new() };
                Some((kind, symbol.to_string()))
            }
            "form:listbox" => Some((ControlKind::DropDown { items: Vec::new(), last_value: String::new() }, String::new())),
            "form:combobox" => Some((ControlKind::ComboBox { items: Vec::new(), last_value: String::new() }, value())),
            "form:date" => {
                let v = value();
                // An xsd:date (`2026-11-01`).
                let full_date =
                    if v.len() >= 10 && v.is_char_boundary(10) { format!("{}T00:00:00Z", v.get(..10).unwrap_or_default()) } else { String::new() };
                Some((
                    ControlKind::Date {
                        full_date,
                        format: "yyyy-MM-dd".into(),
                        lid: String::new(),
                        calendar: String::new(),
                        store_as: String::new(),
                    },
                    v,
                ))
            }
            "form:option" | "form:item" => {
                if let Some((_, c, selected)) = &mut self.open
                    && let ControlKind::DropDown { items, .. } | ControlKind::ComboBox { items, .. } = &mut c.kind
                    && items.len() < MAX_ITEMS
                {
                    let display = get(a, "form:label").map(capped).unwrap_or_default();
                    let value = get(a, "form:value").map(capped).filter(|v| *v != display).unwrap_or_default();
                    if get(a, "form:current-selected").or_else(|| get(a, "form:selected")) == Some("true") {
                        selected.push(display.clone());
                    }
                    items.push(ListItem { display, value });
                }
                None
            }
            _ => None,
        };
        if let Some((kind, text)) = kind
            && !id.is_empty()
            && self.controls.len() < MAX_CONTROLS
        {
            let mut c = ContentControl::new(kind);
            c.tag = name;
            c.title = get(a, "form:label").map(capped).unwrap_or_default();
            self.open = Some((id, c, vec![text]));
        }
        true
    }

    /// Element `q` ends: whether it was part of `office:forms`.
    pub(in crate::odt) fn end(&mut self, q: &str) -> bool {
        if self.depth == 0 {
            return false;
        }
        self.depth -= 1;
        if let Some(k) = &mut self.kept {
            k.xml.push_str(&format!("</{q}>"));
            k.open = k.open.saturating_sub(1);
            if k.open == 0
                && let Some(k) = self.kept.take()
            {
                // Too long to keep whole, or naming what WordCraft can't write: not kept.
                if k.xml.len() <= MAX_KEPT && !k.foreign && k.id.is_empty() {
                    self.hidden.push(k.xml);
                } else if k.xml.len() <= MAX_KEPT && !k.foreign {
                    let mut c = ContentControl::new(ControlKind::Text { multi_line: false });
                    c.tag = k.name;
                    c.lock = wordcraft_doc::control::ControlLock::ContentLocked;
                    c.form_xml = k.xml;
                    self.controls.insert(k.id, (c, k.label));
                }
            }
            return true;
        }
        if matches!(
            q,
            "form:text" | "form:formatted-text" | "form:textarea" | "form:checkbox" | "form:radio" | "form:listbox" | "form:combobox" | "form:date"
        ) && let Some((id, mut c, texts)) = self.open.take()
        {
            // The text: a list box's selected entry, else the value read with the control.
            let mut text = match &mut c.kind {
                ControlKind::DropDown { last_value, .. } => {
                    let sel = texts.get(1).cloned().unwrap_or_default();
                    last_value.clone_from(&sel);
                    sel
                }
                ControlKind::ComboBox { last_value, .. } => {
                    let v = texts.first().cloned().unwrap_or_default();
                    last_value.clone_from(&v);
                    v
                }
                _ => texts.first().cloned().unwrap_or_default(),
            };
            // An empty text control or list shows its placeholder.
            if text.is_empty() && !matches!(c.kind, ControlKind::CheckBox { .. }) {
                c.showing_placeholder = true;
                text = c.placeholder_shown().to_string();
            }
            self.controls.insert(id, (c, text));
        }
        true
    }
}

/// The text of a control's selected value, unless it shows its placeholder.
fn value(c: &ContentControl, text: &str) -> String {
    if c.showing_placeholder { String::new() } else { text.to_string() }
}

/// A content control as a form control in `office:forms`, with id `id`: the element, and the
/// size of its `draw:control` (points). `None` for kinds a form has no control for.
pub(in crate::odt) fn form_xml(c: &ContentControl, text: &str, id: &str, n: usize) -> Option<(String, f32, f32)> {
    let size = |(xml, w, h): (String, f32, f32)| {
        let fit = |v: f32| if v.is_finite() { v.clamp(1.0, 1584.0) } else { 16.0 };
        match c.form_size {
            Some([fw, fh]) => (xml, fit(fw), fit(fh)),
            None => (xml, w, h),
        }
    };
    // A control kept verbatim, with its new id, while it is still the control it was read as
    // (made another kind, it is written as that).
    if !c.form_xml.is_empty() && c.kind == (ControlKind::Text { multi_line: false }) && !is_placeholder(c) {
        let end = c.form_xml.find(|ch: char| ch.is_whitespace() || ch == '>')?;
        let (head, rest) = (c.form_xml.get(..end)?, c.form_xml.get(end..)?);
        return Some(size((format!("{head} xml:id=\"{id}\" form:id=\"{id}\"{rest}"), 72.0, 20.0)));
    }
    let name = if c.tag.is_empty() { format!("Control {n}") } else { c.tag.clone() };
    let common = format!(" form:name=\"{}\" xml:id=\"{id}\" form:id=\"{id}\"", x(&name));
    let label = if c.title.is_empty() { String::new() } else { format!(" form:label=\"{}\"", x(&c.title)) };
    let v = value(c, text);
    // Wide enough for its text (at least a short word), one line high.
    let width = |s: &str| (s.chars().count().clamp(8, 60) as f32 * 6.0 + 16.0).min(432.0);
    // No `form:control-implementation`: the element says what the control is.
    let impl_ = |_: &str| "";
    let out = match &c.kind {
        ControlKind::Text { multi_line: false } => {
            (format!("<form:text{common}{}{} form:current-value=\"{}\"/>", impl_("TextField"), label, x(&v)), width(&v), 16.0)
        }
        ControlKind::Text { multi_line: true } => {
            // Several lines (LibreOffice's `MultiLine` property): without it LibreOffice shows
            // one line, centred in the box.
            (
                format!(
                    "<form:textarea{common}{}{} form:current-value=\"{}\"><form:properties><form:property form:property-name=\"MultiLine\" office:value-type=\"boolean\" office:boolean-value=\"true\"/></form:properties></form:textarea>",
                    impl_("TextField"),
                    label,
                    x(&v)
                ),
                216.0,
                48.0,
            )
        }
        ControlKind::CheckBox { checked, checked_char, .. } if *checked_char == RADIO_ON => (
            format!("<form:radio{common}{}{label} form:current-selected=\"{checked}\" form:selected=\"{checked}\"/>", impl_("RadioButton")),
            width(&c.title).min(18.0 + c.title.chars().count() as f32 * 6.0),
            16.0,
        ),
        ControlKind::CheckBox { checked, .. } => {
            let state = if *checked { "checked" } else { "unchecked" };
            (
                format!("<form:checkbox{common}{}{label} form:current-state=\"{state}\" form:state=\"{state}\"/>", impl_("CheckBox")),
                18.0 + c.title.chars().count() as f32 * 6.0,
                16.0,
            )
        }
        ControlKind::DropDown { items, last_value } => {
            let chosen = if last_value.is_empty() { v.as_str() } else { last_value.as_str() };
            let options: String = items
                .iter()
                .map(|i| {
                    let value = if i.value.is_empty() { String::new() } else { format!(" form:value=\"{}\"", x(&i.value)) };
                    let sel = if !chosen.is_empty() && (i.display == chosen || i.value() == chosen) {
                        " form:current-selected=\"true\" form:selected=\"true\""
                    } else {
                        ""
                    };
                    format!("<form:option form:label=\"{}\"{value}{sel}/>", x(&i.display))
                })
                .collect();
            let widest = items.iter().map(|i| i.display.as_str()).max_by_key(|s| s.chars().count()).unwrap_or("");
            (format!("<form:listbox{common}{} form:dropdown=\"true\">{options}</form:listbox>", impl_("ListBox")), width(widest) + 14.0, 16.0)
        }
        ControlKind::ComboBox { items, .. } => {
            let entries: String = items.iter().map(|i| format!("<form:item form:label=\"{}\"/>", x(&i.display))).collect();
            let widest = items.iter().map(|i| i.display.as_str()).chain([v.as_str()]).max_by_key(|s| s.chars().count()).unwrap_or("");
            (
                format!(
                    "<form:combobox{common}{} form:dropdown=\"true\" form:current-value=\"{}\">{entries}</form:combobox>",
                    impl_("ComboBox"),
                    x(&v)
                ),
                width(widest) + 14.0,
                16.0,
            )
        }
        ControlKind::Date { full_date, .. } => {
            let date = full_date
                .get(..10)
                .filter(|d| d.len() == 10 && d.as_bytes().get(4) == Some(&b'-') && d.as_bytes().get(7) == Some(&b'-'))
                .unwrap_or("");
            let current = if date.is_empty() || c.showing_placeholder { String::new() } else { format!(" form:current-value=\"{date}\"") };
            (format!("<form:date{common}{}{current}/>", impl_("DateField")), 96.0, 16.0)
        }
        _ => return None,
    };
    Some(size(out))
}

/// The `office:forms` holding the controls `xml` (empty when there are none).
pub(in crate::odt) fn forms_xml(xml: &[String], hidden: &[String]) -> String {
    if xml.is_empty() && hidden.is_empty() {
        return String::new();
    }
    // Hidden controls, each with an id of its own.
    let hidden: String = hidden
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let end = h.find(|ch: char| ch.is_whitespace() || ch == '>')?;
            Some(format!("{} xml:id=\"hidden{i}\" form:id=\"hidden{i}\"{}", h.get(..end)?, h.get(end..)?))
        })
        .collect();
    format!(
        "<office:forms form:automatic-focus=\"false\" form:apply-design-mode=\"false\"><form:form form:name=\"Form\">{}{hidden}</form:form></office:forms>",
        xml.concat()
    )
}

/// `Document::passthrough` key of a document's hidden form controls (their XML, one after
/// another), kept for the next save.
pub(in crate::odt) const HIDDEN_PART: &str = "odf-hidden-controls";

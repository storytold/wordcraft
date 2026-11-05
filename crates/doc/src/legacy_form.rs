//! Legacy form fields (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`): what their `w:ffData` says,
//! read from the raw XML a [`crate::InlineObject::Field`] keeps in its `form` (ECMA-376 Part 1
//! §17.16.17). Only what layout and PDF export need; the XML itself stays the source of truth
//! for saving.

/// Most drop-down entries kept (Word allows 25).
pub const MAX_ENTRIES: usize = 64;

/// A legacy form field's kind and state.
#[derive(Clone, Debug, PartialEq)]
pub enum LegacyForm {
    /// A check box: `checked` is its state (`w:checked`, else `w:default`), `size` its explicit
    /// size in points (`w:size`), `None` when it follows the text (`w:sizeAuto`).
    CheckBox { name: String, checked: bool, size: Option<f32> },
    /// A text field; its text is the field's result.
    Text { name: String, max_len: Option<u32> },
    /// A drop-down list showing entry `selected` (`w:result`, else `w:default`, else the first).
    DropDown { name: String, entries: Vec<String>, selected: usize },
}

impl LegacyForm {
    /// The field's definition, or `None` when `ff_data` holds none of the three.
    pub fn parse(ff_data: &str) -> Option<LegacyForm> {
        let name = attr_of(ff_data, "w:name").unwrap_or_default();
        if let Some(cb) = element(ff_data, "w:checkBox") {
            let default = attr_of(cb, "w:default").is_some_and(|v| on(&v));
            // `<w:checked/>` with no value means checked.
            let checked = match tag(cb, "w:checked") {
                Some(t) => attr_in(t).is_none_or(|v| on(&v)),
                None => default,
            };
            let size =
                attr_of(cb, "w:size").and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite() && *v > 0.0).map(|v| (v / 2.0).min(1584.0));
            return Some(LegacyForm::CheckBox { name, checked, size });
        }
        if let Some(dd) = element(ff_data, "w:ddList") {
            let entries: Vec<String> = tags(dd, "w:listEntry").filter_map(attr_in).take(MAX_ENTRIES).collect();
            let pick = |n: &str| attr_of(dd, n).and_then(|v| v.trim().parse::<usize>().ok());
            let selected = pick("w:result").or_else(|| pick("w:default")).filter(|i| *i < entries.len()).unwrap_or(0);
            return Some(LegacyForm::DropDown { name, entries, selected });
        }
        if ff_data.contains("<w:textInput") {
            let max_len =
                element(ff_data, "w:textInput").and_then(|t| attr_of(t, "w:maxLength")).and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| *n > 0);
            return Some(LegacyForm::Text { name, max_len });
        }
        None
    }

    /// The text a drop-down shows (its selected entry); `None` for other kinds or an empty list.
    pub fn shown_text(&self) -> Option<&str> {
        match self {
            LegacyForm::DropDown { entries, selected, .. } => entries.get(*selected).map(String::as_str),
            _ => None,
        }
    }

    /// The field's bookmark-style name (`w:name`), possibly empty.
    pub fn name(&self) -> &str {
        match self {
            LegacyForm::CheckBox { name, .. } | LegacyForm::Text { name, .. } | LegacyForm::DropDown { name, .. } => name,
        }
    }
}

fn on(v: &str) -> bool {
    !matches!(v.trim(), "0" | "false" | "off")
}

/// The element `name` in `xml` with its content: from its start tag to its end tag, or the
/// self-closing tag alone.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = tag(xml, name)?;
    let start = xml.find(open)?;
    if open.ends_with("/>") {
        return Some(open);
    }
    let close = format!("</{name}>");
    let end = xml.get(start..)?.find(&close)? + start + close.len();
    xml.get(start..end)
}

/// The first start (or empty) tag `<name ...>` in `xml`.
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    tags(xml, name).next()
}

/// Every start (or empty) tag `<name ...>` in `xml`, in order.
fn tags<'a>(xml: &'a str, name: &str) -> impl Iterator<Item = &'a str> + use<'a> {
    let pat = format!("<{name}");
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let at = from + xml.get(from..)?.find(&pat)?;
            let after = at + pat.len();
            from = after;
            // `<w:checkBox` must not match `<w:checkBoxes`: the name ends at a space, `/` or `>`.
            if xml.get(after..).and_then(|s| s.chars().next()).is_some_and(|c| c == ' ' || c == '/' || c == '>') {
                let end = after + xml.get(after..)?.find('>')? + 1;
                return xml.get(at..end);
            }
        }
    })
}

/// The `w:val` of the first tag `name` in `xml`.
fn attr_of(xml: &str, name: &str) -> Option<String> {
    tag(xml, name).and_then(attr_in)
}

/// The `w:val` of a single tag, XML entities resolved.
fn attr_in(tag: &str) -> Option<String> {
    let at = tag.find("w:val=")? + "w:val=".len();
    let quote = tag.get(at..)?.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = tag.get(at + 1..)?;
    let end = rest.find(quote)?;
    Some(unescape(rest.get(..end)?))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_boxes_text_fields_and_drop_downs() {
        let cb = |inner: &str| format!(r#"<w:ffData><w:name w:val="Box1"/><w:enabled/><w:checkBox>{inner}</w:checkBox></w:ffData>"#);
        assert_eq!(
            LegacyForm::parse(&cb(r#"<w:sizeAuto/><w:default w:val="0"/>"#)),
            Some(LegacyForm::CheckBox { name: "Box1".into(), checked: false, size: None })
        );
        assert_eq!(
            LegacyForm::parse(&cb(r#"<w:sizeAuto/><w:default w:val="1"/>"#)).map(|f| matches!(f, LegacyForm::CheckBox { checked: true, .. })),
            Some(true)
        );
        // `w:checked` wins over `w:default`; without a value it means checked.
        assert!(matches!(
            LegacyForm::parse(&cb(r#"<w:default w:val="1"/><w:checked w:val="0"/>"#)),
            Some(LegacyForm::CheckBox { checked: false, .. })
        ));
        assert!(matches!(LegacyForm::parse(&cb(r#"<w:default w:val="0"/><w:checked/>"#)), Some(LegacyForm::CheckBox { checked: true, .. })));
        assert!(matches!(
            LegacyForm::parse(&cb(r#"<w:size w:val="30"/><w:default w:val="0"/>"#)),
            Some(LegacyForm::CheckBox { size: Some(15.0), .. })
        ));
        let dd = r#"<w:ffData><w:name w:val="List1"/><w:ddList><w:result w:val="1"/><w:listEntry w:val="Alpha"/><w:listEntry w:val="B &amp; C"/></w:ddList></w:ffData>"#;
        let f = LegacyForm::parse(dd).unwrap();
        assert_eq!(f.shown_text(), Some("B & C"));
        assert_eq!(f.name(), "List1");
        let dd0 = r#"<w:ffData><w:ddList><w:listEntry w:val="Only"/></w:ddList></w:ffData>"#;
        assert_eq!(LegacyForm::parse(dd0).and_then(|f| f.shown_text().map(str::to_string)).as_deref(), Some("Only"));
        let tx = r#"<w:ffData><w:name w:val="Name1"/><w:textInput><w:maxLength w:val="40"/></w:textInput></w:ffData>"#;
        assert_eq!(LegacyForm::parse(tx), Some(LegacyForm::Text { name: "Name1".into(), max_len: Some(40) }));
        assert_eq!(LegacyForm::parse(""), None);
        assert_eq!(LegacyForm::parse("<w:ffData><w:name w:val=\"x\"/></w:ffData>"), None);
    }
}

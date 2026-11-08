//! Form controls (ODF 1.3 §13): `office:forms` controls placed by `draw:control`, read as
//! WordCraft content controls and written back as form controls LibreOffice shows.

use crate::model::Inline;
use crate::odt::tests::{NS_T, entry, package, paras};
use crate::odt::{export, import, parse};
use wordcraft_doc::control::{ControlKind, ListItem};

const NS_F: &str = r#"xmlns:form="urn:oasis:names:tc:opendocument:xmlns:form:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// LibreOffice's form controls: a text field, a check box, a list box (Green chosen), a combo
/// box, a multi-line field, a radio button, a date field, an empty field and a button.
const FORMS: &str = r#"<office:forms form:automatic-focus="false" form:apply-design-mode="false"><form:form form:name="Form"><form:text form:name="FullName" xml:id="control1" form:id="control1" form:current-value="Ada Lovelace"><form:properties><form:property form:property-name="x" office:value-type="string" office:string-value="y"/></form:properties></form:text><form:checkbox form:name="Subscribe" xml:id="control2" form:id="control2" form:label="Yes please" form:current-state="checked"/><form:listbox form:name="Colour" xml:id="control3" form:id="control3" form:dropdown="true"><form:option form:label="Red"/><form:option form:label="Green" form:current-selected="true" form:selected="true"/><form:option form:label="Blue" form:value="b"/></form:listbox><form:combobox form:name="City" xml:id="control4" form:id="control4" form:current-value="Bergen"><form:item form:label="Oslo"/><form:item form:label="Bergen"/></form:combobox><form:textarea form:name="Notes" xml:id="control5" form:id="control5" form:current-value="Line one"/><form:radio form:name="Size" xml:id="control6" form:id="control6" form:label="Large" form:current-selected="true"/><form:date form:name="When" xml:id="control7" form:id="control7" form:current-value="2026-11-01"/><form:text form:name="Empty" xml:id="control8" form:id="control8"/><form:button form:name="Submit" xml:id="control9" form:id="control9" form:label="Send"/></form:form></office:forms>"#;

fn doc_xml() -> String {
    let ctl = |id: &str| format!(r#"<draw:control text:anchor-type="as-char" svg:width="1in" svg:height="0.2in" draw:control="{id}"/>"#);
    let p: String = (1..=9).map(|i| format!("<text:p>Field {i}: {}</text:p>", ctl(&format!("control{i}")))).collect();
    format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text>{FORMS}{p}<text:p>After {}</text:p></office:text></office:body></office:document-content>"#,
        ctl("nothing")
    )
}

/// Each paragraph's controls: (kind, tag, title, text, showing placeholder).
fn controls(flow: &crate::model::Flow) -> Vec<(ControlKind, String, String, String, bool)> {
    paras(flow)
        .into_iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| match i {
            Inline::Control { control, text, .. } => {
                Some((control.kind.clone(), control.tag.clone(), control.title.clone(), text.clone(), control.showing_placeholder))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn form_controls_open_as_content_controls() {
    let flow = parse(&package(&doc_xml(), None)).unwrap();
    let got = controls(&flow);
    assert_eq!(got.len(), 9, "a missing control is left out: {got:?}");
    // A button, which WordCraft has no kind for, shows its label (see `buttons_are_kept`).
    assert_eq!((got[8].1.as_str(), got[8].3.as_str()), ("Submit", "Send"));
    assert_eq!(got[0], (ControlKind::Text { multi_line: false }, "FullName".into(), String::new(), "Ada Lovelace".into(), false));
    assert!(matches!(got[1].0, ControlKind::CheckBox { checked: true, checked_char: '\u{2612}', unchecked_char: '\u{2610}', .. }));
    assert_eq!((got[1].1.as_str(), got[1].2.as_str(), got[1].3.as_str()), ("Subscribe", "Yes please", "\u{2612}"));
    let items = vec![
        ListItem { display: "Red".into(), value: String::new() },
        ListItem { display: "Green".into(), value: String::new() },
        ListItem { display: "Blue".into(), value: "b".into() },
    ];
    assert_eq!(got[2], (ControlKind::DropDown { items, last_value: "Green".into() }, "Colour".into(), String::new(), "Green".into(), false));
    assert!(matches!(&got[3].0, ControlKind::ComboBox { items, .. } if items.len() == 2) && got[3].3 == "Bergen");
    assert_eq!((got[4].0.clone(), got[4].3.as_str()), (ControlKind::Text { multi_line: true }, "Line one"));
    assert!(matches!(got[5].0, ControlKind::CheckBox { checked: true, checked_char: '\u{25C9}', .. }) && got[5].3 == "\u{25C9}");
    assert!(matches!(&got[6].0, ControlKind::Date { full_date, .. } if full_date == "2026-11-01T00:00:00Z") && got[6].3 == "2026-11-01");
    // An empty field shows its placeholder.
    assert!(got[7].4 && got[7].3 == "Type here.");
    // The forms' own elements aren't text.
    assert!(paras(&flow).iter().all(|p| !matches!(p.inlines.first(), Some(Inline::Text(t, _)) if t.contains('y') && t.len() == 1)));
}

#[test]
fn content_controls_are_saved_as_form_controls() {
    let doc = import(&package(&doc_xml(), None)).unwrap();
    let saved = export(&doc).unwrap();
    let content = entry(&saved, "content.xml");
    for want in [
        r#"<office:forms form:automatic-focus="false" form:apply-design-mode="false"><form:form form:name="Form""#,
        r#"<form:text form:name="FullName" xml:id="control1" form:id="control1""#,
        r#"form:current-value="Ada Lovelace""#,
        r#"<form:checkbox form:name="Subscribe""#,
        r#"form:label="Yes please" form:current-state="checked""#,
        r#"<form:option form:label="Green" form:current-selected="true" form:selected="true"/>"#,
        r#"<form:option form:label="Blue" form:value="b"/>"#,
        r#"<form:combobox form:name="City""#,
        r#"<form:textarea form:name="Notes""#,
        r#"<form:radio form:name="Size""#,
        r#"<form:date form:name="When""#,
        r#"form:current-value="2026-11-01""#,
        r#"draw:control="control1""#,
    ] {
        assert!(content.contains(want), "{want} in {content}");
    }
    // `office:forms` comes first in the text (§13.2's place for it).
    assert!(content.contains("<office:text>\n<office:forms"), "{content}");
    // The empty field is saved empty, not with its placeholder.
    assert!(content.contains(r#"<form:text form:name="Empty" xml:id="control8" form:id="control8" form:current-value=""/>"#), "{content}");
    // And they open again the same.
    assert_eq!(controls(&parse(&saved).unwrap()), controls(&parse(&package(&doc_xml(), None)).unwrap()));
}

#[test]
fn hostile_forms_are_bounded() {
    let many: String = (0..20_000).map(|i| format!(r#"<form:text xml:id="c{i}" form:id="c{i}" form:current-value="v"/>"#)).collect();
    let items: String = (0..5000).map(|i| format!(r#"<form:option form:label="o{i}"/>"#)).collect();
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form>{many}<form:listbox form:id="big">{items}</form:listbox><form:text/></form:form></office:forms><text:p><draw:control draw:control="big"/><draw:control draw:control="c5"/><draw:control/></text:p></office:text></office:body></office:document-content>"#
    );
    let flow = parse(&package(&content, None)).unwrap();
    let got = controls(&flow);
    // Ids past the limit aren't read; a list keeps at most its limit of entries.
    assert!(got.iter().all(|(k, ..)| k.items().is_none_or(|i| i.len() <= 1000)));
    assert!(got.len() <= 2);
}

#[test]
fn controls_keep_their_size() {
    let saved = export(&import(&package(&doc_xml(), None)).unwrap()).unwrap();
    let content = entry(&saved, "content.xml");
    // Each control 1in × 0.2in, as it was, not sized to its text.
    assert_eq!(content.matches(r#"svg:width="72pt" svg:height="14.4pt""#).count(), 9, "{content}");
}

#[test]
fn buttons_are_kept() {
    let button = r#"<form:button form:name="Go" form:control-implementation="ooo:com.sun.star.form.component.CommandButton" xml:id="b1" form:id="b1" form:label="Send &amp; close" form:image-position="center"><form:properties><form:property form:property-name="DefaultControl" office:value-type="string" office:string-value="com.sun.star.form.control.CommandButton"/></form:properties></form:button>"#;
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form form:name="Form">{button}</form:form></office:forms><text:p>Press <draw:control text:anchor-type="as-char" svg:width="1in" svg:height="0.3in" draw:control="b1"/></text:p></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, None)).unwrap();
    // It shows its label, which can't be edited.
    let p = doc.body[0].as_para().unwrap();
    assert!(p.text.contains("Send & close"), "{:?}", p.text);
    let saved = export(&doc).unwrap();
    let c = entry(&saved, "content.xml");
    assert!(
        c.contains(r#"<form:button xml:id="control1" form:id="control1" form:name="Go" form:control-implementation="ooo:com.sun.star.form.component.CommandButton" form:label="Send &amp; close" form:image-position="center"><form:properties><form:property form:property-name="DefaultControl""#),
        "{c}"
    );
    assert!(c.contains(r#"svg:width="72pt" svg:height="21.6pt" draw:control="control1""#), "{c}");
    // Made a check box, it is saved as one.
    let mut c = wordcraft_doc::control::ContentControl::new(ControlKind::Text { multi_line: false });
    c.form_xml = "<form:button form:name=\"Go\"></form:button>".into();
    let kept = crate::odt::forms::form_xml(&c, "Go", "control1", 1).unwrap().0;
    assert!(kept.starts_with(r#"<form:button xml:id="control1" form:id="control1" form:name="Go">"#), "{kept}");
    c.kind = ControlKind::CheckBox {
        checked: true,
        checked_char: '\u{2612}',
        checked_font: String::new(),
        unchecked_char: '\u{2610}',
        unchecked_font: String::new(),
    };
    let made = crate::odt::forms::form_xml(&c, "\u{2612}", "control1", 1).unwrap().0;
    assert!(made.starts_with("<form:checkbox "), "{made}");
    // And again: still a button.
    let again = entry(&export(&import(&saved).unwrap()).unwrap(), "content.xml");
    assert_eq!(again.matches("<form:button ").count(), 1, "{again}");
}

#[test]
fn hostile_kept_controls_are_bounded() {
    let deep = format!("{}{}", "<form:properties>".repeat(5000), "</form:properties>".repeat(5000));
    let wide =
        r#"<form:property form:property-name="p" office:string-value="vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv"/>"#
            .repeat(5000);
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form><form:button form:id="a">{deep}</form:button><form:button form:id="b">{wide}</form:button><form:button form:label="no id"/><form:image form:id="c"><form:button form:id="d"/></form:image></form:form></office:forms><text:p><draw:control draw:control="a" svg:width="-1in" svg:height="1e40in"/><draw:control draw:control="b"/><draw:control draw:control="c"/><draw:control draw:control="d"/></text:p></office:text></office:body></office:document-content>"#
    );
    let doc = import(&package(&content, None)).unwrap();
    let c = entry(&export(&doc).unwrap(), "content.xml");
    // Too long to keep whole: left out; a control inside a kept one is part of it.
    assert_eq!(c.matches("<form:image ").count(), 1, "{c}");
    assert!(!c.contains(r#"office:string-value="vvvv"#), "the over-long button isn't kept");
    // One naming a namespace prefix the saved content doesn't declare isn't kept.
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form><form:button form:id="a" xmlns:odd="urn:x"><odd:thing/></form:button><form:button form:id="b" odd:attr="1"/><form:button form:id="c"><office:event-listeners><script:event-listener script:language="ooo:script" script:event-name="dom:click" xlink:href="vnd.sun.star.script:x"/></office:event-listeners></form:button></form:form></office:forms><text:p><draw:control draw:control="a"/><draw:control draw:control="b"/><draw:control draw:control="c"/></text:p></office:text></office:body></office:document-content>"#
    );
    let c = entry(&export(&import(&package(&content, None)).unwrap()).unwrap(), "content.xml");
    assert_eq!(c.matches("<form:button ").count(), 1, "{c}");
    assert!(c.contains("<script:event-listener "), "{c}");
}

#[test]
fn hidden_controls_are_kept() {
    // A hidden control (a value the form sends, not shown) has no place in the text.
    let hidden = r#"<form:hidden form:name="Source" xml:id="control1" form:id="control1" form:value="web &amp; mail"><form:properties><form:property form:property-name="Tag" office:value-type="string" office:string-value="t"/></form:properties></form:hidden>"#;
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form form:name="Form">{hidden}<form:text form:name="Name" form:id="control2"/></form:form></office:forms><text:p>Name <draw:control draw:control="control2"/></text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, None)).unwrap()).unwrap();
    let c = entry(&saved, "content.xml");
    assert!(
        c.contains(r#"<form:hidden xml:id="hidden0" form:id="hidden0" form:name="Source" form:value="web &amp; mail"><form:properties><form:property form:property-name="Tag""#),
        "{c}"
    );
    // Its id doesn't clash with the text control's, and it survives another save.
    assert_eq!(c.matches(r#"form:id="control1""#).count(), 1, "{c}");
    let again = entry(&export(&import(&saved).unwrap()).unwrap(), "content.xml");
    assert_eq!(again.matches("<form:hidden ").count(), 1, "{again}");
    // Only hidden controls: still written.
    let only = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form>{hidden}</form:form></office:forms><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let c = entry(&export(&import(&package(&only, None)).unwrap()).unwrap(), "content.xml");
    assert!(c.contains("<office:forms") && c.contains("<form:hidden "), "{c}");
}

#[test]
fn controls_keep_their_place_on_the_line_and_a_text_area_its_lines() {
    // LibreOffice hangs controls it makes from the baseline (`from-top`, `svg:y` 0); one saved
    // without a placement stands at the line's top. A text area is several lines only with its
    // `MultiLine` property.
    let content = doc_xml()
        .replace(r#"<office:body>"#, r#"<office:automatic-styles><style:style style:name="gr1" style:family="graphic"><style:graphic-properties style:vertical-pos="from-top"/></style:style></office:automatic-styles><office:body>"#)
        .replace(r#"<draw:control text:anchor-type="as-char""#, r#"<draw:control draw:style-name="gr1" text:anchor-type="as-char" svg:y="0in""#);
    let doc = import(&package(&content, None)).unwrap();
    let raises: Vec<Option<f32>> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let wordcraft_doc::para::InlineObject::ControlStart { control } = o { Some(control.form_raise) } else { None })
        .collect();
    assert!(!raises.is_empty() && raises.iter().all(|r| r.is_some_and(|v| (v + 14.4).abs() < 0.01)), "{raises:?}");
    let out = entry(&export(&doc).unwrap(), "content.xml");
    assert!(out.contains(r#"style:vertical-pos="from-top" style:vertical-rel="baseline""#) && out.contains(r#"svg:y="0pt""#), "{out}");
    assert!(out.contains(r#"form:property-name="MultiLine" office:value-type="boolean" office:boolean-value="true""#), "{out}");
    // Saved without a raise, a control still says it stands on the baseline.
    let plain = entry(&export(&import(&package(&doc_xml(), None)).unwrap()).unwrap(), "content.xml");
    assert!(plain.contains(r#"style:vertical-pos="top" style:vertical-rel="baseline""#), "{plain}");
}

#[test]
fn password_number_time_and_grid_controls_are_kept() {
    // ODF 1.3 §13.5 controls WordCraft has no kind for: kept as read, each in its place.
    let controls = r#"<form:password form:name="Secret" xml:id="p1" form:id="p1" form:echo-char="*"/><form:number form:name="Count" xml:id="n1" form:id="n1" form:value="3"/><form:time form:name="At" xml:id="t1" form:id="t1" form:value="10:30:00"/><form:grid form:name="Rows" xml:id="g1" form:id="g1"><form:column form:name="Col" form:label="Name"><form:text form:name="Cell"/></form:column></form:grid>"#;
    let anchor = |id: &str| format!(r#"<draw:control text:anchor-type="as-char" svg:width="1in" svg:height="0.3in" draw:control="{id}"/>"#);
    let content = format!(
        r#"<office:document-content {NS_T} {NS_F}><office:body><office:text><office:forms><form:form form:name="Form">{controls}</form:form></office:forms><text:p>Fill {} {} {} {}</text:p></office:text></office:body></office:document-content>"#,
        anchor("p1"),
        anchor("n1"),
        anchor("t1"),
        anchor("g1")
    );
    let saved = export(&import(&package(&content, None)).unwrap()).unwrap();
    let c = entry(&saved, "content.xml");
    for el in [r#"<form:password "#, r#"form:echo-char="*""#, "<form:number ", "<form:time ", "<form:grid ", "<form:column ", r#"form:label="Name""#]
    {
        assert!(c.contains(el), "{el}: {c}");
    }
    // Each in its place in the text, naming its control.
    assert_eq!(c.matches("<draw:control ").count(), 4, "{c}");
    // And again.
    let again = entry(&export(&import(&saved).unwrap()).unwrap(), "content.xml");
    assert!(again.contains("<form:password ") && again.contains("<form:grid ") && again.matches("<draw:control ").count() == 4, "{again}");
}

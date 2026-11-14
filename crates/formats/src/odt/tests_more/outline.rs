//! Heading numbering (ODF 1.3 §16.34 `text:outline-style`): the outline style's levels number the
//! headings of those levels; WordCraft numbers them through the heading styles, as Word does.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::section::NumFormat;

fn open(outline: &str) -> Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"/><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:default-outline-level="2"/>{outline}</office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">One</text:h><text:h text:style-name="Heading_20_2" text:outline-level="2">One one</text:h><text:p>Body</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

const NUMBERED: &str = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format="1" style:num-suffix="."/><text:outline-level-style text:level="2" style:num-format="1" style:num-suffix="." text:display-levels="2"/><text:outline-level-style text:level="3" style:num-format=""/></text:outline-style>"#;

/// Each heading style's numbering: (its level's format and text).
fn heading_numbers(d: &Document) -> Vec<Option<(NumFormat, String)>> {
    (1..=3)
        .map(|n| {
            let (pp, _) = d.styles.para_style_props(&format!("Heading{n}"));
            let r = pp.numbering?;
            let lv = d.numbering.level(r.num, r.level)?;
            (lv.format != NumFormat::None).then(|| (lv.format, lv.text.clone()))
        })
        .collect()
}

#[test]
fn an_outline_style_numbers_the_headings() {
    let d = open(NUMBERED);
    assert_eq!(heading_numbers(&d), [Some((NumFormat::Decimal, "%1.".into())), Some((NumFormat::Decimal, "%1.%2.".into())), None]);
    // Both levels are one list, so 1.1 follows 1.
    let num = |n: u32| d.styles.para_style_props(&format!("Heading{n}")).0.numbering.map(|r| (r.num, r.level));
    assert_eq!(num(1).map(|n| n.0), num(2).map(|n| n.0));
    assert_eq!((num(1).map(|n| n.1), num(2).map(|n| n.1)), (Some(0), Some(1)));
}

#[test]
fn an_unnumbered_outline_numbers_nothing() {
    let none = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format=""/></text:outline-style>"#;
    assert_eq!(heading_numbers(&open(none)), [None, None, None]);
    assert_eq!(heading_numbers(&open("")), [None, None, None]);
}

#[test]
fn heading_numbering_round_trips() {
    let d = open(NUMBERED);
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(heading_numbers(&back), heading_numbers(&d));
    // A document without numbered headings stays without.
    let plain = open("");
    assert_eq!(heading_numbers(&import(&export(&plain).unwrap()).unwrap()), [None, None, None]);
}

#[test]
fn exported_heading_numbers_are_followed_by_a_tab() {
    // Without label alignment LibreOffice set the number against the heading's text.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open(NUMBERED)).unwrap());
    let outline = xml.split("<text:outline-style").nth(1).unwrap_or("");
    assert!(outline.contains(r#"text:label-followed-by="listtab""#), "{outline}");
    assert!(outline.contains(r#"text:list-level-position-and-space-mode="label-alignment""#));
}

#[test]
fn numbered_headings_are_not_written_as_list_items() {
    // The outline style numbers them; wrapped in a list they took that list's indents.
    let xml = crate::odt::tests::content_of(&export(&open(NUMBERED)).unwrap());
    assert!(!xml.contains("<text:list"), "{xml}");
    assert!(xml.contains(r#"<text:h text:style-name="Heading_20_2" text:outline-level="2">One one</text:h>"#), "{xml}");
}

#[test]
fn unnumbered_heading_styles_are_detached_from_the_outline() {
    // LibreOffice gives a heading style the outline's numbering unless it names no list style;
    // an unnumbered level then still indented the heading.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open("")).unwrap());
    let h1 = xml.split("<style:style ").find(|s| s.starts_with(r#"style:name="Heading_20_1""#)).unwrap_or("");
    assert!(h1.contains(r#"style:list-style-name="""#), "{h1}");
    // A numbered level stays attached.
    let xml = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&open(NUMBERED)).unwrap());
    let h1 = xml.split("<style:style ").find(|s| s.starts_with(r#"style:name="Heading_20_1""#)).unwrap_or("");
    assert!(!h1.contains("style:list-style-name"), "{h1}");
}

#[test]
fn a_heading_looks_as_its_style_and_has_the_level_its_element_gives() {
    // Google Docs' 2011 export: `<text:h>` without `text:outline-level` (level 1, §19.850.4) in a
    // style based on Heading 2. It looks as Heading 2 (as LibreOffice draws it) and is at outline
    // level 1; saved, both stay.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph"><style:text-properties fo:font-size="18pt" fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P7" style:family="paragraph" style:parent-style-name="Heading_20_2"/></office:automatic-styles><office:body><office:text><text:h text:style-name="P7">Clarifying the beat</text:h><text:h text:style-name="P7" text:outline-level="2">Second</text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let props = |d: &Document, i: usize| d.body[i].as_para().unwrap().props.clone();
    let p = props(&d, 0);
    assert_eq!((p.style.as_deref(), p.outline_level), (Some("Heading2"), Some(0)));
    let p = props(&d, 1);
    assert_eq!(p.style.as_deref(), Some("Heading2"));
    assert!(p.outline_level.is_none() || p.outline_level == Some(1));
    assert_eq!(d.styles.resolve_char(p.style.as_deref(), &Default::default()).size, 18.0);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"text:outline-level="1">Clarifying"#) || c.contains(r#"text:outline-level="1"><"#), "{c}");
    let back = import(&saved).unwrap();
    assert_eq!((props(&back, 0).style.as_deref(), props(&back, 0).outline_level), (Some("Heading2"), Some(0)));
}

/// A document with these common `styles` and this `body`.
fn in_custom_style(styles: &str, body: &str) -> Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/>{styles}</office:styles></office:document-styles>"#
    );
    let content =
        format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    import(&package(&content, Some(&styles))).unwrap()
}

const HEAD: &str = r#"<style:style style:name="Head" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="1" style:class="text"><style:text-properties fo:font-size="26pt" fo:font-weight="bold"/></style:style>"#;

fn saved_after_an_edit(d: &mut Document) -> Vec<u8> {
    d.insert_text(&wordcraft_doc::Pos::body(0, 0), "Yearly ", &Default::default()).unwrap();
    export(d).unwrap()
}

#[test]
fn a_custom_style_with_an_outline_level_stays_the_paragraphs_style() {
    // The outline level is the style's; the look stays the file's own, not Heading 1's.
    let mut d = in_custom_style(HEAD, r#"<text:p text:style-name="Head">Annual report edition</text:p>"#);
    let p = d.body[0].as_para().unwrap().props.clone();
    assert_eq!(p.style.as_deref(), Some("Head"));
    assert_eq!(d.styles.resolve_para(&p).outline_level, Some(0), "still a heading in WordCraft");
    let look = d.styles.resolve_char(p.style.as_deref(), &Default::default());
    assert_eq!((look.size, look.bold), (26.0, true));
    let r = d.body[0].as_para().unwrap().runs.first().unwrap().props.clone();
    assert!(r.size.is_none() && r.bold.is_none(), "the look stays in the style: {r:?}");

    // Edited, it is written from the model, under its own name, at its level.
    let saved = saved_after_an_edit(&mut d);
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:h text:style-name="Head" text:outline-level="1">Yearly Annual"#), "{c}");
    let s = crate::odt::tests::styles_of(&saved);
    let head = s.split("<style:style ").find(|x| x.starts_with(r#"style:name="Head""#)).unwrap_or("");
    assert!(head.contains(r#"style:default-outline-level="1""#) && head.contains(r#"fo:font-size="26pt""#), "{head}");

    // And read back, it is the same heading.
    let back = import(&saved).unwrap();
    let p = back.body[0].as_para().unwrap().props.clone();
    assert_eq!((p.style.as_deref(), back.styles.resolve_para(&p).outline_level), (Some("Head"), Some(0)));
}

#[test]
fn an_unedited_paragraph_in_such_a_style_is_written_as_read() {
    let body = r#"<text:p text:style-name="Head">Annual report edition</text:p>"#;
    let saved = export(&in_custom_style(HEAD, body)).unwrap();
    assert!(crate::odt::tests::content_of(&saved).contains(body));
    let s = crate::odt::tests::styles_of(&saved);
    assert_eq!(s.matches(r#"style:name="Head""#).count(), 1, "{s}");
    assert!(s.contains(r#"style:default-outline-level="1""#), "{s}");
}

#[test]
fn every_level_of_such_a_style_is_written_as_it_is() {
    // The paragraph kinds stop at Heading 6; the outline levels go on, and a style based on another
    // heading has its own.
    let styles = r#"<style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph"/><style:style style:name="Deep" style:family="paragraph" style:default-outline-level="8"/><style:style style:name="Under" style:family="paragraph" style:parent-style-name="Heading_20_2" style:default-outline-level="3"/><style:style style:name="Child" style:family="paragraph" style:parent-style-name="Deep"/>"#;
    let body =
        r#"<text:p text:style-name="Deep">deep</text:p><text:p text:style-name="Under">under</text:p><text:p text:style-name="Child">child</text:p>"#;
    let d = in_custom_style(styles, body);
    let level = |d: &Document, i: usize| d.styles.resolve_para(&d.body[i].as_para().unwrap().props).outline_level;
    assert_eq!([level(&d, 0), level(&d, 1), level(&d, 2)], [Some(7), Some(2), Some(7)]);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:h text:style-name="Deep" text:outline-level="8">deep"#), "{c}");
    assert!(c.contains(r#"<text:h text:style-name="Under" text:outline-level="3">under"#), "{c}");
    assert!(c.contains(r#"<text:h text:style-name="Child" text:outline-level="8">child"#), "{c}");
    let back = import(&saved).unwrap();
    assert_eq!([level(&back, 0), level(&back, 1), level(&back, 2)], [Some(7), Some(2), Some(7)]);
}

#[test]
fn a_paragraphs_own_level_and_formatting_stay_over_such_a_style() {
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{HEAD}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Head"><style:paragraph-properties fo:text-align="center"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="P1" text:outline-level="2">Moved</text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let p = d.body[0].as_para().unwrap().props.clone();
    assert_eq!((p.style.as_deref(), p.outline_level, p.align), (Some("Head"), Some(1), Some(wordcraft_doc::Align::Center)));
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"text:outline-level="2">Moved"#), "{c}");
    assert!(c.contains(r#"style:parent-style-name="Head""#) && c.contains(r#"fo:text-align="center""#), "{c}");
}

#[test]
fn a_hostile_outline_level_on_a_custom_style_is_no_heading() {
    for bad in ["0", "11", "255", "999", "-1", "x", ""] {
        let styles = format!(r#"<style:style style:name="Head" style:family="paragraph" style:default-outline-level="{bad}"/>"#);
        let d = in_custom_style(&styles, r#"<text:p text:style-name="Head">Plain</text:p>"#);
        let p = d.body[0].as_para().unwrap().props.clone();
        assert_eq!((p.style.as_deref(), d.styles.resolve_para(&p).outline_level), (Some("Head"), None), "{bad}");
        let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
        assert!(c.contains(r#"<text:p text:style-name="Head">Plain"#), "{bad}: {c}");
    }
}

#[test]
fn such_a_style_is_numbered_with_its_level() {
    // LibreOffice numbers it through the outline style, one list with the level's headings.
    let d = in_custom_style(
        &format!(
            r#"{HEAD}<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"/>{NUMBERED}"#
        ),
        r#"<text:p text:style-name="Head">First</text:p><text:h text:style-name="Heading_20_1" text:outline-level="1">Second</text:h>"#,
    );
    let num = |d: &Document, i: usize| d.styles.resolve_para(&d.body[i].as_para().unwrap().props).numbering;
    assert!(num(&d, 0).is_some());
    assert_eq!(num(&d, 0), num(&d, 1));
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(!c.contains("<text:list"), "the outline numbers it: {c}");
    assert!(c.contains(r#"<text:h text:style-name="Head" text:outline-level="1">First"#), "{c}");
    // A numbered level stays attached to the outline; an unnumbered one doesn't take its indent.
    let head = |xml: String| xml.split("<style:style ").find(|x| x.starts_with(r#"style:name="Head""#)).map(String::from).unwrap_or_default();
    assert!(!head(crate::odt::tests::styles_of(&saved)).contains("style:list-style-name"));
    let back = import(&saved).unwrap();
    assert!(num(&back, 0).is_some());
    assert_eq!(num(&back, 0), num(&back, 1));
    let plain = in_custom_style(HEAD, r#"<text:p text:style-name="Head">First</text:p>"#);
    let s = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&plain).unwrap());
    assert!(head(s).contains(r#"style:list-style-name="""#));
}

#[test]
fn custom_styles_based_on_any_kinds_style_keep_their_name_when_edited() {
    // The kind's style took their place, without their own look.
    let styles = r##"<style:style style:name="Title" style:family="paragraph"/><style:style style:name="Quotations" style:family="paragraph"/><style:style style:name="Preformatted_20_Text" style:display-name="Preformatted Text" style:family="paragraph"/><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph"/><style:style style:name="Banner" style:family="paragraph" style:parent-style-name="Title"><style:text-properties fo:color="#cc6600"/></style:style><style:style style:name="Aside" style:family="paragraph" style:parent-style-name="Quotations"/><style:style style:name="Listing" style:family="paragraph" style:parent-style-name="Preformatted_20_Text"/><style:style style:name="Plaque" style:family="paragraph" style:parent-style-name="Heading_20_2"/>"##;
    let body = r#"<text:p text:style-name="Banner">b</text:p><text:p text:style-name="Aside">a</text:p><text:p text:style-name="Listing">l</text:p><text:p text:style-name="Plaque">p</text:p>"#;
    let saved = crate::odt::tests::export_model(&in_custom_style(styles, body)).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    for want in [
        r#"<text:p text:style-name="Banner">b"#,
        r#"<text:p text:style-name="Aside">a"#,
        r#"<text:p text:style-name="Listing">l"#,
        r#"<text:h text:style-name="Plaque" text:outline-level="2">p"#,
    ] {
        assert!(c.contains(want), "{want} in {c}");
    }
    let s = crate::odt::tests::styles_of(&saved);
    assert!(s.contains(r#"style:name="Banner" style:display-name="Banner" style:family="paragraph" style:parent-style-name="Title""#), "{s}");
}

/// A heading's number is its own outline level's, as in LibreOffice, not its style's: `(level)` of
/// each paragraph's number, as layout takes it.
fn number_levels(d: &Document) -> Vec<Option<u8>> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| p.props.numbering.or(d.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0).map(|n| n.level))
        .collect()
}

#[test]
fn a_headings_own_outline_level_gives_its_number() {
    let outline = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format="1" style:num-suffix="."/><text:outline-level-style text:level="2" style:num-format="1" text:display-levels="2" style:num-suffix="."/><text:outline-level-style text:level="3" style:num-format=""/></text:outline-style>"#;
    let styles = format!(
        r#"{HEAD}<style:style style:name="Low" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="3"/><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"/>{outline}"#
    );
    let h = |style: &str, level: u8| format!(r#"<text:h text:style-name="{style}" text:outline-level="{level}">{style} {level}</text:h>"#);
    let body = [h("Head", 1), h("Head", 2), h("Head", 3), h("Low", 3), h("Low", 1), h("Low", 2), h("Heading_20_1", 2), h("Heading_20_1", 3)].concat();
    let d = in_custom_style(&styles, &body);
    let want = vec![Some(0), Some(1), None, None, Some(0), Some(1), Some(1), None];
    assert_eq!(number_levels(&d), want);
    let nums: std::collections::BTreeSet<u32> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter_map(|p| p.props.numbering.or(d.styles.resolve_para(&p.props).numbering))
        .map(|n| n.num)
        .filter(|n| *n != 0)
        .collect();
    assert_eq!(nums.len(), 1, "one list with the headings': {nums:?}");
    // Written from the model the outline still numbers them, by the levels stated, and read back
    // they are as they were.
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(!c.contains("<text:list"), "{c}");
    for (style, level) in [("Head", 2), ("Low", 1), ("Heading_20_1", 3)] {
        assert!(c.contains(&format!(r#"<text:h text:style-name="{style}" text:outline-level="{level}">{style} {level}"#)), "{c}");
    }
    assert_eq!(number_levels(&import(&saved).unwrap()), want);
}

/// A heading in a list is that list's, whatever its outline level would number.
#[test]
fn a_heading_in_a_list_keeps_the_lists_number() {
    let styles = format!(
        r#"{HEAD}{NUMBERED}<text:list-style style:name="L"><text:list-level-style-number text:level="1" style:num-format="a"/></text:list-style>"#
    );
    let d = in_custom_style(
        &styles,
        r#"<text:list text:style-name="L"><text:list-item><text:h text:style-name="Head" text:outline-level="2">x</text:h></text:list-item></text:list>"#,
    );
    let p = d.body[0].as_para().unwrap();
    let n = p.props.numbering.expect("its list");
    assert_eq!(d.numbering.level(n.num, n.level).map(|l| l.format), Some(NumFormat::LowerLetter));
}

/// A style based on Title with an outline level is a heading at that level, in its own name.
#[test]
fn a_title_based_style_with_an_outline_level_stays_a_heading() {
    let styles = r#"<style:style style:name="Title" style:family="paragraph"/><style:style style:name="Banner" style:family="paragraph" style:parent-style-name="Title" style:default-outline-level="1"/>"#;
    let mut d = in_custom_style(styles, r#"<text:p text:style-name="Banner">report</text:p>"#);
    let p = d.body[0].as_para().unwrap().props.clone();
    assert_eq!((p.style.as_deref(), d.styles.resolve_para(&p).outline_level), (Some("Banner"), Some(0)));
    let saved = saved_after_an_edit(&mut d);
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:h text:style-name="Banner" text:outline-level="1">Yearly report"#), "{c}");
}

/// A style based on a numbered heading, at a level the outline doesn't number, has no number: as
/// in LibreOffice, not its parent's.
#[test]
fn a_style_at_an_unnumbered_level_has_no_number_from_its_parent() {
    let outline = r#"<text:outline-style style:name="Outline"><text:outline-level-style text:level="1" style:num-format="1" style:num-suffix="."/><text:outline-level-style text:level="2" style:num-format=""/><text:outline-level-style text:level="3" style:num-format="1" style:num-suffix="."/></text:outline-style>"#;
    let styles = format!(
        r#"<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="1"/><style:style style:name="Child" style:family="paragraph" style:parent-style-name="Heading_20_1" style:default-outline-level="2"/><style:style style:name="Grand" style:family="paragraph" style:parent-style-name="Heading_20_1" style:default-outline-level="3"/>{outline}"#
    );
    let body = r#"<text:h text:style-name="Heading_20_1" text:outline-level="1">One</text:h><text:p text:style-name="Child">As p</text:p><text:h text:style-name="Child" text:outline-level="2">As h</text:h><text:p text:style-name="Grand">Third</text:p><text:h text:style-name="Heading_20_1" text:outline-level="1">Two</text:h>"#;
    let d = in_custom_style(&styles, body);
    let want = vec![Some(0), None, None, Some(2), Some(0)];
    assert_eq!(number_levels(&d), want);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let written = crate::odt::tests::styles_of(&saved);
    let child = written.split("<style:style ").find(|x| x.starts_with(r#"style:name="Child""#)).unwrap_or_default();
    assert!(child.contains(r#"style:list-style-name="""#), "LibreOffice gives it no number: {child}");
    assert_eq!(number_levels(&import(&saved).unwrap()), want);
}

/// A heading in a list of its own stays in it, whatever the list's levels are linked to: only the
/// number the outline itself gives a heading goes with the outline.
#[test]
fn a_heading_in_a_list_linked_to_a_style_stays_in_it() {
    use std::sync::Arc;
    use wordcraft_doc::props::NumRef;
    let mut d = Document::new();
    let numbering = Arc::make_mut(&mut d.numbering);
    let num = numbering.add_list(wordcraft_doc::numbering::ListKind::Numbered);
    let first = numbering.abstracts.last_mut().and_then(|a| a.levels.first_mut()).unwrap();
    first.style = Some("Heading1".into());
    let mut p = wordcraft_doc::Paragraph::with_text("x", Default::default());
    p.props.style = Some("Heading2".into());
    p.props.numbering = Some(NumRef { num, level: 1 });
    let has_list = |d: &Document| crate::odt::tests::content_of(&crate::odt::tests::export_model(d).unwrap()).contains("<text:list");
    // At a deeper level than the link, no level of its own stated.
    d.body = vec![wordcraft_doc::para_block(p.clone())];
    assert!(has_list(&d), "a list of its own");
    // At its own level too, but its heading style isn't numbered: the outline wouldn't number it.
    p.props.outline_level = Some(1);
    d.body = vec![wordcraft_doc::para_block(p)];
    assert!(has_list(&d), "the outline has no number for it");
    // Its heading style numbered there by this list: the outline's number.
    Arc::make_mut(&mut d.styles).get_mut("Heading2").unwrap().para.numbering = Some(NumRef { num, level: 1 });
    assert!(!has_list(&d), "the outline's");
}

/// Past Heading 6 the outline written has no number to give, so a heading there keeps its list.
#[test]
fn a_heading_past_level_six_keeps_its_list() {
    use std::sync::Arc;
    use wordcraft_doc::props::NumRef;
    let mut d = Document::new();
    let num = Arc::make_mut(&mut d.numbering).add_list(wordcraft_doc::numbering::ListKind::Numbered);
    Arc::make_mut(&mut d.styles).get_mut("Heading1").unwrap().para.numbering = Some(NumRef { num, level: 0 });
    for level in 6..=8 {
        let mut p = wordcraft_doc::Paragraph::with_text("x", Default::default());
        p.props.style = Some("Heading6".into());
        p.props.outline_level = Some(level);
        p.props.numbering = Some(NumRef { num, level });
        d.body = vec![wordcraft_doc::para_block(p)];
        let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
        assert!(c.contains("<text:list"), "level {level}: {c}");
    }
}

/// A heading past level six numbered through its style keeps its list too, as the outline written
/// numbers only six: read, written from the model and with its style edited.
#[test]
fn a_heading_past_level_six_numbered_by_its_style_keeps_its_list() {
    use std::sync::Arc;
    for level in 7..=9u8 {
        let styles = format!(
            r#"<style:style style:name="Deep" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="{level}"/><text:outline-style style:name="Outline"><text:outline-level-style text:level="{level}" style:num-format="1" style:num-suffix="."/></text:outline-style>"#
        );
        let body = format!(r#"<text:h text:style-name="Deep" text:outline-level="{level}">x</text:h>"#);
        let mut d = in_custom_style(&styles, &body);
        let check = |saved: &[u8], what: &str| {
            let c = crate::odt::tests::content_of(saved);
            assert!(c.contains("<text:list"), "level {level}, {what}: {c}");
        };
        check(&crate::odt::tests::export_model(&d).unwrap(), "from the model");
        let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
        Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.italic = Some(true);
        let saved = export(&d).unwrap();
        check(&saved, "style edited");
        let back = import(&saved).unwrap();
        let p = back.body[0].as_para().unwrap();
        let n = p.props.numbering.or_else(|| back.styles.para_style_props(p.props.style.as_deref().unwrap_or("Normal")).0.numbering);
        assert_eq!(n.map(|n| n.level), Some(level - 1), "level {level}, reopened");
    }
}

/// A style at an unnumbered level with a heading numbered by its own level stays on the outline,
/// or LibreOffice shows that heading no number; with none, it comes off, for the level's indent.
#[test]
fn a_style_with_a_heading_numbered_by_its_own_level_stays_on_the_outline() {
    let styles = format!(
        r#"<style:style style:name="Low" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="3"/><style:style style:name="Heading_20_3" style:display-name="Heading 3" style:family="paragraph" style:default-outline-level="3"/>{NUMBERED}"#
    );
    let saved = |body: &str| crate::odt::tests::styles_of(&crate::odt::tests::export_model(&in_custom_style(&styles, body)).unwrap());
    let style_of = |xml: &str, name: &str| {
        xml.split("<style:style ").find(|x| x.starts_with(&format!(r#"style:name="{name}""#))).map(String::from).unwrap_or_default()
    };
    let xml = saved(
        r#"<text:h text:style-name="Low" text:outline-level="1">a</text:h><text:h text:style-name="Heading_20_3" text:outline-level="2">b</text:h>"#,
    );
    assert!(!style_of(&xml, "Low").contains("style:list-style-name"), "{xml}");
    assert!(!style_of(&xml, "Heading_20_3").contains("style:list-style-name"), "{xml}");
    let xml = saved(r#"<text:h text:style-name="Low" text:outline-level="3">a</text:h>"#);
    assert!(style_of(&xml, "Low").contains(r#"style:list-style-name="""#), "{xml}");
}

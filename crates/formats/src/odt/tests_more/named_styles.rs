//! Custom paragraph styles (ODF 1.3 §16.2 common `style:style` of family paragraph): kept as
//! document styles with their name, parent and own formatting, the paragraphs in them keeping
//! the style rather than its formatting as direct formatting; written back as common styles.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{LineSpacing, TextColor};
use wordcraft_doc::{Document, Rgb};

fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Text_20_body" style:display-name="Text body" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0.247cm" fo:line-height="115%"/></style:style><style:style style:name="My_20_Custom" style:display-name="My Custom" style:family="paragraph" style:parent-style-name="Text_20_body"><style:paragraph-properties fo:margin-left="1cm"/><style:text-properties fo:color="#0000aa" fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="My_20_Custom"><style:paragraph-properties fo:text-align="center"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Text_20_body">Body</text:p><text:p text:style-name="My_20_Custom">Custom</text:p><text:p text:style-name="P1">Centred custom</text:p><text:p>Plain</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

fn styles_of(d: &Document) -> Vec<Option<String>> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.style.clone()).collect()
}

#[test]
fn paragraphs_keep_their_custom_style() {
    let d = open();
    let ids = styles_of(&d);
    let (body, custom) = (ids[0].clone().expect("Text body"), ids[1].clone().expect("My Custom"));
    assert_eq!(ids[2].as_ref(), Some(&custom), "an automatic style keeps its common parent");
    assert_eq!(ids[3], None);
    let st = d.styles.get(&custom).expect("the style");
    assert_eq!(st.name, "My Custom");
    assert_eq!(st.based_on.as_ref(), Some(&body));
    assert_eq!(d.styles.get(&body).map(|s| s.name.as_str()), Some("Text body"));
    // The style's own formatting is in the style, not on the paragraph.
    let p = d.body[1].as_para().unwrap();
    assert!(p.props.indent_left.is_none() && p.props.space_after.is_none(), "{:?}", p.props);
    assert!(p.runs.iter().all(|r| r.props.bold.is_none() && r.props.color.is_none()), "{:?}", p.runs);
    // Resolved, it looks as before: its own and inherited formatting.
    let r = d.styles.resolve_para(&p.props);
    assert!((r.indent_left - 28.35).abs() < 0.01 && (r.space_after - 7.0).abs() < 0.01 && r.line_spacing == LineSpacing::Multiple(1.15), "{r:?}");
    let (_, cp) = d.styles.para_style_props(&custom);
    assert_eq!((cp.bold, cp.color), (Some(true), Some(TextColor::Rgb(Rgb(0, 0, 0xaa)))));
    // The automatic style's own formatting is direct.
    assert_eq!(d.body[2].as_para().unwrap().props.align, Some(wordcraft_doc::Align::Center));
}

#[test]
fn custom_styles_round_trip() {
    let d = open();
    let back = import(&export(&d).unwrap()).unwrap();
    let names = |d: &Document| styles_of(d).into_iter().map(|id| id.and_then(|id| d.styles.get(&id).map(|s| s.name.clone()))).collect::<Vec<_>>();
    assert_eq!(names(&back), names(&d));
    for (a, b) in d.body.iter().zip(back.body.iter()) {
        let (a, b) = (a.as_para().unwrap(), b.as_para().unwrap());
        let (ra, rb) = (d.styles.resolve_para(&a.props), back.styles.resolve_para(&b.props));
        assert!(
            (ra.indent_left - rb.indent_left).abs() < 0.01 && (ra.space_after - rb.space_after).abs() < 0.01 && ra.align == rb.align,
            "{ra:?} {rb:?}"
        );
    }
    let id = styles_of(&back)[1].clone().unwrap();
    let (_, cp) = back.styles.para_style_props(&id);
    assert_eq!((cp.bold, cp.color), (Some(true), Some(TextColor::Rgb(Rgb(0, 0, 0xaa)))));
}

#[test]
fn hostile_style_chains_do_not_panic() {
    // A loop, a missing parent and a very long name.
    let long = "n".repeat(5000);
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="A" style:family="paragraph" style:parent-style-name="B"/><style:style style:name="B" style:family="paragraph" style:parent-style-name="A"/><style:style style:name="C" style:family="paragraph" style:parent-style-name="Missing"/><style:style style:name="{long}" style:family="paragraph"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="A">a</text:p><text:p text:style-name="C">c</text:p><text:p text:style-name="{long}">l</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let _ = export(&d).unwrap();
}

#[test]
fn style_names_are_xml_names() {
    use crate::odt::writer::odf_name;
    assert_eq!(odf_name("MyCustom"), "MyCustom");
    assert_eq!(odf_name("My Style-2"), "My_x20_Style-2");
    assert_eq!(odf_name("1st"), "_x31_st");
    assert_eq!(odf_name(""), "Custom");
}

#[test]
fn no_style_is_written_twice() {
    // A custom style named like one the writer writes itself (Footnote) made two styles of one
    // name, and LibreOffice dropped the note text's indent.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Footnote" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.6cm" fo:text-indent="-0.6cm"/></style:style><style:style style:name="Standard_20_x" style:display-name="Standard" style:family="paragraph"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p text:style-name="Footnote">n</text:p></text:note-body></text:note></text:p><text:p text:style-name="Standard_20_x">y</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let xml = crate::odt::tests::styles_of(&export(&d).unwrap());
    for name in ["Footnote", "Standard", "Endnote"] {
        let paragraph_styles = xml
            .split("<style:style ")
            .filter(|s| s.starts_with(&format!(r#"style:name="{name}""#)) && s.contains(r#"style:family="paragraph""#))
            .count();
        assert_eq!(paragraph_styles, 1, "{name}");
    }
}

#[test]
fn many_paragraphs_in_custom_styles_open_quickly() {
    // Each paragraph checked its style chain by building every custom style in it.
    let tabs: String = (0..60).map(|i| format!(r#"<style:tab-stop style:position="{i}pt"/>"#)).collect();
    let chain: String = (0..15)
        .map(|i| {
            let parent = if i == 0 { "Standard".to_string() } else { format!("S{}", i - 1) };
            format!(r#"<style:style style:name="S{i}" style:family="paragraph" style:parent-style-name="{parent}"><style:paragraph-properties><style:tab-stops>{tabs}</style:tab-stops></style:paragraph-properties></style:style>"#)
        })
        .collect();
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{chain}</office:styles></office:document-styles>"#);
    let body = r#"<text:p text:style-name="S14">x</text:p>"#.repeat(20_000);
    let content =
        format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    let t = std::time::Instant::now();
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(d.body.len(), 20_000);
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
}

#[test]
fn a_style_based_on_libreoffices_heading_keeps_it() {
    // LibreOffice's headings and index headings are based on its "Heading" style, which gives
    // their font and spacing; saving keeps it, and keeps them based on it.
    let styles = format!(
        r#"<office:document-styles {}><office:styles><style:style style:name="Heading" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.17in"/><style:text-properties fo:font-size="14pt"/></style:style><style:style style:name="Plaque" style:family="paragraph" style:parent-style-name="Heading"><style:text-properties fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"#,
        crate::odt::tests::NS_T
    );
    let content = format!(
        r#"<office:document-content {}><office:body><office:text><text:p text:style-name="Plaque">On the wall</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    let s = crate::odt::tests::styles_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(
        s.contains(r#"style:name="Heading""#)
            && s.contains(r#"style:name="Plaque" style:display-name="Plaque" style:family="paragraph" style:parent-style-name="Heading""#),
        "{s}"
    );
}

#[test]
fn what_a_style_turns_off_stays_off() {
    // LibreOffice's agenda: "AgendaDate" is based on the bold "AgendaTitles" and turns bold off.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Titles" style:family="paragraph"><style:text-properties fo:font-weight="bold" fo:font-style="italic" style:text-underline-style="solid"/></style:style><style:style style:name="Date" style:family="paragraph" style:parent-style-name="Titles"><style:text-properties fo:font-weight="normal" fo:font-style="normal" style:text-underline-style="none"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Titles">T</text:p><text:p text:style-name="Date">D</text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, Some(&styles))).unwrap()).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    let date = s.split(r#"style:name="Date""#).nth(1).and_then(|r| r.split("</style:style>").next()).unwrap_or("missing");
    for want in [r#"fo:font-weight="normal""#, r#"fo:font-style="normal""#, r#"style:text-underline-style="none""#] {
        assert!(date.contains(want), "{want} in {date}");
    }
}

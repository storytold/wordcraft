//! The look of the styles WordCraft maps ODT's to (Heading 1–6, Title, Quotations, Preformatted
//! Text): read from the file's own definitions on open, written from the document's on save, so
//! headings look in each program as they did in the other.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{CharProps, ParaProps, TextColor};
use wordcraft_doc::{Document, Paragraph, Rgb, para_block};

fn libreoffice_styles(heading1: &str) -> String {
    format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Heading" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="12pt" fo:margin-bottom="6pt" fo:keep-with-next="always"/><style:text-properties style:font-name="Liberation Sans" fo:font-family="'Liberation Sans'" fo:font-size="14pt"/></style:style><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Heading" style:default-outline-level="1">{heading1}</style:style><style:style style:name="Quotations" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="1cm" fo:margin-right="1cm"/></style:style></office:styles></office:document-styles>"#
    )
}

fn open(styles: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">Title</text:h><text:p text:style-name="Quotations">Quote</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(styles))).unwrap()
}

fn style_look(d: &Document, id: &str) -> (ParaProps, CharProps) {
    d.styles.para_style_props(id)
}

#[test]
fn a_heading_looks_as_the_file_defines_it() {
    let d = open(&libreoffice_styles(r#"<style:text-properties fo:font-size="18pt" fo:font-weight="bold"/>"#));
    assert_eq!(d.body[0].as_para().unwrap().props.style.as_deref(), Some("Heading1"));
    let (pp, cp) = style_look(&d, "Heading1");
    assert_eq!(cp.font.as_deref(), Some("Liberation Sans"));
    assert_eq!((cp.size, cp.bold), (Some(18.0), Some(true)));
    // Not WordCraft's blue: the file's heading has no colour of its own.
    assert!(!matches!(cp.color, Some(TextColor::Rgb(c)) if c != Rgb(0, 0, 0)), "{:?}", cp.color);
    assert_eq!((pp.space_before, pp.space_after, pp.keep_next), (Some(12.0), Some(6.0), Some(true)));
    // The body text isn't touched: direct formatting stays empty.
    let h = d.body[0].as_para().unwrap();
    assert!(h.runs.iter().all(|r| r.props.size.is_none() && r.props.bold.is_none() && r.props.font.is_none()), "{:?}", h.runs);
    let (qp, _) = style_look(&d, "Quote");
    assert!(qp.indent_left.is_some_and(|l| (l - 28.35).abs() < 0.01));
}

#[test]
fn a_size_in_percent_is_of_the_parent_size() {
    let d = open(&libreoffice_styles(r#"<style:text-properties fo:font-size="130%"/>"#));
    let (_, cp) = style_look(&d, "Heading1");
    assert!(cp.size.is_some_and(|s| (s - 18.2).abs() < 0.01), "{:?}", cp.size);
}

#[test]
fn heading_styles_round_trip() {
    // A new document's headings (WordCraft's look) keep it through an ODT save.
    let mut d = Document::new();
    let mut p = Paragraph::new();
    p.text = "Head".into();
    p.runs = vec![wordcraft_doc::para::Run { len: 4, props: Default::default() }];
    p.props.style = Some("Heading2".into());
    d.body = vec![para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    for id in ["Heading1", "Heading2", "Heading3", "Title", "Quote"] {
        let (pa, ca) = style_look(&d, id);
        let (pb, cb) = style_look(&back, id);
        assert_eq!((ca.font, ca.size, ca.bold, ca.italic, ca.color), (cb.font, cb.size, cb.bold, cb.italic, cb.color), "{id}");
        assert_eq!((pa.space_before, pa.space_after, pa.keep_next), (pb.space_before, pb.space_after, pb.keep_next), "{id}");
    }
}

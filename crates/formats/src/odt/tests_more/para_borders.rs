//! Paragraph borders, their padding and background (ODF 1.3 §20.182 `fo:background-color`,
//! §20.183 `fo:border*`, §20.217–20.221 `fo:padding*` on `style:paragraph-properties`).

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{Border, BorderStyle, Borders, ParaProps};
use wordcraft_doc::{Document, Paragraph, Rgb, para_block};

fn open(props: &str, text: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties {props}/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">{text}</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn props(d: &Document) -> ParaProps {
    d.body[0].as_para().unwrap().props.clone()
}

#[test]
fn borders_padding_and_background_are_read() {
    let d = open(
        r##"fo:border-top="1pt solid #000080" fo:border-bottom="1pt solid #000080" fo:border-left="none" fo:padding="2pt" fo:background-color="#eeeeee""##,
        "Boxed",
    );
    let p = props(&d);
    let b = p.borders.expect("borders");
    let top = b.top.expect("top");
    assert_eq!((top.style, top.width, top.color, top.space), (BorderStyle::Single, 1.0, Some(Rgb(0, 0, 0x80)), 2.0));
    assert!(b.bottom.is_some_and(|x| x.is_visible()));
    assert!(!b.left.is_some_and(|x| x.is_visible()));
    // The right side has no line but keeps its padding (LibreOffice gives it room).
    let right = b.right.expect("the right side's padding");
    assert_eq!((right.is_visible(), right.space), (false, 2.0));
    assert_eq!(p.shading, Some(Rgb(0xee, 0xee, 0xee)));
    // A transparent background is none.
    assert_eq!(props(&open(r#"fo:background-color="transparent""#, "x")).shading, None);
}

#[test]
fn padding_without_a_border_keeps_its_room() {
    // LibreOffice's report templates: padding on a side without a line still takes room
    // (measured: 0.5 in of padding and no border moves the text 0.5 in in and down).
    let d = open(r#"fo:padding="0.5in""#, "Padded");
    let p = props(&d);
    let b = p.borders.expect("the padding");
    assert!(!b.any_visible() && b.takes_room());
    assert_eq!(d.styles.resolve_para(&p).padding, [36.0; 4]);
    assert!(d.settings.odf.is_some());
    // Written back as padding.
    let out = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(out.contains(r#"fo:border-top="none" fo:padding-top="36pt""#), "{out}");
    let back = crate::odt::import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
    assert_eq!(back.styles.resolve_para(&props(&back)).padding, [36.0; 4]);
}

#[test]
fn an_empty_paragraph_with_only_a_bottom_border_is_still_a_rule() {
    let d = open(r#"fo:border-bottom="0.75pt solid #a0a0a0""#, "");
    let p = props(&d);
    assert!(p.borders.and_then(|b| b.bottom).is_some_and(|b| b.is_visible()));
}

#[test]
fn borders_and_background_round_trip() {
    let line = Border { style: BorderStyle::Double, width: 1.5, color: Some(Rgb(200, 0, 0)), space: 3.0 };
    let want = ParaProps {
        borders: Some(Borders { top: Some(line), left: Some(line), bottom: Some(line), right: Some(line), ..Default::default() }),
        shading: Some(Rgb(255, 255, 204)),
        ..Default::default()
    };
    let mut p = Paragraph::new();
    p.text = "Boxed".into();
    p.runs = vec![wordcraft_doc::para::Run { len: 5, props: Default::default() }];
    p.props = want.clone();
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let got = props(&import(&export(&d).unwrap()).unwrap());
    assert_eq!(got.shading, want.shading);
    let (g, w) = (got.borders.expect("borders"), want.borders.expect("borders"));
    for (a, b) in [(g.top, w.top), (g.left, w.left), (g.bottom, w.bottom), (g.right, w.right)] {
        let (a, b) = (a.expect("side"), b.expect("side"));
        assert_eq!((a.style, a.width, a.color, a.space), (b.style, b.width, b.color, b.space));
    }
}

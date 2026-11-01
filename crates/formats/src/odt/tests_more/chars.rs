//! Character formatting beyond bold, italic and colour (ODF 1.3 §20.2xx `style:text-properties`):
//! underline styles and colour, double strike, caps, small caps, hidden text, letter spacing,
//! scale, raised and lowered text, kerning, outline, shadow, relief and language.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{CharProps, Underline};
use wordcraft_doc::{Document, Paragraph, Rgb, para_block};

fn spans(props: &[&str]) -> Document {
    let styles: String = props
        .iter()
        .enumerate()
        .map(|(i, p)| format!(r#"<style:style style:name="T{i}" style:family="text"><style:text-properties {p}/></style:style>"#))
        .collect();
    let body: String = (0..props.len()).map(|i| format!(r#"<text:span text:style-name="T{i}">s{i}</text:span>|"#)).collect();
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text><text:p>{body}</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// The formatting of each `sN` span.
fn runs(d: &Document) -> Vec<CharProps> {
    let p = d.body[0].as_para().unwrap();
    p.run_ranges()
        .filter(|(r, _)| p.text.get(r.clone()).is_some_and(|t| t.trim_start_matches('|').starts_with('s')))
        .map(|(_, props)| props.clone())
        .collect()
}

#[test]
fn underline_styles_and_strikes_are_read() {
    let d = spans(&[
        r#"style:text-underline-style="solid" style:text-underline-type="double""#,
        r#"style:text-underline-style="dotted""#,
        r#"style:text-underline-style="wave""#,
        r#"style:text-underline-style="solid" style:text-underline-width="bold""#,
        r##"style:text-underline-style="dash" style:text-underline-color="#ff0000""##,
        r#"style:text-underline-style="solid" style:text-underline-mode="skip-white-space""#,
        r#"style:text-line-through-style="solid" style:text-line-through-type="double""#,
    ]);
    let r = runs(&d);
    let u: Vec<Option<Underline>> = r.iter().map(|p| p.underline).collect();
    assert_eq!(
        u[..6],
        [
            Some(Underline::Double),
            Some(Underline::Dotted),
            Some(Underline::Wave),
            Some(Underline::Thick),
            Some(Underline::Dash),
            Some(Underline::Words)
        ]
    );
    assert_eq!(r[4].underline_color, Some(Rgb(255, 0, 0)));
    assert_eq!((r[6].double_strike, r[6].strike), (Some(true), None));
}

#[test]
fn caps_spacing_and_effects_are_read() {
    let d = spans(&[
        r#"fo:text-transform="uppercase""#,
        r#"fo:font-variant="small-caps""#,
        r#"text:display="none""#,
        r#"fo:letter-spacing="2pt""#,
        r#"style:text-scale="80%""#,
        r#"style:text-outline="true""#,
        r#"fo:text-shadow="1pt 1pt""#,
        r#"style:font-relief="embossed""#,
        r#"style:font-relief="engraved""#,
        r#"fo:language="nb" fo:country="NO""#,
        r#"style:text-position="33% 100%" fo:font-size="12pt""#,
    ]);
    let r = runs(&d);
    assert_eq!(r[0].caps, Some(true));
    assert_eq!(r[1].small_caps, Some(true));
    assert_eq!(r[2].hidden, Some(true));
    assert_eq!(r[3].spacing, Some(2.0));
    assert_eq!(r[4].scale, Some(80.0));
    assert_eq!(r[5].outline, Some(true));
    assert_eq!(r[6].shadow, Some(true));
    assert_eq!(r[7].emboss, Some(true));
    assert_eq!(r[8].engrave, Some(true));
    assert_eq!(r[9].lang.as_deref(), Some("nb-NO"));
    // Raised by a third of the text's height, at full size: a position, not superscript.
    assert!(r[10].position.is_some_and(|p| (p - 3.96).abs() < 0.01) && r[10].vert_align.is_none(), "{:?}", r[10]);
}

#[test]
fn character_formatting_round_trips() {
    let want = [
        CharProps { underline: Some(Underline::Double), ..Default::default() },
        CharProps { underline: Some(Underline::Wave), underline_color: Some(Rgb(0, 0, 255)), ..Default::default() },
        CharProps { underline: Some(Underline::Words), ..Default::default() },
        CharProps { underline: Some(Underline::DotDotDash), ..Default::default() },
        CharProps { double_strike: Some(true), ..Default::default() },
        CharProps { caps: Some(true), ..Default::default() },
        CharProps { small_caps: Some(true), ..Default::default() },
        CharProps { hidden: Some(true), ..Default::default() },
        CharProps { spacing: Some(-0.5), ..Default::default() },
        CharProps { scale: Some(150.0), ..Default::default() },
        CharProps { position: Some(3.0), size: Some(12.0), ..Default::default() },
        CharProps { outline: Some(true), shadow: Some(true), ..Default::default() },
        CharProps { emboss: Some(true), ..Default::default() },
        CharProps { lang: Some("de-DE".into()), ..Default::default() },
    ];
    let mut p = Paragraph::new();
    for (i, props) in want.iter().enumerate() {
        let s = format!("s{i}|");
        p.text.push_str(&s);
        p.runs.push(wordcraft_doc::para::Run { len: s.len() - 1, props: props.clone() });
        p.runs.push(wordcraft_doc::para::Run { len: 1, props: Default::default() });
    }
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    let got = runs(&back);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "run {i}");
    }
    assert_eq!(got.len(), want.len());
}

#[test]
fn hostile_text_properties_are_clamped() {
    let d = spans(&[
        r#"fo:letter-spacing="1e30pt" style:text-scale="1e9%""#,
        r#"fo:letter-spacing="-1e30pt" style:text-scale="-5%" style:text-position="1e9% 100%""#,
        r#"fo:language="" fo:country="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx""#,
    ]);
    let r = runs(&d);
    assert!(r[0].spacing.is_some_and(|s| s <= 1584.0) && r[0].scale.is_some_and(|s| s <= 600.0), "{:?}", r[0]);
    assert!(r[1].spacing.is_some_and(|s| s >= -1584.0) && r[1].scale.is_none_or(|s| s >= 1.0), "{:?}", r[1]);
    assert!(r[1].position.is_none_or(|p| p.abs() <= 1584.0));
    assert!(r[2].lang.as_ref().is_none_or(|l| l.len() <= 64));
    let _ = export(&d).unwrap();
}

#[test]
fn a_monospaced_font_keeps_its_name() {
    // A fixed-pitch font was read as "code" and written as Courier New.
    let d = spans(&[r#"fo:font-family="'Liberation Mono'" style:font-pitch="fixed""#, r#"style:font-name="DejaVu Sans Mono""#]);
    let fonts: Vec<Option<String>> = runs(&d).into_iter().map(|p| p.font).collect();
    assert_eq!(fonts, [Some("Liberation Mono".to_string()), Some("DejaVu Sans Mono".to_string())]);
    let back = import(&export(&d).unwrap()).unwrap();
    let fonts: Vec<Option<String>> = runs(&back).into_iter().map(|p| p.font).collect();
    assert_eq!(fonts, [Some("Liberation Mono".to_string()), Some("DejaVu Sans Mono".to_string())]);
}

#[test]
fn the_default_letter_kerning_round_trips() {
    // LibreOffice kerns pairs by default (`style:letter-kerning` on the default style); without
    // it lines broke elsewhere after a save.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties style:letter-kerning="true"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert!(d.styles.default_chr.kern.is_some_and(|k| k > 0.0));
    let none = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties style:letter-kerning="false"/></style:default-style></office:styles></office:document-styles>"#
    );
    let off = import(&package(&content, Some(&none))).unwrap();
    assert!(off.styles.default_chr.kern.is_none_or(|k| k == 0.0));
    let back = import(&export(&d).unwrap()).unwrap();
    assert!(back.styles.default_chr.kern.is_some_and(|k| k > 0.0));
    let back = import(&export(&off).unwrap()).unwrap();
    assert!(back.styles.default_chr.kern.is_none_or(|k| k == 0.0));
}

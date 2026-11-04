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

#[test]
fn kerning_a_span_turns_on_or_off_round_trips() {
    // ONLYOFFICE turns kerning on span by span, the default style saying nothing; without it
    // LibreOffice set the text wider and every page broke elsewhere after a save.
    let d = spans(&[r#"style:letter-kerning="true""#, r#"style:letter-kerning="false""#, r#"fo:font-weight="bold""#]);
    let kerns = |d: &Document| runs(d).iter().map(|r| r.kern).collect::<Vec<_>>();
    assert_eq!(kerns(&d), [Some(1.0), Some(0.0), None]);
    // Written where it differs from the document's default (here off).
    let on = |d: &Document| runs(d).iter().map(|r| r.kern.or(d.styles.default_chr.kern).is_some_and(|k| k > 0.0)).collect::<Vec<_>>();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(on(&back), [true, false, false]);
}

#[test]
fn asian_and_complex_script_fonts_and_languages() {
    let d = spans(&[
        r#"style:font-name-complex="Noto Sans Hebrew" style:font-size-complex="14pt" style:font-weight-complex="bold" style:language-complex="he" style:country-complex="IL""#,
        r#"style:font-name-asian="Noto Sans CJK JP" style:language-asian="ja" style:country-asian="JP""#,
    ]);
    let r = runs(&d);
    assert_eq!((r[0].font_cs.as_deref(), r[0].size_cs, r[0].bold_cs), (Some("Noto Sans Hebrew"), Some(14.0), Some(true)));
    assert_eq!(r[0].lang_bidi.as_deref(), Some("he-IL"));
    // Latin bold isn't taken from the complex-script weight once the run has its own.
    assert_eq!((r[1].font_ea.as_deref(), r[1].lang_ea.as_deref()), (Some("Noto Sans CJK JP"), Some("ja-JP")));
    let back = import(&export(&d).unwrap()).unwrap();
    let b = runs(&back);
    assert_eq!(
        (b[0].font_cs.as_deref(), b[0].size_cs, b[0].bold_cs, b[0].lang_bidi.as_deref()),
        (Some("Noto Sans Hebrew"), Some(14.0), Some(true), Some("he-IL"))
    );
    assert_eq!((b[1].font_ea.as_deref(), b[1].lang_ea.as_deref()), (Some("Noto Sans CJK JP"), Some("ja-JP")));
}

#[test]
fn the_default_styles_script_fonts_are_the_document_defaults() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Liberation Serif" style:font-name-asian="WenQuanYi Zen Hei" style:font-name-complex="FreeSans" style:language-complex="hi" style:country-complex="IN"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let c = &d.styles.default_chr;
    assert_eq!((c.font_ea.as_deref(), c.font_cs.as_deref(), c.lang_bidi.as_deref()), (Some("WenQuanYi Zen Hei"), Some("FreeSans"), Some("hi-IN")));
    let back = import(&export(&d).unwrap()).unwrap();
    let c = &back.styles.default_chr;
    assert_eq!((c.font_ea.as_deref(), c.font_cs.as_deref(), c.lang_bidi.as_deref()), (Some("WenQuanYi Zen Hei"), Some("FreeSans"), Some("hi-IN")));
}

#[test]
fn a_border_around_text_is_read_and_written() {
    // LibreOffice's text border (`loext:border`, or its sides, and `loext:padding`): Word's border
    // around text, one line all round.
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties loext:padding="0.02in" loext:border-left="none" loext:border-top="0.74pt solid #ff0000"/></style:style></office:automatic-styles><office:body><office:text><text:p>A <text:span text:style-name="T1">boxed</text:span> word.</text:p></office:text></office:body></office:document-content>"#
    );
    let d = crate::odt::import(&crate::odt::tests::package(&content, None)).unwrap();
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    let b = p.runs.iter().find_map(|r| r.props.border).expect("a text border");
    assert_eq!((b.color, (b.width * 100.0).round(), (b.space * 100.0).round()), (Some(wordcraft_doc::props::Rgb(255, 0, 0)), 74.0, 144.0));
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"loext:border="0.74pt solid #ff0000" loext:padding="1.44pt""#), "{c}");
}

#[test]
fn emphasis_marks_are_read_and_written() {
    use wordcraft_doc::props::EmphasisMark as E;
    let style = |n: &str, v: &str| {
        format!(r#"<style:style style:name="{n}" style:family="text"><style:text-properties style:text-emphasize="{v}"/></style:style>"#)
    };
    let styles = [
        style("T1", "dot above"),
        style("T2", "accent above"),
        style("T3", "circle above"),
        style("T4", "dot below"),
        style("T5", "disc above"),
        style("T6", "none"),
    ]
    .concat();
    let spans: String = (1..=6).map(|i| format!(r#"<text:span text:style-name="T{i}">{i}</text:span>"#)).collect();
    let content = format!(
        r#"<office:document-content {}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:text><text:p>{spans}</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let d = crate::odt::import(&crate::odt::tests::package(&content, None)).unwrap();
    let marks: Vec<Option<E>> = d.body[0].as_para().unwrap().runs.iter().map(|r| r.props.emphasis).collect();
    assert_eq!(marks, [E::Dot, E::Comma, E::Circle, E::UnderDot, E::Dot, E::None].map(Some));
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    for v in ["dot above", "accent above", "circle above", "dot below"] {
        assert!(c.contains(&format!(r#"style:text-emphasize="{v}""#)), "{v} in {c}");
    }
    // Read back the same (an explicit "none" with nothing to turn off is unset: the same look).
    let back = crate::odt::import(&saved).unwrap();
    let back: Vec<Option<E>> = back.body[0].as_para().unwrap().runs.iter().map(|r| r.props.emphasis).collect();
    assert_eq!(back, [Some(E::Dot), Some(E::Comma), Some(E::Circle), Some(E::UnderDot), Some(E::Dot), None]);
}

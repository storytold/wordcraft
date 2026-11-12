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

#[test]
fn objects_in_hidden_text_are_kept() {
    // Hidden text is written, marked hidden: so are the bookmarks and fields it holds.
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    let mut p = Paragraph::with_text("Shown. Hidden words.", CharProps::default());
    let n = p.len();
    p.format(7, n, &|c| c.hidden = Some(true)).unwrap();
    p.insert_object(14, wordcraft_doc::para::InlineObject::BookmarkStart { name: "Secret".into() }, &hidden).unwrap();
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    let p = back.body[0].as_para().unwrap();
    assert!(p.text.contains("Hidden"), "{:?}", p.text);
    assert!(p.objects.iter().any(|o| matches!(o, wordcraft_doc::para::InlineObject::BookmarkStart { name } if name == "Secret")), "{:?}", p.objects);
}

/// A paragraph in style `style` holding `span`, with `auto` among its automatic styles.
fn off_doc(styles: &str, style: &str, auto: &str, span: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text><text:p text:style-name="{style}">{span}</text:p></office:text></office:body></office:document-content>"#
    );
    let styles = format!("<office:document-styles {NS_T}><office:styles>{styles}</office:styles></office:document-styles>");
    import(&crate::odt::tests::package(&content, Some(&styles))).unwrap()
}

#[test]
fn text_turning_off_what_its_style_turns_on_stays_off() {
    // A word not bold in a bold heading: it showed bold, on screen and after a save.
    let on = r#"fo:font-weight="bold" fo:font-style="italic" style:text-underline-style="solid" style:text-line-through-style="solid" fo:font-variant="small-caps" fo:text-transform="uppercase""#;
    let off = r#"fo:font-weight="normal" fo:font-style="normal" style:text-underline-style="none" style:text-line-through-style="none" fo:font-variant="normal" fo:text-transform="none""#;
    let styles = format!(r#"<style:style style:name="Laut" style:family="paragraph"><style:text-properties {on}/></style:style>"#);
    let auto = format!(r#"<style:style style:name="T1" style:family="text"><style:text-properties {off}/></style:style>"#);
    let check = |d: &Document, what: &str| {
        let p = d.body[0].as_para().unwrap();
        let shown = |run: usize| d.styles.resolve_char(p.props.style.as_deref(), &p.runs[run].props);
        assert_eq!(p.text, "onoff", "{what}");
        let (a, b) = (shown(0), shown(1));
        assert_eq!((a.bold, a.italic, a.strike, a.small_caps, a.caps), (true, true, true, true, true), "{what}: the style's");
        assert!(a.underline != Underline::None, "{what}");
        assert_eq!((b.bold, b.italic, b.strike, b.small_caps, b.caps), (false, false, false, false, false), "{what}: turned off");
        assert_eq!(b.underline, Underline::None, "{what}");
    };
    let d = off_doc(&styles, "Laut", &auto, r#"on<text:span text:style-name="T1">off</text:span>"#);
    check(&d, "read");
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "written from the model");
    check(&import(&export(&d).unwrap()).unwrap(), "written as read");
    // Turned back on inside it: on.
    let auto = format!(
        r#"{auto}<style:style style:name="T2" style:family="text"><style:text-properties fo:font-weight="bold" style:text-underline-style="solid"/></style:style>"#
    );
    let d = off_doc(&styles, "Laut", &auto, r#"<text:span text:style-name="T1"><text:span text:style-name="T2">x</text:span></text:span>"#);
    let p = d.body[0].as_para().unwrap();
    let x = d.styles.resolve_char(p.props.style.as_deref(), &p.runs[0].props);
    assert!(x.bold);
    assert!(x.underline != Underline::None, "{x:?}");
    assert!(!x.italic, "still off");
    // A character style's: off in the automatic style based on it.
    let styles = r#"<style:style style:name="Zeichen" style:family="text"><style:text-properties fo:font-weight="bold" style:text-underline-style="solid"/></style:style>"#;
    let auto = r#"<style:style style:name="T3" style:family="text" style:parent-style-name="Zeichen"><style:text-properties fo:font-weight="normal" style:text-underline-style="none"/></style:style>"#;
    let d = off_doc(styles, "Standard", auto, r#"<text:span text:style-name="T3">x</text:span>"#);
    let p = d.body[0].as_para().unwrap();
    let x = d.styles.resolve_char(p.props.style.as_deref(), &p.runs[0].props);
    assert_eq!((x.bold, x.underline), (false, Underline::None), "{x:?}");
    // Text nothing turned on says nothing.
    let d = off_doc("", "Standard", "", "plain");
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(!c.contains(r#"fo:font-weight="normal""#) && !c.contains(r#"style:text-underline-style="none""#), "{c}");
}

/// A weight or slant only for Asian or complex-script text says nothing of Latin text when it is
/// normal: the heading's bold stays, on screen and saved. Latin text's own "normal" still counts.
#[test]
fn an_asian_or_complex_normal_doesnt_turn_latin_text_off() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style></office:styles></office:document-styles>"#
    );
    let open = |attr: &str| {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties {attr}/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">Bold <text:span text:style-name="T1">word</text:span></text:h></office:text></office:body></office:document-content>"#
        );
        import(&package(&content, Some(&styles))).unwrap()
    };
    let run = |d: &Document| d.body[0].as_para().unwrap().runs.last().unwrap().props.clone();
    for attr in [
        r#"style:font-weight-complex="normal""#,
        r#"style:font-weight-asian="normal""#,
        r#"style:font-style-complex="normal""#,
        r#"style:font-style-asian="normal""#,
    ] {
        let d = open(attr);
        assert_eq!((run(&d).bold, run(&d).italic), (None, None), "{attr}");
        let saved = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
        assert!(!saved.contains(r#"fo:font-weight="normal""#) && !saved.contains(r#"fo:font-style="normal""#), "{attr}: {saved}");
    }
    assert_eq!(run(&open(r#"fo:font-weight="normal""#)).bold, Some(false));
    assert_eq!(run(&open(r#"fo:font-style="normal""#)).italic, Some(false));
}

/// Strike-through turned off over a double one is off, not the double one again.
#[test]
fn strike_turned_off_over_a_double_strike_stays_off() {
    let double = r#"<style:text-properties style:text-line-through-style="solid" style:text-line-through-type="double"/>"#;
    let none = r#"<style:text-properties style:text-line-through-style="none"/>"#;
    let check = |d: &Document, what: &str| {
        let p = d.body[0].as_para().unwrap();
        let c = d.styles.resolve_char(p.props.style.as_deref(), &p.runs.last().unwrap().props);
        assert_eq!((c.strike, c.double_strike), (false, false), "{what}");
    };
    // A style over its parent.
    let styles = format!(
        r#"<style:style style:name="Parent" style:family="paragraph">{double}</style:style><style:style style:name="Kid" style:family="paragraph" style:parent-style-name="Parent">{none}</style:style>"#
    );
    let d = off_doc(&styles, "Kid", "", "One");
    check(&d, "a style, read");
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "a style, written from the model");
    // A span over its paragraph's style.
    let styles = format!(r#"<style:style style:name="Parent" style:family="paragraph">{double}</style:style>"#);
    let auto = format!(r#"<style:style style:name="T1" style:family="text">{none}</style:style>"#);
    let d = off_doc(&styles, "Parent", &auto, r#"<text:span text:style-name="T1">One</text:span>"#);
    check(&d, "a span, read");
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "a span, written from the model");
}

/// A span's character style is its own: a paragraph turning bold or italic off doesn't reach past it.
#[test]
fn a_spans_character_style_wins_over_its_paragraph_turning_off() {
    let styles = r#"<style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Strong_20_Emphasis" style:display-name="Strong Emphasis" style:family="text"><style:text-properties fo:font-weight="bold" fo:font-style="italic"/></style:style>"#;
    let auto = r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-weight="normal" fo:font-style="normal"/></style:style>"#;
    let d = off_doc(styles, "P1", auto, r#"<text:span text:style-name="Strong_20_Emphasis">One</text:span>"#);
    let check = |d: &Document, what: &str| {
        let p = d.body[0].as_para().unwrap();
        let c = d.styles.resolve_char(p.props.style.as_deref(), &p.runs.last().unwrap().props);
        assert_eq!((c.bold, c.italic), (true, true), "{what}: {:?}", p.runs.last().unwrap().props);
    };
    check(&d, "read");
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "written from the model");
}

/// `default` as a document's default paragraph style's text properties, with `standard` as its
/// Standard style's.
fn with_default_text(default: &str, standard: &str) -> Vec<u8> {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties {default}/></style:default-style><style:style style:name="Standard" style:family="paragraph"><style:text-properties {standard}/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Hallo</text:p></office:text></office:body></office:document-content>"#
    );
    crate::odt::tests::package(&content, Some(&styles))
}

/// `styles`' default paragraph style.
fn default_style(styles: &str) -> &str {
    let at = styles.find(r#"<style:default-style style:family="paragraph">"#).expect("a default paragraph style");
    styles[at..].split("</style:default-style>").next().unwrap_or_default()
}

/// A document's language is the one its file states, and a model-written save says it.
#[test]
fn the_default_language_is_the_files_and_written_back_from_the_model() {
    let d = import(&with_default_text(r#"fo:language="de" fo:country="DE""#, "")).unwrap();
    assert_eq!(d.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert_eq!(d.styles.resolve_char(None, &Default::default()).lang.as_deref(), Some("de-DE"));
    // Written from the model (a style or list definition changed), it is still the file's: without
    // it LibreOffice shows the document in its own default language.
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let styles = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(default_style(&styles).contains(r#"fo:language="de" fo:country="DE""#), "{styles}");
    assert!(!styles.contains(r#"fo:language="en""#), "no style says English: {styles}");
    assert_eq!(import(&saved).unwrap().styles.default_chr.lang.as_deref(), Some("de-DE"));
    // Saved as read, the styles WordCraft adds beside the file's don't say English either.
    let styles = crate::odt::tests::entry(&export(&d).unwrap(), "styles.xml");
    assert!(!styles.contains(r#"fo:language="en""#), "{styles}");
}

/// The document's language as WordCraft has it is Normal's (a Word file's Normal may state
/// another than its defaults): that is what the default style says when written from the model.
#[test]
fn the_documents_language_written_is_normals() {
    use std::sync::Arc;
    let mut d = Document::new();
    Arc::make_mut(&mut d.styles).default_chr.lang = Some("en-US".into());
    Arc::make_mut(&mut d.styles).get_mut("Normal").unwrap().chr.lang = Some("fr-FR".into());
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    let def = s.split(r#"<style:default-style style:family="paragraph""#).nth(1).and_then(|t| t.split("</style:default-style>").next()).unwrap();
    assert!(def.contains(r#"fo:language="fr""#) && def.contains(r#"fo:country="FR""#), "{def}");
    let back = import(&saved).unwrap();
    assert_eq!(back.styles.resolve_char(None, &Default::default()).lang.as_deref(), Some("fr-FR"));
}

/// A file stating no language, or `none` or `zxx`, has none: not the English of new documents.
#[test]
fn a_file_stating_no_language_has_none_and_writes_none() {
    let d = import(&with_default_text("", "")).unwrap();
    assert_eq!(d.styles.default_chr.lang, None, "not the English of WordCraft's new documents");
    let saved = crate::odt::tests::export_model(&d).unwrap();
    assert!(!crate::odt::tests::entry(&saved, "styles.xml").contains("fo:language"));
    assert_eq!(import(&saved).unwrap().styles.default_chr.lang, None);
    // `none` and `zxx` are no language.
    for none in ["none", "zxx"] {
        let d = import(&with_default_text(&format!(r#"fo:language="{none}" fo:country="none""#), "")).unwrap();
        assert_eq!(d.styles.default_chr.lang, None, "{none}");
    }
}

/// A language only Standard states is the document's, and a model-written save says it in the
/// default style.
#[test]
fn a_language_only_the_standard_style_states_survives_a_rewrite() {
    let d = import(&with_default_text("", r#"fo:language="nb" fo:country="NO""#)).unwrap();
    assert_eq!(d.styles.default_chr.lang.as_deref(), Some("nb-NO"));
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let styles = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(default_style(&styles).contains(r#"fo:language="nb" fo:country="NO""#), "{styles}");
    let back = import(&saved).unwrap();
    assert_eq!(back.styles.default_chr.lang.as_deref(), Some("nb-NO"));
    let p = back.body[0].as_para().unwrap();
    let lang =
        p.runs.first().and_then(|r| r.props.lang.clone()).or_else(|| back.styles.resolve_char(p.props.style.as_deref(), &Default::default()).lang);
    assert_eq!(lang.as_deref(), Some("nb-NO"), "{}", crate::odt::tests::entry(&saved, "styles.xml"));
}

/// A language with a script keeps it, read and written: the document's, a style's for each kind
/// of text, and a run's.
#[test]
fn a_language_with_a_script_keeps_it() {
    let d = import(&with_default_text(r#"fo:language="sr" fo:script="Latn" fo:country="RS""#, "")).unwrap();
    assert_eq!(d.styles.default_chr.lang.as_deref(), Some("sr-Latn-RS"));
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let styles = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(default_style(&styles).contains(r#"fo:language="sr" fo:script="Latn" fo:country="RS""#), "{styles}");
    assert!(!styles.contains(r#"fo:country="Latn""#), "{styles}");
    assert_eq!(import(&saved).unwrap().styles.default_chr.lang.as_deref(), Some("sr-Latn-RS"));

    let (latin, asian, complex) = (
        r#"fo:language="sr" fo:script="Cyrl" fo:country="BA""#,
        r#"style:language-asian="zh" style:script-asian="Hant" style:country-asian="TW""#,
        r#"style:language-complex="az" style:script-complex="Arab" style:country-complex="IR""#,
    );
    let run = r#"fo:language="bs" fo:script="Latn" fo:country="BA""#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Eigen" style:display-name="Eigen" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties {latin} {asian} {complex}/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties {run}/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Eigen">Own <text:span text:style-name="T1">span</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let check = |d: &wordcraft_doc::Document| {
        let chr = &d.styles.get("Eigen").unwrap().chr;
        assert_eq!(
            (chr.lang.as_deref(), chr.lang_ea.as_deref(), chr.lang_bidi.as_deref()),
            (Some("sr-Cyrl-BA"), Some("zh-Hant-TW"), Some("az-Arab-IR"))
        );
        assert_eq!(d.body[0].as_para().unwrap().runs.iter().find_map(|r| r.props.lang.as_deref()), Some("bs-Latn-BA"));
    };
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    check(&d);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let written = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(written.contains(latin) && written.contains(asian) && written.contains(complex), "{written}");
    assert!(crate::odt::tests::entry(&saved, "content.xml").contains(run));
    check(&import(&saved).unwrap());

    // No country, a region of digits, and what isn't a script.
    let attrs = crate::odt::writer::lang_attrs;
    assert_eq!(attrs("zh-Hant", ""), r#" fo:language="zh" fo:script="Hant""#);
    assert_eq!(attrs("es-419", ""), r#" fo:language="es" fo:country="419""#);
    assert_eq!(attrs("de-DE", "-asian"), r#" style:language-asian="de" style:country-asian="DE""#);
    let d = import(&with_default_text(r#"fo:language="sr" fo:script="Latin1" fo:country="RS""#, "")).unwrap();
    assert_eq!(d.styles.default_chr.lang.as_deref(), Some("sr-RS"));
}

/// A style with no parent inherits the default style's language, not Standard's (the document's
/// here), and one based on Standard takes Standard's.
#[test]
fn a_style_with_no_parent_has_the_default_styles_language() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:language="en" fo:country="US"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"><style:text-properties fo:language="de" fo:country="DE"/></style:style><style:style style:name="Eigen" style:display-name="Eigen" style:family="paragraph"/><style:style style:name="Gesagt" style:display-name="Gesagt" style:family="paragraph"><style:text-properties fo:language="fr" fo:country="FR"/></style:style><style:style style:name="Kind" style:display-name="Kind" style:family="paragraph" style:parent-style-name="Standard"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Eigen">a</text:p><text:p text:style-name="Gesagt">b</text:p><text:p text:style-name="Kind">c</text:p></office:text></office:body></office:document-content>"#
    );
    let check = |d: &wordcraft_doc::Document| {
        assert_eq!(d.styles.default_chr.lang.as_deref(), Some("de-DE"));
        let lang = |id: &str| d.styles.resolve_char(Some(id), &Default::default()).lang;
        assert_eq!(lang("Eigen").as_deref(), Some("en-US"), "no parent: the default style's");
        assert_eq!(lang("Gesagt").as_deref(), Some("fr-FR"), "its own");
        assert_eq!(lang("Kind").as_deref(), Some("de-DE"), "based on Standard");
    };
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    check(&d);
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap());
}

/// WordCraft's own documents say their language, so a reader doesn't use its own default.
#[test]
fn a_new_documents_language_is_stated() {
    let styles = crate::odt::tests::entry(&export(&wordcraft_doc::Document::new()).unwrap(), "styles.xml");
    assert!(default_style(&styles).contains(r#"fo:language="en" fo:country="US""#), "{styles}");
}

#[test]
fn a_size_is_every_scripts_size() {
    // Chinese text set to 24 pt stayed at the default size in LibreOffice: only the Latin size was
    // written.
    let mut d = Document::new();
    let mut p = Paragraph::with_text("漢字", CharProps { size: Some(24.0), ..Default::default() });
    p.props = Default::default();
    d.body = vec![para_block(p)];
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"fo:font-size="24pt" style:font-size-asian="24pt" style:font-size-complex="24pt""#), "{c}");
    let styles = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(styles.contains(r#"fo:font-size="12pt" style:font-size-asian="12pt" style:font-size-complex="12pt""#), "{styles}");
    // Read back every script has it.
    let back = import(&saved).unwrap();
    let run = &back.body[0].as_para().unwrap().runs[0].props;
    assert_eq!(back.styles.resolve_char(None, run).size_cs, 24.0);
    // A complex-script size of its own stays its own, written once.
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("x", CharProps { size: Some(24.0), size_cs: Some(30.0), ..Default::default() }))];
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert_eq!(c.matches("style:font-size-complex").count(), 1, "{c}");
    assert!(c.contains(r#"style:font-size-complex="30pt""#), "{c}");
}

/// Text and its complex-script size stated alike, over a default style with a smaller one (as
/// LibreOffice's files have): that size stays, on screen and saved, for a run and for a heading.
#[test]
fn a_complex_size_like_the_texts_stays_over_the_default() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt" style:font-size-complex="14pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"><style:text-properties fo:font-size="18pt" style:font-size-complex="18pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="24pt" style:font-size-complex="24pt"/></style:style></office:automatic-styles><office:body><office:text><text:p>a <text:span text:style-name="T1">big</text:span></text:p><text:h text:style-name="Heading_20_1" text:outline-level="1">h</text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let check = |d: &Document| {
        let p = d.body[0].as_para().unwrap();
        let c = d.styles.resolve_char(p.props.style.as_deref(), &p.runs.last().unwrap().props);
        assert_eq!((c.size, c.size_cs), (24.0, 24.0));
        let h = d.body[1].as_para().unwrap();
        let c = d.styles.resolve_char(h.props.style.as_deref(), &h.runs[0].props);
        assert_eq!((c.size, c.size_cs), (18.0, 18.0));
    };
    check(&d);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let written = crate::odt::tests::entry(&saved, "styles.xml");
    let heading = written.split(r#"style:name="Heading_20_1""#).nth(1).and_then(|t| t.split("</style:style>").next()).unwrap();
    assert!(heading.contains(r#"style:font-size-complex="18pt""#) && !heading.contains(r#"style:font-size-complex="14pt""#), "{heading}");
    check(&import(&saved).unwrap());

    // A default stating one size for all goes with that size: Normal changed takes it along.
    let same = styles.replace(r#"style:font-size-complex="14pt""#, r#"style:font-size-complex="12pt""#);
    let d = import(&package(&content, Some(&same))).unwrap();
    assert_eq!(d.styles.default_chr.size_cs, None);
}

/// A complex size equal to the text's goes only where no run's complex size changes without it:
/// not under a character style over another size, nor under a size stated alone.
#[test]
fn a_complex_size_something_shows_stays() {
    let open = |s: &str, auto: &str, body: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt" style:font-size-complex="12pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/>{s}</office:styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        import(&package(&content, Some(&styles))).unwrap()
    };
    let sizes = |d: &Document| {
        let p = d.body[0].as_para().unwrap();
        let c = d.styles.resolve_char(p.props.style.as_deref(), &p.runs.last().unwrap().props);
        (c.size, c.size_cs)
    };
    // A character style's, over a paragraph style with another complex size.
    let d = open(
        r#"<style:style style:name="P" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="12pt" style:font-size-complex="18pt"/></style:style><style:style style:name="C" style:family="text"><style:text-properties fo:font-size="24pt" style:font-size-complex="24pt"/></style:style>"#,
        "",
        r#"<text:p text:style-name="P"><text:span text:style-name="C">One</text:span></text:p>"#,
    );
    assert_eq!(sizes(&d), (24.0, 24.0));
    let parent = r#"<style:style style:name="Parent" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="16pt" style:font-size-complex="16pt"/></style:style>"#;
    // A parent's, under a child stating only a size.
    let kid = format!(
        r#"{parent}<style:style style:name="Kid" style:family="paragraph" style:parent-style-name="Parent"><style:text-properties fo:font-size="20pt"/></style:style>"#
    );
    assert_eq!(sizes(&open(&kid, "", r#"<text:p text:style-name="Kid">One</text:p>"#)), (20.0, 16.0));
    // A style's, under a run stating only a size.
    let d = open(
        parent,
        r#"<style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="30pt"/></style:style>"#,
        r#"<text:p text:style-name="Parent"><text:span text:style-name="T1">One</text:span></text:p>"#,
    );
    assert_eq!(sizes(&d), (30.0, 16.0));
    // Nothing shows it: one size for all, so a change of size takes it along.
    let d = open(parent, "", r#"<text:p text:style-name="Parent">One</text:p>"#);
    assert_eq!(sizes(&d), (16.0, 16.0));
    assert_eq!(d.styles.get("Parent").unwrap().chr.size_cs, None);
}

/// A size stated alone, by a run or a style, under a different complex size above it: saved from the
/// model, and saved again, the complex size is still the one above, not the stated size.
#[test]
fn a_size_stated_alone_keeps_the_complex_size_above_it_when_saved() {
    let sizes = |d: &Document, i: usize| {
        let p = d.body[i].as_para().unwrap();
        let c = d.styles.resolve_char(p.props.style.as_deref(), &p.runs.last().unwrap().props);
        (c.size, c.size_cs)
    };
    let twice = |d: Document| {
        let once = import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
        let again = import(&crate::odt::tests::export_model(&once).unwrap()).unwrap();
        (once, again)
    };
    // From a file: a run stating only a size, and a style stating only a size, under a parent
    // style with its own complex size.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt" style:font-size-complex="12pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Parent" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="16pt" style:font-size-complex="16pt"/></style:style><style:style style:name="Kid" style:family="paragraph" style:parent-style-name="Parent"><style:text-properties fo:font-size="20pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="30pt"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Parent"><text:span text:style-name="T1">One</text:span></text:p><text:p text:style-name="Kid">Two</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!((sizes(&d, 0), sizes(&d, 1)), ((30.0, 16.0), (20.0, 16.0)));
    let (once, again) = twice(d);
    assert_eq!((sizes(&once, 0), sizes(&once, 1)), ((30.0, 16.0), (20.0, 16.0)), "saved once");
    assert_eq!((sizes(&again, 0), sizes(&again, 1)), ((30.0, 16.0), (20.0, 16.0)), "saved twice");
    // Built in the model (a DOCX run with `w:sz` and no `w:szCs`): the default's complex size.
    let mut d = Document::new();
    let styles = std::sync::Arc::make_mut(&mut d.styles);
    styles.default_chr.size = Some(12.0);
    styles.default_chr.size_cs = Some(16.0);
    let mut p = Paragraph::with_text("x", CharProps { size: Some(30.0), ..Default::default() });
    p.props = Default::default();
    d.body = vec![para_block(p)];
    assert_eq!(sizes(&d, 0), (30.0, 16.0));
    let (once, again) = twice(d);
    assert_eq!((sizes(&once, 0), sizes(&again, 0)), ((30.0, 16.0), (30.0, 16.0)));
}

#[test]
fn a_headings_text_of_another_size_keeps_it() {
    // Only the size its style gives goes with the style: a smaller word in a heading came back
    // from a save at the heading's size.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:default-outline-level="1"><style:text-properties fo:font-size="16pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="10pt"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">big<text:span text:style-name="T1">small</text:span></text:h></office:text></office:body></office:document-content>"#
    );
    let check = |d: &Document, what: &str| {
        let p = d.body[0].as_para().unwrap();
        let size = |run: usize| d.styles.resolve_char(p.props.style.as_deref(), &p.runs[run].props).size;
        assert_eq!((size(0), size(1)), (16.0, 10.0), "{what}");
    };
    let d = import(&package(&content, Some(&styles))).unwrap();
    check(&d, "read");
    assert_eq!(d.body[0].as_para().unwrap().runs[0].props.size, None, "the style's stays the style's");
    check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "written from the model");
}

/// In a heading style with no parent, where Standard's size and the default's differ, text stating
/// a size keeps that size.
#[test]
fn a_parentless_headings_text_keeps_its_size() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="10pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"><style:text-properties fo:font-size="14pt"/></style:style><style:style style:name="H" style:family="paragraph" style:default-outline-level="1"/></office:styles></office:document-styles>"#
    );
    for size in ["10pt", "14pt"] {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="{size}"/></style:style><style:style style:name="T2" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="H" text:outline-level="1"><text:span text:style-name="T1">One</text:span><text:span text:style-name="T2">Two</text:span></text:h></office:text></office:body></office:document-content>"#
        );
        let want: f32 = size.trim_end_matches("pt").parse().unwrap();
        let check = |d: &Document, what: &str| {
            let p = d.body[0].as_para().unwrap();
            assert_eq!(p.runs[1].props.size, None, "{size} {what}: a span stating no size has none");
            assert_eq!(
                d.styles.resolve_char(p.props.style.as_deref(), &p.runs[0].props).size,
                want,
                "{size} {what}: {:?} {:?}",
                p.props.style,
                p.runs[0].props.size
            );
        };
        let d = import(&package(&content, Some(&styles))).unwrap();
        check(&d, "read");
        check(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap(), "written from the model");
    }
}

/// A percent in a heading style with no parent is of the default style's size, not Standard's.
#[test]
fn a_percent_in_a_parentless_heading_is_of_the_default_size() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="10pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"><style:text-properties fo:font-size="14pt"/></style:style><style:style style:name="H" style:family="paragraph" style:default-outline-level="1"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="50%"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="H" text:outline-level="1"><text:span text:style-name="T1">One</text:span></text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(d.body[0].as_para().unwrap().runs[0].props.size, Some(5.0));
}

/// A heading style with no parent and a percent size is that percent of the default's size, so text
/// stating that size is at the style's size.
#[test]
fn a_parentless_heading_style_in_percent_is_of_the_default_size() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="10pt"/></style:default-style><style:style style:name="H" style:family="paragraph"><style:text-properties fo:font-size="50%"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-size="5pt"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="H" text:outline-level="1"><text:span text:style-name="T1">One</text:span>Two</text:h></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let p = d.body[0].as_para().unwrap();
    for r in &p.runs {
        assert_eq!(d.styles.resolve_char(p.props.style.as_deref(), &r.props).size, 5.0, "{:?} {:?}", p.props.style, r.props.size);
    }
}

/// Text at its heading's size goes with the heading style, stated or not, and a size in percent
/// is of the heading's size; text of another size keeps it, and so does a paragraph's own.
#[test]
fn a_headings_text_sizes_are_measured_against_the_heading() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:font-size="20pt"/></style:style><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Heading" style:default-outline-level="1"><style:text-properties fo:font-size="80%"/></style:style></office:styles></office:document-styles>"#
    );
    let span = |name: &str, size: &str| {
        format!(r#"<style:style style:name="{name}" style:family="text"><style:text-properties fo:font-size="{size}"/></style:style>"#)
    };
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{}{}{}<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Heading_20_1"><style:text-properties fo:font-size="30pt"/></style:style></office:automatic-styles><office:body><office:text><text:h text:style-name="Heading_20_1" text:outline-level="1">plain <text:span text:style-name="Same">same</text:span> <text:span text:style-name="Pct">pct</text:span> <text:span text:style-name="Other">other</text:span></text:h><text:h text:style-name="P1" text:outline-level="1">own</text:h></office:text></office:body></office:document-content>"#,
        span("Same", "16pt"),
        span("Pct", "50%"),
        span("Other", "10pt"),
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let sizes = |d: &Document| {
        let p = d.body[0].as_para().unwrap();
        let of = |word: &str| {
            let at = p.text.find(word).unwrap();
            let mut from = 0;
            let run = p.runs.iter().find(|r| {
                from += r.len;
                at < from
            });
            d.styles.resolve_char(p.props.style.as_deref(), &run.unwrap().props).size
        };
        [of("plain"), of("same"), of("pct"), of("other")]
    };
    let want = |t: f32| [t, t, 8.0, 10.0];
    let plain = |d: &Document| d.body[0].as_para().unwrap().runs.iter().filter(|r| r.props.size.is_some()).count();
    assert_eq!(sizes(&d), want(16.0));
    assert_eq!(plain(&d), 2, "only the two of another size state one");
    // The paragraph's own size stays with its text.
    let own = d.body[1].as_para().unwrap();
    assert_eq!(d.styles.resolve_char(own.props.style.as_deref(), &own.runs[0].props).size, 30.0);
    // A change to the style reaches what goes with it.
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    std::sync::Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(24.0);
    assert_eq!(sizes(&d), want(24.0));
}

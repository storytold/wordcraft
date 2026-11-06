//! Paragraph spacing, indents and line spacing (ODF 1.3 §20.204 `fo:line-height`, §20.206–209
//! `fo:margin-*`, §20.225 `fo:text-indent`, §20.317 `style:line-height-at-least`, §20.318
//! `style:line-spacing`).

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{LineSpacing, ParaProps};
use wordcraft_doc::{Document, Paragraph, para_block};

fn props(d: &Document) -> Vec<ParaProps> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.clone()).collect()
}

fn content(auto: &str, body: &str) -> String {
    format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    )
}

fn p_style(name: &str, parent: &str, props: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="paragraph" style:parent-style-name="{parent}"><style:paragraph-properties {props}/></style:style>"#
    )
}

fn close(a: Option<f32>, b: f32) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.01)
}

#[test]
fn margins_indents_and_line_spacing_are_read() {
    let auto = [
        p_style("P1", "Standard", r#"fo:margin-top="24pt" fo:margin-bottom="0.5cm""#),
        p_style("P2", "Standard", r#"fo:margin-left="2cm" fo:margin-right="1cm" fo:text-indent="-1cm""#),
        p_style("P3", "Standard", r#"fo:line-height="200%""#),
        p_style("P4", "Standard", r#"fo:line-height="20pt""#),
        p_style("P5", "Standard", r#"style:line-height-at-least="17pt""#),
        p_style("P6", "Standard", r#"fo:line-height="normal""#),
    ]
    .concat();
    let body = (1..=6).map(|i| format!(r#"<text:p text:style-name="P{i}">p{i}</text:p>"#)).collect::<String>();
    let d = import(&package(&content(&auto, &body), None)).unwrap();
    let p = props(&d);
    assert!(close(p[0].space_before, 24.0) && close(p[0].space_after, 14.17), "{:?}", p[0]);
    assert!(close(p[1].indent_left, 56.69) && close(p[1].indent_right, 28.35) && close(p[1].indent_first, -28.35), "{:?}", p[1]);
    assert_eq!(p[2].line_spacing, Some(LineSpacing::Multiple(2.0)));
    assert_eq!(p[3].line_spacing, Some(LineSpacing::Exactly(20.0)));
    assert_eq!(p[4].line_spacing, Some(LineSpacing::AtLeast(17.0)));
    assert_eq!(p[5].line_spacing, Some(LineSpacing::Multiple(1.0)));
}

#[test]
fn spacing_is_inherited_through_common_styles() {
    // LibreOffice's "Text body" style carries spacing that its paragraphs show.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/>{}</office:styles></office:document-styles>"#,
        p_style("Text_20_body", "Standard", r#"fo:margin-top="0cm" fo:margin-bottom="0.247cm" fo:line-height="115%""#)
    );
    let auto = p_style("P1", "Text_20_body", r#"fo:margin-left="1cm""#);
    let body = r#"<text:p text:style-name="Text_20_body">a</text:p><text:p text:style-name="P1">b</text:p>"#;
    let d = import(&package(&content(&auto, body), Some(&styles))).unwrap();
    for p in props(&d) {
        let r = d.styles.resolve_para(&p);
        assert!((r.space_after - 7.0).abs() < 0.01 && r.space_before == 0.0, "{r:?}");
        assert_eq!(r.line_spacing, LineSpacing::Multiple(1.15));
    }
    assert!(close(props(&d)[1].indent_left, 28.35));
}

#[test]
fn the_default_paragraph_style_gives_the_document_defaults() {
    // An ODT paragraph with no spacing has none (ODF's initial values), not Word's 8 pt after
    // and 1.15 lines.
    let body = "<text:p>plain</text:p>";
    let d = import(&package(&content("", body), None)).unwrap();
    let r = d.styles.resolve_para(&props(&d)[0]);
    assert_eq!((r.space_before, r.space_after, r.line_spacing), (0.0, 0.0, LineSpacing::Multiple(1.0)));
    // The default style and Standard set them: Standard wins.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties fo:margin-bottom="6pt" fo:line-height="150%"/></style:default-style><style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties fo:margin-bottom="3pt"/></style:style></office:styles></office:document-styles>"#
    );
    let d = import(&package(&content("", body), Some(&styles))).unwrap();
    let p = props(&d);
    assert_eq!(p[0].space_after, None, "the defaults are not direct formatting");
    let r = d.styles.resolve_para(&p[0]);
    assert_eq!((r.space_after, r.line_spacing), (3.0, LineSpacing::Multiple(1.5)));
}

#[test]
fn hostile_spacing_values_are_clamped() {
    let auto = [
        p_style("P1", "Standard", r#"fo:margin-top="-5pt" fo:margin-bottom="1e30pt" fo:margin-left="-1e30pt" fo:text-indent="NaNpt""#),
        p_style("P2", "Standard", r#"fo:line-height="100000%""#),
        p_style("P3", "Standard", r#"fo:line-height="-3pt""#),
        p_style("P4", "Standard", r#"fo:line-height="0%" style:line-height-at-least="1e9pt""#),
        p_style("P5", "Standard", r#"fo:margin-top="10%" fo:line-height="x""#),
    ]
    .concat();
    let body = (1..=5).map(|i| format!(r#"<text:p text:style-name="P{i}">p{i}</text:p>"#)).collect::<String>();
    let d = import(&package(&content(&auto, &body), None)).unwrap();
    let p = props(&d);
    assert_eq!(p[0].space_before, Some(0.0));
    assert_eq!(p[0].space_after, Some(1584.0));
    assert_eq!(p[0].indent_left, Some(-1584.0));
    assert_eq!(p[0].indent_first, None);
    assert_eq!(p[1].line_spacing, Some(LineSpacing::Multiple(132.0)));
    assert_eq!(p[2].line_spacing, None);
    assert!(matches!(p[3].line_spacing, Some(LineSpacing::AtLeast(v)) if v == 1584.0), "{:?}", p[3].line_spacing);
    assert_eq!((p[4].space_before, p[4].line_spacing), (None, None));
}

fn para(text: &str, props: ParaProps) -> std::sync::Arc<wordcraft_doc::Block> {
    let mut p = Paragraph::new();
    p.text = text.into();
    p.runs = vec![wordcraft_doc::para::Run { len: text.len(), props: Default::default() }];
    p.props = props;
    para_block(p)
}

#[test]
fn direct_spacing_round_trips() {
    let mut d = Document::new();
    let want = vec![
        ParaProps { space_before: Some(18.0), space_after: Some(6.0), ..Default::default() },
        ParaProps { indent_left: Some(36.0), indent_right: Some(18.0), indent_first: Some(-18.0), ..Default::default() },
        ParaProps { line_spacing: Some(LineSpacing::Multiple(1.5)), ..Default::default() },
        ParaProps { line_spacing: Some(LineSpacing::Exactly(20.0)), ..Default::default() },
        ParaProps { line_spacing: Some(LineSpacing::AtLeast(15.0)), ..Default::default() },
        ParaProps::default(),
    ];
    d.body = want.iter().enumerate().map(|(i, p)| para(&format!("p{i}"), p.clone())).collect();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(props(&back), want);
}

#[test]
fn the_document_defaults_round_trip() {
    // A new document's Normal (8 pt after, 1.15 lines) is written as the default paragraph style,
    // so the copy reopens with the same spacing.
    let mut d = Document::new();
    d.body = vec![para("x", ParaProps::default())];
    let back = import(&export(&d).unwrap()).unwrap();
    let r = back.styles.resolve_para(&props(&back)[0]);
    let o = d.styles.resolve_para(&ParaProps::default());
    assert_eq!((r.space_before, r.space_after, r.line_spacing), (o.space_before, o.space_after, o.line_spacing));
}

#[test]
fn contextual_spacing_round_trips() {
    // `style:contextual-spacing` (ODF 1.3 §20.255): no space between paragraphs of one style.
    let auto = p_style("P1", "Standard", r#"fo:margin-bottom="10pt" style:contextual-spacing="true""#);
    let d = import(&package(&content(&auto, r#"<text:p text:style-name="P1">a</text:p>"#), None)).unwrap();
    assert_eq!(props(&d)[0].contextual_spacing, Some(true));
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(props(&back)[0].contextual_spacing, Some(true));
}

#[test]
fn the_list_paragraph_style_is_written() {
    // Its spacing (and contextual spacing) was dropped: list items were written in Standard.
    let mut d = Document::new();
    let mut p = Paragraph::new();
    p.text = "item".into();
    p.runs = vec![wordcraft_doc::para::Run { len: 4, props: Default::default() }];
    p.props = ParaProps { style: Some("ListParagraph".into()), ..Default::default() };
    d.body = vec![para_block(p)];
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).get_mut("ListParagraph") {
        st.para.contextual_spacing = Some(true);
        st.para.space_after = Some(3.0);
    }
    let back = import(&export(&d).unwrap()).unwrap();
    let r = back.styles.resolve_para(&props(&back)[0]);
    assert!(r.contextual_spacing && r.space_after == 3.0, "{r:?}");
}

#[test]
fn leading_is_at_least_a_line_and_the_leading() {
    // LibreOffice's leading (`style:line-spacing`) draws its extra space above each line, as an
    // at-least spacing does: a 12 pt line (14 pt high) and 3 mm.
    let d = import(&package(&content(&p_style("P1", "Standard", r#"style:line-spacing="3mm""#), r#"<text:p text:style-name="P1">x</text:p>"#), None))
        .unwrap();
    match props(&d)[0].line_spacing {
        Some(LineSpacing::AtLeast(h)) => assert!((h - (14.0 + 3.0 * 72.0 / 25.4)).abs() < 0.01, "{h}"),
        other => panic!("{other:?}"),
    }
    // LibreOffice leaves the leading after the last line too: space after the paragraph, on
    // top of its own.
    let after = |extra: &str| {
        let d = import(&package(
            &content(&p_style("P1", "Standard", &format!(r#"style:line-spacing="3mm"{extra}"#)), r#"<text:p text:style-name="P1">x</text:p>"#),
            None,
        ))
        .unwrap();
        let r = d.styles.resolve_para(&props(&d)[0]);
        (r.space_after * 100.0).round() / 100.0
    };
    assert_eq!(after(""), 8.5);
    assert_eq!(after(r#" fo:margin-bottom="6pt""#), 14.5);
}

/// Each paragraph's resolved space before, space after and line spacing.
fn resolved(d: &Document) -> Vec<(f32, f32, LineSpacing)> {
    props(d)
        .iter()
        .map(|p| {
            let r = d.styles.resolve_para(p);
            (r.space_before, r.space_after, r.line_spacing)
        })
        .collect()
}

fn assert_spacing(got: &[(f32, f32, LineSpacing)], want: &[(f32, f32, f32)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        let ok = (g.0 - w.0).abs() < 0.01 && (g.1 - w.1).abs() < 0.01 && g.2 == LineSpacing::Multiple(w.2);
        assert!(ok, "got {got:?}, want {want:?}");
    }
}

#[test]
fn automatic_styles_on_a_parentless_no_spacing_style_keep_its_spacing() {
    // #5101, as a LibreOffice résumé lays it out: Standard sets 0.139in after and 115% lines,
    // but most paragraphs use automatic styles that only change the font, whose parent is "No
    // Spacing", a common style with no parent that sets 0 / 0 / 100%. Those paragraphs take No
    // Spacing's values, not Standard's (in LibreOffice 9 pages; Standard's spacing made 14).
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles>
        <style:style style:name="Standard" style:family="paragraph" style:class="text"><style:paragraph-properties fo:margin-top="0in" fo:margin-bottom="0.139in" fo:line-height="115%"/></style:style>
        <style:style style:name="No_20_Spacing" style:display-name="No Spacing" style:family="paragraph"><style:paragraph-properties fo:margin-top="0in" fo:margin-bottom="0in" fo:line-height="100%"/></style:style>
        </office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
        <style:style style:name="P1" style:family="paragraph" style:parent-style-name="No_20_Spacing"><style:text-properties fo:font-size="12pt" fo:font-weight="bold"/></style:style>
        <style:style style:name="P2" style:family="paragraph" style:parent-style-name="No_20_Spacing"><style:text-properties fo:font-size="12pt"/></style:style>
        </office:automatic-styles><office:body><office:text>
        <text:p text:style-name="P1">Heading line</text:p><text:p text:style-name="P2">Body line</text:p>
        <text:p text:style-name="No_20_Spacing">Plain no spacing</text:p><text:p>Default</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_spacing(&resolved(&d), &[(0.0, 0.0, 1.0), (0.0, 0.0, 1.0), (0.0, 0.0, 1.0), (0.0, 10.008, 1.15)]);
}

#[test]
fn a_parentless_style_inherits_the_default_styles_spacing_not_standards() {
    // ODF 1.3 §16.2: a common style with no parent inherits from the default style. Standard is
    // the document default only for paragraphs that use it (or no style, or an automatic style
    // with no parent, which LibreOffice puts on Standard), so what "Plain" leaves unset comes
    // from the default style (0.05in after), then ODF's initial values (single lines).
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles>
        <style:default-style style:family="paragraph"><style:paragraph-properties fo:margin-bottom="0.05in"/></style:default-style>
        <style:style style:name="Standard" style:family="paragraph" style:class="text"><style:paragraph-properties fo:margin-top="0.1in" fo:margin-bottom="0.139in" fo:line-height="115%"/></style:style>
        <style:style style:name="Plain" style:family="paragraph"><style:paragraph-properties fo:margin-top="6pt"/></style:style>
        <style:style style:name="Body" style:family="paragraph" style:parent-style-name="Standard"/>
        </office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
        <style:style style:name="P1" style:family="paragraph" style:parent-style-name="Plain"><style:text-properties fo:font-size="12pt"/></style:style>
        <style:style style:name="P2" style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:style>
        </office:automatic-styles><office:body><office:text>
        <text:p text:style-name="Plain">Plain</text:p><text:p text:style-name="P1">Plain, automatic</text:p>
        <text:p text:style-name="Body">Body</text:p><text:p>Default</text:p><text:p text:style-name="P2">Automatic only</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let plain = (6.0, 3.6, 1.0);
    let standard = (7.2, 10.008, 1.15);
    assert_spacing(&resolved(&d), &[plain, plain, standard, standard, standard]);
}

#[test]
fn standards_own_parent_style_gives_what_standard_leaves_unset() {
    // §16.2 again: should Standard have a parent, what Standard leaves unset comes from that
    // parent (12pt after) before the default style (2pt after, 200% lines).
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles>
        <style:default-style style:family="paragraph"><style:paragraph-properties fo:margin-bottom="2pt" fo:line-height="200%"/></style:default-style>
        <style:style style:name="Base" style:family="paragraph"><style:paragraph-properties fo:margin-top="3pt" fo:margin-bottom="12pt"/></style:style>
        <style:style style:name="Standard" style:family="paragraph" style:class="text" style:parent-style-name="Base"><style:paragraph-properties fo:margin-top="6pt"/></style:style>
        <style:style style:name="Body" style:family="paragraph" style:parent-style-name="Standard"/>
        </office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text>
        <text:p>Default</text:p><text:p text:style-name="Body">Body</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_spacing(&resolved(&d), &[(6.0, 12.0, 2.0), (6.0, 12.0, 2.0)]);
}

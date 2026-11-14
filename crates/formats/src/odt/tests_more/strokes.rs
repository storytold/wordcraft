//! A shape's line and fill beyond plain colour: arrowheads (ODF 1.3 §16.39 `draw:marker`,
//! `draw:marker-start`/`-end`), dashes (§16.41 `draw:stroke-dash`) and gradients (§16.40
//! `draw:gradient`), defined by name in the styles and used by graphic styles; read into
//! WordCraft's arrowheads, dash patterns and gradient fills, and written back.

use crate::odt::tests::{NS_T, package, styles_of};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::fill::{Fill, GradientKind, PathShape};
use wordcraft_doc::line_dash::LineDash;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// Each shape: (arrowheads at start and end, dash, fill).
fn looks(d: &Document) -> Vec<((bool, bool), LineDash, Option<Fill>)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { fill, extra, .. } => Some(((extra.arrow_start, extra.arrow_end), extra.dash.clone(), fill.clone())),
            _ => None,
        })
        .collect()
}

/// The direction a line is drawn in: (from its right end, from its bottom end).
fn line_flips(d: &Document) -> Vec<(bool, bool)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind: wordcraft_doc::para::ShapeKind::Line, float, .. } => Some((float.flip_h, float.flip_v)),
            _ => None,
        })
        .collect()
}

/// LibreOffice's own definitions and uses (from a document it saved).
fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T} {NS_D}><office:styles>
<draw:gradient draw:name="Gradient_20_1" draw:style="linear" draw:start-color="#ff0000" draw:end-color="#ffff00" draw:start-intensity="100%" draw:end-intensity="100%" draw:angle="0deg" draw:border="0%"/>
<draw:gradient draw:name="Gradient_20_2" draw:style="radial" draw:cx="50%" draw:cy="50%" draw:start-color="#0000ff" draw:end-color="#ffffff" draw:start-intensity="100%" draw:end-intensity="100%" draw:border="0%"/>
<draw:gradient draw:name="Old" draw:style="axial" draw:start-color="#000000" draw:end-color="#ffffff" draw:angle="900"/>
<draw:marker draw:name="Arrow" svg:viewBox="0 0 20 20" svg:d="M0 20l10-20 10 20z"/>
<draw:stroke-dash draw:name="Dash_20_Dot_20_4" draw:style="rect" draw:dots1="1" draw:dots1-length="400%" draw:dots2="1" draw:dots2-length="100%" draw:distance="300%"/>
<draw:stroke-dash draw:name="Dot" draw:style="rect" draw:dots1="1" draw:dots1-length="100%" draw:distance="100%"/>
<draw:stroke-dash draw:name="Abs" draw:style="rect" draw:dots1="2" draw:dots1-length="0.1in" draw:distance="0.05in"/>
</office:styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T} {NS_D}><office:automatic-styles>
<style:style style:name="gr1" style:family="graphic"><style:graphic-properties svg:stroke-width="1pt" draw:marker-end="Arrow"/></style:style>
<style:style style:name="gr2" style:family="graphic"><style:graphic-properties draw:stroke="dash" draw:stroke-dash="Dash_20_Dot_20_4" svg:stroke-width="1pt" draw:marker-start="Arrow" draw:marker-end="Arrow"/></style:style>
<style:style style:name="gr3" style:family="graphic"><style:graphic-properties draw:stroke="dash" draw:stroke-dash="Dot" svg:stroke-width="1pt" draw:fill="gradient" draw:fill-gradient-name="Gradient_20_1"/></style:style>
<style:style style:name="gr4" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Gradient_20_2" draw:stroke="dash" draw:stroke-dash="Abs" svg:stroke-width="3.6pt"/></style:style>
<style:style style:name="gr5" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Old"/></style:style>
</office:automatic-styles><office:body><office:text><text:p>
<draw:line text:anchor-type="paragraph" draw:style-name="gr1" svg:x1="0in" svg:y1="0in" svg:x2="2in" svg:y2="0.5in"/>
<draw:line text:anchor-type="paragraph" draw:style-name="gr2" svg:x1="2in" svg:y1="1in" svg:x2="0in" svg:y2="1in"/>
<draw:rect text:anchor-type="paragraph" draw:style-name="gr3" svg:width="1in" svg:height="1in"/>
<draw:ellipse text:anchor-type="paragraph" draw:style-name="gr4" svg:width="1in" svg:height="1in"/>
<draw:rect text:anchor-type="paragraph" draw:style-name="gr5" svg:width="1in" svg:height="1in"/>
</text:p></office:text></office:body></office:document-content>"##
    );
    import(&package(&content, Some(&styles))).unwrap()
}

fn gradient(kind: GradientKind, stops: &[(f32, Rgb)]) -> Option<Fill> {
    Some(Fill::Gradient(std::sync::Arc::new(wordcraft_doc::fill::Gradient {
        stops: stops.iter().map(|&(pos, color)| wordcraft_doc::wordart::GradStop { pos, color, transparency: 0.0 }).collect(),
        kind,
        tile: None,
        rot_with_shape: true,
        flip: Default::default(),
    })))
}

#[test]
fn arrowheads_dashes_and_gradients_open() {
    let d = open();
    let (red, yellow, blue, white, black) = (Rgb(255, 0, 0), Rgb(255, 255, 0), Rgb(0, 0, 255), Rgb::WHITE, Rgb::BLACK);
    let l = looks(&d);
    assert_eq!(l[0].0, (false, true));
    assert_eq!(l[0].1, LineDash::Solid);
    // Drawn from right to left: its start, with an arrowhead too, is on the right.
    assert_eq!(l[1].0, (true, true));
    assert_eq!(l[1].1, LineDash::DashDot);
    assert_eq!(line_flips(&d), [(false, false), (true, false)]);
    // ODF's 0° runs top to bottom.
    assert_eq!(
        (l[2].1.clone(), l[2].2.clone()),
        (LineDash::SystemDot, gradient(GradientKind::Linear { angle: 90.0, scaled: false }, &[(0.0, red), (100.0, yellow)]))
    );
    // Radial: the end colour in the middle, the start colour at the edge.
    assert_eq!(l[3].2, gradient(GradientKind::Path { shape: PathShape::Circle, focus: [50.0, 50.0, 50.0, 50.0] }, &[(0.0, white), (100.0, blue)]));
    // Lengths in inches, for a 3.6pt line: 2 dashes of 7.2pt (2 widths), 3.6pt (1 width) apart.
    assert_eq!(l[3].1, LineDash::Custom(vec![(200_000, 100_000), (200_000, 100_000)]));
    // An angle without a unit is in tenths of a degree (ODF 1.2); axial runs out from the middle.
    assert_eq!(l[4].2, gradient(GradientKind::Linear { angle: 0.0, scaled: false }, &[(0.0, black), (50.0, white), (100.0, black)]));
}

#[test]
fn arrowheads_dashes_and_gradients_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let s = styles_of(&bytes);
    assert!(s.contains("<draw:marker ") && s.contains("<draw:stroke-dash ") && s.contains("<draw:gradient "), "{s}");
    let back = import(&bytes).unwrap();
    assert_eq!(looks(&back), looks(&d));
    assert_eq!(line_flips(&back), line_flips(&d));
}

#[test]
fn every_dash_round_trips() {
    use LineDash::*;
    let dashes = [
        Dot,
        Dash,
        LongDash,
        DashDot,
        LongDashDot,
        LongDashDotDot,
        SystemDash,
        SystemDot,
        SystemDashDot,
        SystemDashDotDot,
        Custom(vec![(250_000, 150_000)]),
    ];
    let mut p = wordcraft_doc::Paragraph::new();
    for dash in &dashes {
        p.text.push(wordcraft_doc::para::OBJ);
        let extra = wordcraft_doc::connector::ShapeExtra { dash: dash.clone(), ..Default::default() };
        p.objects.push(InlineObject::Shape {
            kind: wordcraft_doc::para::ShapeKind::Rectangle,
            w: 50.0,
            h: 40.0,
            fill: None,
            stroke: Some(Rgb::BLACK),
            stroke_width: 2.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra,
        });
    }
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(looks(&back).into_iter().map(|l| l.1).collect::<Vec<_>>(), dashes);
}

#[test]
fn hostile_definitions_do_not_panic() {
    for def in [
        r#"<draw:stroke-dash draw:name="D" draw:dots1="4000000000" draw:dots1-length="1e30%" draw:dots2="-1" draw:distance="NaN%"/>"#,
        r#"<draw:stroke-dash draw:name="D" draw:dots1-length="1in"/>"#,
        r#"<draw:gradient draw:name="G" draw:style="radial" draw:cx="1e30%" draw:cy="-5%" draw:border="900%" draw:angle="1e40deg" draw:start-intensity="-50%"/>"#,
        r#"<draw:gradient draw:name="G" draw:style="nonsense" draw:start-color="red"/>"#,
    ] {
        let styles = format!(r#"<office:document-styles {NS_T} {NS_D}><office:styles>{def}</office:styles></office:document-styles>"#);
        let content = format!(
            r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="g" style:family="graphic"><style:graphic-properties draw:stroke="dash" draw:stroke-dash="D" svg:stroke-width="0pt" draw:fill="gradient" draw:fill-gradient-name="G"/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:rect draw:style-name="g" svg:width="1in" svg:height="1in"/></text:p></office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, Some(&styles))).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn unitless_gradient_angles_follow_the_files_version() {
    // §18.3.1: an angle without a unit is in degrees. LibreOffice and OpenOffice wrote gradient
    // angles in tenths of a degree in files before ODF 1.3 and read them so: a deviation from
    // the standard, out of necessity (`strokes::legacy_angles`). From 1.3 on, the standard.
    let fill = |version: &str, angle: &str| {
        let styles = format!(
            r##"<office:document-styles {NS_T} {NS_D} office:version="{version}"><office:styles><draw:gradient draw:name="G" draw:style="linear" draw:start-color="#000000" draw:end-color="#ffffff" draw:angle="{angle}"/></office:styles></office:document-styles>"##
        );
        let content = format!(
            r##"<office:document-content {NS_T} {NS_D} office:version="{version}"><office:automatic-styles><style:style style:name="gr" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="G"/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:rect text:anchor-type="paragraph" draw:style-name="gr" svg:width="1in" svg:height="1in"/></text:p></office:text></office:body></office:document-content>"##
        );
        let d = import(&package(&content, Some(&styles))).unwrap();
        looks(&d).into_iter().next().and_then(|(_, _, f)| f)
    };
    // 30° as written before 1.3 (tenths), as 1.3 writes it (with a unit), and as 1.3 reads a
    // bare number (degrees) are the same gradient.
    assert_eq!(fill("1.2", "300"), fill("1.3", "30deg"));
    assert_eq!(fill("1.3", "30"), fill("1.3", "30deg"));
    assert_ne!(fill("1.3", "300"), fill("1.3", "30deg"));
    assert!(fill("1.2", "300").is_some());
}

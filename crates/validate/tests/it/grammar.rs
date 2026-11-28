//! Integration tests for the five ECMA-376 grammar checks ported at Warning severity
//! (issue #7640):
//! - `GRAMMAR-REQUIRED-CHILD`
//! - `GRAMMAR-CHILD-NOT-ALLOWED`
//! - `GRAMMAR-CHILD-TOO-MANY`
//! - `GRAMMAR-ORDER`
//! - `GRAMMAR-NUMFMT-VALUE`
//!
//! `GRAMMAR-ROOT` is excluded by design. Every grammar rule ships at Warning severity and must
//! never move a document's verdict.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::{
    CT_CHART, CT_COMMENTS, CT_FOOTER, CT_HEADER, CT_MACRO_MAIN, CT_NUMBERING, CT_SETTINGS, CT_THEME, Doc, content_types_xml, document_xml,
};
use wordcraft_validate::{Evidence, Finding, Report, Severity, Verdict, validate_bytes};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const WP_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const B_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
const M_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

fn validate(doc: &Doc) -> Report {
    validate_bytes(&doc.build())
}

fn rule_findings<'r>(report: &'r Report, rule: &str) -> Vec<&'r Finding> {
    report.findings.iter().filter(|f| f.rule == rule).collect()
}

fn grammar_findings(report: &Report) -> Vec<&Finding> {
    report.findings.iter().filter(|f| f.rule.starts_with("GRAMMAR-")).collect()
}

fn chart_doc(plot_area: &str) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/charts/chart1.xml",
        CT_CHART,
        format!(
            r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea>{plot_area}</c:plotArea></c:chart></c:chartSpace>"#
        ),
    );
    d
}

fn theme_doc(major_font: &str) -> Doc {
    let colors: String = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"]
        .iter()
        .map(|n| format!(r#"<a:{n}><a:srgbClr val="000000"/></a:{n}>"#))
        .collect();
    let fills = "<a:noFill/>".repeat(3);
    let lines = "<a:ln/>".repeat(3);
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/theme/theme1.xml",
        CT_THEME,
        format!(
            r#"<?xml version="1.0"?><a:theme xmlns:a="{A_NS}" name="Test"><a:themeElements><a:clrScheme name="Test">{colors}</a:clrScheme><a:fontScheme name="Test"><a:majorFont>{major_font}</a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Test"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{lines}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
        ),
    );
    d
}

fn math_doc(body: &str) -> Doc {
    let mut d = Doc::minimal();
    d.set("word/document.xml", format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:m="{M_NS}"><w:body>{body}</w:body></w:document>"#));
    d
}

fn numfmt_doc(val: &str) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="{val}"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d
}

#[test]
fn grammar_required_child_fires_as_warning_with_location_and_valid_twin_clean() {
    // Table missing w:tblPr.
    let mut d = Doc::minimal();
    d.set_document("<w:tbl><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].evidence, Evidence::Observed);
    assert_eq!(hits[0].part.as_deref(), Some("word/document.xml"));
    assert!(hits[0].message.contains("w:tblPr") && hits[0].message.contains("at w:document/w:body[1]/w:tbl[1]"), "{:?}", hits[0]);
    d.set_document("<w:tbl><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let clean = validate(&d);
    assert_eq!(clean.verdict, Verdict::Pass);
    assert!(grammar_findings(&clean).is_empty(), "{:?}", clean.findings);

    // Chart plotArea missing a chart-type choice member.
    let report = validate(&chart_doc("<c:layout/>"));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].part.as_deref(), Some("word/charts/chart1.xml"));
    assert!(hits[0].message.contains("c:barChart") && hits[0].message.contains("at c:chartSpace/c:chart[1]/c:plotArea[1]"), "{:?}", hits[0]);
    let twin = validate(&chart_doc("<c:barChart><c:barDir/><c:axId/><c:axId/></c:barChart>"));
    assert!(grammar_findings(&twin).is_empty(), "{:?}", twin.findings);

    // Theme font collection missing a:ea and a:cs.
    let report = validate(&theme_doc(r#"<a:latin typeface="Calibri Light"/>"#));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|f| f.severity == Severity::Warning && f.part.as_deref() == Some("word/theme/theme1.xml")));
    assert!(hits.iter().any(|f| f.message.contains("a:ea")));
    assert!(hits.iter().any(|f| f.message.contains("a:cs")));
    let twin = validate(&theme_doc(r#"<a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/>"#));
    assert!(grammar_findings(&twin).is_empty(), "{:?}", twin.findings);

    // Math fraction missing m:num, and n-ary missing m:e.
    let report = validate(&math_doc("<w:p><m:oMath><m:f><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></w:p>"));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("m:num") && hits[0].message.contains("m:f[1]"), "{:?}", hits[0]);
    let twin =
        validate(&math_doc("<w:p><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></w:p>"));
    assert!(grammar_findings(&twin).is_empty(), "{:?}", twin.findings);

    let report = validate(&math_doc(
        "<w:p><m:oMath><m:nary><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup></m:nary></m:oMath></w:p>",
    ));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("m:e"), "{:?}", hits[0]);
}

#[test]
fn grammar_required_child_nullable_choices_group_branches_veto_and_any() {
    // c:dLbls: show-flags, bare c:delete, and empty are all XSD-valid.
    let ser = |dlbls: &str| {
        format!(
            "<c:barChart><c:barDir/><c:ser><c:idx c:val=\"0\"/><c:order c:val=\"0\"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount c:val=\"1\"/><c:pt idx=\"0\"><c:v>S</c:v></c:pt></c:strCache></c:strRef></c:tx>{dlbls}<c:cat><c:strRef><c:f>Sheet1!$A$2</c:f><c:strCache><c:ptCount c:val=\"1\"/><c:pt idx=\"0\"><c:v>A</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2</c:f><c:numCache><c:ptCount c:val=\"1\"/><c:pt idx=\"0\"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId/><c:axId/></c:barChart>"
        )
    };
    for dlbls in ["<c:dLbls><c:showLegendKey/><c:showVal/><c:showCatName/></c:dLbls>", "<c:dLbls><c:delete/></c:dLbls>", "<c:dLbls/>"] {
        let report = validate(&chart_doc(&ser(dlbls)));
        assert!(grammar_findings(&report).is_empty(), "{dlbls}: {:?}", grammar_findings(&report));
    }

    // w:ffData: nullable choice; w:checkBox still requires size or sizeAuto.
    let ff_doc = |inner: &str| {
        let mut d = Doc::minimal();
        d.set_document(&format!(r#"<w:p><w:r><w:fldChar w:fldCharType="begin"><w:ffData>{inner}</w:ffData></w:fldChar></w:r></w:p>"#));
        d
    };
    for inner in ["", "<w:name w:val=\"example\"/>", "<w:textInput/>", "<w:label w:val=\"1\"/>"] {
        let report = validate(&ff_doc(inner));
        assert!(grammar_findings(&report).is_empty(), "{inner:?}: {:?}", grammar_findings(&report));
    }
    let report = validate(&ff_doc("<w:checkBox/>"));
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("w:checkBox") && hits[0].message.contains("w:size"), "{:?}", hits[0]);

    // Transitional veto on w:CT_NumPicBullet: w:pict is vetoed, empty still fires.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:numPicBullet><w:pict/></w:numPicBullet></w:numbering>"#),
    );
    assert!(grammar_findings(&validate(&d)).is_empty());
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:numPicBullet/></w:numbering>"#),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("w:pict") && hits[0].message.contains("w:drawing"), "{:?}", hits[0]);

    // mc:AlternateContent direct child prevents false required-child firing on parent.
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:numPicBullet><mc:AlternateContent><mc:Choice Requires="w"><w:drawing/></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent></w:numPicBullet></w:numbering>"#
        ),
    );
    assert!(grammar_findings(&validate(&d)).is_empty());

    // <any> slot on c:ext: foreign payload satisfies, empty fires.
    let ext_chart = |ext: &str| {
        let mut d = Doc::minimal();
        d.add_with_override(
            "word/charts/chart1.xml",
            CT_CHART,
            format!(
                r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:x="urn:example:x"><c:chart><c:plotArea><c:barChart><c:barDir/><c:axId/><c:axId/></c:barChart></c:plotArea></c:chart><c:extLst>{ext}</c:extLst></c:chartSpace>"#
            ),
        );
        d
    };
    assert!(grammar_findings(&validate(&ext_chart("<c:ext><x:payload/></c:ext>"))).is_empty());
    let report = validate(&ext_chart("<c:ext/>"));
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("no child elements"), "{:?}", hits[0]);

    // Closed element's required-child finding survives later truncation.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body><w:tbl><w:tblGrid/></w:tbl><w:p><w:r><w:t>truncated"#),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert!(rule_findings(&report, "GRAMMAR-REQUIRED-CHILD").iter().any(|f| f.message.contains("w:tblPr")));
}

#[test]
fn grammar_child_not_allowed_fires_as_warning_with_bare_groups_veto_fallback_and_ordinals() {
    // Disallowed w:sectPr inside w:hdr (bare-group type CT_HdrFtr), allowed in w:body.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/header1.xml", CT_HEADER)]));
    d.set("word/header1.xml", format!(r#"<w:hdr xmlns:w="{W_NS}"><w:p><w:r><w:t>ok</w:t></w:r></w:p><w:sectPr/></w:hdr>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let hits = rule_findings(&report, "GRAMMAR-CHILD-NOT-ALLOWED");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].evidence, Evidence::MsDocumented);
    assert_eq!(hits[0].part.as_deref(), Some("word/header1.xml"));
    assert!(hits[0].message.contains("w:sectPr") && hits[0].message.contains("at w:hdr/w:sectPr[1]"), "{:?}", hits[0]);

    // Disallowed w:r in second paragraph's w:pPr carries 1-based sibling ordinal w:p[2].
    let mut d = Doc::minimal();
    d.set_document("<w:p><w:r><w:t>first</w:t></w:r></w:p><w:p><w:pPr><w:r/></w:pPr></w:p>");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-NOT-ALLOWED");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("at w:document/w:body[1]/w:p[2]/w:pPr[1]/w:r[1]"), "{:?}", hits[0]);

    // Transitional veto: w:left/w:right in w:tcBorders are clean, as is bare-group w:hyperlink.
    d.set_document(
        r#"<w:tbl><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:tcPr><w:tcBorders><w:left w:val="single"/><w:right w:val="single"/></w:tcBorders></w:tcPr><w:p><w:hyperlink><w:r><w:t>ok</w:t></w:r></w:hyperlink></w:p></w:tc></w:tr></w:tbl>"#,
    );
    assert!(grammar_findings(&validate(&d)).is_empty());

    // Unmodelled MS extensions (w:uiCompat97to2003) are skipped.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/settings.xml",
        CT_SETTINGS,
        format!(r#"<w:settings xmlns:w="{W_NS}"><w:compat><w:uiCompat97to2003/></w:compat></w:settings>"#),
    );
    assert!(grammar_findings(&validate(&d)).is_empty());

    // Fallback union membership: misplaced w:sdtContent in w:body falls back; w:p allowed, w:sectPr flagged.
    let mut d = Doc::minimal();
    d.set_document("<w:sdtContent><w:p/><w:p/><w:sectPr/></w:sdtContent>");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-NOT-ALLOWED");
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY").is_empty());
    assert!(report.coverage_notes.iter().any(|c| c.contains("could not be selected by parent context")));
}

#[test]
fn grammar_child_too_many_fires_once_at_crossing_as_warning() {
    // Max 1: second w:tblPr fires once at w:tblPr[2]; third stays silent.
    let mut d = Doc::minimal();
    d.set_document("<w:tbl><w:tblPr/><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].evidence, Evidence::MsDocumented);
    assert!(
        hits[0].message.contains("at w:document/w:body[1]/w:tbl[1]/w:tblPr[2]") && hits[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        hits[0]
    );
    assert!(!hits[0].message.contains("has 2 w:tblPr"), "{:?}", hits[0]);

    // Max 2: c:barChart allows at most 2 c:axId; a 3rd fires once at c:axId[3].
    let report = validate(&chart_doc("<c:barChart><c:barDir/><c:axId/><c:axId/><c:axId/></c:barChart>"));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("c:axId[3]") && hits[0].message.contains("at most 2"), "{:?}", hits[0]);

    // Theme font collection: second a:latin fires once.
    let report = validate(&theme_doc(r#"<a:latin typeface="a"/><a:latin typeface="b"/><a:ea typeface=""/><a:cs typeface=""/>"#));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("a:latin[2]"), "{:?}", hits[0]);
}

#[test]
fn grammar_order_fires_once_per_rpr_as_warning_and_ignores_ppr() {
    // Run w:rPr inversion: w:sz before w:rFonts fires with exact occurrence path.
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:rPr><w:sz w:val="24"/><w:rFonts w:ascii="Aptos"/></w:rPr><w:t>x</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-ORDER");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].evidence, Evidence::Observed);
    assert!(hits[0].message.contains("w:rFonts") && hits[0].message.contains("w:rPr[1]/w:rFonts[1]"), "{:?}", hits[0]);

    // Multiple inversions in one w:rPr still fire only once, at the first inversion.
    d.set_document(
        r#"<w:p><w:r><w:rPr><w:sz w:val="24"/><w:color w:val="FF0000"/><w:rFonts w:ascii="Aptos"/><w:b/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-ORDER");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("w:color") && hits[0].message.contains("w:rPr[1]/w:color[1]"), "{:?}", hits[0]);

    // Schema order stays silent.
    d.set_document(
        r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Aptos"/><w:b/><w:color w:val="FF0000"/><w:sz w:val="24"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
    );
    assert!(rule_findings(&validate(&d), "GRAMMAR-ORDER").is_empty());

    // Paragraph-mark w:rPr inside w:pPr is in scope too.
    d.set_document(r#"<w:p><w:pPr><w:rPr><w:sz w:val="24"/><w:rFonts w:ascii="Aptos"/></w:rPr></w:pPr></w:p>"#);
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-ORDER");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("w:pPr[1]/w:rPr[1]/w:rFonts[1]"), "{:?}", hits[0]);

    // Inverted w:pPr is intentionally out of scope.
    d.set_document(r#"<w:p><w:pPr><w:jc w:val="center"/><w:spacing w:after="200"/></w:pPr></w:p>"#);
    assert!(rule_findings(&validate(&d), "GRAMMAR-ORDER").is_empty());
}

#[test]
fn grammar_numfmt_value_fires_as_warning_and_accepts_full_enumeration() {
    let report = validate(&numfmt_doc("banana"));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(hits[0].evidence, Evidence::MsDocumented);
    assert_eq!(hits[0].part.as_deref(), Some("word/numbering.xml"));
    assert!(hits[0].message.contains("at w:numbering/w:abstractNum[1]/w:lvl[1]/w:numFmt[1]"), "{:?}", hits[0]);

    // Case-sensitive.
    assert_eq!(rule_findings(&validate(&numfmt_doc("Decimal")), "GRAMMAR-NUMFMT-VALUE").len(), 1);

    // Wholesale enumeration including i18n formats and custom.
    for ok in ["decimal", "custom", "hebrew1", "arabicAlpha", "hindiNumbers", "thaiCounting", "bahtText"] {
        assert!(rule_findings(&validate(&numfmt_doc(ok)), "GRAMMAR-NUMFMT-VALUE").is_empty(), "{ok}");
    }

    // Reaches w:numFmt under w:footnotePr in settings.xml; missing w:val stays silent.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/settings.xml",
        CT_SETTINGS,
        format!(
            r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:footnotePr><w:numFmt w:val="banana"/><w:numFmt/></w:footnotePr></w:settings>"#
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].part.as_deref(), Some("word/settings.xml"));
}

#[test]
fn grammar_only_violations_preserve_pass_verdict_across_all_five_rules() {
    let mut d = Doc::minimal();
    d.set_document(concat!(
        r#"<w:tbl><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:pPr><w:r/></w:pPr><w:r><w:rPr><w:sz w:val="24"/><w:rFonts w:ascii="Aptos"/></w:rPr><w:t>hi</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
    ));
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="notAFormat"/></w:lvl></w:abstractNum></w:numbering>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "grammar warnings must never move verdict: {report:?}");
    for rule in ["GRAMMAR-REQUIRED-CHILD", "GRAMMAR-CHILD-NOT-ALLOWED", "GRAMMAR-CHILD-TOO-MANY", "GRAMMAR-ORDER", "GRAMMAR-NUMFMT-VALUE"] {
        let hits = rule_findings(&report, rule);
        assert!(!hits.is_empty(), "expected {rule} to fire: {:?}", report.findings);
        assert!(hits.iter().all(|f| f.severity == Severity::Warning), "{hits:?}");
    }
}

#[test]
fn grammar_over_32_unmodelled_children_does_not_produce_incomplete() {
    // 40 distinct foreign-namespace children under w:p AND under w:tbl (with required children after):
    // must stay Verdict::Pass, never Verdict::Incomplete, and still observe modelled children after the cap.
    let ext: String = (0..40).map(|i| format!("<x:e{i}/>")).collect();
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:x="urn:example:x"><w:body><w:p>{ext}</w:p><w:tbl>{ext}<w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}, notes: {:?}", report.findings, report.coverage_notes);
    assert!(grammar_findings(&report).is_empty(), "{:?}", report.findings);
}

#[test]
fn grammar_root_is_excluded_and_preexisting_output_shape_is_unchanged() {
    // Wrong root on a header part does not emit GRAMMAR-ROOT, and GRAMMAR-ROOT is not registered.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/header1.xml", CT_HEADER)]));
    d.set("word/header1.xml", format!(r#"<w:document xmlns:w="{W_NS}"><w:body><w:p/></w:body></w:document>"#));
    let report = validate(&d);
    assert!(!report.checks_run.contains(&"GRAMMAR-ROOT"));
    assert!(rule_findings(&report, "GRAMMAR-ROOT").is_empty());

    // Pre-existing XML-MALFORMED finding keeps its exact 5-field JSON shape and message without location fields.
    let mut d = Doc::minimal();
    d.set("word/document.xml", format!(r#"<w:document xmlns:w="{W_NS}"><w:body><w:p>"#));
    let report = validate(&d);
    let malformed = report.findings.iter().find(|f| f.rule == "XML-MALFORMED").expect("XML-MALFORMED");
    let json = serde_json::to_value(malformed).unwrap();
    let obj = json.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["evidence", "message", "part", "rule", "severity"]);
    assert_eq!(malformed.message, "word/document.xml is not well-formed XML: unclosed element at end of part");
}

#[test]
fn grammar_runs_on_macro_enabled_main_part_and_conflicting_scope_is_order_independent() {
    // Macro-enabled .docm main part gets Wml grammar scope.
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        String::from_utf8(d.get("[Content_Types].xml").to_vec())
            .unwrap()
            .replace("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", CT_MACRO_MAIN),
    );
    d.set("word/document.xml", document_xml("<w:p><w:pPr><w:r/></w:pPr></w:p>"));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_eq!(rule_findings(&report, "GRAMMAR-CHILD-NOT-ALLOWED").len(), 1);

    // Conflicting scopes skip grammar cleanly in both entry orders and in BOTH directions
    // (chart content type + header rel, and header content type + chart rel), so neither a
    // "content type wins" nor a "relationship wins" classifier can pass.
    for (ct, rel_kind) in [(CT_CHART, "header"), (CT_HEADER, "chart")] {
        let mut reports = Vec::new();
        for reverse in [false, true] {
            let mut d = Doc::minimal();
            d.add_with_override("word/weird.xml", ct, format!(r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}"><w:sectPr/></w:hdr>"#));
            d.set_doc_rels(&format!(
                r#"<Relationship Id="rW" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{rel_kind}" Target="weird.xml"/>"#
            ));
            if reverse {
                d.entries.reverse();
            }
            reports.push(validate(&d));
        }
        assert_eq!(grammar_findings(&reports[0]).len(), 0, "ct={ct} rel={rel_kind}: {:?}", reports[0].findings);
        assert_eq!(serde_json::to_value(&reports[0]).unwrap(), serde_json::to_value(&reports[1]).unwrap());
    }
}

#[test]
fn grammar_verdict_invariance_ab_probe_default_limits_large_ordinary_notes() {
    // Finding 1 A/B probe at default limits (notes = 10,000):
    // 33 distinct foreign children under one main-document paragraph plus 10,000 unique
    // unreferenced comments in the linked comments part must keep Verdict::Pass with
    // zero findings and identical ordinary coverage notes compared to the package without
    // the foreign children.
    let comments_inner: String = (0..10_000).map(|i| format!(r#"<w:comment w:id="{i}"><w:p/></w:comment>"#)).collect();
    let comments_xml = format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}">{comments_inner}</w:comments>"#);
    let build = |with_foreign: bool| {
        let mut d = Doc::minimal();
        let body = if with_foreign {
            let ext: String = (0..33).map(|i| format!("<x:e{i}/>")).collect();
            format!("<w:p>{ext}</w:p>")
        } else {
            String::from("<w:p/>")
        };
        d.set(
            "word/document.xml",
            format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:x="urn:example:x"><w:body>{body}</w:body></w:document>"#),
        );
        d.add_with_override("word/comments.xml", CT_COMMENTS, comments_xml.clone());
        d.set_doc_rels(
            r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
        );
        d
    };
    let with_foreign = validate(&build(true));
    let baseline = validate(&build(false));
    assert_eq!(baseline.verdict, Verdict::Pass, "{:?}", baseline.findings);
    assert_eq!(with_foreign.verdict, Verdict::Pass, "grammar note must never flip Pass to Incomplete: {:?}", with_foreign.findings);
    assert!(baseline.findings.is_empty(), "{:?}", baseline.findings);
    assert!(with_foreign.findings.is_empty(), "{:?}", with_foreign.findings);
    assert_eq!(with_foreign.coverage_notes.len(), 10_000);
    assert_eq!(with_foreign.coverage_notes, baseline.coverage_notes);
}

#[test]
fn grammar_required_child_suppressed_by_dtd_taint_and_external_entity() {
    // Internal-subset DOCTYPE taints the entire part for grammar: no table in the part emits
    // GRAMMAR-REQUIRED-CHILD (neither the entity-bearing table nor other tables in the tainted part),
    // and the DTD taint is disclosed once on the grammar note channel.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY tblpr "<w:tblPr/>">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl>&tblpr;<w:tblGrid/><w:tr><w:tc><w:tbl><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"<w:tbl><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert!(rule_findings(&report, "GRAMMAR-REQUIRED-CHILD").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n == "DTD present in word/document.xml; grammar checks are incomplete here"),
        "{:?}",
        report.coverage_notes
    );

    // Even a plain-text internal entity carries an internal subset, so the coarse DTD-taint gate
    // suppresses GRAMMAR-REQUIRED-CHILD in that part and discloses the taint.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY txt "hello">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl>&txt;<w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert!(rule_findings(&report, "GRAMMAR-REQUIRED-CHILD").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n == "DTD present in word/document.xml; grammar checks are incomplete here"),
        "{:?}",
        report.coverage_notes
    );

    // External-only DOCTYPE (no internal subset) is not DTD-tainted: &ext; suppresses required-child
    // on the affected table via mark_child_set_incomplete, while the unaffected second table still fires.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document SYSTEM "ext.dtd">"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl>&ext;<w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"<w:tbl><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("at w:document/w:body[1]/w:tbl[2] "), "{hits:?}");
}

#[test]
fn grammar_ordinals_stay_exact_for_modelled_names_and_uncertain_past_unmodelled_cap() {
    // Finding 3 (exact probe): after 1,030 distinct foreign children under w:p,
    // two w:pPr elements must report the offending second instance w:pPr[2], never w:pPr[1].
    let foreign: String = (0..1030).map(|i| format!("<x:e{i}/>")).collect();
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:x="urn:example:x"><w:body><w:p>{foreign}<w:pPr/><w:pPr/></w:p></w:body></w:document>"#),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("at w:document/w:body[1]/w:p[1]/w:pPr[2] "), "{:?}", hits[0]);
    assert!(!hits[0].message.contains("w:pPr[1]"), "{:?}", hits[0]);

    // Class generalization: an unmodelled ancestor arriving past the 1,024-name cap must render [?],
    // never fabricate [1], while modelled descendants inside it keep exact ordinals.
    let foreign_body: String = (0..1024).map(|i| format!("<x:b{i}/>")).collect();
    let wrapped_tbl = "<x:overflow><w:tbl><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></x:overflow>";
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:x="urn:example:x"><w:body>{foreign_body}{wrapped_tbl}{wrapped_tbl}</w:body></w:document>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 2, "{hits:?}");
    for hit in &hits {
        assert!(hit.message.contains("at w:document/w:body[1]/{urn:example:x}overflow[?]/w:tbl[1]/w:tblPr[2] "), "{hit:?}");
        assert!(!hit.message.contains("overflow[1]"), "{hit:?}");
    }
}

#[test]
fn grammar_numfmt_delivers_internal_entities_and_distinguishes_missing_vs_unreadable() {
    // Finding 4 (exact probe): internal plain-text entity delivering invalid format fires GRAMMAR-NUMFMT-VALUE.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY bad "notAFormat"><!ENTITY ok "decimal"><!ENTITY part "dec">]>"#,
                r#"<w:numbering xmlns:w="{w}">"#,
                r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt w:val="&bad;"/>"#,
                r#"<w:numFmt w:val="&ok;"/>"#,
                r#"<w:numFmt w:val="&part;imal"/>"#,
                r#"<w:numFmt w:val="&part;imal&#9;"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|f| f.severity == Severity::Warning));
    assert!(hits.iter().any(|f| f.message.contains("\"notAFormat\"") && f.message.contains("w:numFmt[>=1]")), "{hits:?}");
    assert!(hits.iter().any(|f| f.message.contains("\"decimal\t\"") && f.message.contains("w:numFmt[>=4]")), "{hits:?}");
    assert!(
        report.coverage_notes.iter().any(|n| n == "DTD present in word/numbering.xml; grammar checks are incomplete here"),
        "{:?}",
        report.coverage_notes
    );

    // Unreadable entity-bearing attribute (nested entity): no false NUMFMT warning, but disclosed via grammar note.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY inner "notAFormat"><!ENTITY outer "&inner;">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt w:val="&outer;"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert!(rule_findings(&report, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val") && n.contains("could not be delivered")),
        "{:?}",
        report.coverage_notes
    );

    // Unresolved namespace prefix on val attribute: disclosed as uncertain attribute identity,
    // never claiming w:numFmt/@w:val exists or that its value could not be delivered.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY wns "{w}">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt xmlns:u="&wns;" u:val="notAFormat"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert!(rule_findings(&report, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("unresolved namespace prefix") && n.contains("whether it is w:val could not be established")),
        "{:?}",
        report.coverage_notes
    );
    assert!(
        !report.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val")),
        "unresolved prefix must not claim w:numFmt/@w:val exists: {:?}",
        report.coverage_notes
    );

    // Control: genuinely missing w:val stays silent with no unreadable-attribute note.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt/></w:lvl></w:abstractNum></w:numbering>"#),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert!(rule_findings(&report, "GRAMMAR-NUMFMT-VALUE").is_empty());
    assert!(!report.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val")));
}

#[test]
fn grammar_required_child_shortfall_reports_observed_counts_truthfully() {
    // Finding 6 (exact probe): a:fillStyleLst requires 3 fill choices; 2 a:noFill must report
    // 2 observed and short by 1 (never "has none of"). Similarly a:lnStyleLst with 2 a:ln (named child).
    let colors: String = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"]
        .iter()
        .map(|n| format!(r#"<a:{n}><a:srgbClr val="000000"/></a:{n}>"#))
        .collect();
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/theme/theme1.xml",
        CT_THEME,
        format!(
            r#"<?xml version="1.0"?><a:theme xmlns:a="{A_NS}" name="Test"><a:themeElements><a:clrScheme name="Test">{colors}</a:clrScheme><a:fontScheme name="Test"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Test"><a:fillStyleLst><a:noFill/><a:noFill/></a:fillStyleLst><a:lnStyleLst><a:ln/><a:ln/></a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst><a:noFill/><a:noFill/><a:noFill/></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 2, "{hits:?}");
    let fill = hits.iter().find(|f| f.message.contains("a:fillStyleLst")).expect("fillStyleLst shortfall");
    assert!(fill.message.contains("has 2 of ") && fill.message.contains("(short by 1)") && fill.message.contains("at least 3 of them"), "{fill:?}");
    assert!(!fill.message.contains("has none of"), "{fill:?}");
    let line = hits.iter().find(|f| f.message.contains("a:lnStyleLst")).expect("lnStyleLst shortfall");
    assert!(line.message.contains("has 2 a:ln elements (short by 1)") && line.message.contains("at least 3"), "{line:?}");
    assert!(!line.message.contains("has no a:ln"), "{line:?}");

    // Chart named-child shortfall: c:barChart requires 2 c:axId; 1 observed reports 1 (short by 1).
    let report = validate(&chart_doc("<c:barChart><c:barDir/><c:axId/></c:barChart>"));
    assert_eq!(report.verdict, Verdict::Pass);
    let hits = rule_findings(&report, "GRAMMAR-REQUIRED-CHILD");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].message.contains("has 1 c:axId element (short by 1)") && hits[0].message.contains("at least 2"), "{:?}", hits[0]);
    assert!(!hits[0].message.contains("has no c:axId"), "{:?}", hits[0]);
}

#[test]
fn grammar_footer_deep_nested_tables_and_numfmt_boundary_spellings() {
    // Regression surface for positive generalization probes:
    // Footer with two levels of nested tables: single w:tblPr stay clean, second w:pPr in second
    // nested table fires TOO-MANY once, and rPr inversion in first nested table fires ORDER at deep path.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/footer1.xml", CT_FOOTER)]));
    d.set(
        "word/footer1.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?><w:ftr xmlns:w="{w}">"#,
                r#"<w:tbl><w:tblPr/><w:tblGrid/><w:tr><w:tc>"#,
                r#"<w:tbl><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p><w:r><w:rPr><w:sz w:val="20"/><w:rFonts w:ascii="Calibri"/></w:rPr><w:t>a</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
                r#"<w:tbl><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p><w:pPr/><w:pPr/></w:p></w:tc></w:tr></w:tbl>"#,
                r#"<w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let too_many = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(too_many.len(), 1, "{too_many:?}");
    assert_eq!(too_many[0].part.as_deref(), Some("word/footer1.xml"));
    assert!(too_many[0].message.contains("at w:ftr/w:tbl[1]/w:tr[1]/w:tc[1]/w:tbl[2]/w:tr[1]/w:tc[1]/w:p[1]/w:pPr[2] "), "{:?}", too_many[0]);
    let order = rule_findings(&report, "GRAMMAR-ORDER");
    assert_eq!(order.len(), 1, "{order:?}");
    assert_eq!(order[0].part.as_deref(), Some("word/footer1.xml"));
    assert!(
        order[0].message.contains("at w:ftr/w:tbl[1]/w:tr[1]/w:tc[1]/w:tbl[1]/w:tr[1]/w:tc[1]/w:p[1]/w:r[1]/w:rPr[1]/w:rFonts[1] "),
        "{:?}",
        order[0]
    );
    assert!(rule_findings(&report, "GRAMMAR-REQUIRED-CHILD").is_empty());

    // Footer numFmt boundary spellings: first/last codes and numeric-reference decimal clean;
    // empty, case-changed, and trailing-space each fire GRAMMAR-NUMFMT-VALUE.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/footer1.xml", CT_FOOTER)]));
    d.set(
        "word/footer1.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?><w:ftr xmlns:w="{w}"><w:p>"#,
                r#"<w:numFmt w:val="decimal"/><w:numFmt w:val="custom"/><w:numFmt w:val="&#100;ecimal"/>"#,
                r#"<w:numFmt w:val=""/><w:numFmt w:val="Decimal"/><w:numFmt w:val="decimal "/>"#,
                r#"</w:p></w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits.iter().any(|f| f.message.contains("numFmt format \"\"") && f.message.contains("w:numFmt[4]")), "{hits:?}");
    assert!(hits.iter().any(|f| f.message.contains("numFmt format \"Decimal\"") && f.message.contains("w:numFmt[5]")), "{hits:?}");
    assert!(hits.iter().any(|f| f.message.contains("numFmt format \"decimal \"") && f.message.contains("w:numFmt[6]")), "{hits:?}");
}

#[test]
fn preexisting_mc_ignorable_delivery_is_unchanged_by_grammar_attribute_bound() {
    // Round-2 Finding 1 regression: an entity reference followed by 8,200 spaces in mc:Ignorable
    // must still be delivered to check_prefixes and emit the exact pre-port XML-UNDECLARED-PREFIX
    // finding (never starved into "prefixes were not checked").
    let spaces = " ".repeat(8200);
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY props "lostprefix ">]>"#,
                r#"<w:document xmlns:w="{w}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="&props;{spaces}">"#,
                r#"<w:body><w:p/></w:body></w:document>"#,
            ),
            w = W_NS,
            spaces = spaces,
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    let undeclared = rule_findings(&report, "XML-UNDECLARED-PREFIX");
    assert_eq!(undeclared.len(), 1, "mc:Ignorable finding must not disappear: {:?}", report.findings);
    assert_eq!(undeclared[0].severity, Severity::Corrupt);
    assert_eq!(undeclared[0].evidence, Evidence::ByConstruction);
    assert_eq!(undeclared[0].part.as_deref(), Some("word/document.xml"));
    assert_eq!(undeclared[0].message, "the namespace prefix `lostprefix` used in word/document.xml is not declared");
    assert_eq!(
        serde_json::to_string(undeclared[0]).unwrap(),
        r#"{"rule":"XML-UNDECLARED-PREFIX","severity":"Corrupt","evidence":"ByConstruction","part":"word/document.xml","message":"the namespace prefix `lostprefix` used in word/document.xml is not declared"}"#
    );
    // Ordinary coverage notes remain empty (no mc:Ignorable unread note); only the grammar DTD-taint note is recorded.
    let ordinary_notes: Vec<_> = report.coverage_notes.iter().filter(|n| !n.starts_with("DTD present in ")).collect();
    assert!(ordinary_notes.is_empty(), "pre-existing mc:Ignorable consumer must not emit unread note: {:?}", report.coverage_notes);
    assert_eq!(report.coverage_notes, vec!["DTD present in word/document.xml; grammar checks are incomplete here".to_string()]);

    // Grammar-side bound still applies to w:numFmt/@w:val without touching ordinary consumers or verdict.
    let mut d2 = Doc::minimal();
    d2.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY bad "notAFormat">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt w:val="&bad;{spaces}"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
            spaces = spaces,
        ),
    );
    let report2 = validate(&d2);
    assert!(rule_findings(&report2, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report2.findings);
    assert!(
        report2.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val") && n.contains("could not be delivered")),
        "{:?}",
        report2.coverage_notes
    );
}

#[test]
fn grammar_paths_and_counts_use_lower_bounds_after_loss_and_mark_unknown_namespaces() {
    // Round-2 Finding 2(a) exact probe: entity expanding to <w:tblPr/> before two literal <w:tblPr/>
    // children must report lower-bound path w:tblPr[>=2] and count "has at least 2", never exact w:tblPr[2] / "has 2".
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY one "<w:tblPr/>">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl>&one;<w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].message.contains("at w:document/w:body[>=1]/w:tbl[>=1]/w:tblPr[>=2] ") && hits[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        hits[0]
    );
    assert!(!hits[0].message.contains("w:tblPr[2]"), "{:?}", hits[0]);
    assert!(!hits[0].message.contains("has 2 w:tblPr"), "{:?}", hits[0]);

    // Class generalization: skipped mc:AlternateContent before two literal <w:tblPr/> children (no DTD).
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<w:document xmlns:w="{w}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body>"#,
                r#"<w:tbl><mc:AlternateContent><mc:Choice Requires="w"><w:tblPr/></mc:Choice><mc:Fallback><w:tblPr/></mc:Fallback></mc:AlternateContent>"#,
                r#"<w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].message.contains("at w:document/w:body[1]/w:tbl[1]/w:tblPr[>=2] ") && hits[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        hits[0]
    );

    // Class generalization: unresolved-namespace sibling with matching local name (<u:tblPr>)
    // turns subsequent w:tblPr ordinals and counts into lower bounds; under an external-only DOCTYPE
    // (not DTD-tainted), unrelated local names stay exact, while under an internal-subset DOCTYPE
    // all ordinals and counts become lower bounds.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document SYSTEM "ext.dtd">"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl><u:tblPr xmlns:u="&wns;"/><w:tblPr/><w:tblPr/><w:tblGrid/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 2, "{hits:?}");
    let tbl_pr = hits.iter().find(|f| f.message.contains("w:tblPr")).expect("tblPr too-many");
    assert!(
        tbl_pr.message.contains("at w:document/w:body[1]/w:tbl[1]/w:tblPr[>=2] ") && tbl_pr.message.contains("has at least 2 w:tblPr elements"),
        "{tbl_pr:?}"
    );
    let tbl_grid = hits.iter().find(|f| f.message.contains("w:tblGrid")).expect("tblGrid too-many");
    assert!(
        tbl_grid.message.contains("at w:document/w:body[1]/w:tbl[1]/w:tblGrid[2] ") && tbl_grid.message.contains("has at least 2 w:tblGrid elements"),
        "{tbl_grid:?}"
    );

    // Same document with an internal subset: DTD taint makes every ordinal and count a lower bound.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY wns "{w}">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<w:tbl><u:tblPr xmlns:u="&wns;"/><w:tblPr/><w:tblPr/><w:tblGrid/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 2, "{hits:?}");
    let tbl_grid = hits.iter().find(|f| f.message.contains("w:tblGrid")).expect("tblGrid too-many");
    assert!(
        tbl_grid.message.contains("at w:document/w:body[>=1]/w:tbl[>=1]/w:tblGrid[>=2] ")
            && tbl_grid.message.contains("has at least 2 w:tblGrid elements"),
        "{tbl_grid:?}"
    );

    // Class generalization: unexpanded entity inside an UNMODELLED wrapper in a tainted part
    // keeps lower-bound ordinals and counts throughout the tainted part.
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY extra "<w:tbl><w:tblPr/><w:tblGrid/></w:tbl>">]>"#,
                r#"<w:document xmlns:w="{w}" xmlns:x="urn:foreign"><w:body>"#,
                r#"<x:wrapper>&extra;<w:tbl><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></x:wrapper>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].message.contains("at w:document/w:body[>=1]/{urn:foreign}wrapper[>=1]/w:tbl[>=1]/w:tblPr[>=2] ")
            && hits[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        hits[0]
    );

    // Round-2 Finding 2(b) exact probe: unresolved-namespace ancestor renders {?}wrapper[?],
    // never bare wrapper[1] (which would falsely assert no namespace and an exact ordinal).
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY ns "urn:foreign">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body>"#,
                r#"<u:wrapper xmlns:u="&ns;"><w:numFmt w:val="bad"/></u:wrapper>"#,
                r#"<wrapper xmlns="&ns;"><w:numFmt w:val="bad2"/></wrapper>"#,
                r#"<wrapper><w:numFmt w:val="bad3"/></wrapper>"#,
                r#"</w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(hits.len(), 3, "{hits:?}");
    let prefixed = hits.iter().find(|f| f.message.contains("\"bad\"")).expect("prefixed unresolved ancestor");
    assert!(prefixed.message.contains("at w:document/w:body[>=1]/{?}wrapper[?]/w:numFmt[>=1] "), "{prefixed:?}");
    assert!(!prefixed.message.contains("/wrapper["), "{prefixed:?}");
    let default_ns = hits.iter().find(|f| f.message.contains("\"bad2\"")).expect("default unresolved ancestor");
    assert!(default_ns.message.contains("at w:document/w:body[>=1]/{?}wrapper[?]/w:numFmt[>=1] "), "{default_ns:?}");
    let plain = hits.iter().find(|f| f.message.contains("\"bad3\"")).expect("true no-namespace ancestor");
    assert!(plain.message.contains("at w:document/w:body[>=1]/wrapper[>=1]/w:numFmt[>=1] "), "{plain:?}");
}

#[test]
fn grammar_numfmt_distinguishes_unresolved_attribute_identity_from_unreadable_value() {
    // Round-2 Finding 3 exact probe: foreign-namespace u:val via unexpanded entity must NOT claim
    // "has a w:numFmt/@w:val attribute" or "whose value could not be delivered".
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY ns "urn:foreign">]>"#,
                r#"<w:document xmlns:w="{w}"><w:body><w:p>"#,
                r#"<w:numFmt xmlns:u="&ns;" u:val="bad"/>"#,
                r#"</w:p></w:body></w:document>"#,
            ),
            w = W_NS,
        ),
    );
    let report = validate(&d);
    assert!(rule_findings(&report, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("val attribute with an unresolved namespace prefix on w:numFmt")
            && n.contains("whether it is w:val could not be established")),
        "{:?}",
        report.coverage_notes
    );
    assert!(
        !report.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val") || n.contains("whose value could not be delivered")),
        "{:?}",
        report.coverage_notes
    );

    // Both conditions in the same part produce both distinct disclosures independently.
    let mut d2 = Doc::minimal();
    d2.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY ns "urn:foreign"><!ENTITY inner "bad"><!ENTITY outer "&inner;">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt xmlns:u="&ns;" u:val="bad"/>"#,
                r#"<w:numFmt w:val="&outer;"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report2 = validate(&d2);
    assert!(rule_findings(&report2, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report2.findings);
    assert!(
        report2.coverage_notes.iter().any(|n| n.contains("val attribute with an unresolved namespace prefix on w:numFmt")),
        "{:?}",
        report2.coverage_notes
    );
    assert!(
        report2.coverage_notes.iter().any(|n| n.contains("w:numFmt/@w:val") && n.contains("whose value could not be delivered")),
        "{:?}",
        report2.coverage_notes
    );
}

#[test]
fn grammar_rels_named_content_part_with_skipped_entity_emits_no_absence_or_exact_claim() {
    // Round-3 Finding 1 exact probe: a linked footer named `word/footer.rels` with CT_FOOTER and
    // an internal entity expanding to `<w:tblPr/>` must NEVER emit a false absence claim, exact
    // count, or exact ordinal, and must disclose DTD taint on the grammar note channel.
    let build = |table_inner: &str| {
        let mut d = Doc::minimal();
        d.add_with_override(
            "word/footer.rels",
            CT_FOOTER,
            format!(
                concat!(
                    r#"<?xml version="1.0"?>"#,
                    r#"<!DOCTYPE w:ftr [<!ENTITY one "<w:tblPr/>">]>"#,
                    r#"<w:ftr xmlns:w="{w}"><w:tbl>{inner}</w:tbl></w:ftr>"#,
                ),
                w = W_NS,
                inner = table_inner,
            ),
        );
        d.set_doc_rels(
            r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer.rels"/>"#,
        );
        d
    };

    // Case 1: `&one;` supplies the sole `w:tblPr` -> zero GRAMMAR-REQUIRED-CHILD, taint note present.
    let report = validate(&build("&one;<w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr>"));
    assert!(rule_findings(&report, "GRAMMAR-REQUIRED-CHILD").is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n == "DTD present in word/footer.rels; grammar checks are incomplete here"),
        "{:?}",
        report.coverage_notes
    );

    // Case 2: `&one;` followed by two literal `<w:tblPr/>` -> TOO-MANY uses lower-bound count and ordinal.
    let report2 = validate(&build("&one;<w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr>"));
    assert!(rule_findings(&report2, "GRAMMAR-REQUIRED-CHILD").is_empty(), "{:?}", report2.findings);
    let too_many = rule_findings(&report2, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(too_many.len(), 1, "{too_many:?}");
    assert!(
        too_many[0].message.contains("at w:ftr/w:tbl[>=1]/w:tblPr[>=2] ") && too_many[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        too_many[0]
    );
    assert!(!too_many[0].message.contains("w:tblPr[2]"), "{:?}", too_many[0]);
    assert!(!too_many[0].message.contains("has 2 w:tblPr"), "{:?}", too_many[0]);
    assert!(
        report2.coverage_notes.iter().any(|n| n == "DTD present in word/footer.rels; grammar checks are incomplete here"),
        "{:?}",
        report2.coverage_notes
    );

    // Case 3: external-only DOCTYPE on a `.rels`-named content part proves grammar's loss state comes
    // from its own GeneralRef stream rather than `rels_content_modelled`.
    let mut d3 = Doc::minimal();
    d3.add_with_override(
        "word/footer.rels",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:ftr SYSTEM "ext.dtd">"#,
                r#"<w:ftr xmlns:w="{w}"><w:tbl>&ext;<w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    d3.set_doc_rels(
        r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer.rels"/>"#,
    );
    let report3 = validate(&d3);
    assert!(rule_findings(&report3, "GRAMMAR-REQUIRED-CHILD").is_empty(), "{:?}", report3.findings);
    let too_many3 = rule_findings(&report3, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(too_many3.len(), 1, "{too_many3:?}");
    assert!(too_many3[0].message.contains("w:tblPr[>=2] ") && too_many3[0].message.contains("has at least 2 w:tblPr elements"), "{:?}", too_many3[0]);
}

#[test]
fn grammar_attlist_suppresses_absence_structural_and_numfmt_claims_while_entity_only_keeps_positives() {
    // Round-3 Finding 2 exact probe: ATTLIST-defaulted `xmlns` on `<tblPr/>` in `word/footer1.xml`
    // must NOT report a false absence for `w:tblPr`, and must disclose ATTLIST taint on the grammar note channel.
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/footer1.xml",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:ftr [<!ATTLIST tblPr xmlns CDATA "{w}">]>"#,
                r#"<w:ftr xmlns:w="{w}"><w:tbl><tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    assert!(grammar_findings(&report).is_empty(), "{:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n == "DTD present in word/footer1.xml (with ATTLIST); grammar checks are incomplete here"),
        "{:?}",
        report.coverage_notes
    );

    // ATTLIST overriding `xmlns:w` on structural children must also suppress TOO-MANY, CHILD-NOT-ALLOWED, and ORDER.
    let mut d_struct = Doc::minimal();
    d_struct.add_with_override(
        "word/footer1.xml",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:ftr [<!ATTLIST w:tblPr xmlns:w CDATA "urn:other"><!ATTLIST w:r xmlns:w CDATA "urn:other"><!ATTLIST w:rFonts xmlns:w CDATA "urn:other">]>"#,
                r#"<w:ftr xmlns:w="{w}">"#,
                r#"<w:tbl><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p><w:pPr><w:r/></w:pPr><w:r><w:rPr><w:sz w:val="20"/><w:rFonts w:ascii="Calibri"/></w:rPr><w:t>x</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
                r#"</w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    let report_struct = validate(&d_struct);
    assert!(grammar_findings(&report_struct).is_empty(), "{:?}", report_struct.findings);

    // ATTLIST default on `w:val` suppresses NUMFMT across the entire ATTLIST-bearing part
    // (both defaulted `<w:numFmt/>` and literal/entity `w:val` in the same part).
    let mut d_att_numfmt = Doc::minimal();
    d_att_numfmt.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY bad "notAFormat"><!ATTLIST w:numFmt w:val CDATA "notAFormat">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
                r#"<w:numFmt/><w:numFmt w:val="alsoBad"/><w:numFmt w:val="&bad;"/>"#,
                r#"</w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report_att_numfmt = validate(&d_att_numfmt);
    assert!(rule_findings(&report_att_numfmt, "GRAMMAR-NUMFMT-VALUE").is_empty(), "{:?}", report_att_numfmt.findings);
    assert!(
        report_att_numfmt.coverage_notes.iter().any(|n| n == "DTD present in word/numbering.xml (with ATTLIST); grammar checks are incomplete here"),
        "{:?}",
        report_att_numfmt.coverage_notes
    );

    // Tainted part WITHOUT ATTLIST: delivered internal-entity NUMFMT AND positive structural checks
    // (CHILD-NOT-ALLOWED, ORDER, TOO-MANY as at-least) still fire with uncertain paths.
    let mut d_no_att = Doc::minimal();
    d_no_att.add_with_override(
        "word/footer1.xml",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:ftr [<!ENTITY bad "notAFormat">]>"#,
                r#"<w:ftr xmlns:w="{w}">"#,
                r#"<w:p><w:pPr><w:r/></w:pPr><w:r><w:rPr><w:sz w:val="20"/><w:rFonts w:ascii="Calibri"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
                r#"<w:tbl><w:tblPr/><w:tblPr/><w:tblGrid/><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                r#"</w:ftr>"#,
            ),
            w = W_NS,
        ),
    );
    d_no_att.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:numbering [<!ENTITY bad "notAFormat">]>"#,
                r#"<w:numbering xmlns:w="{w}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="&bad;"/></w:lvl></w:abstractNum></w:numbering>"#,
            ),
            w = W_NS,
        ),
    );
    let report_no_att = validate(&d_no_att);
    let numfmt = rule_findings(&report_no_att, "GRAMMAR-NUMFMT-VALUE");
    assert_eq!(numfmt.len(), 1, "{numfmt:?}");
    assert!(numfmt[0].message.contains("\"notAFormat\"") && numfmt[0].message.contains("w:numFmt[>=1] "), "{:?}", numfmt[0]);
    let disallowed = rule_findings(&report_no_att, "GRAMMAR-CHILD-NOT-ALLOWED");
    assert_eq!(disallowed.len(), 1, "{disallowed:?}");
    assert!(disallowed[0].message.contains("w:p[>=1]/w:pPr[>=1]/w:r[>=1] "), "{:?}", disallowed[0]);
    let order = rule_findings(&report_no_att, "GRAMMAR-ORDER");
    assert_eq!(order.len(), 1, "{order:?}");
    assert!(order[0].message.contains("w:rPr[>=1]/w:rFonts[>=1] "), "{:?}", order[0]);
    let too_many = rule_findings(&report_no_att, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(too_many.len(), 1, "{too_many:?}");
    assert!(
        too_many[0].message.contains("w:tbl[>=1]/w:tblPr[>=2] ") && too_many[0].message.contains("has at least 2 w:tblPr elements"),
        "{:?}",
        too_many[0]
    );
    assert!(
        report_no_att.coverage_notes.iter().any(|n| n == "DTD present in word/footer1.xml; grammar checks are incomplete here"),
        "{:?}",
        report_no_att.coverage_notes
    );
    assert!(
        report_no_att.coverage_notes.iter().any(|n| n == "DTD present in word/numbering.xml; grammar checks are incomplete here"),
        "{:?}",
        report_no_att.coverage_notes
    );
}

#[test]
fn grammar_attribute_8192_byte_bound_applies_identically_across_literal_charref_and_entity() {
    // Round-3 Finding 3 exact probe: 8,193 ASCII 'z' characters delivered as all-literal,
    // trailing numeric character reference `&#122;`, or leading internal entity `&z;` must all
    // take the identical unreadable-value path (zero NUMFMT warnings, unreadable-value note).
    let z8193 = "z".repeat(8193);
    let z8192 = "z".repeat(8192);
    let z8191 = "z".repeat(8191);

    let footer_over = |doctype: &str, attr_spelling: &str| {
        let mut d = Doc::minimal();
        d.add_with_override(
            "word/footer1.xml",
            CT_FOOTER,
            format!(r#"<?xml version="1.0"?>{doctype}<w:ftr xmlns:w="{W_NS}"><w:p><w:numFmt w:val="{attr_spelling}"/></w:p></w:ftr>"#),
        );
        d
    };

    for (label, doctype, spelling) in [
        ("literal 8193", "", z8193.as_str()),
        ("charref 8193", "", &format!("{z8192}&#122;")),
        ("entity 8193", r#"<!DOCTYPE w:ftr [<!ENTITY z "z">]>"#, &format!("&z;{z8192}")),
    ] {
        let report = validate(&footer_over(doctype, spelling));
        assert!(
            rule_findings(&report, "GRAMMAR-NUMFMT-VALUE").is_empty(),
            "{label} must take unreadable-value path, not emit NUMFMT: {:?}",
            report.findings
        );
        assert!(
            report
                .coverage_notes
                .iter()
                .any(|n| n.contains("word/footer1.xml has a w:numFmt/@w:val attribute") && n.contains("whose value could not be delivered")),
            "{label} must emit unreadable-value note: {:?}",
            report.coverage_notes
        );
    }

    // Exact boundary (8,192 bytes): all three spellings deliver the 8,192-byte value and fire NUMFMT identically.
    for (label, doctype, spelling) in [
        ("literal 8192", "", z8192.as_str()),
        ("charref 8192", "", &format!("{z8191}&#122;")),
        ("entity 8192", r#"<!DOCTYPE w:ftr [<!ENTITY z "z">]>"#, &format!("&z;{z8191}")),
    ] {
        let report = validate(&footer_over(doctype, spelling));
        let hits = rule_findings(&report, "GRAMMAR-NUMFMT-VALUE");
        assert_eq!(hits.len(), 1, "{label} at 8,192 bytes must fire NUMFMT: {:?}", report.findings);
        assert!(hits[0].message.contains(&format!("\"{z8192}\"")), "{label}");
        assert!(
            !report.coverage_notes.iter().any(|n| n.contains("whose value could not be delivered")),
            "{label} at 8,192 bytes must not emit unreadable-value note: {:?}",
            report.coverage_notes
        );
    }
}

#[test]
fn ordinary_output_stays_byte_identical_on_astras_two_differential_packages() {
    // Differential 1: mc:Ignorable with entity `&props;` + 8,200 spaces (no table).
    // Verdict, ordered non-grammar findings, and ordinary (non-grammar) notes stay byte-identical
    // to pre-port base (Fail, XML-DOCTYPE + XML-UNDECLARED-PREFIX, zero ordinary notes).
    let spaces = " ".repeat(8200);
    let mut d1 = Doc::minimal();
    d1.set(
        "word/document.xml",
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:document [<!ENTITY props "lostprefix ">]>"#,
                r#"<w:document xmlns:w="{w}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="&props;{spaces}">"#,
                r#"<w:body><w:p/></w:body></w:document>"#,
            ),
            w = W_NS,
            spaces = spaces,
        ),
    );
    let r1 = validate(&d1);
    assert_eq!(r1.verdict, Verdict::Fail, "{r1:?}");
    let ord_findings_1: Vec<String> =
        r1.findings.iter().filter(|f| !f.rule.starts_with("GRAMMAR-")).map(|f| serde_json::to_string(f).unwrap()).collect();
    assert_eq!(
        ord_findings_1,
        vec![
            r#"{"rule":"XML-DOCTYPE","severity":"Error","evidence":"Observed","part":"word/document.xml","message":"word/document.xml contains a DOCTYPE declaration; see corpus/EVIDENCE.md for measured outcomes"}"#,
            r#"{"rule":"XML-UNDECLARED-PREFIX","severity":"Corrupt","evidence":"ByConstruction","part":"word/document.xml","message":"the namespace prefix `lostprefix` used in word/document.xml is not declared"}"#,
        ]
    );
    assert!(grammar_findings(&r1).is_empty(), "{:?}", r1.findings);
    let ord_notes_1: Vec<&String> = r1.coverage_notes.iter().filter(|n| !n.starts_with("DTD present in ")).collect();
    assert!(ord_notes_1.is_empty(), "{:?}", r1.coverage_notes);
    assert_eq!(r1.coverage_notes, vec!["DTD present in word/document.xml; grammar checks are incomplete here"]);

    // Differential 2: Astra's reference/field footer differential — linked footer with DOCTYPE,
    // two `w:tblGrid` children (one carrying unresolved-prefix `u:val`, one carrying unreadable `w:val`),
    // a dangling relationship ID (`r:embed="rIdMissing"`), an unmatched field end (`w:fldCharType="end"`),
    // and an undeclared element prefix (`nope:x`).
    let mut d2 = Doc::minimal();
    d2.add_with_override(
        "word/footer1.xml",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<!DOCTYPE w:ftr [<!ENTITY ns "urn:foreign"><!ENTITY inner "bad"><!ENTITY outer "&inner;">]>"#,
                r#"<w:ftr xmlns:w="{w}" xmlns:a="{a}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
                r#"<w:tbl><w:tblPr/><w:tblGrid xmlns:u="&ns;" u:val="bad"/><w:tblGrid w:val="&outer;"/><w:tr><w:tc>"#,
                r#"<w:p><w:r><w:fldChar w:fldCharType="end"/><a:blip r:embed="rIdMissing"/><nope:x/></w:r></w:p>"#,
                r#"</w:tc></w:tr></w:tbl></w:ftr>"#,
            ),
            w = W_NS,
            a = A_NS,
        ),
    );
    d2.set_doc_rels(
        r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
    );
    let r2 = validate(&d2);
    assert_eq!(r2.verdict, Verdict::Fail, "{r2:?}");
    let ord_findings_2: Vec<String> =
        r2.findings.iter().filter(|f| !f.rule.starts_with("GRAMMAR-")).map(|f| serde_json::to_string(f).unwrap()).collect();
    assert_eq!(
        ord_findings_2,
        vec![
            r#"{"rule":"FLD-UNBALANCED","severity":"Error","evidence":"Observed","part":"word/footer1.xml","message":"word/footer1.xml has 1 w:fldChar end(s) with no open begin"}"#,
            r#"{"rule":"REL-REF-MISSING","severity":"Error","evidence":"Inferred","part":"word/footer1.xml","message":"the r:embed value rIdMissing in word/footer1.xml does not resolve to a relationship in that part's .rels"}"#,
            r#"{"rule":"XML-DOCTYPE","severity":"Error","evidence":"Observed","part":"word/footer1.xml","message":"word/footer1.xml contains a DOCTYPE declaration; see corpus/EVIDENCE.md for measured outcomes"}"#,
            r#"{"rule":"XML-UNDECLARED-PREFIX","severity":"Corrupt","evidence":"ByConstruction","part":"word/footer1.xml","message":"the namespace prefix `nope` used in word/footer1.xml is not declared"}"#,
        ]
    );
    let ord_notes_2: Vec<&String> = r2.coverage_notes.iter().filter(|n| !n.starts_with("DTD present in ")).collect();
    assert!(ord_notes_2.is_empty(), "{:?}", r2.coverage_notes);
    let too_many_2 = rule_findings(&r2, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(too_many_2.len(), 1, "{too_many_2:?}");
    assert!(
        too_many_2[0].message.contains("at w:ftr/w:tbl[>=1]/w:tblGrid[>=2] ") && too_many_2[0].message.contains("has at least 2 w:tblGrid elements"),
        "{:?}",
        too_many_2[0]
    );
    assert_eq!(r2.coverage_notes, vec!["DTD present in word/footer1.xml; grammar checks are incomplete here"]);

    // Non-DOCTYPE control (does not exercise changed grammar paths): zero grammar notes and exact ordinary output.
    let mut d3 = Doc::minimal();
    d3.add_with_override(
        "word/footer1.xml",
        CT_FOOTER,
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<w:ftr xmlns:w="{w}" xmlns:a="{a}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
                r#"<w:p><w:r><w:fldChar w:fldCharType="end"/><a:blip r:embed="rIdMissing"/><nope:x/></w:r></w:p>"#,
                r#"</w:ftr>"#,
            ),
            w = W_NS,
            a = A_NS,
        ),
    );
    d3.set_doc_rels(
        r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
    );
    let r3 = validate(&d3);
    assert_eq!(r3.verdict, Verdict::Fail, "{r3:?}");
    assert!(r3.coverage_notes.is_empty(), "non-DOCTYPE part must emit zero grammar notes: {:?}", r3.coverage_notes);
    assert!(grammar_findings(&r3).is_empty(), "{:?}", r3.findings);
}

#[test]
fn grammar_enforces_finite_child_limits_across_namespace_boundaries_without_widening_child_scope() {
    // wp:inline, wp:anchor, and wp:graphicFrame each allow at most one a:graphic child.
    // One a:graphic is valid (and its inner a:* subtree stays out of Wml scope).
    let make_doc = |drawing_inner: &str| {
        let mut d = Doc::minimal();
        d.set(
            "word/document.xml",
            format!(
                r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:wp="{WP_NS}" xmlns:a="{A_NS}"><w:body><w:p><w:r><w:drawing>{drawing_inner}</w:drawing></w:r></w:p></w:body></w:document>"#
            ),
        );
        d
    };

    // Valid single a:graphic inside wp:inline (with unmodelled-in-Wml a:graphic content).
    let valid = validate(&make_doc(
        r#"<wp:inline><wp:extent cx="1" cy="1"/><wp:docPr id="1" name="p"/><a:graphic><a:notCheckedInWml/></a:graphic></wp:inline>"#,
    ));
    assert_eq!(valid.verdict, Verdict::Pass, "{:?}", valid.findings);
    assert!(grammar_findings(&valid).is_empty(), "{:?}", valid.findings);

    // Duplicate a:graphic in wp:inline fires GRAMMAR-CHILD-TOO-MANY while leaving a:graphic's subtree unjudged.
    let dup_inline = validate(&make_doc(
        r#"<wp:inline><wp:extent cx="1" cy="1"/><wp:docPr id="1" name="p"/><a:graphic><a:notCheckedInWml/></a:graphic><a:graphic/></wp:inline>"#,
    ));
    assert_eq!(dup_inline.verdict, Verdict::Pass, "{:?}", dup_inline.findings);
    let hits = rule_findings(&dup_inline, "GRAMMAR-CHILD-TOO-MANY");
    assert_eq!(hits.len(), 1, "{:?}", dup_inline.findings);
    assert!(
        hits[0].message.contains("a:graphic[2]")
            && hits[0].message.contains("has at least 2 a:graphic elements")
            && hits[0].message.contains("at most 1"),
        "{:?}",
        hits[0]
    );
    assert!(rule_findings(&dup_inline, "GRAMMAR-CHILD-NOT-ALLOWED").is_empty(), "{:?}", dup_inline.findings);

    // Duplicate a:graphic in wp:anchor fires GRAMMAR-CHILD-TOO-MANY.
    let dup_anchor = validate(&make_doc(
        r#"<wp:anchor><wp:simplePos x="0" y="0"/><wp:positionH/><wp:positionV/><wp:extent cx="1" cy="1"/><wp:wrapNone/><wp:docPr id="1" name="p"/><a:graphic/><a:graphic/></wp:anchor>"#,
    ));
    assert_eq!(rule_findings(&dup_anchor, "GRAMMAR-CHILD-TOO-MANY").len(), 1, "{:?}", dup_anchor.findings);

    // Duplicate a:graphic in wp:graphicFrame fires GRAMMAR-CHILD-TOO-MANY.
    let dup_frame =
        validate(&make_doc(r#"<wp:graphicFrame><wp:cNvPr id="1" name="g"/><wp:cNvFrPr/><wp:xfrm/><a:graphic/><a:graphic/></wp:graphicFrame>"#));
    assert_eq!(rule_findings(&dup_frame, "GRAMMAR-CHILD-TOO-MANY").len(), 1, "{:?}", dup_frame.findings);

    // Out-of-scope undeclared a:* child under w:p remains ignored (no GRAMMAR-CHILD-NOT-ALLOWED).
    let mut foreign = Doc::minimal();
    foreign.set(
        "word/document.xml",
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:a="{A_NS}"><w:body><w:p><a:graphic/></w:p></w:body></w:document>"#),
    );
    assert!(grammar_findings(&validate(&foreign)).is_empty());
}

#[test]
fn grammar_classifies_standard_customxml_bibliography_parts_and_keeps_other_customxml_inactive() {
    let custom_xml_rel = r#"<Relationship Id="rBib" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#;

    // 1. Valid standard ECMA-376 bibliography part (application/xml + customXml rel + b:Sources root) is clean.
    let mut valid = Doc::minimal();
    valid.add_with_override(
        "customXml/item1.xml",
        "application/xml",
        format!(r#"<?xml version="1.0"?><b:Sources xmlns:b="{B_NS}"><b:Source><b:Tag>S1</b:Tag></b:Source></b:Sources>"#),
    );
    valid.set_doc_rels(custom_xml_rel);
    let r_valid = validate(&valid);
    assert_eq!(r_valid.verdict, Verdict::Pass, "{:?}", r_valid.findings);
    assert!(grammar_findings(&r_valid).is_empty(), "{:?}", r_valid.findings);

    // 2. Defective standard bibliography part (b:Tag directly under b:Sources) emits GRAMMAR-CHILD-NOT-ALLOWED.
    let mut invalid = Doc::minimal();
    invalid.add_with_override(
        "customXml/item1.xml",
        "application/xml",
        format!(r#"<?xml version="1.0"?><b:Sources xmlns:b="{B_NS}"><b:Tag>bad</b:Tag></b:Sources>"#),
    );
    invalid.set_doc_rels(custom_xml_rel);
    let r_invalid = validate(&invalid);
    assert_eq!(r_invalid.verdict, Verdict::Pass, "{:?}", r_invalid.findings);
    let hits = rule_findings(&r_invalid, "GRAMMAR-CHILD-NOT-ALLOWED");
    assert_eq!(hits.len(), 1, "{:?}", r_invalid.findings);
    assert_eq!(hits[0].part.as_deref(), Some("customXml/item1.xml"));
    assert!(hits[0].message.contains("b:Tag") && hits[0].message.contains("b:CT_Sources"), "{:?}", hits[0]);

    // 3. Unrelated customXml part stays inactive even if it contains WML/bibliography-named elements.
    let mut unrelated = Doc::minimal();
    unrelated.add_with_override(
        "customXml/item1.xml",
        "application/xml",
        format!(r#"<?xml version="1.0"?><custom xmlns="urn:example:custom" xmlns:w="{W_NS}"><w:pPr><w:r/></w:pPr></custom>"#),
    );
    unrelated.set_doc_rels(custom_xml_rel);
    let r_unrelated = validate(&unrelated);
    assert_eq!(r_unrelated.verdict, Verdict::Pass, "{:?}", r_unrelated.findings);
    assert!(grammar_findings(&r_unrelated).is_empty(), "{:?}", r_unrelated.findings);

    // 4. Conflicting scope (CT_CHART content type + customXml bibliography root) merges to Conflicting and skips grammar.
    let mut conflict = Doc::minimal();
    conflict.add_with_override(
        "customXml/item1.xml",
        CT_CHART,
        format!(r#"<?xml version="1.0"?><b:Sources xmlns:b="{B_NS}"><b:Tag>bad</b:Tag></b:Sources>"#),
    );
    conflict.set_doc_rels(custom_xml_rel);
    let r_conflict = validate(&conflict);
    assert!(grammar_findings(&r_conflict).is_empty(), "{:?}", r_conflict.findings);
}

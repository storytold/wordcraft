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

use crate::common::{CT_CHART, CT_HEADER, CT_MACRO_MAIN, CT_NUMBERING, CT_SETTINGS, CT_THEME, Doc, content_types_xml, document_xml};
use wordcraft_validate::{Evidence, Finding, Report, Severity, Verdict, validate_bytes};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
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
    assert!(hits[0].message.contains("w:drawing"), "{:?}", hits[0]);

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
    assert!(hits[0].message.contains("at w:document/w:body[1]/w:tbl[1]/w:tblPr[2]"), "{:?}", hits[0]);

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

    // Conflicting scopes (chart content type + header relationship) skip grammar cleanly in both entry orders.
    let mut reports = Vec::new();
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.add_with_override("word/weird.xml", CT_CHART, format!(r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}"><w:sectPr/></w:hdr>"#));
        d.set_doc_rels(
            r#"<Relationship Id="rW" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="weird.xml"/>"#,
        );
        if reverse {
            d.entries.reverse();
        }
        reports.push(validate(&d));
    }
    assert_eq!(grammar_findings(&reports[0]).len(), 0);
    assert_eq!(serde_json::to_value(&reports[0]).unwrap(), serde_json::to_value(&reports[1]).unwrap());
}

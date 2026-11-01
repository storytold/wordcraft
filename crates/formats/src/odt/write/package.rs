//! The package: `mimetype` (stored, first), `META-INF/manifest.xml`, `meta.xml`,
//! `settings.xml`, the two style/content parts and the packaged pictures (ODF 1.3 Part 2).

use std::collections::BTreeSet;
use std::io::{Cursor, Write};

use wordcraft_doc::Document;

use super::Packaged;
use crate::odt::{NS, x};

/// Compatibility settings for a layout that follows Word's rules: the layout settings LibreOffice
/// gives a document it opens from Word formats (read from the ODT it saves of one).
/// `TabsRelativeToIndent` matters most: the model's tab stops are measured from the text
/// column's edge, ODF's from the paragraph indent by default.
const COMPAT: &[(&str, &str)] = &[
    ("TabsRelativeToIndent", "false"),
    ("TabOverMargin", "false"),
    ("TabOverflow", "true"),
    ("TabOverSpacing", "true"),
    ("TabAtLeftIndentForParagraphsInList", "true"),
    ("AddParaTableSpacing", "false"),
    ("AddParaTableSpacingAtStart", "true"),
    ("AddParaSpacingToTableCells", "true"),
    ("AddParaLineSpacingToTableCells", "true"),
    ("AddExternalLeading", "true"),
    ("UnxForceZeroExtLeading", "false"),
    ("UseFormerLineSpacing", "false"),
    ("UseFormerObjectPositioning", "false"),
    ("UseFormerTextWrapping", "false"),
    ("UseOldNumbering", "false"),
    ("UseOldPrinterMetrics", "false"),
    ("ApplyParagraphMarkFormatToNumbering", "true"),
    ("MsWordCompTrailingBlanks", "true"),
    ("MsWordCompMinLineHeightByFly", "false"),
    ("PropLineSpacingShrinksFirstLine", "true"),
    ("ClippedPictures", "true"),
    ("TreatSingleColumnBreakAsPageBreak", "true"),
    ("IgnoreFirstLineIndentInNumbering", "false"),
    ("DoNotJustifyLinesWithManualBreak", "false"),
    ("CollapseEmptyCellPara", "true"),
    ("NoGapAfterNoteNumber", "true"),
    ("HeaderSpacingBelowLastPara", "true"),
    ("AddVerticalFrameOffsets", "true"),
    ("AddFrameOffsets", "false"),
    ("InvertBorderSpacing", "true"),
    ("AlignTabStopPosition", "true"),
    ("IgnoreTabsAndBlanksForLineCalculation", "true"),
    ("ConsiderTextWrapOnObjPos", "true"),
    ("OutlineLevelYieldsNumbering", "false"),
    ("TableRowKeep", "true"),
    ("SurroundTextWrapSmall", "true"),
    ("MathBaselineAlignment", "true"),
    ("MsWordCompGridMetrics", "true"),
    ("MsWordUlTrailSpace", "false"),
    ("ApplyParagraphMarkFormatToEmptyLineAtEndOfParagraph", "true"),
    ("ApplyTextAttrToEmptyLineAtEndOfParagraph", "false"),
    ("AutoFirstLineIndentDisregardLineSpace", "true"),
    ("IgnoreHiddenCharsForLineCalculation", "true"),
    ("HiddenParagraphMarkPerLineProperties", "false"),
    ("BalanceSpacesAndIdeographicSpaces", "false"),
    ("UseVariableWidthNBSP", "false"),
    ("SmallCapsPercentage66", "false"),
    ("DropCapPunctuation", "true"),
    ("UnbreakableNumberings", "true"),
    ("NoNumberingShowFollowBy", "true"),
    ("DoNotResetParaAttrsForNumFont", "false"),
    ("MinRowHeightInclBorder", "true"),
    ("AdjustTableLineHeightsToGridHeight", "false"),
    ("FootnoteInColumnToPageEnd", "true"),
    ("BackgroundParaOverDrawings", "true"),
    ("PaintHellOverHeaderFooter", "true"),
    ("DisableOffPagePositioning", "true"),
    ("DoNotCaptureDrawObjsOnPage", "true"),
    ("NoClippingWithWrapPolygon", "true"),
    ("FrameAutowidthWithMorePara", "true"),
    ("SubtractFlysAnchoredAtFlys", "false"),
    ("ClipAsCharacterAnchoredWriterFlyFrames", "false"),
    ("ForceTopAlignmentInCellWithFloatingAnchor", "false"),
    ("DoNotMirrorRtlDrawObjs", "true"),
    ("GutterAtTop", "false"),
    ("HyphenateURLs", "false"),
    // Endnotes follow the text, as Word sets them, rather than on a page of their own.
    ("ContinuousEndnotes", "true"),
];

/// `settings.xml`. The set's name `ooo:configuration-settings` is a qualified name: without
/// `xmlns:ooo` on the root LibreOffice ignores every item in it.
pub(super) fn settings_xml(doc: &Document) -> String {
    // Word 2013 (compatibility mode 15) shrinks the spaces of a justified line before it breaks
    // it; documents in an earlier mode don't.
    let shrink = if doc.settings.compat_mode >= 15 { "true" } else { "false" };
    let items: String = COMPAT
        .iter()
        .chain(&[("JustifyLinesWithShrinking", shrink)])
        .map(|(k, v)| format!("<config:config-item config:name=\"{k}\" config:type=\"boolean\">{v}</config:config-item>"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:ooo=\"http://openoffice.org/2004/office\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" office:version=\"1.3\"><office:settings><config:config-item-set config:name=\"ooo:configuration-settings\">{items}<config:config-item config:name=\"PrinterIndependentLayout\" config:type=\"string\">high-resolution</config:config-item></config:config-item-set></office:settings></office:document-settings>\n"
    )
}

/// `office:font-face-decls` for the families used.
pub(super) fn font_decls(fonts: &BTreeSet<String>) -> String {
    let mut s = String::from("<office:font-face-decls>");
    for f in fonts {
        let fam = if f.contains(' ') { format!("'{f}'") } else { f.clone() };
        s.push_str(&format!("<style:font-face style:name=\"{}\" svg:font-family=\"{}\"/>", x(f), x(&fam)));
    }
    s.push_str("</office:font-face-decls>");
    s
}

fn meta_xml(doc: &Document) -> String {
    let c = &doc.core;
    let mut m = String::from("<meta:generator>WordCraft</meta:generator>");
    for (v, tag) in [(&c.title, "dc:title"), (&c.subject, "dc:subject"), (&c.description, "dc:description")] {
        if !v.is_empty() {
            m.push_str(&format!("<{tag}>{}</{tag}>", x(v)));
        }
    }
    if !c.creator.is_empty() {
        m.push_str(&format!("<meta:initial-creator>{}</meta:initial-creator>", x(&c.creator)));
    }
    let last = if c.last_modified_by.is_empty() { &c.creator } else { &c.last_modified_by };
    if !last.is_empty() {
        m.push_str(&format!("<dc:creator>{}</dc:creator>", x(last)));
    }
    for k in c.keywords.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        m.push_str(&format!("<meta:keyword>{}</meta:keyword>", x(k)));
    }
    for (v, tag) in [(&c.created, "meta:creation-date"), (&c.modified, "dc:date")] {
        if !v.is_empty() {
            m.push_str(&format!("<{tag}>{}</{tag}>", x(v.trim_end_matches('Z'))));
        }
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta {NS}><office:meta>{m}</office:meta></office:document-meta>\n")
}

fn manifest_xml(files: &[Packaged]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\n",
    );
    for p in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        s.push_str(&format!("<manifest:file-entry manifest:full-path=\"{p}\" manifest:media-type=\"text/xml\"/>\n"));
    }
    for f in files {
        s.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>\n", x(&f.path), x(&f.mime)));
    }
    s.push_str("</manifest:manifest>\n");
    s
}

/// Zip the parts into a package.
pub(super) fn zip(doc: &Document, content: &str, styles: &str, files: &[Packaged]) -> Result<Vec<u8>, String> {
    let err = |e: zip::result::ZipError| format!("ODT: {e}");
    let ioerr = |e: std::io::Error| format!("ODT: {e}");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).map_err(err)?;
    z.write_all(b"application/vnd.oasis.opendocument.text").map_err(ioerr)?;
    let (manifest, meta, settings) = (manifest_xml(files), meta_xml(doc), settings_xml(doc));
    for (name, data) in [
        ("META-INF/manifest.xml", manifest.as_str()),
        ("content.xml", content),
        ("styles.xml", styles),
        ("meta.xml", meta.as_str()),
        ("settings.xml", settings.as_str()),
    ] {
        z.start_file(name, deflated).map_err(err)?;
        z.write_all(data.as_bytes()).map_err(ioerr)?;
    }
    // Directories (`Object1/`) are manifest entries only.
    for f in files.iter().filter(|f| !f.path.ends_with('/')) {
        z.start_file(f.path.as_str(), stored).map_err(err)?;
        z.write_all(&f.data).map_err(ioerr)?;
    }
    Ok(z.finish().map_err(err)?.into_inner())
}

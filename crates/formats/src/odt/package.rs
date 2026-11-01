use super::writer::{NS, Writer, pt, spacing_props, x};
use super::{Columns, Cursor, Document, FBlock, Meta, SectionProps, SectionStart, Write, model};

/// `style:columns` (ODF 1.3 §17.12) of equal columns with their gap and, when on, the line between.
fn columns_xml(c: &Columns) -> String {
    let sep = if c.separator { "<style:column-sep style:width=\"0.5pt\" style:style=\"solid\" style:color=\"#000000\"/>" } else { "" };
    format!("<style:columns fo:column-count=\"{}\" fo:column-gap=\"{}\">{sep}</style:columns>", c.count.clamp(1, 45), pt(c.space))
}

/// One master page per entry of `pages` (name, setup), each with its own page layout `pm1`, `pm2`…
/// (§16.5, §16.9); the first is the default one, "Standard".
/// A master page's headers and footers (ODF 1.3 §16.10), written.
#[derive(Default)]
struct PageParts {
    /// The `style:header…`/`style:footer…` elements.
    xml: String,
    header: bool,
    footer: bool,
}

/// The headers and footers section `sec` shows, written by `w` (its styles go in `styles.xml`):
/// the default one for every page, the even one as left pages' when the document has different
/// odd and even pages, the first one with a different first page. A slot Word leaves empty is an
/// empty element, so the page shows nothing there rather than the default one.
fn page_parts(doc: &Document, sec: &SectionProps, w: &mut Writer) -> PageParts {
    let mut out = PageParts::default();
    let even = doc.settings.even_odd_headers;
    for (set, name) in [(&sec.headers, "header"), (&sec.footers, "footer")] {
        let wanted = set.default.is_some() || (even && set.even.is_some()) || (sec.title_page && set.first.is_some());
        if !wanted {
            continue;
        }
        for (id, el, on) in
            [(set.default, name.to_string(), true), (set.even, format!("{name}-left"), even), (set.first, format!("{name}-first"), sec.title_page)]
        {
            if !on {
                continue;
            }
            let mut body = String::new();
            if let Some(part) = id.and_then(|i| doc.parts.get(&i)) {
                let blocks = model::part_flow(
                    doc,
                    &part.blocks,
                    model::FlowOpts { fields: true, lists: true, chars: true, paras: true, ..Default::default() },
                );
                w.blocks(&blocks, &mut body, 0);
            }
            if body.is_empty() {
                body.push_str("<text:p/>");
            }
            out.xml.push_str(&format!("<style:{el}>{body}</style:{el}>"));
        }
        if name == "header" {
            out.header = true;
        } else {
            out.footer = true;
        }
    }
    out
}

/// `defaults` is the spacing of the document's Normal paragraphs, written as the default style's;
/// `parts` the headers and footers of each page in `pages` and `part_styles` their automatic
/// styles.
fn styles_xml(
    pages: &[(String, wordcraft_doc::SectionProps)],
    parts: &[PageParts],
    part_styles: &str,
    body_font: &str,
    defaults: &model::Spacing,
    kern: bool,
    tab: f32,
) -> String {
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles>");
    s.push_str(&format!(
        "<style:default-style style:family=\"paragraph\"><style:paragraph-properties{} style:tab-stop-distance=\"{tab}\"/><style:text-properties fo:font-family=\"'{}'\" fo:font-size=\"12pt\" style:letter-kerning=\"{kern}\"/></style:default-style>",
        spacing_props(defaults),
        x(body_font),
        tab = pt(tab)
    ));
    s.push_str("<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>");
    s.push_str("<style:style style:name=\"Heading\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"text\"><style:paragraph-properties fo:margin-top=\"12pt\" fo:margin-bottom=\"4pt\" fo:keep-with-next=\"always\"/><style:text-properties fo:color=\"#0f4761\"/></style:style>");
    let sizes = [20.0, 16.0, 14.0, 12.0, 12.0, 11.0];
    for (i, sz) in sizes.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!(
            "<style:style style:name=\"Heading_20_{n}\" style:display-name=\"Heading {n}\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:next-style-name=\"Standard\" style:default-outline-level=\"{n}\" style:class=\"text\"><style:text-properties fo:font-size=\"{}\"/></style:style>",
            pt(*sz)
        ));
    }
    s.push_str("<style:style style:name=\"Title\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"chapter\"><style:text-properties fo:font-size=\"28pt\"/></style:style>");
    s.push_str("<style:style style:name=\"Quotations\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:paragraph-properties fo:margin-left=\"36pt\" fo:margin-right=\"36pt\"/><style:text-properties fo:font-style=\"italic\" fo:color=\"#404040\"/></style:style>");
    s.push_str(&format!("<style:style style:name=\"Preformatted_20_Text\" style:display-name=\"Preformatted Text\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\"/><style:text-properties fo:font-family=\"'{}'\" style:font-family-generic=\"modern\" style:font-pitch=\"fixed\" fo:font-size=\"10pt\"/></style:style>", model::MONO_FONT));
    // The note text styles, as WordCraft's Footnote Text and Endnote Text: 10 pt, no spacing.
    for (name, class) in [("Footnote", "extra"), ("Endnote", "extra")] {
        s.push_str(&format!(
            "<style:style style:name=\"{name}\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"{class}\"><style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:line-height=\"100%\"/><style:text-properties fo:font-size=\"10pt\"/></style:style>"
        ));
    }
    s.push_str("</office:styles><office:automatic-styles>");
    s.push_str(part_styles);
    let none = PageParts::default();
    for (i, (_, sec)) in pages.iter().enumerate() {
        let pp = parts.get(i).unwrap_or(&none);
        // A page of several columns carries them in its layout.
        let cols = if sec.columns.count > 1 { columns_xml(&sec.columns) } else { String::new() };
        // Word measures the header from the page edge and the text from the page edge; ODF
        // starts the header at the page margin and the text after the header's height. So with
        // a header the page margin is the header's distance, and the header is tall enough to
        // reach the text (it grows with its content, as Word's pushes the text down).
        let (top, header_h) = if pp.header { (sec.header, (sec.margin_top - sec.header).max(0.0)) } else { (sec.margin_top, 0.0) };
        let (bottom, footer_h) = if pp.footer { (sec.footer, (sec.margin_bottom - sec.footer).max(0.0)) } else { (sec.margin_bottom, 0.0) };
        let header = if pp.header {
            format!(
                "<style:header-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-bottom=\"0pt\"/></style:header-style>",
                pt(header_h)
            )
        } else {
            String::new()
        };
        let footer = if pp.footer {
            format!(
                "<style:footer-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-top=\"0pt\"/></style:footer-style>",
                pt(footer_h)
            )
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<style:page-layout style:name=\"pm{}\"><style:page-layout-properties fo:page-width=\"{}\" fo:page-height=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" style:print-orientation=\"{}\">{cols}</style:page-layout-properties>{header}{footer}</style:page-layout>",
            i + 1,
            pt(sec.page_w),
            pt(sec.page_h),
            pt(top),
            pt(bottom),
            pt(sec.margin_left),
            pt(sec.margin_right),
            if sec.page_w > sec.page_h { "landscape" } else { "portrait" }
        ));
    }
    s.push_str("</office:automatic-styles><office:master-styles>");
    for (i, (name, _)) in pages.iter().enumerate() {
        let xml = parts.get(i).map(|p| p.xml.as_str()).unwrap_or("");
        s.push_str(&format!("<style:master-page style:name=\"{}\" style:page-layout-name=\"pm{}\">{xml}</style:master-page>", x(name), i + 1));
    }
    s.push_str("</office:master-styles></office:document-styles>\n");
    s
}

/// `settings.xml` (ODF 1.3 §3.10): tab stops are measured from the margin, as WordCraft's and
/// Word's are (LibreOffice's own default measures them from the paragraph's indent).
const SETTINGS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" office:version=\"1.3\">",
    "<office:settings><config:config-item-set config:name=\"ooo:configuration-settings\">",
    "<config:config-item config:name=\"TabsRelativeToIndent\" config:type=\"boolean\">false</config:config-item>",
    "</config:config-item-set></office:settings></office:document-settings>\n"
);

fn meta_xml(m: &Meta) -> String {
    let mut s =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta {NS}><office:meta><meta:generator>WordCraft</meta:generator>");
    if !m.title.is_empty() {
        s.push_str(&format!("<dc:title>{}</dc:title>", x(&m.title)));
    }
    if !m.subject.is_empty() {
        s.push_str(&format!("<dc:subject>{}</dc:subject>", x(&m.subject)));
    }
    if !m.description.is_empty() {
        s.push_str(&format!("<dc:description>{}</dc:description>", x(&m.description)));
    }
    if !m.author.is_empty() {
        s.push_str(&format!("<meta:initial-creator>{}</meta:initial-creator><dc:creator>{}</dc:creator>", x(&m.author), x(&m.author)));
    }
    for k in m.keywords.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        s.push_str(&format!("<meta:keyword>{}</meta:keyword>", x(k)));
    }
    if !m.created.is_empty() {
        s.push_str(&format!("<meta:creation-date>{}</meta:creation-date>", x(m.created.trim_end_matches('Z'))));
    }
    s.push_str("</office:meta></office:document-meta>\n");
    s
}

/// The sections of a flow: each one's setup and the top-level blocks it holds. A paragraph that
/// carries a section's properties ends it; the document's last section follows the last such
/// paragraph (and is left out when nothing follows it, unless it is the only one).
fn flow_sections(blocks: &[FBlock], last: &SectionProps) -> Vec<(SectionProps, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut from = 0;
    for (i, b) in blocks.iter().enumerate() {
        if let FBlock::Para(p) = b
            && let Some(s) = &p.section
        {
            out.push(((**s).clone(), from..i + 1));
            from = i + 1;
        }
    }
    if from < blocks.len() || out.is_empty() {
        out.push((last.clone(), from..blocks.len()));
    }
    out
}

/// Write an `.odt` package.
pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    let flow = model::from_doc_with(
        doc,
        model::FlowOpts { tracked: true, fields: true, notes: true, comments: true, lists: true, chars: true, paras: true },
    );
    let mut w = Writer::default();
    let mut body = String::new();
    let sections = flow_sections(&flow.blocks, &doc.last_section);
    // One master page per section that starts a page; the first one is "Standard".
    let mut pages: Vec<(String, SectionProps)> = Vec::new();
    // The columns of the page in force: a continuous section with others is a `text:section`.
    let mut page_cols = Columns::default();
    for (k, (sec, range)) in sections.iter().enumerate() {
        let new_page = k == 0 || sec.start != SectionStart::Continuous;
        if new_page {
            let name = if k == 0 { "Standard".to_string() } else { format!("Page{}", pages.len() + 1) };
            pages.push((name.clone(), sec.clone()));
            page_cols = sec.columns.clone();
            // The first section needs its paragraph to name the page only to restart numbering.
            if k > 0 || sec.page_num_start.is_some() {
                w.pending_start = Some((name, sec.page_num_start));
            }
        }
        // A continuous section whose columns differ from the page's is a `text:section` (§5.4)
        // with them (one column included). A restart of page numbering isn't written for it: ODF
        // applies `style:page-number` only together with a master page.
        let wrapped = k > 0
            && sec.start == SectionStart::Continuous
            && (sec.columns.count != page_cols.count || (sec.columns.count > 1 && sec.columns != page_cols));
        // A list cut at the boundary goes on numbering in the new section.
        let list_at = |b: Option<&FBlock>| matches!(b, Some(FBlock::Para(p)) if p.list.is_some());
        w.continue_list = k > 0 && list_at(flow.blocks.get(range.start)) && list_at(range.start.checked_sub(1).and_then(|i| flow.blocks.get(i)));
        if wrapped {
            w.close_ins(&mut body);
            let n = w.section_styles.len() + 1;
            w.section_styles.push(format!(
                "<style:style style:name=\"Sect{n}\" style:family=\"section\"><style:section-properties>{}</style:section-properties></style:style>",
                columns_xml(&sec.columns)
            ));
            body.push_str(&format!("<text:section text:style-name=\"Sect{n}\" text:name=\"Section{n}\">"));
        }
        w.blocks(flow.blocks.get(range.clone()).unwrap_or(&[]), &mut body, 0);
        w.pending_start = None;
        w.continue_list = false;
        if wrapped {
            w.close_ins(&mut body);
            body.push_str("</text:section>\n");
        }
    }
    w.close_ins(&mut body);
    let tracked = if w.regions.is_empty() && !doc.settings.track_changes {
        String::new()
    } else {
        format!("<text:tracked-changes text:track-changes=\"{}\">{}</text:tracked-changes>\n", doc.settings.track_changes, w.regions.concat())
    };
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}>{}<office:body><office:text>\n{tracked}{body}</office:text></office:body></office:document-content>\n",
        w.automatic_styles()
    );
    let body_font = doc.styles.default_chr.font.clone().unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string());
    let normal = doc.styles.resolve_para(&Default::default());
    let defaults = model::Spacing {
        before: Some(normal.space_before),
        after: Some(normal.space_after),
        line: Some(normal.line_spacing),
        left: Some(normal.indent_left),
        right: Some(normal.indent_right),
        first: Some(normal.indent_first),
    };
    let mut hw = Writer::with_prefix("M");
    let parts: Vec<PageParts> = pages.iter().map(|(_, sec)| page_parts(doc, sec, &mut hw)).collect();
    let kern = doc.styles.default_chr.kern.is_some_and(|k| k > 0.0);
    let styles = styles_xml(&pages, &parts, &hw.style_defs(), &body_font, &defaults, kern, doc.settings.default_tab.clamp(1.0, 1584.0));
    w.pictures.append(&mut hw.pictures);
    let meta = meta_xml(&flow.meta);
    let mut manifest = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\n<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/>\n",
    );
    for (name, _, mime) in &w.pictures {
        manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{mime}\"/>\n", x(name)));
    }
    manifest.push_str("</manifest:manifest>\n");

    let err = |e: zip::result::ZipError| format!("ODT: {e}");
    let ioerr = |e: std::io::Error| format!("ODT: {e}");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).map_err(err)?;
    z.write_all(b"application/vnd.oasis.opendocument.text").map_err(ioerr)?;
    for (name, data) in [
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("meta.xml", meta.as_bytes()),
        ("settings.xml", SETTINGS.as_bytes()),
    ] {
        z.start_file(name, deflated).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    for (name, data, _) in &w.pictures {
        z.start_file(name.as_str(), stored).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    let cur = z.finish().map_err(err)?;
    Ok(cur.into_inner())
}

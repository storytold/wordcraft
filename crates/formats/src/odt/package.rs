use super::writer::{NS, Writer, align_attr, props_text, pt, spacing_props, x};
use super::{Columns, Cursor, Document, FBlock, Meta, SectionProps, SectionStart, Write, model};

/// `style:columns` (ODF 1.3 §17.12) of equal columns with their gap and, when on, the line between.
fn columns_xml(c: &Columns) -> String {
    let sep = if c.separator { "<style:column-sep style:width=\"0.5pt\" style:style=\"solid\" style:color=\"#000000\"/>" } else { "" };
    format!("<style:columns fo:column-count=\"{}\" fo:column-gap=\"{}\">{sep}</style:columns>", c.count.clamp(1, 45), pt(c.space))
}

/// One master page per entry of `pages` (name, setup), each with its own page layout `pm1`, `pm2`…
/// (§16.5, §16.9); the first is the default one, "Standard".
/// The styles WordCraft's paragraph kinds are written in (Heading 1–6, Title, Quotations,
/// Preformatted Text), each with the look of the document's own style: font, size, bold,
/// italic, colour and the rest of its text formatting, spacing, indents, alignment and keep with
/// next, written out in full so other programs show them as WordCraft does.
fn kind_styles(doc: &Document) -> String {
    let mut s = String::new();
    let mut kinds: Vec<(String, String, String, Option<u8>, &str)> =
        (1..=6).map(|n| (format!("Heading_20_{n}"), format!("Heading {n}"), format!("Heading{n}"), Some(n), "text")).collect();
    kinds.push(("Title".into(), "Title".into(), "Title".into(), None, "chapter"));
    kinds.push(("Quotations".into(), "Quotations".into(), "Quote".into(), None, "html"));
    kinds.push(("Preformatted_20_Text".into(), "Preformatted Text".into(), model::CODE_STYLE.into(), None, "html"));
    for (name, display, id, outline, class) in kinds {
        let (pp, cp) = match doc.styles.get(&id) {
            Some(_) => doc.styles.para_style_props(&id),
            // Preformatted text in a document without its style: monospaced, no spacing.
            None => (
                wordcraft_doc::props::ParaProps { space_before: Some(0.0), space_after: Some(0.0), ..Default::default() },
                wordcraft_doc::props::CharProps { font: Some(model::MONO_FONT.into()), size: Some(10.0), ..Default::default() },
            ),
        };
        let mut para = spacing_props(&model::Spacing {
            before: pp.space_before,
            after: pp.space_after,
            line: pp.line_spacing,
            // Zero indents are the defaults; stating them would override the indents of the
            // heading numbering in LibreOffice (its number then sits against the text).
            left: pp.indent_left.filter(|v| *v != 0.0),
            right: pp.indent_right.filter(|v| *v != 0.0),
            first: pp.indent_first.filter(|v| *v != 0.0),
        });
        para.push_str(align_attr(pp.align));
        if pp.keep_next == Some(true) {
            para.push_str(" fo:keep-with-next=\"always\"");
        }
        if let Some(c) = pp.contextual_spacing {
            para.push_str(&format!(" style:contextual-spacing=\"{c}\""));
        }
        // A heading level the outline doesn't number names no list style: LibreOffice would
        // otherwise give it the outline's (unnumbered) level, and its indent.
        let numbered = pp
            .numbering
            .and_then(|r| doc.numbering.level(r.num, r.level))
            .is_some_and(|l| !matches!(l.format, wordcraft_doc::section::NumFormat::None | wordcraft_doc::section::NumFormat::Bullet));
        let detach = if outline.is_some() && !numbered { " style:list-style-name=\"\"" } else { "" };
        let outline = outline.map(|n| format!(" style:default-outline-level=\"{n}\"{detach}")).unwrap_or_default();
        s.push_str(&format!(
            "<style:style style:name=\"{name}\" style:display-name=\"{display}\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:next-style-name=\"Standard\"{outline} style:class=\"{class}\"><style:paragraph-properties{para}/><style:text-properties{}/></style:style>",
            props_text(&cp)
        ));
    }
    s
}

/// The ODF parent of document style `id`: Standard for Normal, the kinds' styles for theirs, a
/// custom style by its id.
fn odf_parent(doc: &Document, id: Option<&str>) -> String {
    match id {
        None | Some("Normal") => "Standard".into(),
        Some("Title") => "Title".into(),
        Some("Quote") => "Quotations".into(),
        Some(id) if id == model::CODE_STYLE => "Preformatted_20_Text".into(),
        Some(id) => match id.strip_prefix("Heading").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n)) {
            Some(n) => format!("Heading_20_{}", n.min(6)),
            None if model::is_custom_style(doc, id) => super::writer::odf_name(id),
            None => "Standard".into(),
        },
    }
}

/// The custom paragraph styles in `named` and the custom styles they are based on, as common
/// styles (ODF 1.3 §16.2): named by their document id, shown by their name, each with its own
/// paragraph and text formatting over its parent's.
fn custom_styles(doc: &Document, named: &std::collections::BTreeSet<String>) -> String {
    let mut todo: Vec<String> = named.iter().cloned().collect();
    let mut done = std::collections::BTreeSet::new();
    let mut s = String::new();
    while let Some(id) = todo.pop() {
        if !done.insert(id.clone()) || done.len() > 10_000 {
            continue;
        }
        let Some(st) = doc.styles.get(&id) else { continue };
        if let Some(b) = st.based_on.as_deref().filter(|b| model::is_custom_style(doc, b)) {
            todo.push(b.to_string());
        }
        let pp = &st.para;
        let mut attrs = spacing_props(&model::Spacing {
            before: pp.space_before,
            after: pp.space_after,
            line: pp.line_spacing,
            left: pp.indent_left,
            right: pp.indent_right,
            first: pp.indent_first,
        });
        attrs.push_str(align_attr(pp.align));
        if pp.keep_next == Some(true) {
            attrs.push_str(" fo:keep-with-next=\"always\"");
        }
        if let Some(c) = pp.contextual_spacing {
            attrs.push_str(&format!(" style:contextual-spacing=\"{c}\""));
        }
        if pp.bidi == Some(true) {
            attrs.push_str(" style:writing-mode=\"rl-tb\"");
        }
        attrs.push_str(&super::writer::border_attrs(pp.borders));
        if let Some(bg) = pp.shading {
            attrs.push_str(&format!(" fo:background-color=\"#{}\"", bg.hex().to_ascii_lowercase()));
        }
        let tabs = pp.tabs.as_deref().map(super::writer::tab_stops).unwrap_or_default();
        let parent = odf_parent(doc, st.based_on.as_deref());
        s.push_str(&format!(
            "<style:style style:name=\"{}\" style:display-name=\"{}\" style:family=\"paragraph\" style:parent-style-name=\"{parent}\"><style:paragraph-properties{attrs}>{tabs}</style:paragraph-properties><style:text-properties{}/></style:style>",
            super::writer::odf_name(&id),
            x(&super::writer::generated_style(&id).map_or_else(|| st.name.clone(), |(_, d)| d)),
            props_text(&st.chr)
        ));
    }
    s
}

/// The custom character styles in `named` and those they are based on, as common text styles.
fn char_styles(doc: &Document, named: &std::collections::BTreeSet<String>) -> String {
    let mut todo: Vec<String> = named.iter().cloned().collect();
    let mut done = std::collections::BTreeSet::new();
    let mut s = String::new();
    while let Some(id) = todo.pop() {
        if !done.insert(id.clone()) || done.len() > 10_000 {
            continue;
        }
        let Some(st) = doc.styles.get(&id) else { continue };
        let parent = match st.based_on.as_deref().filter(|b| model::is_custom_char_style(doc, b)) {
            Some(b) => {
                todo.push(b.to_string());
                format!(" style:parent-style-name=\"{}\"", super::writer::odf_name(b))
            }
            None => String::new(),
        };
        s.push_str(&format!(
            "<style:style style:name=\"{}\" style:display-name=\"{}\" style:family=\"text\"{parent}><style:text-properties{}/></style:style>",
            super::writer::odf_name(&id),
            x(&st.name),
            props_text(&st.chr)
        ));
    }
    s
}

/// `text:outline-style` (ODF 1.3 §16.34): the numbering of Heading 1–6, from the list level each
/// heading style numbers its paragraphs with; levels without one show no number.
fn outline_style(doc: &Document) -> String {
    let mut s = String::from("<text:outline-style style:name=\"Outline\">");
    for n in 1..=10u8 {
        let level = (n <= 6)
            .then(|| doc.styles.para_style_props(&format!("Heading{n}")).0.numbering)
            .flatten()
            .and_then(|r| doc.numbering.level(r.num, r.level).cloned().map(|l| (r.level, l)));
        let (indent, hanging) = level.as_ref().map(|(_, l)| (l.indent, l.hanging)).unwrap_or((0.0, 0.0));
        let attrs = match level {
            Some((lv, l)) if !matches!(l.format, wordcraft_doc::section::NumFormat::None | wordcraft_doc::section::NumFormat::Bullet) => {
                let label = model::Label::of_level(l.format, &l.text);
                let mut a = format!(" style:num-format=\"{}\"", super::writer::odf_num_format(l.format));
                for (k, v) in [("style:num-prefix", label.prefix.as_str()), ("style:num-suffix", label.suffix.as_str())] {
                    if !v.is_empty() {
                        a.push_str(&format!(" {k}=\"{}\"", x(v)));
                    }
                }
                if label.shown > 1 {
                    a.push_str(&format!(" text:display-levels=\"{}\"", label.shown.min(lv.saturating_add(1))));
                }
                if l.start != 1 {
                    a.push_str(&format!(" text:start-value=\"{}\"", l.start));
                }
                a
            }
            _ => " style:num-format=\"\"".into(),
        };
        // The number at the level's indent, followed by a tab (to the next default tab stop
        // when the level has no indent), as Word lays out numbered headings.
        let fit = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
        // A level with no indent states none, so LibreOffice uses its own (a tab after the number).
        let place = if fit(indent) > 0.0 || fit(hanging) > 0.0 {
            format!(
                " text:list-tab-stop-position=\"{}\" fo:text-indent=\"{}\" fo:margin-left=\"{}\"",
                pt(fit(indent)),
                pt(-fit(hanging)),
                pt(fit(indent))
            )
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<text:outline-level-style text:level=\"{n}\"{attrs}><style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\"{place}/></style:list-level-properties></text:outline-level-style>"
        ));
    }
    s.push_str("</text:outline-style>");
    s
}

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

/// `styles.xml` of `doc`: the default style from the document defaults (Normal's spacing, the
/// body font, pair kerning, the tab interval), the paragraph kinds' styles, and one master page
/// per entry of `pages` with `parts`, its headers and footers, whose automatic styles are
/// `part_styles`.
fn styles_xml(
    doc: &Document,
    pages: &[(String, wordcraft_doc::SectionProps)],
    parts: &[PageParts],
    part_styles: &str,
    named: &std::collections::BTreeSet<String>,
) -> String {
    // The document defaults: Normal's font, spacing and indents, pair kerning and tab interval.
    let body_font = doc.styles.default_chr.font.clone().unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string());
    let normal = doc.styles.resolve_para(&Default::default());
    let defaults = &model::Spacing {
        before: Some(normal.space_before),
        after: Some(normal.space_after),
        line: Some(normal.line_spacing),
        left: Some(normal.indent_left),
        right: Some(normal.indent_right),
        first: Some(normal.indent_first),
    };
    let kern = doc.styles.default_chr.kern.is_some_and(|k| k > 0.0);
    let tab = doc.settings.default_tab.clamp(1.0, 1584.0);
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles>");
    s.push_str(&format!(
        "<style:default-style style:family=\"paragraph\"><style:paragraph-properties{} style:tab-stop-distance=\"{tab}\"/><style:text-properties fo:font-family=\"'{}'\" fo:font-size=\"12pt\" style:letter-kerning=\"{kern}\"{scripts}/></style:default-style>",
        spacing_props(defaults),
        x(&body_font),
        tab = pt(tab),
        scripts = super::writer::script_attrs(&doc.styles.default_chr)
    ));
    s.push_str("<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>");
    s.push_str(&kind_styles(doc));
    s.push_str(&outline_style(doc));
    s.push_str(&custom_styles(doc, named));
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

/// `Document::passthrough` key of an ODT's own `settings.xml`, kept for the next save.
pub(super) const SETTINGS_PART: &str = "odt/settings.xml";

/// Largest `settings.xml` kept, in bytes.
pub(super) const MAX_SETTINGS: usize = 1 << 20;

/// LibreOffice's layout compatibility switches (`ooo:configuration-settings`) for a document
/// laid out as Word lays it out, as LibreOffice sets them for Word files.
const WORD_COMPAT: &[(&str, bool)] = &[
    ("NoNumberingShowFollowBy", true),
    ("HyphenateURLs", false),
    ("FootnoteInColumnToPageEnd", true),
    ("GutterAtTop", false),
    ("ContinuousEndnotes", false),
    ("FrameAutowidthWithMorePara", true),
    ("SubtractFlysAnchoredAtFlys", false),
    ("SurroundTextWrapSmall", true),
    ("TreatSingleColumnBreakAsPageBreak", true),
    ("TabOverSpacing", true),
    ("AutoFirstLineIndentDisregardLineSpace", true),
    ("TabOverMargin", false),
    ("ApplyTextAttrToEmptyLineAtEndOfParagraph", true),
    ("DisableOffPagePositioning", true),
    ("BackgroundParaOverDrawings", true),
    ("TabOverflow", true),
    ("AddVerticalFrameOffsets", true),
    ("AddFrameOffsets", false),
    ("TableRowKeep", true),
    ("ApplyParagraphMarkFormatToNumbering", true),
    ("JustifyLinesWithShrinking", true),
    ("CollapseEmptyCellPara", true),
    ("UnbreakableNumberings", true),
    ("MsWordCompTrailingBlanks", true),
    ("AddParaTableSpacing", false),
    ("OutlineLevelYieldsNumbering", false),
    ("UseOldNumbering", false),
    ("AddParaSpacingToTableCells", true),
    ("PropLineSpacingShrinksFirstLine", true),
    ("TabAtLeftIndentForParagraphsInList", true),
    ("DoNotCaptureDrawObjsOnPage", false),
    ("HeaderSpacingBelowLastPara", true),
    ("ClippedPictures", true),
    ("MathBaselineAlignment", true),
    ("AlignTabStopPosition", true),
    ("UseFormerLineSpacing", false),
    ("DoNotMirrorRtlDrawObjs", true),
    ("AddParaLineSpacingToTableCells", true),
    ("IsKernAsianPunctuation", false),
    ("DoNotJustifyLinesWithManualBreak", false),
    ("UseVariableWidthNBSP", false),
    ("AddParaTableSpacingAtStart", true),
    ("InvertBorderSpacing", true),
    ("AddExternalLeading", true),
    ("UseFormerObjectPositioning", false),
    ("UnxForceZeroExtLeading", false),
    ("UseFormerTextWrapping", false),
    ("ConsiderTextWrapOnObjPos", true),
    ("StylesNoDefault", false),
    ("IgnoreFirstLineIndentInNumbering", false),
    ("NoGapAfterNoteNumber", true),
    ("EmptyDbFieldHidesPara", false),
    ("DoNotResetParaAttrsForNumFont", false),
    ("IgnoreTabsAndBlanksForLineCalculation", true),
    ("ClipAsCharacterAnchoredWriterFlyFrames", false),
    ("DropCapPunctuation", true),
    ("UseOldPrinterMetrics", false),
    ("MsWordCompMinLineHeightByFly", false),
];

/// `settings.xml` (ODF 1.3 §3.10) for `doc`: an ODT's own, kept from opening it, or LibreOffice's
/// switches for Word's layout. Either way tab stops are measured from the margin, as WordCraft
/// writes them (LibreOffice's own default measures them from the paragraph's indent).
fn settings_xml(doc: &Document) -> String {
    const TABS: &str = "<config:config-item config:name=\"TabsRelativeToIndent\" config:type=\"boolean\">false</config:config-item>";
    if let Some(own) = doc.passthrough.get(SETTINGS_PART).filter(|b| b.len() <= MAX_SETTINGS).and_then(|b| std::str::from_utf8(b).ok()) {
        let set = "config:name=\"ooo:configuration-settings\"";
        let tabs = "config:name=\"TabsRelativeToIndent\"";
        if let Some(at) = own.find(tabs) {
            // The file's item, its value made false.
            let rest = own.get(at..).unwrap_or("");
            if let (Some(gt), Some(lt)) = (rest.find('>'), rest.find("</config:config-item>"))
                && gt < lt
            {
                return format!("{}false{}", own.get(..at + gt + 1).unwrap_or(""), own.get(at + lt..).unwrap_or(""));
            }
        } else if let Some(at) = own.find(set).and_then(|at| own.get(at..)?.find('>').map(|gt| at + gt + 1)) {
            return format!("{}{TABS}{}", own.get(..at).unwrap_or(""), own.get(at..).unwrap_or(""));
        }
    }
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" office:version=\"1.3\"><office:settings><config:config-item-set config:name=\"ooo:configuration-settings\">",
    );
    s.push_str(TABS);
    for (name, on) in WORD_COMPAT {
        s.push_str(&format!("<config:config-item config:name=\"{name}\" config:type=\"boolean\">{on}</config:config-item>"));
    }
    s.push_str("</config:config-item-set></office:settings></office:document-settings>\n");
    s
}

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
        model::FlowOpts { tracked: true, fields: true, notes: true, comments: true, lists: true, chars: true, paras: true, boxes: true },
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
    let mut hw = Writer::with_prefix("M");
    let parts: Vec<PageParts> = pages.iter().map(|(_, sec)| page_parts(doc, sec, &mut hw)).collect();
    let mut named = std::mem::take(&mut w.named);
    named.append(&mut hw.named);
    let mut named_text = std::mem::take(&mut w.named_text);
    named_text.append(&mut hw.named_text);
    let named_text_xml = char_styles(doc, &named_text);
    // The dashes, gradients and arrowheads graphic styles name (§16.39–16.41).
    let mut draw_defs = std::mem::take(&mut w.draw_defs.defs);
    draw_defs.append(&mut hw.draw_defs.defs);
    let mut draw_xml: String = draw_defs.into_values().collect();
    // The plain character style index entries' links are in (LibreOffice's Index Link).
    if w.indexes + hw.indexes > 0 {
        draw_xml.push_str("<style:style style:name=\"Index_20_Link\" style:display-name=\"Index Link\" style:family=\"text\"/>");
    }
    let styles = styles_xml(doc, &pages, &parts, &hw.style_defs(), &named).replacen(
        "</office:styles>",
        &format!("{draw_xml}{named_text_xml}</office:styles>"),
        1,
    );
    w.pictures.append(&mut hw.pictures);
    w.formulas.append(&mut hw.formulas);
    let meta = meta_xml(&flow.meta);
    let mut manifest = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\n<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/>\n",
    );
    for (name, _, mime) in &w.pictures {
        manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{mime}\"/>\n", x(name)));
    }
    for (dir, _) in &w.formulas {
        manifest.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"{0}/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.formula\"/>\n<manifest:file-entry manifest:full-path=\"{0}/content.xml\" manifest:media-type=\"text/xml\"/>\n",
            x(dir)
        ));
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
        ("settings.xml", settings_xml(doc).as_bytes()),
    ] {
        z.start_file(name, deflated).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    for (dir, xml) in &w.formulas {
        z.start_file(format!("{dir}/content.xml"), deflated).map_err(err)?;
        z.write_all(xml.as_bytes()).map_err(ioerr)?;
    }
    for (name, data, _) in &w.pictures {
        z.start_file(name.as_str(), stored).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    let cur = z.finish().map_err(err)?;
    Ok(cur.into_inner())
}

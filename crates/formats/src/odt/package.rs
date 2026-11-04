use super::writer::{NS, Writer, align_attr, props_text, pt, spacing_props, x};
use super::{Columns, Cursor, Document, FBlock, Meta, SectionProps, SectionStart, Write, model};

/// `style:columns` (ODF 1.3 §17.12) of equal columns with their gap and, when on, the line between.
fn columns_xml(c: &Columns) -> String {
    let sep = if c.separator { "<style:column-sep style:width=\"0.5pt\" style:style=\"solid\" style:color=\"#000000\"/>" } else { "" };
    let count = c.count.clamp(1, 45);
    // Unequal columns (§17.12): each one's share of the width with the half gaps beside it as
    // its indents, as LibreOffice writes them.
    let mut each = String::new();
    if c.widths.len() == count as usize && count > 1 {
        let clean = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
        let mut start = 0.0;
        for (w, after) in &c.widths {
            let end = clean(*after) / 2.0;
            let rel = ((clean(*w) + start + end) * 100.0).round().max(1.0) as u32;
            each.push_str(&format!("<style:column style:rel-width=\"{rel}*\" fo:start-indent=\"{}\" fo:end-indent=\"{}\"/>", pt(start), pt(end)));
            start = end;
        }
    }
    // LibreOffice ignores a page's column widths beside a gap: the gaps are in the indents.
    let gap = if each.is_empty() { format!(" fo:column-gap=\"{}\"", pt(c.space)) } else { String::new() };
    format!("<style:columns fo:column-count=\"{count}\"{gap}>{sep}{each}</style:columns>")
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
            ..Default::default()
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
            ..Default::default()
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
        let tabs = pp
            .tabs
            .as_deref()
            .map(|tabs| {
                // From the style's indent where the document measures them so (as they are read).
                let indent = if model::tabs_from_indent(doc) { pp.indent_left.unwrap_or(0.0) } else { 0.0 };
                let tabs: Vec<_> =
                    tabs.iter().map(|t| wordcraft_doc::props::TabStop { pos: (t.pos - indent).clamp(-1584.0, 1584.0), ..*t }).collect();
                super::writer::tab_stops(&tabs)
            })
            .unwrap_or_default();
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
    // A watermark is a shape in every header (one written for it alone when there is none).
    let wm = doc.settings.watermark.as_ref().filter(|w| !w.text.trim().is_empty());
    let area = (sec.page_w - sec.margin_left - sec.margin_right, sec.page_h - sec.margin_top - sec.margin_bottom);
    for (set, name) in [(&sec.headers, "header"), (&sec.footers, "footer")] {
        let wanted =
            set.default.is_some() || (even && set.even.is_some()) || (sec.title_page && set.first.is_some()) || (name == "header" && wm.is_some());
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
                    // Pictures, shapes, text boxes, charts and objects too, as in the body.
                    model::FlowOpts { fields: true, lists: true, chars: true, paras: true, boxes: true, ..Default::default() },
                );
                w.blocks(&blocks, &mut body, 0);
            }
            if body.is_empty() {
                body.push_str("<text:p/>");
            }
            if name == "header"
                && let Some(wm) = wm
            {
                body = with_watermark(&body, &w.watermark(wm, (sec.page_w, sec.page_h), area));
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

/// A header's content with `shape` (anchored to a character) at the start of its first
/// paragraph, or in a paragraph of its own before a table that starts it.
fn with_watermark(body: &str, shape: &str) -> String {
    if let Some(rest) = body.strip_prefix("<text:p/>") {
        return format!("<text:p>{shape}</text:p>{rest}");
    }
    if body.starts_with("<text:p") || body.starts_with("<text:h") {
        // Into the first paragraph's element, after its start tag (or in place of an empty one).
        if let Some(end) = body.find('>') {
            let (tag, rest) = body.split_at(end + 1);
            if let Some(open) = tag.strip_suffix("/>") {
                let name = if tag.starts_with("<text:h") { "text:h" } else { "text:p" };
                return format!("{open}>{shape}</{name}>{rest}");
            }
            return format!("{tag}{shape}{rest}");
        }
    }
    format!("<text:p>{shape}</text:p>{body}")
}

/// `styles.xml` of `doc`: the default style/// `styles.xml` of `doc`: the default style from the document defaults (Normal's spacing, the
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
    // WordCraft's page layouts take another prefix next to the file's own (see `super::kept`).
    let pm = if doc.kept.of("odt").is_some() { "wcpm" } else { "pm" };
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
        ..Default::default()
    };
    let kern = doc.styles.default_chr.kern.is_some_and(|k| k > 0.0);
    // Normal's font size (Standard, written empty, takes the default style's).
    let size = doc.styles.resolve_char(None, &Default::default()).size;
    let size = if size.is_finite() { size.clamp(1.0, 1638.0) } else { 12.0 };
    let tab = doc.settings.default_tab.clamp(1.0, 1584.0);
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles>");
    s.push_str(&format!(
        "<style:default-style style:family=\"paragraph\"><style:paragraph-properties{}{widows} style:tab-stop-distance=\"{tab}\"/><style:text-properties fo:font-family=\"'{}'\" fo:font-size=\"{size}\" style:letter-kerning=\"{kern}\"{color}{scripts}/></style:default-style>",
        spacing_props(defaults),
        x(&body_font),
        // Widow and orphan control as Normal has it, said either way (the standard leaves it open
        // when unsaid: see `OdfProfile::widow_control`).
        widows = if normal.widow_control { " fo:widows=\"2\" fo:orphans=\"2\"" } else { " fo:widows=\"0\" fo:orphans=\"0\"" },
        size = pt(size),
        // The document's text colour (a colour, not automatic).
        color = match doc.styles.default_chr.color {
            Some(wordcraft_doc::props::TextColor::Rgb(c)) => format!(" fo:color=\"#{}\"", c.hex().to_ascii_lowercase()),
            _ => String::new(),
        },
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
    s.push_str(&notes_configuration(doc));
    if let Some(l) = pages.first().and_then(|(_, sec)| sec.line_numbers.as_ref()) {
        s.push_str(&line_numbering_xml(l));
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
        // A page border is at ODF's margin, the text (or header, footer) inside its padding.
        let (borders, [top, bottom, left, right]) = page_borders(sec.page_borders, [top, bottom, sec.margin_left, sec.margin_right]);
        // The band's height takes in its spacing (see the reader), which content taller than
        // the band keeps below it.
        let gap = |g: f32, h: f32| if g.is_finite() { g.clamp(0.0, h) } else { 0.0 };
        // Spacing kept below a taller header says so, as the standard leaves the default open.
        let keeps = |g: f32| if g.is_finite() && g > 0.0 { " style:dynamic-spacing=\"false\"" } else { "" };
        let header = if pp.header {
            format!(
                "<style:header-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-bottom=\"{}\"{}/></style:header-style>",
                pt(header_h),
                pt(gap(sec.header_gap, header_h)),
                keeps(sec.header_gap)
            )
        } else {
            String::new()
        };
        let footer = if pp.footer {
            format!(
                "<style:footer-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-top=\"{}\"{}/></style:footer-style>",
                pt(footer_h),
                pt(gap(sec.footer_gap, footer_h)),
                keeps(sec.footer_gap)
            )
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<style:page-layout style:name=\"{pm}{}\"><style:page-layout-properties fo:page-width=\"{}\" fo:page-height=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" style:print-orientation=\"{}\"{}>{cols}</style:page-layout-properties>{header}{footer}</style:page-layout>",
            i + 1,
            pt(sec.page_w),
            pt(sec.page_h),
            pt(top),
            pt(bottom),
            pt(left),
            pt(right),
            if sec.page_w > sec.page_h { "landscape" } else { "portrait" },
            page_look(doc, sec) + &borders
        ));
    }
    s.push_str("</office:automatic-styles><office:master-styles>");
    for (i, (name, _)) in pages.iter().enumerate() {
        let xml = parts.get(i).map(|p| p.xml.as_str()).unwrap_or("");
        s.push_str(&format!("<style:master-page style:name=\"{}\" style:page-layout-name=\"{pm}{}\">{xml}</style:master-page>", x(name), i + 1));
    }
    s.push_str("</office:master-styles></office:document-styles>\n");
    s
}

/// `Document::passthrough` key of an ODT's own `settings.xml`, kept for the next save.
pub(super) const SETTINGS_PART: &str = "odt/settings.xml";

/// Largest `settings.xml` kept, in bytes.
pub(super) const MAX_SETTINGS: usize = 1 << 20;

/// LibreOffice's layout settings for a new document (observed in what LibreOffice 24 saves):
/// written in full, because LibreOffice gives any it doesn't find in a document's set its old,
/// pre-OpenOffice value (observed), which changes how the document is laid out.
const LO_DEFAULTS: &[(&str, bool)] = &[
    ("NoNumberingShowFollowBy", false),
    ("HyphenateURLs", false),
    ("FootnoteInColumnToPageEnd", true),
    ("GutterAtTop", false),
    ("ContinuousEndnotes", false),
    ("FrameAutowidthWithMorePara", false),
    ("SubtractFlysAnchoredAtFlys", false),
    ("SurroundTextWrapSmall", false),
    ("TreatSingleColumnBreakAsPageBreak", false),
    ("TabOverSpacing", false),
    ("AutoFirstLineIndentDisregardLineSpace", true),
    ("TabOverMargin", false),
    ("ApplyTextAttrToEmptyLineAtEndOfParagraph", true),
    ("DisableOffPagePositioning", false),
    ("BackgroundParaOverDrawings", false),
    ("TabOverflow", true),
    ("AddVerticalFrameOffsets", false),
    ("AddFrameOffsets", false),
    ("TableRowKeep", false),
    ("ApplyParagraphMarkFormatToNumbering", false),
    ("JustifyLinesWithShrinking", false),
    ("CollapseEmptyCellPara", true),
    ("UnbreakableNumberings", false),
    ("MsWordCompTrailingBlanks", false),
    ("AddParaTableSpacing", true),
    ("OutlineLevelYieldsNumbering", false),
    ("UseOldNumbering", false),
    ("AddParaSpacingToTableCells", true),
    ("PropLineSpacingShrinksFirstLine", true),
    ("TabAtLeftIndentForParagraphsInList", false),
    ("DoNotCaptureDrawObjsOnPage", false),
    ("HeaderSpacingBelowLastPara", false),
    ("ClippedPictures", false),
    ("MathBaselineAlignment", true),
    ("AlignTabStopPosition", true),
    ("UseFormerLineSpacing", false),
    ("AddParaLineSpacingToTableCells", true),
    ("IsKernAsianPunctuation", false),
    ("DoNotJustifyLinesWithManualBreak", false),
    ("UseVariableWidthNBSP", false),
    ("AddParaTableSpacingAtStart", true),
    ("InvertBorderSpacing", false),
    ("AddExternalLeading", true),
    ("UseFormerObjectPositioning", false),
    ("UnxForceZeroExtLeading", false),
    ("UseFormerTextWrapping", false),
    ("ConsiderTextWrapOnObjPos", false),
    ("StylesNoDefault", false),
    ("IgnoreFirstLineIndentInNumbering", false),
    ("NoGapAfterNoteNumber", false),
    ("EmptyDbFieldHidesPara", true),
    ("DoNotResetParaAttrsForNumFont", false),
    ("IgnoreTabsAndBlanksForLineCalculation", false),
    ("ClipAsCharacterAnchoredWriterFlyFrames", false),
    ("DropCapPunctuation", true),
    ("UseOldPrinterMetrics", false),
    ("TabsRelativeToIndent", true),
    ("MsWordCompMinLineHeightByFly", false),
    ("SmallCapsPercentage66", false),
];

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
/// `settings.xml`: LibreOffice's layout settings for a document laid out as WordCraft lays it
/// out. The set's name, `ooo:configuration-settings`, is a qualified name: without the `ooo`
/// namespace declared LibreOffice ignores the whole set (observed).
fn settings_xml(doc: &Document) -> String {
    // What WordCraft's own layout of the document needs LibreOffice to do the same: tab stops
    // from the margin (as written), and the space between paragraphs as WordCraft lays it out
    // (ODF leaves it open; LibreOffice's `AddParaTableSpacing` false takes the larger of the two).
    let ours = [("TabsRelativeToIndent", model::tabs_from_indent(doc)), ("AddParaTableSpacing", doc.settings.sum_paragraph_spacing)];
    let item = |name: &str, on: bool| format!("<config:config-item config:name=\"{name}\" config:type=\"boolean\">{on}</config:config-item>");
    if let Some(own) = doc.passthrough.get(SETTINGS_PART).filter(|b| b.len() <= MAX_SETTINGS).and_then(|b| std::str::from_utf8(b).ok()) {
        let mut out = own.to_string();
        let mut ok = true;
        for (name, on) in ours {
            match with_setting(&out, name, on, &item(name, on)) {
                Some(next) => out = next,
                None => ok = false,
            }
        }
        if ok {
            return out;
        }
    }
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" xmlns:ooo=\"http://openoffice.org/2004/office\" office:version=\"1.3\"><office:settings><config:config-item-set config:name=\"ooo:configuration-settings\">",
    );
    // LibreOffice's defaults; Word's layout rules for a document WordCraft lays out as Word does
    // (one opened from ODF is laid out by ODF's rules, `Settings::odf`, as LibreOffice's
    // defaults have it); and what WordCraft's layout of this document needs.
    let mut all: Vec<(&str, bool)> = LO_DEFAULTS.to_vec();
    let word = if doc.settings.odf.is_none() { WORD_COMPAT } else { &[] };
    for (name, on) in word.iter().copied().chain(ours) {
        match all.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = on,
            None => all.push((name, on)),
        }
    }
    for (name, on) in all {
        s.push_str(&item(name, on));
    }
    // Metrics independent of any printer, as LibreOffice's own documents.
    s.push_str("<config:config-item config:name=\"PrinterIndependentLayout\" config:type=\"string\">high-resolution</config:config-item>");
    s.push_str("</config:config-item-set></office:settings></office:document-settings>\n");
    s
}

/// A file's own `settings.xml` (`own`) with LibreOffice's setting `name` made `on`: its item's
/// value changed, or `item` added at the start of the configuration settings. `None` when it has
/// neither.
fn with_setting(own: &str, name: &str, on: bool, item: &str) -> Option<String> {
    let key = format!("config:name=\"{name}\"");
    if let Some(at) = own.find(&key) {
        let rest = own.get(at..)?;
        let gt = rest.find('>')?;
        // A self-closed item (another application's, or a hand-edited file's) is replaced whole:
        // its close tag would be the next item's.
        if rest.get(..gt).is_some_and(|t| t.ends_with('/')) {
            let start = own.get(..at)?.rfind('<')?;
            return Some(format!("{}{item}{}", own.get(..start).unwrap_or(""), own.get(at + gt + 1..).unwrap_or("")));
        }
        let lt = rest.find("</config:config-item>")?;
        return (gt < lt).then(|| format!("{}{on}{}", own.get(..at + gt + 1).unwrap_or(""), own.get(at + lt..).unwrap_or("")));
    }
    let set = "config:name=\"ooo:configuration-settings\"";
    let at = own.find(set).and_then(|at| own.get(at..)?.find('>').map(|gt| at + gt + 1))?;
    Some(format!("{}{item}{}", own.get(..at).unwrap_or(""), own.get(at..).unwrap_or("")))
}

fn meta_xml(m: &Meta, core: &wordcraft_doc::CoreProps, props: &[wordcraft_doc::CustomProp]) -> String {
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
    // Who made it, and who changed it last (§4.3.2.6 `dc:creator`, §4.3.2.10 `meta:initial-creator`).
    let last_by = if core.last_modified_by.trim().is_empty() { m.author.as_str() } else { core.last_modified_by.as_str() };
    if !m.author.is_empty() {
        s.push_str(&format!("<meta:initial-creator>{}</meta:initial-creator>", x(&m.author)));
    }
    if !last_by.is_empty() {
        s.push_str(&format!("<dc:creator>{}</dc:creator>", x(last_by)));
    }
    // When it was last changed (§4.3.2.7 `dc:date`), and how often it was saved (§4.3.2.12
    // `meta:editing-cycles`).
    let modified = core.modified.trim().trim_end_matches(['Z', 'z']);
    if !modified.is_empty() && modified.len() < 64 {
        s.push_str(&format!("<dc:date>{}</dc:date>", x(modified)));
    }
    if core.revision > 0 {
        s.push_str(&format!("<meta:editing-cycles>{}</meta:editing-cycles>", core.revision));
    }
    for k in m.keywords.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        s.push_str(&format!("<meta:keyword>{}</meta:keyword>", x(k)));
    }
    if !m.created.is_empty() {
        s.push_str(&format!("<meta:creation-date>{}</meta:creation-date>", x(m.created.trim_end_matches('Z'))));
    }
    // Custom document properties (§4.3.2.14); ones kept as raw XML from a Word file aren't.
    // (User fields go in the content instead: `user_field_decls`.)
    for p in props.iter().filter(|p| p.kind != "raw" && !p.odf_user_field) {
        let (t, value) = odf_value(p);
        s.push_str(&format!("<meta:user-defined meta:name=\"{}\" meta:value-type=\"{t}\">{}</meta:user-defined>", x(&p.name), x(&value)));
    }
    s.push_str("</office:meta></office:document-meta>\n");
    s
}

/// A custom property's ODF value type and value.
fn odf_value(p: &wordcraft_doc::CustomProp) -> (&'static str, String) {
    let t = match p.kind.as_str() {
        "r4" | "r8" | "i1" | "i2" | "i4" | "i8" | "int" | "ui1" | "ui2" | "ui4" | "ui8" | "uint" | "decimal" | "cy" => "float",
        "bool" => "boolean",
        "filetime" | "date" => "date",
        _ => "string",
    };
    let value = if t == "boolean" { if matches!(p.value.trim(), "true" | "1") { "true" } else { "false" }.to_string() } else { p.value.clone() };
    (t, value)
}

/// The document's user fields (§7.4.7 `text:user-field-decls`), declared at the start of the text.
fn user_field_decls(props: &[wordcraft_doc::CustomProp]) -> String {
    let decls: String = props
        .iter()
        .filter(|p| p.odf_user_field && p.kind != "raw")
        .map(|p| {
            let (t, value) = odf_value(p);
            let attr = match t {
                "float" => "office:value",
                "boolean" => "office:boolean-value",
                "date" => "office:date-value",
                _ => "office:string-value",
            };
            let value = if t == "date" { value.trim_end_matches(['Z', 'z']).to_string() } else { value };
            format!("<text:user-field-decl text:name=\"{}\" office:value-type=\"{t}\" {attr}=\"{}\"/>", x(&p.name), x(&value))
        })
        .collect();
    if decls.is_empty() { String::new() } else { format!("<text:user-field-decls>{decls}</text:user-field-decls>") }
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
/// The kinds of the shapes with drawing ids in `blocks` (paragraphs, tables, groups), by id.
fn conn_kinds(blocks: &[FBlock]) -> super::HashMap<u32, wordcraft_doc::para::ShapeKind> {
    fn walk(blocks: &[FBlock], depth: usize, out: &mut super::HashMap<u32, wordcraft_doc::para::ShapeKind>) {
        if depth > model::MAX_DEPTH {
            return;
        }
        for b in blocks {
            match b {
                FBlock::Para(p) => p.inlines.iter().for_each(|i| inline(i, 0, out)),
                FBlock::Table(t) => t.rows.iter().flatten().for_each(|c| walk(&c.blocks, depth + 1, out)),
            }
        }
    }
    fn inline(i: &super::Inline, depth: usize, out: &mut super::HashMap<u32, wordcraft_doc::para::ShapeKind>) {
        match i {
            super::Inline::Shape { kind, look, .. } => {
                if let Some(c) = look.conn.as_deref().filter(|c| c.id != 0) {
                    out.insert(c.id, *kind);
                }
            }
            super::Inline::Group { members, .. } if depth < model::MAX_GROUP_DEPTH => members.iter().for_each(|m| inline(&m.inline, depth + 1, out)),
            _ => {}
        }
    }
    let mut out = super::HashMap::new();
    walk(blocks, 0, &mut out);
    out
}

/// `styles` with the file's own style definitions while `doc`'s are as read (see
/// `super::kept`), and the font faces they name added to those kept paragraphs need (`used`).
fn styles_as_read(doc: &Document, kept: Option<&wordcraft_doc::kept::KeptXml>, styles: String, used: &mut super::kept::Needed) -> String {
    let Some(k) = kept.filter(|k| k.styles_as_read(doc)) else { return styles };
    // With the font faces they name (any of the file's, by its names).
    for (key, d) in &k.defs {
        if let Some(name) = key.strip_prefix("font:") {
            used.2.entry(name.to_string()).or_insert_with(|| d.clone());
        }
    }
    super::kept::with_common_as_read(&styles, &k.defs, &k.order)
}

/// The package files the file's own definitions refer to, and those the markup written as read
/// links to (`used`), written as read with them (added to `files` and `manifest` where no file
/// `taken` has the path). A picture only an edited paragraph showed isn't written.
fn with_kept_files(
    kept: Option<&wordcraft_doc::kept::KeptXml>,
    used: &std::collections::BTreeSet<String>,
    taken: &dyn Fn(&str) -> bool,
    files: &mut Vec<(String, std::sync::Arc<Vec<u8>>)>,
    manifest: &mut String,
) {
    let Some(k) = kept else { return };
    let mut wanted = used.clone();
    for d in k.defs.values() {
        super::kept::hrefs(d, &mut wanted);
    }
    // A file linked, or one in a folder linked (an object's).
    let linked = |p: &str| wanted.contains(p) || wanted.iter().any(|w| p.strip_prefix(w.trim_end_matches('/')).is_some_and(|r| r.starts_with('/')));
    let entry = |manifest: &mut String, path: &str, mime: &str| {
        let e = format!("manifest:full-path=\"{}\"", x(path));
        if !manifest.contains(&e) {
            manifest.push_str(&format!("<manifest:file-entry {e} manifest:media-type=\"{}\"/>\n", x(mime)));
        }
    };
    for (path, bytes) in &k.files {
        if linked(path) && !taken(path) && !files.iter().any(|(p, _)| p == path) {
            // As the file's manifest had it (the folder too), else by its extension.
            if let Some((dir, _)) = path.rsplit_once('/')
                && let Some(mime) = k.defs.get(&format!("manifest:{dir}/"))
            {
                entry(manifest, &format!("{dir}/"), mime);
            }
            let ext = path.rsplit('.').next().unwrap_or("");
            let mime = k.defs.get(&format!("manifest:{path}")).map_or_else(|| model::mime_of(ext).to_string(), |m| m.to_string());
            entry(manifest, path, &mime);
            files.push((path.clone(), bytes.clone()));
        }
    }
}

/// `office:text`'s statement that the text holds the file's soft page breaks (§19.857
/// `text:use-soft-page-breaks`), when the file said so and markup kept as read in `body` holds
/// them.
fn soft_page_breaks(kept: Option<&wordcraft_doc::kept::KeptXml>, body: &str) -> &'static str {
    if kept.is_some_and(|k| k.defs.contains_key("soft-page-breaks")) && body.contains("<text:soft-page-break") {
        " text:use-soft-page-breaks=\"true\""
    } else {
        ""
    }
}

/// The groups of paragraphs written as read together (see `model::Group`) whose paragraphs
/// are all still as read in `blocks`: each group's every paragraph has its markup (none edited),
/// and no more (none copied).
fn whole_groups(blocks: &[FBlock]) -> std::collections::HashSet<u32> {
    fn count(blocks: &[FBlock], depth: usize, seen: &mut std::collections::HashMap<u32, (u32, u32)>) {
        for b in blocks {
            match b {
                FBlock::Para(p) => {
                    if let (Some(_), Some((id, size))) = (&p.kept, p.group.as_ref().and_then(|g| g.id)) {
                        let e = seen.entry(id).or_insert((0, size));
                        e.0 = e.0.saturating_add(1);
                    }
                }
                FBlock::Table(t) if depth < model::MAX_DEPTH => {
                    for c in t.rows.iter().flatten() {
                        count(&c.blocks, depth + 1, seen);
                    }
                }
                FBlock::Table(_) => {}
            }
        }
    }
    let mut seen = std::collections::HashMap::new();
    count(blocks, 0, &mut seen);
    seen.into_iter().filter(|(_, (n, size))| n == size).map(|(id, _)| id).collect()
}

/// The prefix of WordCraft's own automatic style names in a document with paragraphs kept as
/// read (theirs keep the names they were read with).
const KEPT_PREFIX: &str = "wc";

/// `content` and `styles` with the definitions kept paragraphs need (`used`: automatic, common,
/// font faces), each where it goes and only where its name isn't defined already.
fn with_kept_defs(
    mut content: String,
    mut styles: String,
    used: &super::kept::Needed,
    defs: &std::collections::BTreeMap<String, std::sync::Arc<str>>,
) -> (String, String) {
    let (auto, common, fonts) = used;
    if auto.is_empty() && common.is_empty() && fonts.is_empty() && defs.is_empty() {
        return (content, styles);
    }
    // The namespaces the file declared (kept markup may use any), where they aren't declared.
    for (xml, root) in [(&mut content, "<office:document-content "), (&mut styles, "<office:document-styles ")] {
        let Some(at) = xml.find(root).map(|i| i + root.len()) else { continue };
        let head_end = xml.get(at..).and_then(|r| r.find('>')).map_or(at, |e| at + e);
        let head = xml.get(at..head_end).unwrap_or("").to_string();
        let missing: String = defs
            .iter()
            .filter_map(|(k, uri)| k.strip_prefix("ns:").map(|p| (p, uri)))
            .filter(|(p, _)| !head.contains(&format!("xmlns:{p}=")))
            .map(|(p, uri)| format!("xmlns:{p}=\"{}\" ", x(uri)))
            .collect();
        xml.insert_str(at, &missing);
    }
    let add = |xml: &mut String, close: &str, defs: &std::collections::BTreeMap<String, std::sync::Arc<str>>| {
        let missing: String = defs.iter().filter(|(_, d)| !super::kept::defined(xml, d)).map(|(_, d)| &**d).collect();
        if missing.is_empty() {
            return;
        }
        let empty = close.replacen("</", "<", 1).replacen('>', "/>", 1);
        if let Some(at) = xml.find(close) {
            xml.insert_str(at, &missing);
        } else if let Some(at) = xml.find(&empty) {
            xml.replace_range(at..at + empty.len(), &format!("{}{missing}{close}", close.replacen("</", "<", 1)));
        }
    };
    add(&mut content, "</office:automatic-styles>", auto);
    add(&mut styles, "</office:styles>", common);
    for (xml, before) in [(&mut content, "<office:automatic-styles"), (&mut styles, "<office:styles")] {
        // No declarations yet: the element they go in, where it belongs (before the styles).
        if !fonts.is_empty()
            && !xml.contains("<office:font-face-decls")
            && let Some(at) = xml.find(before)
        {
            xml.insert_str(at, "<office:font-face-decls></office:font-face-decls>");
        }
        add(xml, "</office:font-face-decls>", fonts);
    }
    (content, styles)
}

/// An OpenDocument text document's media type, and a template's (ODF 1.3 Part 2 §3.3, Appendix C).
pub const TEXT_MIME: &str = "application/vnd.oasis.opendocument.text";
pub const TEMPLATE_MIME: &str = "application/vnd.oasis.opendocument.text-template";

/// `doc` as an `.odt` package.
pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    export_as(doc, TEXT_MIME)
}

/// `doc` as an OpenDocument text template (`.ott`): the same package with the template's
/// media type.
pub fn export_template(doc: &Document) -> Result<Vec<u8>, String> {
    export_as(doc, TEMPLATE_MIME)
}

fn export_as(doc: &Document, mime: &str) -> Result<Vec<u8>, String> {
    let flow = model::from_doc_with(
        doc,
        model::FlowOpts { tracked: true, fields: true, notes: true, comments: true, lists: true, chars: true, paras: true, boxes: true },
    );
    let accents = doc.settings.theme_colors.get(4..10).map(<[_]>::to_vec).unwrap_or_default();
    // Paragraphs kept as read name their own automatic styles: WordCraft's take another prefix.
    let kept = doc.kept.of("odt");
    let mut w = if kept.is_some() { Writer::with_prefix(KEPT_PREFIX) } else { Writer::default() };
    w.note_numbers = Some([false, true].map(|endnote| {
        let o = doc.note_options(&doc.last_section, endnote);
        (o.num_fmt, o.num_start)
    }));
    w.kept_defs = std::sync::Arc::new(kept.map(|k| k.defs.clone()).unwrap_or_default());
    w.reserved = std::sync::Arc::new(kept.map(|k| k.files.keys().cloned().collect()).unwrap_or_default());
    w.groups_whole = whole_groups(&flow.blocks);
    // The file's master pages are written as read: what was kept may start them.
    w.kept_pages = kept.is_some_and(|k| k.pages_as_read(doc));
    w.accents.clone_from(&accents);
    w.conn_kinds = conn_kinds(&flow.blocks);
    w.sources = doc.sources.iter().map(|s| (s.tag.clone(), s.clone())).collect();
    w.custom_props = doc.custom_props.iter().filter(|p| p.kind != "raw").map(|p| p.name.clone()).collect();
    w.user_fields = doc.custom_props.iter().filter(|p| p.odf_user_field && p.kind != "raw").map(|p| p.name.clone()).collect();
    w.variables = variables(doc);
    w.forms_allowed = true;
    w.ranged = model::ranged_anchors(&flow.blocks);
    let mut body = String::new();
    let sections = flow_sections(&flow.blocks, &doc.last_section);
    // One master page per section that starts a page; the first one is "Standard".
    let mut pages: Vec<(String, SectionProps)> = Vec::new();
    // The columns of the page in force: a continuous section with others is a `text:section`.
    let mut page_cols = Columns::default();
    for (k, (sec, range)) in sections.iter().enumerate() {
        let new_page = k == 0 || sec.start != SectionStart::Continuous;
        if new_page {
            // The file's own master page while its pages are as read.
            let own = kept
                .filter(|kx| kx.pages_as_read(doc) && kx.section_masters.len() == sections.len())
                .and_then(|kx| kx.section_masters.get(k).cloned().flatten());
            let name = match &own {
                Some(m) => m.clone(),
                None if k == 0 => "Standard".to_string(),
                None => format!("{}Page{}", w.prefix, pages.len() + 1),
            };
            pages.push((name.clone(), sec.clone()));
            page_cols = sec.columns.clone();
            // The first section needs its paragraph to name the page only to restart numbering
            // (or when it is the file's own other than Standard).
            if k > 0 || sec.page_num_start.is_some() || own.as_deref().is_some_and(|m| m != "Standard") {
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
            let n = format!("{}{}", w.prefix, w.section_styles.len() + 1);
            // Whether its columns end balanced, said so every reader does the same: as WordCraft
            // lays it out (balanced when text follows on the page, or when the section says).
            let next_continuous = sections.get(k + 1).is_some_and(|(s, _)| s.start == SectionStart::Continuous);
            let balanced = match sec.balance_columns {
                Some(b) => b && (next_continuous || k + 1 == sections.len()),
                None => next_continuous,
            };
            w.section_styles.push(format!(
                "<style:style style:name=\"Sect{n}\" style:family=\"section\"><style:section-properties text:dont-balance-text-columns=\"{}\">{}</style:section-properties></style:style>",
                !balanced,
                columns_xml(&sec.columns)
            ));
            body.push_str(&format!("<text:section text:style-name=\"Sect{n}\" text:name=\"Section{n}\">"));
        }
        w.blocks(flow.blocks.get(range.clone()).unwrap_or(&[]), &mut body, 0);
        // A kept section doesn't cross a page or column section's edge.
        w.close_sections(&mut body);
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
    // The sequences fields count (§7.4.11), declared after any tracked changes.
    // The variables, declared (§7.4.2).
    let var_decls = if w.variables.is_empty() {
        String::new()
    } else {
        let d: String = w.variables.iter().map(|n| format!("<text:variable-decl office:value-type=\"string\" text:name=\"{}\"/>", x(n))).collect();
        format!("<text:variable-decls>{d}</text:variable-decls>")
    };
    let decls = if w.sequences.is_empty() {
        String::new()
    } else {
        let d: String =
            w.sequences.iter().map(|n| format!("<text:sequence-decl text:display-outline-level=\"0\" text:name=\"{}\"/>", x(n))).collect();
        format!("<text:sequence-decls>{d}</text:sequence-decls>")
    };
    let user_decls = user_field_decls(&doc.custom_props);
    // The file's own declarations, with WordCraft's added to them; its scripts.
    let kept_decl = |el: &str, ours: String| super::kept::merge_decls(kept.and_then(|k| k.defs.get(&format!("decls:{el}"))).map(|d| &**d), ours);
    let var_decls = kept_decl("text:variable-decls", var_decls);
    let user_decls = kept_decl("text:user-field-decls", user_decls);
    let decls = kept_decl("text:sequence-decls", decls);
    let other_decls: String = kept
        .map(|k| {
            k.defs
                .iter()
                .filter(|(key, _)| {
                    key.starts_with("decls:") && !["variable", "user-field", "sequence"].iter().any(|d| key.contains(&format!("text:{d}-decls")))
                })
                .map(|(_, d)| &**d)
                .collect()
        })
        .unwrap_or_default();
    let scripts = kept.and_then(|k| k.defs.get("top:office:scripts")).map(|d| d.to_string()).unwrap_or_default();
    let soft = soft_page_breaks(kept, &body);
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}>{scripts}{}<office:body><office:text{soft}>\n{}{tracked}{var_decls}{decls}{user_decls}{other_decls}{body}</office:text></office:body></office:document-content>\n",
        w.automatic_styles(),
        // The file's forms as read, with the document's controls.
        super::kept::merge_forms(kept.and_then(|k| k.defs.get("forms")).map(|d| &**d), super::forms::forms_xml(&w.forms, &hidden_controls(doc)))
    );
    let mut hw = Writer::with_prefix(if kept.is_some() { "wcM" } else { "M" });
    // Headers' and footers' new pictures and objects skip the kept files' names too.
    hw.reserved = std::sync::Arc::clone(&w.reserved);
    hw.accents = accents;
    hw.variables.clone_from(&w.variables);
    let parts: Vec<PageParts> = pages.iter().map(|(_, sec)| page_parts(doc, sec, &mut hw)).collect();
    // A custom style kept paragraphs name is written from the model (as edited), not as read.
    for st in &doc.styles.styles {
        let name = super::writer::odf_name(&st.id);
        if w.kept_used.1.contains_key(&name) || w.kept_used.1.contains_key(&format!("{name}|text")) {
            if model::is_custom_style(doc, &st.id) {
                w.named.insert(st.id.clone());
            } else if model::is_custom_char_style(doc, &st.id) {
                w.named_text.insert(st.id.clone());
            }
        }
    }
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
    let styles = page_background(doc, &mut w, styles_xml(doc, &pages, &parts, &hw.style_defs(), &named));
    let styles = styles.replacen(
        "</office:styles>",
        // The Frame style text boxes derive from (it states nothing: their own styles do).
        &format!("{draw_xml}{named_text_xml}<style:style style:name=\"Frame\" style:family=\"graphic\"/><style:style style:name=\"Graphics\" style:family=\"graphic\"/></office:styles>"),
        1,
    );
    w.pictures.append(&mut hw.pictures);
    w.objects.append(&mut hw.objects);
    w.object_dirs.append(&mut hw.object_dirs);
    let meta = meta_xml(&flow.meta, &doc.core, &doc.custom_props);
    // With the file's properties WordCraft doesn't model.
    let meta = match kept {
        Some(k) => super::kept::with_kept_meta(meta, &k.defs, &k.order),
        None => meta,
    };
    let mut manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"{mime}\"/>\n<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/>\n",
    );
    for (dir, mime) in &w.object_dirs {
        manifest.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"{}\" manifest:version=\"1.3\" manifest:media-type=\"{}\"/>\n",
            x(dir),
            x(mime)
        ));
    }
    for (name, _, mime) in &w.pictures {
        manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>\n", x(name), x(mime)));
    }
    for (dir, _, mime) in &w.objects {
        manifest.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"{0}/\" manifest:version=\"1.3\" manifest:media-type=\"{mime}\"/>\n<manifest:file-entry manifest:full-path=\"{0}/content.xml\" manifest:media-type=\"text/xml\"/>\n",
            x(dir)
        ));
    }
    // The parts WordCraft doesn't read, as they were, unless one would take a path written here.
    let (kept_files, kept_entries) = super::parts::kept(doc);
    let dirs: Vec<String> =
        w.objects.iter().map(|(d, _, _)| d).chain(w.object_dirs.iter().map(|(d, _)| d)).map(|d| format!("{}/", d.trim_end_matches('/'))).collect();
    let taken = |p: &str| w.pictures.iter().any(|(n, _, _)| n == p) || dirs.iter().any(|d| p.starts_with(d.as_str()));
    let mut kept_files: Vec<_> = kept_files.into_iter().filter(|(p, _)| !taken(p)).collect();
    with_kept_files(kept, &w.kept_files_used, &taken, &mut kept_files, &mut manifest);
    for (path, mime) in &kept_entries {
        if (path.ends_with('/') && kept_files.iter().any(|(p, _)| p.starts_with(path.as_str()))) || kept_files.iter().any(|(p, _)| p == path) {
            manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>\n", x(path), x(mime)));
        }
    }
    manifest.push_str("</manifest:manifest>\n");

    let err = |e: zip::result::ZipError| format!("ODT: {e}");
    let ioerr = |e: std::io::Error| format!("ODT: {e}");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).map_err(err)?;
    z.write_all(mime.as_bytes()).map_err(ioerr)?;
    // Each font declared, for readers without it (§16.21).
    let (content, styles) = (with_font_faces(&content), with_font_faces(&styles));
    // What the paragraphs written as read need, as read (see `super::kept`).
    // Style definitions as read while nobody changed them: the file's own `office:styles`.
    let styles = styles_as_read(doc, kept, styles, &mut w.kept_used);
    // Page setup as read while nobody changed it: the file's own page layouts and master pages.
    let styles = match kept.filter(|k| k.pages_as_read(doc)) {
        Some(k) => super::kept::with_pages_as_read(&styles, &k.defs, &k.order),
        None => styles,
    };
    let (content, styles) = with_kept_defs(content, styles, &w.kept_used, &w.kept_defs);
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
    for (dir, xml, _) in &w.objects {
        z.start_file(format!("{dir}/content.xml"), deflated).map_err(err)?;
        z.write_all(xml.as_bytes()).map_err(ioerr)?;
    }
    for (name, data, _) in &w.pictures {
        z.start_file(name.as_str(), stored).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    for (name, data) in &kept_files {
        z.start_file(name.as_str(), deflated).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    let cur = z.finish().map_err(err)?;
    Ok(cur.into_inner())
}

/// A page layout's page number format (§19.500 `style:num-format`), the document's page colour
/// (§20.175 `fo:background-color`) and the section's page borders with the space inside them.
fn page_look(doc: &Document, sec: &SectionProps) -> String {
    let mut a = String::new();
    let f = super::writer::odf_num_format(sec.page_num_format);
    if f != "1" {
        a.push_str(&format!(" style:num-format=\"{f}\""));
    }
    if let Some(c) = doc.settings.page_color {
        a.push_str(&format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    a
}

/// Page borders `b` as ODF writes them, and the margins (top, bottom, left, right) of the page
/// they go round: each border at its distance from the edge (WordCraft's), which becomes the
/// margin, with the rest of the way to the text as its padding.
fn page_borders(b: Option<wordcraft_doc::props::Borders>, margins: [f32; 4]) -> (String, [f32; 4]) {
    let Some(b) = b.filter(|b| b.any_visible()) else { return (String::new(), margins) };
    let mut out = margins;
    let mut attrs = String::new();
    for (i, (side, edge)) in [("top", b.top), ("bottom", b.bottom), ("left", b.left), ("right", b.right)].into_iter().enumerate() {
        let (Some(e), Some(m)) = (edge.filter(|e| e.is_visible()), out.get_mut(i)) else { continue };
        // A hostile margin (negative or not a number) leaves no room, so clamp cannot panic.
        let room = if m.is_finite() { m.max(0.0) } else { 0.0 };
        let at = if e.space.is_finite() { e.space.clamp(0.0, room) } else { 0.0 };
        let pad = (*m - at - e.width).max(0.0);
        *m = at;
        attrs.push_str(&format!(" fo:border-{side}=\"{}\" fo:padding-{side}=\"{}\"", super::writer::odf_border(Some(e)), pt(pad)));
    }
    (attrs, out)
}

/// The footnotes' and endnotes' numbering (§16.29 `text:notes-configuration`): format, first
/// number (ODF counts from 0), restart and, for footnotes, page foot or document end.
fn notes_configuration(doc: &Document) -> String {
    use wordcraft_doc::section::{NotePos, NoteRestart};
    let mut s = String::new();
    for (class, fmt, pr) in
        [("footnote", doc.settings.footnote_format, &doc.settings.footnote_pr), ("endnote", doc.settings.endnote_format, &doc.settings.endnote_pr)]
    {
        // Only what the document sets: the rest stays the reader's default.
        let start = pr.num_start.map(|n| format!(" text:start-value=\"{}\"", n.max(1) - 1)).unwrap_or_default();
        let restart = pr
            .num_restart
            .map(|r| {
                let at = match r {
                    NoteRestart::EachPage => "page",
                    NoteRestart::EachSect => "chapter",
                    NoteRestart::Continuous => "document",
                };
                format!(" text:start-numbering-at=\"{at}\"")
            })
            .unwrap_or_default();
        let position = match (class, pr.pos) {
            ("footnote", Some(NotePos::DocEnd)) => " text:footnotes-position=\"document\"",
            ("footnote", Some(_)) => " text:footnotes-position=\"page\"",
            _ => "",
        };
        s.push_str(&format!(
            "<text:notes-configuration text:note-class=\"{class}\" style:num-format=\"{}\"{start}{restart}{position}/>",
            super::writer::odf_num_format(fmt)
        ));
    }
    s
}

/// Line numbering (§16.30 `text:linenumbering-configuration`): every how many lines, how far
/// from the text, restarting on each page or not.
fn line_numbering_xml(l: &wordcraft_doc::section::LineNumbering) -> String {
    let restart = l.restart == wordcraft_doc::section::LineNumberRestart::Page;
    format!(
        "<text:linenumbering-configuration text:number-lines=\"true\" text:increment=\"{}\" text:offset=\"{}\" style:num-format=\"1\" text:number-position=\"left\" text:restart-on-page=\"{restart}\"/>",
        l.count_by.clamp(1, 100),
        pt(if l.distance.is_finite() { l.distance.clamp(0.0, 1584.0) } else { 0.0 })
    )
}

/// The hidden form controls kept in `doc` (see [`super::forms::HIDDEN_PART`]).
fn hidden_controls(doc: &Document) -> Vec<String> {
    doc.passthrough
        .get(super::forms::HIDDEN_PART)
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(|s| s.split('\0').filter(|h| h.starts_with("<form:hidden")).take(10_000).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The names SET fields in `doc` give values (each story, a bounded number): its variables.
fn variables(doc: &Document) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let stories = std::iter::once(wordcraft_doc::StoryRef::Body).chain(doc.parts.keys().map(|k| wordcraft_doc::StoryRef::Part(*k)));
    for story in stories {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for o in &p.objects {
                if let wordcraft_doc::InlineObject::Field { instr, .. } = o
                    && let Some((name, _)) = super::fields::set_of(instr)
                    && out.len() < 10_000
                    && !name.contains(['"', '<', '&'])
                {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out
}

/// `styles` with the page background picture (Design › Page Color › Fill Effects › Picture) in
/// each page layout (§17.3 `style:background-image`, first in `style:page-layout-properties`),
/// the picture written into the package by `w`: tiled, stretched, or centred at its own size
/// (§20.249 `style:repeat`).
fn page_background(doc: &Document, w: &mut Writer, styles: String) -> String {
    let Some(pic) = doc.settings.page_picture.as_ref() else { return styles };
    let Some(data) = doc.media.get(&pic.media) else { return styles };
    let Some(ext) = model::sniff_image(data) else { return styles };
    let name = format!("Pictures/page-background.{ext}");
    if !w.pictures.iter().any(|(n, _, _)| *n == name) {
        w.pictures.push((name.clone(), data.clone(), model::mime_of(ext).to_string()));
    }
    let repeat = match pic.fill_type.to_ascii_lowercase().as_str() {
        "tile" => "repeat",
        "center" => "no-repeat",
        _ => "stretch",
    };
    let el = format!(
        "<style:background-image xlink:href=\"{}\" xlink:type=\"simple\" xlink:actuate=\"onLoad\" style:position=\"center center\" style:repeat=\"{repeat}\"/>",
        x(&name)
    );
    // After each layout's properties' opening tag.
    let mut out = String::with_capacity(styles.len() + el.len() * 4);
    let mut rest = styles.as_str();
    while let Some(at) = rest.find("<style:page-layout-properties") {
        let Some(end) = rest.get(at..).and_then(|r| r.find('>')).map(|e| at + e) else { break };
        let self_closing = rest.get(end.saturating_sub(1)..end) == Some("/");
        if self_closing {
            out.push_str(rest.get(..end.saturating_sub(1)).unwrap_or_default());
            out.push('>');
            out.push_str(&el);
            out.push_str("</style:page-layout-properties>");
        } else {
            out.push_str(rest.get(..=end).unwrap_or_default());
            out.push_str(&el);
        }
        rest = rest.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// `xml` (content.xml or styles.xml) with each font family it uses declared once (§16.21
/// `style:font-face`: its family, generic family and pitch) and named where it is used
/// (`style:font-name` beside `fo:font-family`, and the Asian and complex ones): a reader without
/// the font picks a like one.
fn with_font_faces(xml: &str) -> String {
    const ATTRS: [(&str, &str); 3] = [
        ("fo:font-family=\"", "style:font-name"),
        ("style:font-family-asian=\"", "style:font-name-asian"),
        ("style:font-family-complex=\"", "style:font-name-complex"),
    ];
    let mut families: Vec<String> = Vec::new();
    let mut out = String::with_capacity(xml.len() + 1024);
    let mut rest = xml;
    loop {
        // The next font family attribute.
        let next = ATTRS.iter().filter_map(|(a, n)| rest.find(a).map(|at| (at, *a, *n))).min_by_key(|(at, ..)| *at);
        let Some((at, attr, name_attr)) = next else { break };
        let value_at = at + attr.len();
        let Some(end) = rest.get(value_at..).and_then(|r| r.find('"')).map(|e| value_at + e) else { break };
        let value = rest.get(value_at..end).unwrap_or("");
        let family = value.replace("&apos;", "'").trim().trim_matches(['\'', '"']).trim().to_string();
        out.push_str(rest.get(..at).unwrap_or(""));
        if !family.is_empty() && family.len() <= 200 && !family.contains(',') {
            if !families.contains(&family) && families.len() < 1000 {
                families.push(family.clone());
            }
            out.push_str(&format!("{name_attr}=\"{}\" ", super::writer::x(&family)));
        }
        out.push_str(rest.get(at..=end).unwrap_or(""));
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    if families.is_empty() {
        return out;
    }
    let decls: String = families
        .iter()
        .map(|f| {
            let (generic, pitch) = font_kind(f);
            format!(
                "<style:font-face style:name=\"{0}\" svg:font-family=\"'{0}'\" style:font-family-generic=\"{generic}\" style:font-pitch=\"{pitch}\"/>",
                super::writer::x(f)
            )
        })
        .collect();
    let decls = format!("<office:font-face-decls>{decls}</office:font-face-decls>");
    // Before the styles: first in office:document-styles, after the scripts in the content.
    for before in ["<office:styles>", "<office:automatic-styles>", "<office:automatic-styles/>", "<office:master-styles>", "<office:body>"] {
        if let Some(at) = out.find(before) {
            out.insert_str(at, &decls);
            return out;
        }
    }
    out
}

/// A font family's generic family and pitch (§20.270, §20.271), from its name: monospaced
/// families are `modern` and fixed, serif ones `roman`, the rest `swiss`.
fn font_kind(family: &str) -> (&'static str, &'static str) {
    let f = family.to_ascii_lowercase();
    if model::is_mono(family) {
        return ("modern", "fixed");
    }
    const SERIF: [&str; 16] = [
        "serif",
        "times",
        "georgia",
        "cambria",
        "caladea",
        "garamond",
        "palatino",
        "book",
        "roman",
        "libertine",
        "minion",
        "baskerville",
        "bodoni",
        "didot",
        "charter",
        "constantia",
    ];
    if SERIF.iter().any(|s| f.contains(s)) && !f.contains("sans") {
        return ("roman", "variable");
    }
    ("swiss", "variable")
}

#[cfg(test)]
mod setting_tests {
    /// A page border round a page whose margin is negative or not a number is written at no
    /// distance, instead of panicking.
    #[test]
    fn a_page_border_round_a_hostile_margin_is_written_at_no_distance() {
        let mut edge = wordcraft_doc::props::Border::single(1.0);
        edge.space = 12.0;
        let b = wordcraft_doc::props::Borders::all(edge);
        let (attrs, out) = super::page_borders(Some(b), [-10.0, f32::NAN, 36.0, 5.0]);
        assert!(attrs.contains("fo:border-top="), "{attrs}");
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 12.0);
        assert_eq!(out[3], 5.0);
    }

    /// A self-closed setting is replaced whole, leaving the item after it as it was.
    #[test]
    fn a_self_closed_setting_is_replaced_whole() {
        let item = "<config:config-item config:name=\"A\" config:type=\"boolean\">true</config:config-item>";
        let own = "<s><config:config-item config:name=\"A\" config:type=\"boolean\"/><config:config-item config:name=\"B\" config:type=\"boolean\">false</config:config-item></s>";
        let out = super::with_setting(own, "A", true, item).unwrap_or_default();
        assert_eq!(out, format!("<s>{item}<config:config-item config:name=\"B\" config:type=\"boolean\">false</config:config-item></s>"));
        // An item with a value has just its value changed.
        let own = "<s><config:config-item config:name=\"A\" config:type=\"boolean\">false</config:config-item></s>";
        assert_eq!(super::with_setting(own, "A", true, item).as_deref(), Some(format!("<s>{item}</s>").as_str()));
    }
}

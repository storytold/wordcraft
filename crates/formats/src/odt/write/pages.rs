//! Sections as master pages: one `style:page-layout` and `style:master-page` per section, with
//! its page size, orientation, margins, columns and page number format, and its headers and
//! footers (default, first page, even pages) inherited from the sections before it as Word links
//! them. A section starts at the body block whose style names its master page and restarts the
//! page numbers there (`style:page-number`). A continuous section on the page before it with
//! other columns is a `text:section` with them.
//!
//! Word measures the header from the page edge to the header's top and the body from the edge to
//! its top margin; ODF has the page margin at the header's top and the header's height between
//! it and the body. So with a header the page margin is the header distance and the header is at
//! least as high as the rest of the top margin (the footer likewise).

use std::ops::Range;

use wordcraft_doc::section::{Columns, HeaderSet, NumFormat, SectionProps, SectionStart};
use wordcraft_doc::{Blocks, Document};

use super::{Ctx, PageStart, pt, text};

/// One master page: the section it was made for and its inherited headers and footers.
pub(super) struct Master {
    pub name: String,
    section: usize,
    headers: HeaderSet,
    footers: HeaderSet,
}

/// A run of body blocks written together: a section, or several sections on one master page.
pub(super) struct Segment {
    pub blocks: Range<usize>,
    /// The master page it starts, when it starts a page.
    pub start: Option<PageStart>,
    /// The columns of a continuous section that differs from its page in them: written as a
    /// `text:section` (ODF 1.3 §5.4).
    pub columns: Option<Columns>,
}

/// Where the body changes master page or columns.
#[derive(Default)]
pub(super) struct Plan {
    pub segments: Vec<Segment>,
    pub masters: Vec<Master>,
}

/// Section `si`'s headers or footers (as `get` picks), with those it leaves out inherited from
/// the sections before it (Word's "link to previous").
fn linked(sections: &[(usize, &SectionProps)], si: usize, get: fn(&SectionProps) -> HeaderSet) -> HeaderSet {
    let mut set = sections.get(si).map(|(_, s)| get(s)).unwrap_or_default();
    for (_, s) in sections.iter().take(si).rev() {
        let o = get(s);
        set.default = set.default.or(o.default);
        set.first = set.first.or(o.first);
        set.even = set.even.or(o.even);
    }
    set
}

/// Whether `b` continues on the page of `a` without anything a master page holds changing.
fn same_page(a: &SectionProps, b: &SectionProps) -> bool {
    let geo = |s: &SectionProps| {
        [s.page_w, s.page_h, s.margin_top, s.margin_bottom, s.margin_left, s.margin_right, s.header, s.footer, s.gutter].map(f32::to_bits)
    };
    geo(a) == geo(b) && a.landscape == b.landscape && a.title_page == b.title_page && a.page_num_format == b.page_num_format && a.rtl == b.rtl
}

/// The master page of each section and the blocks it holds. A continuous section that changes
/// nothing a page holds stays on its predecessor's master page (a new one would break the page),
/// in a `text:section` when its columns differ from the page's. An empty last section is left
/// out.
pub(super) fn plan(doc: &Document) -> Plan {
    let sections = doc.sections();
    let mut plan = Plan::default();
    let mut from = 0usize;
    let mut prev: Option<(&SectionProps, HeaderSet, HeaderSet)> = None;
    // The columns of the page in force.
    let mut page_cols = Columns::default();
    for (si, (end, sect)) in sections.iter().enumerate() {
        let to = (end + 1).min(doc.body.len());
        if from >= to && si > 0 {
            continue;
        }
        let headers = linked(&sections, si, |s| s.headers);
        let footers = linked(&sections, si, |s| s.footers);
        let continues = prev.is_some_and(|(p, h, f)| sect.start == SectionStart::Continuous && same_page(p, sect) && h == headers && f == footers);
        // todo: odd and even page starts; a page number restart inside a continuous section.
        let mut seg = Segment { blocks: from..to, start: None, columns: None };
        if continues {
            let cols = &sect.columns;
            if cols.count != page_cols.count || (cols.count > 1 && *cols != page_cols) {
                seg.columns = Some(cols.clone());
            }
        } else {
            let name = if plan.masters.is_empty() { "Standard".to_string() } else { format!("MP{}", plan.masters.len() + 1) };
            // The first page needs its block to name it only to restart the numbering.
            if si > 0 || sect.page_num_start.is_some() {
                seg.start = Some(PageStart { master: name.clone(), number: sect.page_num_start });
            }
            plan.masters.push(Master { name, section: si, headers, footers });
            page_cols = sect.columns.clone();
        }
        plan.segments.push(seg);
        prev = Some((sect, headers, footers));
        from = to;
    }
    plan
}

pub(super) fn num_format(f: NumFormat) -> &'static str {
    match f {
        NumFormat::LowerRoman => "i",
        NumFormat::UpperRoman => "I",
        NumFormat::LowerLetter => "a",
        NumFormat::UpperLetter => "A",
        // LibreOffice's own values for Cyrillic letters.
        NumFormat::RussianLower => "а, б, .., аа, аб, ... (ru)",
        NumFormat::RussianUpper => "А, Б, .., Аа, Аб, ... (ru)",
        // todo: the other formats (LibreOffice's extended `style:num-format` values).
        _ => "1",
    }
}

/// Which header (or footer) slots a page style shows: default, even pages, first page.
fn slots(set: &HeaderSet, title_page: bool, even_odd: bool) -> [(bool, Option<u32>); 3] {
    [(true, set.default), (even_odd, set.even), (title_page, set.first)]
}

/// `style:page-layout` of section `s`.
fn page_layout(ctx: &Ctx, name: &str, s: &SectionProps, has_header: bool, has_footer: bool) -> String {
    let len = |v: f32| pt(if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 });
    let (top, bottom) = (s.margin_top.abs(), s.margin_bottom.abs());
    let (hd, fd) = (s.header.max(0.0), s.footer.max(0.0));
    let mut p = format!(
        "fo:page-width=\"{}\" fo:page-height=\"{}\" style:print-orientation=\"{}\" style:num-format=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" style:writing-mode=\"{}\"",
        len(s.page_w),
        len(s.page_h),
        if s.landscape { "landscape" } else { "portrait" },
        num_format(s.page_num_format),
        len(s.margin_left + s.gutter),
        len(s.margin_right),
        len(if has_header { hd.min(top) } else { top }),
        len(if has_footer { fd.min(bottom) } else { bottom }),
        if s.rtl { "rl-tb" } else { "lr-tb" }
    );
    if let Some(c) = ctx.doc.settings.page_color {
        p.push_str(&format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    let mut inner = String::new();
    if s.columns.count > 1 {
        inner.push_str(&crate::odt::columns_xml(&s.columns));
    }
    // todo: unequal columns, page borders, vertical alignment, document grid, line numbering.
    let mut s_xml = format!("<style:page-layout-properties {p}>{inner}</style:page-layout-properties>");
    if has_header {
        s_xml.push_str(&format!(
            "<style:header-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-bottom=\"0pt\" style:dynamic-spacing=\"false\"/></style:header-style>",
            len(top - hd.min(top))
        ));
    }
    if has_footer {
        let h = len(bottom - fd.min(bottom));
        s_xml.push_str(&format!(
            "<style:footer-style><style:header-footer-properties fo:min-height=\"{h}\" fo:margin-top=\"{h}\" style:dynamic-spacing=\"true\"/></style:footer-style>"
        ));
    }
    let usage = if ctx.doc.settings.mirror_margins { " style:page-usage=\"mirrored\"" } else { "" };
    format!("<style:page-layout style:name=\"{name}\"{usage}>{s_xml}</style:page-layout>")
}

/// A header or footer story, or an empty paragraph.
fn story(ctx: &mut Ctx, id: Option<u32>, out: &mut String) {
    let doc = ctx.doc;
    let blocks: Option<&Blocks> = id.and_then(|id| doc.parts.get(&id)).map(|p| &p.blocks);
    match blocks.filter(|b| !b.is_empty()) {
        Some(b) => text::blocks(ctx, b, out),
        None => out.push_str("<text:p/>"),
    }
}

/// The page layouts (for `office:automatic-styles`) and master pages (`office:master-styles`)
/// of `styles.xml`. Header and footer contents are written here, with their automatic styles
/// going to `styles.xml`.
pub(super) fn master_styles(ctx: &mut Ctx, plan: &Plan) -> (String, String) {
    let sections = ctx.doc.sections();
    let even_odd = ctx.doc.settings.even_odd_headers;
    let (mut layouts, mut masters) = (String::new(), String::new());
    for (k, m) in plan.masters.iter().enumerate() {
        let Some((_, s)) = sections.get(m.section) else { continue };
        let hs = slots(&m.headers, s.title_page, even_odd);
        let fs = slots(&m.footers, s.title_page, even_odd);
        let has_header = hs.iter().any(|(on, id)| *on && id.is_some());
        let has_footer = fs.iter().any(|(on, id)| *on && id.is_some());
        let layout = format!("pm{}", k + 1);
        layouts.push_str(&page_layout(ctx, &layout, s, has_header, has_footer));
        masters.push_str(&format!("<style:master-page style:name=\"{}\" style:page-layout-name=\"{layout}\">", m.name));
        for (has, set, tag) in [(has_header, hs, "header"), (has_footer, fs, "footer")] {
            if !has {
                continue;
            }
            // Default, then even (`-left`) and first (`-first`) pages, as ODF orders them.
            for ((on, id), suffix) in set.into_iter().zip(["", "-left", "-first"]) {
                if on {
                    masters.push_str(&format!("<style:{tag}{suffix}>"));
                    story(ctx, id, &mut masters);
                    masters.push_str(&format!("</style:{tag}{suffix}>"));
                }
            }
        }
        masters.push_str("</style:master-page>");
    }
    (layouts, masters)
}

//! The page: the `style:page-layout` of the document's section (page size, orientation, margins,
//! gutter, columns, page number format) and the `Standard` master page that uses it.

use wordcraft_doc::section::{NumFormat, SectionProps};

use super::{Ctx, pt};

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

/// `style:page-layout` of section `s`.
fn page_layout(ctx: &Ctx, name: &str, s: &SectionProps) -> String {
    let len = |v: f32| pt(if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 });
    let mut p = format!(
        "fo:page-width=\"{}\" fo:page-height=\"{}\" style:print-orientation=\"{}\" style:num-format=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" style:writing-mode=\"{}\"",
        len(s.page_w),
        len(s.page_h),
        if s.landscape { "landscape" } else { "portrait" },
        num_format(s.page_num_format),
        len(s.margin_left + s.gutter),
        len(s.margin_right),
        len(s.margin_top.abs()),
        len(s.margin_bottom.abs()),
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
    let usage = if ctx.doc.settings.mirror_margins { " style:page-usage=\"mirrored\"" } else { "" };
    format!(
        "<style:page-layout style:name=\"{name}\"{usage}><style:page-layout-properties {p}>{inner}</style:page-layout-properties></style:page-layout>"
    )
}

/// The page layout (for `office:automatic-styles`) and the master page (`office:master-styles`)
/// of `styles.xml`.
pub(super) fn master_styles(ctx: &mut Ctx) -> (String, String) {
    let layout = page_layout(ctx, "pm1", &ctx.doc.last_section);
    (layout, "<style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\"/>".into())
}

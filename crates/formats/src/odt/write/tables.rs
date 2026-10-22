//! Tables, as the exchange-flow writer wrote them: the grid's column widths (equal shares of
//! 6.5 in without a grid), header rows (`w:tblHeader`) repeated on each page, column and row
//! spans, cell shading and vertical alignment, a thin border and 4 pt padding on every cell. Cell
//! contents are stories of their own. A table starting a section carries its master page.
//!
//! todo: the table's and its cells' own formatting (borders, margins, heights, horizontal
//! alignment), its table style.

use wordcraft_doc::Table;
use wordcraft_doc::props::{VAlign, VMerge};

use super::{Ctx, Props, attrs, pt, text};
use crate::odt::x;

/// The rows the cell at row `r`, grid column `g` spans (a vertical merge restarting there).
fn rows_spanned(t: &Table, r: usize, g: usize) -> usize {
    1 + t
        .rows
        .iter()
        .enumerate()
        .skip(r + 1)
        .take_while(|(k, row)| t.cell_at_grid(*k, g).and_then(|i| row.cells.get(i)).is_some_and(|c| c.props.vmerge == VMerge::Continue))
        .count()
}

pub(super) fn table(ctx: &mut Ctx, t: &Table, out: &mut String) {
    let cols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).sum::<usize>()).max().unwrap_or(0).max(1);
    let widths: Vec<f32> =
        if t.grid.len() == cols && t.grid.iter().all(|w| w.is_finite() && *w > 0.0) { t.grid.clone() } else { vec![468.0 / cols as f32; cols] };
    let mut tp = Props::new();
    tp.insert("style:width", pt(widths.iter().sum()));
    tp.insert("table:align", "margins".into());
    if let Some(b) = ctx.break_next.take() {
        tp.insert("fo:break-before", b.into());
    }
    let start = ctx.page_start.take();
    if let Some(n) = start.as_ref().and_then(|s| s.number) {
        tp.insert("style:page-number", n.min(32_767).to_string());
    }
    let master = start.map(|s| format!(" style:master-page-name=\"{}\"", x(&s.master))).unwrap_or_default();
    let tstyle = ctx.auto_style("table", &master, &format!("<style:table-properties{}/>", attrs(&tp)));
    let name = ctx.name("Table");
    out.push_str(&format!("<table:table table:name=\"{name}\" table:style-name=\"{tstyle}\">"));
    for w in &widths {
        let cs = ctx.auto_style("table-column", "", &format!("<style:table-column-properties style:column-width=\"{}\"/>", pt(*w)));
        out.push_str(&format!("<table:table-column table:style-name=\"{cs}\"/>"));
    }
    // Rows marked to repeat (`w:tblHeader`) at the top of the table. The table style's first-row
    // look (`w:firstRow`) only formats a row; Word doesn't repeat it.
    let header_rows = t.rows.iter().take_while(|r| r.props.header).count();
    for (ri, row) in t.rows.iter().enumerate() {
        if ri == 0 && header_rows > 0 {
            out.push_str("<table:table-header-rows>");
        }
        out.push_str("<table:table-row>");
        let mut g = 0usize;
        for c in &row.cells {
            let span = c.span();
            if c.props.vmerge == VMerge::Continue {
                out.push_str(&"<table:covered-table-cell/>".repeat(span));
                g += span;
                continue;
            }
            let bg = c.props.shading.map(|c| format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase())).unwrap_or_default();
            let va = match c.props.valign {
                Some(VAlign::Top) => " style:vertical-align=\"top\"",
                Some(VAlign::Center) => " style:vertical-align=\"middle\"",
                Some(VAlign::Bottom) => " style:vertical-align=\"bottom\"",
                None => "",
            };
            let st = ctx.auto_style(
                "table-cell",
                "",
                &format!("<style:table-cell-properties fo:padding=\"4pt\" fo:border=\"0.5pt solid #000000\"{bg}{va}/>"),
            );
            let mut a = format!(" table:style-name=\"{st}\" office:value-type=\"string\"");
            if span > 1 {
                a.push_str(&format!(" table:number-columns-spanned=\"{span}\""));
            }
            if c.props.vmerge == VMerge::Restart {
                let rows = rows_spanned(t, ri, g);
                if rows > 1 {
                    a.push_str(&format!(" table:number-rows-spanned=\"{rows}\""));
                }
            }
            out.push_str(&format!("<table:table-cell{a}>"));
            if c.blocks.is_empty() {
                out.push_str("<text:p/>");
            }
            text::blocks(ctx, &c.blocks, out);
            out.push_str("</table:table-cell>");
            out.push_str(&"<table:covered-table-cell/>".repeat(span - 1));
            g += span;
        }
        for _ in g..cols {
            out.push_str("<table:table-cell office:value-type=\"string\"><text:p/></table:table-cell>");
        }
        out.push_str("</table:table-row>");
        if ri + 1 == header_rows {
            out.push_str("</table:table-header-rows>");
        }
    }
    out.push_str("</table:table>");
}

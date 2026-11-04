//! Tables written (ODF 1.3 §9.1): columns, rows, cells and their borders, padding and shading.

use super::{Border, BorderStyle, Borders, FBlock, FTable, Rgb, VAlign, Writer, model, odf_border, pt, x};

/// Word's default cell margins (top, left, bottom, right), points.
pub(in crate::odt) const WORD_CELL_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

/// The cell of `row` that starts at grid column `g`.
pub(in crate::odt) fn cell_starting_at(row: &[model::Cell], g: usize) -> Option<&model::Cell> {
    let mut s = 0usize;
    row.iter().find(|x| {
        let here = s == g;
        s += x.colspan.clamp(1, 63) as usize;
        here
    })
}

/// The `style:table-cell-properties` attributes of cell `c` of row `ri` of `t`, starting at grid
/// column `g0` of `cols`: its padding, shading, vertical alignment and borders. Each edge two
/// cells share is written once, as LibreOffice writes it (every cell its left and bottom, the
/// first row its top, the last column its right), so lines don't double; a side the cell
/// doesn't state gets the table's grid line, or none in a table without lines.
pub(in crate::odt) fn cell_props(t: &FTable, ri: usize, g0: usize, c: &model::Cell, cols: usize) -> String {
    let grid = (!t.borderless).then_some(Border { style: BorderStyle::Single, width: 0.5, color: Some(Rgb(0, 0, 0)), space: 0.0 });
    let line = |c: &model::Cell, pick: fn(&Borders) -> Option<Border>| match c.borders.as_ref().and_then(pick) {
        Some(b) => Some(b).filter(|b| b.is_visible()),
        None => grid,
    };
    let row = t.rows.get(ri).map(Vec::as_slice).unwrap_or(&[]);
    let left_cell = g0.checked_sub(1).and_then(|g| {
        let mut s = 0usize;
        row.iter().find(|x| {
            let end = s + x.colspan.clamp(1, 63) as usize;
            let hit = s <= g && g < end;
            s = end;
            hit
        })
    });
    let below = t.rows.get(ri + 1).and_then(|r| cell_starting_at(r, g0));
    let left = line(c, |b| b.left).or_else(|| left_cell.and_then(|l| line(l, |b| b.right)));
    let bottom = line(c, |b| b.bottom).or_else(|| below.and_then(|b| line(b, |x| x.top)));
    let top = if ri == 0 { line(c, |b| b.top) } else { None };
    let right = if g0 + c.colspan.clamp(1, 63) as usize >= cols { line(c, |b| b.right) } else { None };
    let m = c.margins.filter(|m| m.iter().all(|v| v.is_finite())).unwrap_or(WORD_CELL_MARGINS);
    let mut p = format!(
        " fo:padding-top=\"{}\" fo:padding-left=\"{}\" fo:padding-bottom=\"{}\" fo:padding-right=\"{}\"",
        pt(m[0].max(0.0)),
        pt(m[1].max(0.0)),
        pt(m[2].max(0.0)),
        pt(m[3].max(0.0))
    );
    p.push_str(&format!(
        " fo:border-left=\"{}\" fo:border-right=\"{}\" fo:border-top=\"{}\" fo:border-bottom=\"{}\"",
        odf_border(left),
        odf_border(right),
        odf_border(top),
        odf_border(bottom)
    ));
    if let Some(bg) = c.shading {
        p.push_str(&format!(" fo:background-color=\"#{}\"", bg.hex().to_ascii_lowercase()));
    }
    p.push_str(match c.valign {
        Some(VAlign::Top) => " style:vertical-align=\"top\"",
        Some(VAlign::Center) => " style:vertical-align=\"middle\"",
        Some(VAlign::Bottom) => " style:vertical-align=\"bottom\"",
        None => "",
    });
    // Which way its text runs (§20.404; upward is LibreOffice's own `bt-lr`).
    p.push_str(match c.direction {
        wordcraft_doc::props::TextDirection::Horizontal => "",
        wordcraft_doc::props::TextDirection::Down => " style:writing-mode=\"tb-rl\"",
        wordcraft_doc::props::TextDirection::Up => " loext:writing-mode=\"bt-lr\"",
    });
    p
}

/// Separates a cell style's `style:table-cell-properties` attributes from its content (a
/// background picture) in [`Writer::cell_style`]'s key.
pub(in crate::odt) const CELL_CHILD: char = '\u{1}';

impl Writer {
    /// A cell's background picture (§17.3 `style:background-image`), the picture written into
    /// the package (once for the same bytes).
    fn cell_picture(&mut self, pic: &model::CellPic) -> String {
        // The same picture in many cells is one file.
        let hash = pic.img.data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
        let name = format!("Pictures/cell-{hash:016x}.{}", pic.img.ext);
        if !self.pictures.iter().any(|(n, _, _)| *n == name) {
            self.pictures.push((name.clone(), pic.img.data.clone(), super::mime_of(&pic.img.ext).to_string()));
        }
        let repeat = match pic.repeat {
            wordcraft_doc::props::PictureRepeat::None => "no-repeat",
            wordcraft_doc::props::PictureRepeat::Tile => "repeat",
            wordcraft_doc::props::PictureRepeat::Stretch => "stretch",
        };
        let pos = |a: f32, words: [&'static str; 3]| {
            if a < 0.25 {
                words[0]
            } else if a > 0.75 {
                words[2]
            } else {
                words[1]
            }
        };
        format!(
            "<style:background-image xlink:href=\"{}\" xlink:type=\"simple\" xlink:actuate=\"onLoad\" style:position=\"{} {}\" style:repeat=\"{repeat}\"/>",
            x(&name),
            pos(pic.align[1], ["top", "center", "bottom"]),
            pos(pic.align[0], ["left", "center", "right"]),
        )
    }

    /// The automatic style of a cell with `props` (its `style:table-cell-properties` attributes).
    pub(in crate::odt) fn cell_style(&mut self, props: String) -> String {
        let k = match self.cell_styles.iter().position(|s| *s == props) {
            Some(k) => k,
            None => {
                self.cell_styles.push(props);
                self.cell_styles.len() - 1
            }
        };
        format!("{}Cell{}", self.prefix, k + 1)
    }

    pub(in crate::odt) fn table(&mut self, t: &FTable, out: &mut String, depth: usize) {
        // As read, while it is what was read.
        if let Some(xml) = &t.kept
            && (self.master.is_none() || self.kept_pages)
            && self.kept_markup(xml, out)
        {
            self.master = None;
            out.push('\n');
            return;
        }
        self.close_ins(out);
        let cols = t.cols().max(1);
        let widths: Vec<f32> = if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 0.0) {
            t.widths.clone()
        } else {
            vec![468.0 / cols as f32; cols]
        };
        self.tables.push(widths);
        self.table_looks.push(t.extra.as_ref().map(|x| (x.align, x.width_pct.filter(|p| *p < 100.0))).unwrap_or_default());
        // Its first paragraph starting a page: the table does (§20.191).
        let first = t.rows.first().and_then(|r| r.first()).and_then(|c| c.blocks.first());
        self.table_spaces.push(t.extra.as_ref().map(|x| x.space).unwrap_or_default());
        self.table_breaks.push(matches!(first, Some(FBlock::Para(p)) if p.page_break));
        self.table_starts.push(self.master.take());
        let tn = format!("{}{}", self.prefix, self.tables.len());
        out.push_str(&format!("<table:table table:name=\"Table{tn}\" table:style-name=\"Table{tn}\">"));
        for c in 0..cols {
            out.push_str(&format!("<table:table-column table:style-name=\"Table{tn}.C{}\"/>", c + 1));
        }
        // The rows repeated on each page (a table style's header row alone repeats nothing).
        let header_rows = match &t.extra {
            Some(x) => x.header_rows.min(t.rows.len()),
            None => t.rows.iter().take_while(|r| !r.is_empty() && r.iter().all(|c| c.header)).count(),
        };
        for (ri, row) in t.rows.iter().enumerate() {
            if ri == 0 && header_rows > 0 {
                out.push_str("<table:table-header-rows>");
            }
            // A row of a set height: exact (`style:row-height`) or at least (`style:min-row-height`).
            match t.extra.as_ref().and_then(|x| x.heights.get(ri).copied().flatten()) {
                Some((h, exact)) => {
                    let props = if exact { format!(" style:row-height=\"{}\"", pt(h)) } else { format!(" style:min-row-height=\"{}\"", pt(h)) };
                    let k = match self.row_styles.iter().position(|s| *s == props) {
                        Some(k) => k,
                        None => {
                            self.row_styles.push(props);
                            self.row_styles.len() - 1
                        }
                    };
                    out.push_str(&format!("<table:table-row table:style-name=\"{}Row{}\">", self.prefix, k + 1));
                }
                None => out.push_str("<table:table-row>"),
            }
            let mut g = 0usize;
            for c in row {
                let span = c.colspan.clamp(1, 63) as usize;
                g += span;
                if c.covered {
                    for _ in 0..span {
                        out.push_str("<table:covered-table-cell/>");
                    }
                    continue;
                }
                let mut props = cell_props(t, ri, g - span, c, cols);
                if let Some(pic) = &c.picture {
                    props.push(CELL_CHILD);
                    props.push_str(&self.cell_picture(pic));
                }
                let st = self.cell_style(props);
                // A formula field: the cell's formula, and the value it shows (§19.646, §19.388).
                let formula = crate::odt::cell_formula::of_cell(&c.blocks)
                    .and_then(|(instr, result)| Some((crate::odt::cell_formula::odf_of(instr, (ri, g - span), (t.rows.len(), cols))?, result)));
                let value = match formula {
                    Some((f, result)) => match result.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
                        Some(v) => format!(" table:formula=\"{}\" office:value-type=\"float\" office:value=\"{v}\"", x(&f)),
                        None => format!(" table:formula=\"{}\"", x(&f)),
                    },
                    None => match crate::odt::cell_formula::number_of(&c.blocks) {
                        // A number: formulas calculate with it.
                        Some(v) => format!(" office:value-type=\"float\" office:value=\"{v}\""),
                        None => " office:value-type=\"string\"".to_string(),
                    },
                };
                let mut attrs = format!(" table:style-name=\"{st}\"{value}");
                if span > 1 {
                    attrs.push_str(&format!(" table:number-columns-spanned=\"{span}\""));
                }
                if c.rowspan > 1 {
                    attrs.push_str(&format!(" table:number-rows-spanned=\"{}\"", c.rowspan));
                }
                out.push_str(&format!("<table:table-cell{attrs}>"));
                if c.blocks.is_empty() {
                    out.push_str("<text:p text:style-name=\"Standard\"/>");
                }
                // With the sections kept around its blocks, closed in it.
                self.story_blocks(&c.blocks, out, depth + 1);
                self.close_ins(out);
                out.push_str("</table:table-cell>");
                for _ in 1..span {
                    out.push_str("<table:covered-table-cell/>");
                }
            }
            for _ in g..cols {
                out.push_str("<table:table-cell office:value-type=\"string\"><text:p/></table:table-cell>");
            }
            out.push_str("</table:table-row>");
            if ri + 1 == header_rows {
                out.push_str("</table:table-header-rows>");
            }
        }
        out.push_str("</table:table>\n");
    }
}

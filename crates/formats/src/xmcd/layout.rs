use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use wordcraft_doc::props::{Align, Border, Borders, CharProps, TableProps, VAlign};
use wordcraft_doc::{Block, Blocks, Cell, Paragraph, Row, Table};

use super::{Node, position};

pub(super) struct TextStyles<'a>(HashMap<&'a str, &'a Node>);

impl<'a> TextStyles<'a> {
    pub(super) fn new(worksheet: &'a Node) -> Self {
        let styles = worksheet
            .child(1, "settings")
            .and_then(|settings| settings.child(1, "presentation"))
            .and_then(|presentation| presentation.child(1, "textRendering"))
            .and_then(|rendering| rendering.child(1, "textStyles"));
        Self(
            styles
                .into_iter()
                .flat_map(|styles| &styles.children)
                .filter(|style| style.ns == 1 && style.name == "textStyle")
                .map(|style| (style.attr("name"), style))
                .collect(),
        )
    }

    fn property(&self, paragraph: &'a Node, group: &str, key: &str) -> &'a str {
        let direct = paragraph.attr(key);
        if !matches!(direct, "" | "inherit") {
            return direct;
        }
        let mut style = paragraph.attr("style");
        for _ in 0..32 {
            let Some(node) = self.0.get(style) else { break };
            if let Some(value) = node.child(1, group).map(|attrs| attrs.attr(key)).filter(|value| !matches!(*value, "" | "inherit")) {
                return value;
            }
            style = node.attr("base-style");
        }
        ""
    }

    pub(super) fn character_props(&self, source: &'a Node) -> CharProps {
        let property = |key: &str| self.property(source, "inlineAttr", key);
        let mut props = CharProps::default();
        character_attrs(&mut props, property);
        props
    }

    pub(super) fn apply(&self, source: &'a Node, paragraph: &mut Paragraph) {
        paragraph.props.align = match self.property(source, "blockAttr", "text-align") {
            "left" => Some(Align::Left),
            "right" => Some(Align::Right),
            "center" => Some(Align::Center),
            "justify" | "justified" => Some(Align::Justify),
            _ => None,
        };
        let points = |key| self.property(source, "blockAttr", key).parse::<f32>().ok().filter(|value| value.is_finite() && value.abs() <= 720.0);
        paragraph.props.indent_left = points("margin-left");
        paragraph.props.indent_right = points("margin-right");
        paragraph.props.indent_first = points("text-indent");
    }
}

pub(super) fn character_attrs<'a>(props: &mut CharProps, property: impl Fn(&str) -> &'a str) {
    let font = property("font-family");
    if !matches!(font, "" | "inherit") && !font.contains([';', '#']) {
        props.font = Some(font.to_owned());
    }
    if let Some(size) = property("font-size").parse::<f32>().ok().filter(|size| (1.0..=400.0).contains(size)) {
        props.size = Some(size);
    }
    match property("font-weight") {
        "bold" => props.bold = Some(true),
        "normal" => props.bold = Some(false),
        _ => {}
    }
    match property("font-style") {
        "italic" | "oblique" => props.italic = Some(true),
        "normal" => props.italic = Some(false),
        _ => {}
    }
}

pub(super) struct Placed<'a> {
    pub(super) container: &'a Node,
    pub(super) region: &'a Node,
    pub(super) blocks: Range<usize>,
}

#[derive(Clone, Copy)]
struct Bounds {
    left: u32,
    right: u32,
    baseline: f64,
}

fn bounds(region: &Node) -> Option<Bounds> {
    if region.child(1, "math").is_none() && region.child(1, "text").is_none() {
        return None;
    }
    let left = region.attr("left").parse::<f64>().ok()?;
    let width = region.attr("width").parse::<f64>().ok()?;
    let right = left + width;
    if !(0.0..=10_000.0).contains(&left) || !(left + 1.0..=10_000.0).contains(&right) {
        return None;
    }
    let baseline = region.attr("align-y").parse::<f64>().ok().filter(|value| value.is_finite()).unwrap_or_else(|| position(region, "top"));
    Some(Bounds { left: left.round() as u32, right: right.round() as u32, baseline })
}

struct Positioned<'a> {
    bounds: Bounds,
    placement: &'a Placed<'a>,
}

fn same_row(previous: Bounds, next: Bounds) -> bool {
    next.left >= previous.left.saturating_add(12) && next.left.saturating_add(4) >= previous.right && (next.baseline - previous.baseline).abs() <= 6.0
}

fn end_of_cell(row: &[Positioned<'_>], index: usize, cell: &Positioned<'_>) -> u32 {
    let Some(next) = row.get(index + 1) else { return cell.bounds.right };
    if cell.placement.region.child(1, "math").is_some() { next.bounds.left } else { cell.bounds.right.min(next.bounds.left) }
}

fn column_edges<'a>(row: &'a [Positioned<'a>]) -> impl Iterator<Item = u32> + 'a {
    row.iter().enumerate().flat_map(|(index, cell)| [cell.bounds.left, end_of_cell(row, index, cell)])
}

fn table(blocks: &Blocks, rows: &[Vec<Positioned<'_>>], edges: &BTreeSet<u32>, page_width: f32) -> Option<Table> {
    let grid: Vec<_> = edges.iter().copied().collect();
    let start = *grid.first()? as f32;
    let end = *grid.last()? as f32;
    let scale = (page_width / end).min(1.0);
    let make_cell = |blocks: Blocks, span: usize| {
        let mut cell = Cell::empty();
        if !blocks.is_empty() {
            cell.blocks = blocks;
        }
        cell.props.span = span as u32;
        cell.props.valign = Some(VAlign::Top);
        cell
    };
    let mut table_rows = Vec::new();
    for row in rows {
        let mut cursor = 0;
        let mut cells = Vec::new();
        for (index, item) in row.iter().enumerate() {
            let first = grid.binary_search(&item.bounds.left).ok()?;
            let content = blocks.get(item.placement.blocks.clone())?.to_vec();
            let last = if index + 1 == row.len()
                && item.placement.region.child(1, "text").is_some()
                && content.iter().all(|block| block.as_para().is_some_and(|p| !matches!(p.props.align, Some(Align::Right | Align::Center))))
            {
                grid.len() - 1
            } else {
                grid.binary_search(&end_of_cell(row, index, item)).ok()?
            };
            if first > cursor {
                cells.push(make_cell(Vec::new(), first - cursor));
            }
            cells.push(make_cell(content, last.checked_sub(first)?.max(1)));
            cursor = last;
        }
        if cursor + 1 < grid.len() {
            cells.push(make_cell(Vec::new(), grid.len() - 1 - cursor));
        }
        table_rows.push(Row { cells, ..Default::default() });
    }
    Some(Table {
        props: TableProps {
            borders: Some(Borders::all(Border::default())),
            width: Some((end - start) * scale),
            indent: Some(start * scale),
            cell_margins: Some([2.0, 3.0, 2.0, 5.0]),
            fixed: true,
            ..Default::default()
        },
        grid: grid
            .windows(2)
            .filter_map(|pair| {
                let [left, right] = pair else { return None };
                Some((right - left) as f32 * scale)
            })
            .collect(),
        rows: table_rows,
        ..Default::default()
    })
}

pub(super) fn arrange(blocks: &Blocks, placed: &[Placed<'_>], page_width: f32) -> Blocks {
    if !page_width.is_finite() || page_width <= 0.0 {
        return blocks.clone();
    }
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(first) = placed.get(at) {
        let mut rows = Vec::new();
        let mut edges = BTreeSet::new();
        let mut next = at;
        while let Some(head) = placed.get(next).filter(|head| std::ptr::eq(head.container, first.container)) {
            let Some(head_bounds) = bounds(head.region) else { break };
            let mut row = vec![Positioned { bounds: head_bounds, placement: head }];
            for item in placed.iter().skip(next + 1).take(wordcraft_doc::table::MAX_COLS / 2 - 1) {
                let Some(geometry) = bounds(item.region) else { break };
                if !std::ptr::eq(item.container, head.container) || (head_bounds.baseline - geometry.baseline).abs() > 6.0 {
                    break;
                }
                let index = row.partition_point(|cell| cell.bounds.left < geometry.left);
                if index.checked_sub(1).and_then(|index| row.get(index)).is_some_and(|left| !same_row(left.bounds, geometry))
                    || row.get(index).is_some_and(|right| !same_row(geometry, right.bounds))
                    || (item.region.child(1, "math").is_some() && row.iter().skip(index).any(|cell| cell.placement.region.child(1, "math").is_some()))
                {
                    break;
                }
                row.insert(index, Positioned { bounds: geometry, placement: item });
            }
            if row.len() < 2 || rows.len() == wordcraft_doc::table::MAX_ROWS {
                break;
            }
            let combined: BTreeSet<_> = edges.iter().copied().chain(column_edges(&row)).collect();
            if combined.len() > wordcraft_doc::table::MAX_COLS + 1 {
                break;
            }
            next += row.len();
            rows.push(row);
            edges = combined;
        }
        if !rows.is_empty()
            && let Some(table) = table(blocks, &rows, &edges, page_width)
        {
            out.push(Block::Table(table).into());
            at = next;
        } else {
            out.extend_from_slice(blocks.get(first.blocks.clone()).unwrap_or_default());
            at += 1;
        }
    }
    out
}

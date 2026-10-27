use super::block::{Block, Blocks};
use super::parts::Part;
use super::*;

pub fn para_block(p: Paragraph) -> Arc<Block> {
    Arc::new(Block::Para(p))
}

/// The same blocks: the same content. The blocks are compared by address first — the
/// document is copy-on-write, so a block that changed was copied and has another address —
/// and a block that was copied but holds the same content is compared by value.
pub(crate) fn same_blocks(a: &Blocks, b: &Blocks) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y) || same_block(x, y))
}

/// The same block by value. A table's cells compare their blocks by address first too: an edit in
/// one cell copies the whole table, which mustn't mean comparing every cell by value.
fn same_block(a: &Block, b: &Block) -> bool {
    match (a, b) {
        (Block::Table(x), Block::Table(y)) => same_table(x, y),
        _ => a == b,
    }
}

/// The same table. Every field is listed, so a new one does not build until someone decides
/// whether a change to it counts.
fn same_table(a: &Table, b: &Table) -> bool {
    let Table { props, grid, rows, controls } = a;
    let Table { props: other_props, grid: other_grid, rows: other_rows, controls: other_controls } = b;
    props == other_props
        && grid == other_grid
        && controls == other_controls
        && rows.len() == other_rows.len()
        && rows.iter().zip(other_rows).all(|(x, y)| same_row(x, y))
}

fn same_row(a: &Row, b: &Row) -> bool {
    let Row { props, cells, controls } = a;
    let Row { props: other_props, cells: other_cells, controls: other_controls } = b;
    props == other_props
        && controls == other_controls
        && cells.len() == other_cells.len()
        && cells.iter().zip(other_cells).all(|(x, y)| {
            let Cell { props, blocks, controls } = x;
            let Cell { props: other_props, blocks: other_blocks, controls: other_controls } = y;
            props == other_props && controls == other_controls && same_blocks(blocks, other_blocks)
        })
}

/// The same building blocks, their content compared by address first.
pub(crate) fn same_glossary(a: &[BuildingBlock], b: &[BuildingBlock]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            let BuildingBlock { name, gallery, category, description, guid, style, types, behaviors, blocks } = x;
            let BuildingBlock {
                name: other_name,
                gallery: other_gallery,
                category: other_category,
                description: other_description,
                guid: other_guid,
                style: other_style,
                types: other_types,
                behaviors: other_behaviors,
                blocks: other_blocks,
            } = y;
            name == other_name
                && gallery == other_gallery
                && category == other_category
                && description == other_description
                && guid == other_guid
                && style == other_style
                && types == other_types
                && behaviors == other_behaviors
                && same_blocks(blocks, other_blocks)
        })
}

/// The same parts: the same keys, and each part the same.
pub(crate) fn same_parts(a: &BTreeMap<u32, Part>, b: &BTreeMap<u32, Part>) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|((k1, p1), (k2, p2))| k1 == k2 && same_part(p1, p2))
}

/// The same part: the same kind, text body and blocks. Every field is listed, so a new one
/// does not build until someone decides whether a change to it counts.
fn same_part(a: &Part, b: &Part) -> bool {
    let Part { kind, blocks, body } = a;
    let Part { kind: other_kind, blocks: other_blocks, body: other_body } = b;
    kind == other_kind && body == other_body && same_blocks(blocks, other_blocks)
}

/// The same bytes: the same content. The bytes are compared by address first — a changed
/// picture was copied, so it has another address — and a picture that was copied but holds
/// the same bytes is compared by value.
pub(crate) fn same_bytes(a: &BTreeMap<String, Arc<Vec<u8>>>, b: &BTreeMap<String, Arc<Vec<u8>>>) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|((k1, v1), (k2, v2))| k1 == k2 && (Arc::ptr_eq(v1, v2) || **v1 == **v2))
}

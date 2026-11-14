use super::{ArtifactType, DocPath, Document, Draw, GraphicKind, Node, NonZeroU32, StoryRef, TableHeaderScope, Tag, TagGroup, TagKind, TagTree};

/// A chart's or diagram's alt text: its own, else what it is ("chart", "diagram").
pub(crate) fn graphic_alt(alt: &str, kind: GraphicKind) -> &str {
    if alt.trim().is_empty() { kind.noun() } else { alt }
}

/// A picture, or a rotation or clip holding one at any depth (a rotated group's turned members, a
/// turned picture cut at its line).
pub(crate) fn holds_picture(d: &Draw) -> bool {
    match d {
        Draw::Image { .. } => true,
        Draw::Rotated { items, .. } | Draw::Clip { items, .. } => items.iter().any(holds_picture),
        _ => false,
    }
}

/// Text, or a rotation (or clip) holding it at any depth: a turned text box's lines.
pub(crate) fn holds_text(d: &Draw) -> bool {
    match d {
        Draw::Glyphs { .. } | Draw::Turned { .. } => true,
        Draw::Rotated { items, .. } | Draw::Clip { items, .. } => items.iter().any(holds_text),
        _ => false,
    }
}

/// The alt text of an inline figure: a graphic's (see [`graphic_alt`]) or a described shape's.
pub(crate) fn figure_alt(alt: &str, kind: Option<GraphicKind>) -> &str {
    match kind {
        Some(k) => graphic_alt(alt, k),
        None => alt,
    }
}

/// A structure element being collected (one per paragraph or figure, in reading order).
pub(crate) struct TagEntry {
    pub(crate) kind: TagKind,
    pub(crate) children: Vec<Node>,
    /// Paragraph `(story, path)` this entry belongs to, used to nest table cell entries under `Table`/`TR`/`TH` or `TD`.
    pub(crate) source: Option<(StoryRef, DocPath)>,
}

/// What the content being drawn is, for tagging.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Role<'a> {
    /// Paragraph content of the structure entry at this index.
    Para(usize),
    /// Header/footer, decorations: not part of the structure.
    Artifact(ArtifactType),
    /// A figure (its own `Figure`), optionally anchored to `(story, path)`.
    Figure(Option<(StoryRef, &'a DocPath)>),
}

enum StructNode {
    Leaf(TagGroup),
    Table(TableNode),
}

struct TableNode {
    story: StoryRef,
    path: DocPath,
    rows: Vec<RowNode>,
}

struct RowNode {
    index: usize,
    cells: Vec<CellNode>,
}

struct CellNode {
    index: usize,
    children: Vec<StructNode>,
}

/// Build the PDF structure tree, nesting table cell entries under `Table` -> `TR` -> `TH` / `TD`.
pub(crate) fn build_tag_tree(doc: &Document, tags: Vec<TagEntry>) -> TagTree {
    let mut roots = Vec::new();
    for e in tags {
        let group = (!e.children.is_empty()).then(|| TagGroup::with_children(e.kind, e.children));
        match (e.source, group) {
            // A blank cell still gets its TD/TH, so the row keeps every column's position.
            (Some((story, path)), group) if path.0.len() >= 4 => insert_into_nodes(&mut roots, story, &path, group),
            (_, Some(group)) => roots.push(StructNode::Leaf(group)),
            _ => {}
        }
    }
    let mut tree = TagTree::new().with_lang(doc.styles.default_chr.lang.clone());
    for node in roots {
        tree.push(into_tag_group(doc, node));
    }
    tree
}

fn insert_into_nodes(mut nodes: &mut Vec<StructNode>, story: StoryRef, path: &DocPath, group: Option<TagGroup>) {
    let mut k = 0usize;
    while k.saturating_add(3) < path.0.len() {
        let Some(prefix) = path.0.get(..=k) else { break };
        let Some(&row_raw) = path.0.get(k + 1) else { break };
        let Some(&cell_raw) = path.0.get(k + 2) else { break };
        let table_path = DocPath(prefix.to_vec());
        let row_idx = row_raw as usize;
        let cell_idx = cell_raw as usize;

        let table_pos = nodes.iter().rposition(|n| matches!(n, StructNode::Table(t) if t.story == story && t.path == table_path));
        let table_pos = match table_pos {
            Some(pos) => pos,
            None => {
                nodes.push(StructNode::Table(TableNode { story, path: table_path, rows: Vec::new() }));
                nodes.len() - 1
            }
        };
        let Some(StructNode::Table(table)) = nodes.get_mut(table_pos) else { return };

        let row_pos = match table.rows.binary_search_by_key(&row_idx, |r| r.index) {
            Ok(pos) => pos,
            Err(pos) => {
                table.rows.insert(pos, RowNode { index: row_idx, cells: Vec::new() });
                pos
            }
        };
        let Some(row) = table.rows.get_mut(row_pos) else { return };

        let cell_pos = match row.cells.binary_search_by_key(&cell_idx, |c| c.index) {
            Ok(pos) => pos,
            Err(pos) => {
                row.cells.insert(pos, CellNode { index: cell_idx, children: Vec::new() });
                pos
            }
        };
        let Some(cell) = row.cells.get_mut(cell_pos) else { return };

        nodes = &mut cell.children;
        k = k.saturating_add(3);
    }
    if let Some(group) = group {
        nodes.push(StructNode::Leaf(group));
    }
}

fn into_tag_group(doc: &Document, node: StructNode) -> TagGroup {
    match node {
        StructNode::Leaf(g) => g,
        StructNode::Table(t) => {
            let doc_table = doc.table(t.story, &t.path);
            let row_nodes = t
                .rows
                .into_iter()
                .map(|r| {
                    let doc_row = doc_table.and_then(|dt| dt.rows.get(r.index));
                    let is_header = doc_row.is_some_and(|dr| dr.props.header);
                    let cell_nodes = r
                        .cells
                        .into_iter()
                        .map(|c| {
                            let doc_cell = doc_row.and_then(|dr| dr.cells.get(c.index));
                            let col_span = doc_cell.and_then(|dc| NonZeroU32::new(dc.span() as u32)).filter(|n| n.get() > 1);
                            let row_span =
                                doc_table.and_then(|dt| cell_row_span(dt, r.index, c.index)).and_then(NonZeroU32::new).filter(|n| n.get() > 1);
                            let tag: TagKind = if is_header {
                                Tag::TH(TableHeaderScope::Column).with_col_span(col_span).with_row_span(row_span).into()
                            } else {
                                Tag::TD.with_col_span(col_span).with_row_span(row_span).into()
                            };
                            let children = c.children.into_iter().map(|ch| Node::Group(into_tag_group(doc, ch))).collect();
                            Node::Group(TagGroup::with_children(tag, children))
                        })
                        .collect();
                    Node::Group(TagGroup::with_children(Tag::TR, cell_nodes))
                })
                .collect();
            TagGroup::with_children(Tag::Table, row_nodes)
        }
    }
}

fn cell_row_span(table: &wordcraft_doc::Table, row_idx: usize, cell_idx: usize) -> Option<u32> {
    let row = table.rows.get(row_idx)?;
    let cell = row.cells.get(cell_idx)?;
    if cell.props.vmerge != wordcraft_doc::props::VMerge::Restart {
        return None;
    }
    let grid_col = row.first_grid().saturating_add(row.cells.get(..cell_idx)?.iter().map(wordcraft_doc::Cell::span).sum::<usize>());
    let mut span = 1u32;
    for next_row in table.rows.get(row_idx.saturating_add(1)..)? {
        let mut g = next_row.first_grid();
        let mut matched = false;
        for nc in &next_row.cells {
            if g == grid_col && nc.props.vmerge == wordcraft_doc::props::VMerge::Continue {
                matched = true;
                break;
            }
            g = g.saturating_add(nc.span());
            if g > grid_col {
                break;
            }
        }
        if matched {
            span = span.saturating_add(1);
        } else {
            break;
        }
    }
    (span > 1).then_some(span)
}

#[cfg(test)]
mod holds_tests {
    use super::{Draw, holds_text};

    /// A rotated shape with no text isn't the paragraph's text; a turned text box's lines are.
    #[test]
    fn only_a_rotation_holding_text_holds_text() {
        let turned = |items| Draw::Rotated { cx: 0.0, cy: 0.0, spin: Default::default(), items };
        let text = Draw::Turned { x: 0.0, y: 0.0, turn: Default::default(), items: Vec::new() };
        assert!(!holds_text(&turned(Vec::new())));
        assert!(!holds_text(&turned(vec![turned(Vec::new())])));
        assert!(holds_text(&turned(vec![text.clone()])));
        assert!(holds_text(&turned(vec![Draw::Clip { rect: Default::default(), items: vec![turned(vec![text])] }])));
    }
}

use super::{ArtifactType, BTreeMap, Draw, GraphicKind, ListNumbering, Node, StoryRef, Tag, TagGroup, TagKind};

/// A chart's or diagram's alt text: its own, else what it is ("chart", "diagram").
pub(crate) fn graphic_alt(alt: &str, kind: GraphicKind) -> &str {
    if alt.trim().is_empty() { kind.noun() } else { alt }
}

/// A picture, or a rotation holding one at any depth (a rotated group's turned members).
pub(crate) fn holds_picture(d: &Draw) -> bool {
    match d {
        Draw::Image { .. } => true,
        Draw::Rotated { items, .. } => items.iter().any(holds_picture),
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

/// A structure element being collected (one per paragraph, in reading order).
pub(crate) struct TagEntry {
    pub(crate) kind: TagKind,
    pub(crate) children: Vec<Node>,
    /// A list item's place in its list; its content goes in the item's `LBody`.
    pub(crate) list: Option<ListPlace>,
    /// A list item's number or bullet, for the item's `Lbl`.
    pub(crate) label: Vec<Node>,
    /// Where its content is in the document (story and paragraph path): content in a table cell
    /// goes in that cell's `TD` or `TH`.
    pub(crate) at: Option<(StoryRef, Vec<u32>)>,
}

impl TagEntry {
    pub(crate) fn new(kind: TagKind, children: Vec<Node>) -> Self {
        Self { kind, children, list: None, label: Vec::new(), at: None }
    }
}

/// Structure entries in reading order, the content of each table gathered in the table's place.
pub(crate) enum Slot {
    Entry(TagEntry),
    Table(TableSlot),
}

/// A table being collected: the content of its cells by (row, cell), each in reading order. A
/// row split across pages, or a cell whose text runs onto the next page, still fills one cell.
pub(crate) struct TableSlot {
    pub(crate) story: StoryRef,
    /// The table block's path.
    pub(crate) path: Vec<u32>,
    pub(crate) cells: BTreeMap<(usize, usize), Vec<Slot>>,
}

/// Put entry `e`, whose paragraph is at `path`, in `slots`: in the cell holding it when the path
/// goes on into a table past its first `k` indices (a cell's paragraph path is the table's path,
/// then row, cell and block), nested tables included.
pub(crate) fn slot_in(slots: &mut Vec<Slot>, story: StoryRef, path: &[u32], k: usize, e: TagEntry) {
    let cell = match (path.get(..k), path.get(k..)) {
        (Some(prefix), Some(&[t, r, c, _, ..])) => Some((prefix, t, r, c)),
        _ => None,
    };
    let Some((prefix, t, r, c)) = cell else {
        slots.push(Slot::Entry(e));
        return;
    };
    let mut table = prefix.to_vec();
    table.push(t);
    let i = match slots.iter().rposition(|s| matches!(s, Slot::Table(ts) if ts.story == story && ts.path == table)) {
        Some(i) => i,
        None => {
            slots.push(Slot::Table(TableSlot { story, path: table, cells: BTreeMap::new() }));
            slots.len() - 1
        }
    };
    if let Some(Slot::Table(ts)) = slots.get_mut(i) {
        slot_in(ts.cells.entry((r as usize, c as usize)).or_default(), story, path, k + 3, e);
    }
}

/// Where a list paragraph sits: consecutive paragraphs of the same story and container (body,
/// cell, note…) are items of one list, nested by their list level.
#[derive(Clone, PartialEq)]
pub(crate) struct ListPlace {
    pub(crate) story: StoryRef,
    /// The path of the paragraph's container (its own path without the last index).
    pub(crate) container: Vec<u32>,
    /// The paragraph's index in its container.
    pub(crate) block: u32,
    /// The list (numbering instance) it belongs to: a different list right after it starts a new
    /// `L`, as in Word.
    pub(crate) num: u32,
    pub(crate) level: u8,
    pub(crate) numbering: ListNumbering,
}

/// A list item waiting to be grouped: its list, level and numbering, its label and its body.
pub(crate) type PendingItem = Option<(u32, u8, ListNumbering, Vec<Node>, Vec<Node>)>;

/// A list item being assembled: its label and its body (the paragraph's content, then any
/// deeper list nested in it).
struct ListItem {
    label: Vec<Node>,
    body: Vec<Node>,
}

/// The list items in `items` from `*i` on at `level` or deeper, as one `L`; deeper items are
/// nested lists inside the body of the item before them (an item with no item before it at
/// this level gets an item of its own with no label).
pub(crate) fn list_group(items: &mut [PendingItem], i: &mut usize, list: (u32, u8, ListNumbering)) -> Node {
    let (num, level, numbering) = list;
    let mut out: Vec<ListItem> = Vec::new();
    while let Some(Some((m, l, n, _, _))) = items.get(*i) {
        let (m, l, n) = (*m, *l, *n);
        if l < level {
            break;
        }
        if l == level {
            // Another list (or another kind of numbering) at this level: a list of its own.
            if (m, n) != (num, numbering) {
                break;
            }
            let Some((_, _, _, label, body)) = items.get_mut(*i).and_then(Option::take) else { break };
            out.push(ListItem { label, body });
            *i += 1;
        } else {
            let nested = list_group(items, i, (m, l, n));
            match out.last_mut() {
                Some(item) => item.body.push(nested),
                None => out.push(ListItem { label: Vec::new(), body: vec![nested] }),
            }
        }
    }
    let lis = out
        .into_iter()
        .map(|item| {
            let mut parts = Vec::new();
            if !item.label.is_empty() {
                parts.push(Node::Group(TagGroup::with_children(Tag::Lbl, item.label)));
            }
            // An item with a number but no text has no body.
            if !item.body.is_empty() {
                parts.push(Node::Group(TagGroup::with_children(Tag::LBody, item.body)));
            }
            Node::Group(TagGroup::with_children(Tag::LI, parts))
        })
        .collect();
    Node::Group(TagGroup::with_children(Tag::L(numbering), lis))
}

/// The PDF list numbering for a list level's number format.
pub(crate) fn list_numbering(format: wordcraft_doc::section::NumFormat) -> ListNumbering {
    use wordcraft_doc::section::NumFormat as F;
    match format {
        F::Bullet => ListNumbering::Disc,
        F::Decimal | F::DecimalZero | F::DecimalFullWidth | F::DecimalHalfWidth | F::DecimalFullWidth2 => ListNumbering::Decimal,
        F::LowerRoman => ListNumbering::LowerRoman,
        F::UpperRoman => ListNumbering::UpperRoman,
        F::LowerLetter => ListNumbering::LowerAlpha,
        F::UpperLetter => ListNumbering::UpperAlpha,
        _ => ListNumbering::None,
    }
}

/// What the content being drawn is, for tagging.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Role {
    /// Paragraph content of the structure entry at this index.
    Para(usize),
    /// The number or bullet of the list item at this index.
    Label(usize),
    /// Link text in the structure entry at this index, in the `Link` element at this child index.
    Link(usize, usize),
    /// Header/footer, decorations: not part of the structure.
    Artifact(ArtifactType),
    /// A floating picture (its own Figure).
    Figure,
}

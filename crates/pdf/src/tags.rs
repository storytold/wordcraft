use super::{ArtifactType, Draw, GraphicKind, Node, TagKind};

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
}

/// What the content being drawn is, for tagging.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Role {
    /// Paragraph content of the structure entry at this index.
    Para(usize),
    /// Header/footer, decorations: not part of the structure.
    Artifact(ArtifactType),
    /// A floating picture (its own Figure).
    Figure,
}

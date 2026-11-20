//! DOCX bookmark attributes and the structural placement of their inline model markers.
use serde::{Deserialize, Serialize};

/// Siblings of blocks (body, header or cell), rows (table) or cells (row).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkContainer {
    Block,
    Row,
    Cell,
}

/// A source boundary follows the inline marker while it remains at its paragraph's edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkPlace {
    pub container: BookmarkContainer,
    pub after: bool,
    /// Nested tables entered from this boundary's block/row/cell to its paragraph.
    pub tables: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookmarkAttrs {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col_first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col_last: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<BookmarkPlace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<BookmarkPlace>,
}

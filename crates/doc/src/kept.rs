//! What a file had that the model doesn't place on its own, kept so saving puts it back:
//! where a bookmark's start or end sat when it was not inside a paragraph, and a picture's
//! own `a:blip` extensions.

use serde::{Deserialize, Serialize};

/// Where a bookmark marker sat in the file, when not inside a paragraph. The model keeps every
/// marker inside the paragraph it borders (the first one after it, or the last one before it);
/// a writer that finds the marker still at that edge puts it back between the blocks (or rows)
/// it came from. `tables` counts the tables entered from that block (or row) to reach the
/// paragraph: a marker before a table is in the first paragraph of its first cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkPlace {
    /// Inside the paragraph that holds it.
    #[default]
    Inline,
    /// Just before the block whose first paragraph starts with it.
    BeforeBlock { tables: u8 },
    /// Just after the block whose last paragraph ends with it.
    AfterBlock { tables: u8 },
    /// In a table, between its rows: just before the row whose first paragraph starts with it.
    BeforeRow { tables: u8 },
    /// In a table, between its rows: just after the row whose last paragraph ends with it.
    AfterRow { tables: u8 },
}

/// A bookmark's markers as the file had them (see [`MarkPlace`]), and the table columns a
/// column bookmark covers (`w:colFirst`, `w:colLast`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BookmarkPlace {
    pub start: MarkPlace,
    pub end: MarkPlace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<(u16, u16)>,
}

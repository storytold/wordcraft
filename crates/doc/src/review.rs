use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Comment {
    pub author: String,
    pub initials: String,
    /// ISO 8601.
    pub date: String,
    /// Reply to this comment id.
    pub parent: Option<u32>,
    pub resolved: bool,
    /// `Document::parts` id of the comment text.
    pub part: u32,
    /// Word's durable id for the comment (`w16cid:durableId`, hex), kept across saves so Word
    /// keeps tracking the same comment. `None` for comments made here: the writer assigns one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_id: Option<String>,
    /// The comment's UTC timestamp: from the file (`w16cex:dateUtc`, where `date` is local time),
    /// or the time WordCraft stamped the comment with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_utc: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    #[default]
    Insert,
    Delete,
    Format,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: String,
    pub date: String,
    /// A move (ECMA-376 §17.13.5.21–28): the name linking the source (an `Delete` revision,
    /// `w:moveFrom`) with the destination (an `Insert` revision, `w:moveTo`). `None` = a plain
    /// insertion or deletion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
}

impl Revision {
    pub fn new(kind: RevisionKind, author: impl Into<String>, date: impl Into<String>) -> Revision {
        Revision { kind, author: author.into(), date: date.into(), move_name: None }
    }
}

impl Document {
    /// The move name of revision `idx`, when it is one end of a move.
    pub fn move_name(&self, idx: Option<u32>) -> Option<&str> {
        idx.and_then(|i| self.revisions.get(i as usize)).and_then(|r| r.move_name.as_deref())
    }
}

/// A building block (Insert › Quick Parts, AutoText…) kept in the document's glossary
/// (ECMA-376 §17.12: `w:docPart`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildingBlock {
    /// The entry's name (`w:docPartPr/w:name`).
    pub name: String,
    /// Its gallery (`w:category/w:gallery`: `docParts` for Quick Parts, `autoTxt` for AutoText…).
    pub gallery: String,
    /// Its category within the gallery (`w:category/w:name`, e.g. "General").
    pub category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The entry's identity (`w:guid`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub guid: String,
    /// The paragraph style it's associated with (`w:style`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// Its types (`w:types/w:type`: `normal`, `bbPlcHdr` for a content control's placeholder…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// How it's inserted (`w:behaviors/w:behavior`: `content`, `p`, `pg`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub behaviors: Vec<String>,
    /// The content (`w:docPartBody`).
    pub blocks: Blocks,
}

/// The gallery Quick Parts saves to (`w:gallery w:val="docParts"`).
pub const QUICK_PARTS_GALLERY: &str = "docParts";

/// The AutoText gallery (`w:gallery w:val="autoTxt"`).
pub const AUTO_TEXT_GALLERY: &str = "autoTxt";

use super::{Arc, Deserialize, Paragraph, Serialize, Table};

/// A block in a story. (Blocks are always behind an `Arc`, so the size difference is fine.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Block {
    Para(Paragraph),
    Table(Table),
}

impl Block {
    pub fn as_para(&self) -> Option<&Paragraph> {
        match self {
            Block::Para(p) => Some(p),
            Block::Table(_) => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Block::Table(t) => Some(t),
            Block::Para(_) => None,
        }
    }
}

pub type Blocks = Vec<Arc<Block>>;

/// Text boxes inside text boxes count (and show) this many levels deep.
pub const MAX_TEXT_BOX_DEPTH: usize = 4;

/// Bounds the work of expanding text boxes inside text boxes (layout, walks, saving), whatever a
/// document says: a box isn't expanded inside itself, nesting stops at [`MAX_TEXT_BOX_DEPTH`], and
/// an outermost box expands at most [`BoxBudget::MAX_NESTED`] boxes inside it. (A box whose
/// shapes all show that same box, or chains of boxes each showing the next many times, would
/// otherwise grow exponentially.) Outermost boxes aren't capped: their number is the document's
/// (or its pages', for headers), so the work stays linear and no box is left blank for budget.
#[derive(Debug, Default)]
pub struct BoxBudget {
    open: Vec<u32>,
    nested: usize,
    total: usize,
}

impl BoxBudget {
    /// Boxes expanded inside one outermost box.
    pub const MAX_NESTED: usize = 64;

    /// Start expanding text box story `id`; false if it mustn't be (pair a true with `leave`).
    pub fn enter(&mut self, id: u32) -> bool {
        if self.open.contains(&id) || self.open.len() >= MAX_TEXT_BOX_DEPTH {
            return false;
        }
        if self.open.is_empty() {
            self.nested = 0;
        } else if self.nested >= Self::MAX_NESTED {
            return false;
        } else {
            self.nested += 1;
        }
        self.total += 1;
        self.open.push(id);
        true
    }

    /// Done expanding the box last entered.
    pub fn leave(&mut self) {
        self.open.pop();
    }

    /// Whether a text box is being expanded now.
    pub fn inside(&self) -> bool {
        !self.open.is_empty()
    }

    /// Boxes expanded so far.
    pub fn total(&self) -> usize {
        self.total
    }
}

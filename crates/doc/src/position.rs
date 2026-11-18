use super::{Deserialize, Serialize};

/// Which story a position is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StoryRef {
    #[default]
    Body,
    Part(u32),
}

/// Path to a paragraph: `[block]` in a story, `[block, row, cell, block]` inside a table cell,
/// three more indexes per nesting level. Lexicographic order = document order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Path(pub Vec<u32>);

impl Path {
    pub fn top(i: usize) -> Path {
        Path(vec![i as u32])
    }
    /// The block index within its container.
    pub fn last(&self) -> usize {
        self.0.last().copied().unwrap_or(0) as usize
    }
    pub fn with_last(&self, i: usize) -> Path {
        let mut p = self.clone();
        if let Some(l) = p.0.last_mut() {
            *l = i as u32;
        }
        p
    }
    /// The container prefix (everything but the last index).
    pub fn parent(&self) -> &[u32] {
        self.0.split_last().map(|(_, r)| r).unwrap_or(&[])
    }
    /// Nesting depth (0 = top-level story block).
    pub fn depth(&self) -> usize {
        self.0.len().saturating_sub(1) / 3
    }
    /// The enclosing table cell (table block path, row, cell) if any.
    pub fn cell(&self) -> Option<(Path, usize, usize)> {
        if self.0.len() < 4 {
            return None;
        }
        let n = self.0.len();
        let t = Path(self.0.get(..n - 3)?.to_vec());
        Some((t, *self.0.get(n - 3)? as usize, *self.0.get(n - 2)? as usize))
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        write!(f, "{}", s.join("/"))
    }
}

/// A position in a document.
///
/// `affinity` is a transient visual-line hint for wrapped inline objects (such as multi-line field
/// results) whose fragments share the same document byte range. Equality, ordering, and hashing
/// compare only `(story, path, off)` and ignore `affinity`.
#[derive(Clone, Debug, Eq, Serialize, Deserialize, Default)]
pub struct Pos {
    pub story: StoryRef,
    pub path: Path,
    pub off: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<usize>,
}

impl PartialEq for Pos {
    fn eq(&self, other: &Self) -> bool {
        (self.story, &self.path, self.off) == (other.story, &other.path, other.off)
    }
}

impl Ord for Pos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.story, &self.path, self.off).cmp(&(other.story, &other.path, other.off))
    }
}

impl PartialOrd for Pos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::hash::Hash for Pos {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.story.hash(state);
        self.path.hash(state);
        self.off.hash(state);
    }
}

impl Pos {
    pub fn new(story: StoryRef, path: Path, off: usize) -> Pos {
        Pos { story, path, off, affinity: None }
    }
    pub fn with_affinity(story: StoryRef, path: Path, off: usize, affinity: Option<usize>) -> Pos {
        Pos { story, path, off, affinity }
    }
    pub fn body(block: usize, off: usize) -> Pos {
        Pos { story: StoryRef::Body, path: Path::top(block), off, affinity: None }
    }
    /// Same story and paragraph path at byte offset `off`, with visual-line affinity cleared.
    #[must_use]
    pub fn at(&self, off: usize) -> Pos {
        Pos { story: self.story, path: self.path.clone(), off, affinity: None }
    }
}

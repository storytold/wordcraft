use super::{AtomicU64, CharProps, Deserialize, Ordering, Serialize};

/// Object replacement character: marks an inline object in paragraph text.
pub const OBJ: char = '\u{FFFC}';

/// Manual line break (Shift+Enter).
pub const LINE_BREAK: char = '\n';

/// Page break (Ctrl+Enter).
pub const PAGE_BREAK: char = '\u{000C}';

/// Column break (Ctrl+Shift+Enter).
pub const COLUMN_BREAK: char = '\u{000E}';

/// Optional hyphen (Ctrl+-).
pub const SOFT_HYPHEN: char = '\u{00AD}';

/// Nonbreaking hyphen (Ctrl+Shift+-).
pub const NB_HYPHEN: char = '\u{2011}';

/// Nonbreaking space (Ctrl+Shift+Space).
pub const NBSP: char = '\u{00A0}';

static REV: AtomicU64 = AtomicU64::new(1);

/// A fresh revision number (process-unique), used by layout caches to detect edits.
pub fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}

/// A formatting run: `len` bytes of text sharing `props`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub len: usize,
    #[serde(default, skip_serializing_if = "CharProps::is_empty")]
    pub props: CharProps,
}

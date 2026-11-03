//! Fonts a document embeds belong to that document. They are added to the process-wide database
//! with an owner ([`FontFace::owner`]); lookups by family name see them only while their document
//! is laid out on this thread ([`DocumentFonts`]), so another document asking for the same family
//! (often only a subset was embedded) gets its own embedded copy, the installed font or a
//! substitute, as it would when opened alone.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use super::FontFace;

thread_local! {
    static CURRENT: Cell<u64> = const { Cell::new(0) };
}

/// While alive, lookups on this thread also see the faces `owner`'s document embedded. Dropping
/// it restores the previous document (guards nest).
#[must_use = "the document's fonts are visible only while the guard lives"]
pub struct DocumentFonts {
    prev: u64,
}

impl DocumentFonts {
    /// Make `owner`'s fonts visible on this thread (0: only the shared fonts).
    pub fn enter(owner: u64) -> Self {
        DocumentFonts { prev: CURRENT.with(|c| c.replace(owner)) }
    }

    /// The document whose fonts lookups on this thread see (0: none).
    pub fn current() -> u64 {
        CURRENT.with(Cell::get)
    }

    /// A new owner id for a document's embedded fonts (never 0).
    pub fn new_owner() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed).max(1)
    }
}

impl Drop for DocumentFonts {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.prev));
    }
}

impl FontFace {
    /// Whether lookups by name on this thread see the face: shared, or its document's.
    pub(super) fn visible(&self) -> bool {
        self.owner == 0 || self.owner == DocumentFonts::current()
    }
}

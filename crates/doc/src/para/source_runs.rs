//! Non-semantic source run boundaries for lossless DOCX reserialization.
use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};

/// Original run boundaries, guarded by a text fingerprint. No copy of deleted text is
/// retained. Changing the text invalidates the hints; changing its formatting
/// does not. The fingerprint is a compatibility hint, not a security checksum.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRuns {
    text_hash: u64,
    offsets: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    formatting_revisions: Vec<(usize, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    editing_revisions: Vec<(usize, String)>,
}

impl SourceRuns {
    pub fn new(text: &str, offsets: Vec<usize>) -> Option<Self> {
        Self::new_with_formatting_revisions(text, offsets, Vec::new())
    }

    /// `w:rsidRPr` identifies the formatting run even when adjacent runs have
    /// equal properties. Word's tracked character spacing depends on this ID.
    pub fn new_with_formatting_revisions(text: &str, offsets: Vec<usize>, revisions: Vec<(usize, String)>) -> Option<Self> {
        Self::new_with_revisions(text, offsets, revisions, Vec::new())
    }

    /// Retain both formatting (`rsidRPr`) and editing (`rsidR`) run identities.
    pub fn new_with_revisions(
        text: &str,
        mut offsets: Vec<usize>,
        mut formatting: Vec<(usize, String)>,
        mut editing: Vec<(usize, String)>,
    ) -> Option<Self> {
        offsets.retain(|&i| i > 0 && i < text.len() && text.is_char_boundary(i));
        offsets.sort_unstable();
        offsets.dedup();
        for revisions in [&mut formatting, &mut editing] {
            revisions.retain(|(i, id)| *i < text.len() && (*i == 0 || offsets.binary_search(i).is_ok()) && valid_id(id));
            revisions.sort_unstable_by_key(|r| r.0);
            revisions.dedup_by_key(|r| r.0);
        }
        (!offsets.is_empty() || !formatting.is_empty() || !editing.is_empty()).then(|| Self {
            text_hash: fingerprint(text),
            offsets,
            formatting_revisions: formatting,
            editing_revisions: editing,
        })
    }

    /// Empty for edited text or malformed hints from a native JSON document.
    pub fn offsets_for(&self, text: &str) -> &[usize] {
        if self.valid_for(text) { &self.offsets } else { &[] }
    }

    /// Validated source-run starts and their formatting revision IDs.
    pub fn formatting_revisions_for(&self, text: &str) -> &[(usize, String)] {
        self.revisions_for(text, &self.formatting_revisions)
    }

    /// Validated source-run starts and their editing revision IDs.
    pub fn editing_revisions_for(&self, text: &str) -> &[(usize, String)] {
        self.revisions_for(text, &self.editing_revisions)
    }

    fn revisions_for<'a>(&self, text: &str, revisions: &'a [(usize, String)]) -> &'a [(usize, String)] {
        if self.valid_for(text)
            && revisions.iter().all(|(i, id)| *i < text.len() && (*i == 0 || self.offsets.binary_search(i).is_ok()) && valid_id(id))
            && revisions.windows(2).all(|w| w[0].0 < w[1].0)
        {
            revisions
        } else {
            &[]
        }
    }

    fn valid_for(&self, text: &str) -> bool {
        !(self.text_hash != fingerprint(text)
            || self.offsets.iter().any(|&i| i == 0 || i >= text.len() || !text.is_char_boundary(i))
            || self.offsets.windows(2).any(|w| w[0] >= w[1]))
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn fingerprint(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

//! Pictures a reader could not bring in or an exporter could not draw, for strict export (#6005).
//! Kept beside the document while it is open; never saved, never drawn, and not part of the
//! document's content (`==` ignores it).

use serde::{Deserialize, Serialize};

/// Most pictures remembered per document; past it [`MissingPictures::truncated`] is set.
pub const MAX_MISSING_PICTURES: usize = 256;

/// Most characters of a picture's part or story name a record keeps (names come from the file).
pub const MAX_PICTURE_NAME_CHARS: usize = 1024;

/// Why a picture is not in the document or not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissingPictureReason {
    /// The relationship names a part the file does not contain, or no relationship at all.
    MissingPart,
    /// The target is outside the file (`TargetMode="External"`); it is never fetched.
    ExternalTarget,
    /// The picture only links to a file (`r:link`) and holds no embedded copy.
    LinkOnly,
    /// The bytes are a picture format we know but they do not decode.
    Undecodable,
    /// The bytes are not a picture format we can draw.
    UnsupportedFormat,
}

impl MissingPictureReason {
    /// The words used in reports (`"missing part"`, `"external target"`, `"link only"`,
    /// `"undecodable"`, `"unsupported format"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingPart => "missing part",
            Self::ExternalTarget => "external target",
            Self::LinkOnly => "link only",
            Self::Undecodable => "undecodable",
            Self::UnsupportedFormat => "unsupported format",
        }
    }

    /// The reason a [`code`](Self::code) names.
    pub fn from_code(code: &str) -> Option<Self> {
        [Self::MissingPart, Self::ExternalTarget, Self::LinkOnly, Self::Undecodable, Self::UnsupportedFormat].into_iter().find(|r| r.code() == code)
    }

    /// The stable snake_case code the JSON report carries (`"missing_part"`, `"external_target"`,
    /// `"link_only"`, `"undecodable"`, `"unsupported_format"`); never reworded.
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingPart => "missing_part",
            Self::ExternalTarget => "external_target",
            Self::LinkOnly => "link_only",
            Self::Undecodable => "undecodable",
            Self::UnsupportedFormat => "unsupported_format",
        }
    }
}

/// One picture that is missing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MissingPicture {
    /// The target as written, or the part name it resolved to (a media key for a decoded one).
    pub part: String,
    /// The story part it is in (`word/document.xml`, `word/header2.xml`), as the source file
    /// names it; empty when not known.
    pub story: String,
    pub reason: MissingPictureReason,
}

/// A capped list of [`MissingPicture`]s, with the source names a report needs to say where a
/// picture is (a reader fills them; a document made or opened another way leaves them empty).
#[derive(Clone, Debug, Default)]
pub struct MissingPictures {
    pub list: Vec<MissingPicture>,
    /// More than [`MAX_MISSING_PICTURES`] were found; the rest are not listed.
    pub truncated: bool,
    /// Pictures found, those past the cap included (saturating). Each distinct one counts once.
    pub total: u64,
    /// The main story's part name in the source file (`word/document.xml`).
    pub main: String,
    /// A header, footer, note or comment story's part id ([`crate::Document::parts`]) to the
    /// part it was read from (`word/header2.xml`). Ids freed and reused after opening may name
    /// the old part.
    pub part_names: std::collections::BTreeMap<u32, std::sync::Arc<str>>,
    /// A [`crate::Document::media`] key to the part its bytes were read from
    /// (`word/media/image2.bin`).
    pub media_parts: std::collections::BTreeMap<String, String>,
    /// What `list` holds, for an O(1) test of "already there" (a hostile file has many).
    seen: std::collections::HashSet<MissingPicture>,
    /// Hashes of the distinct pictures past the cap (8 bytes each), so the total counts them once.
    unlisted: std::collections::HashSet<u64>,
    /// The chunk parts whose unlisted pictures [`Self::add_unlisted`] has counted, so a chunk
    /// imported twice counts them once.
    counted_chunks: std::collections::HashSet<String>,
}

impl MissingPictures {
    /// Add `n` pictures of the chunk part `chunk` that its reader counted but did not list. The
    /// same chunk part counts once however often it is imported (its listed pictures dedupe
    /// through [`Self::push`]).
    pub fn add_unlisted(&mut self, chunk: &str, n: u64) {
        if n > 0 && self.counted_chunks.insert(chunk.to_string()) {
            self.total = self.total.saturating_add(n);
        }
    }

    /// Add one (the same part, story and reason once).
    pub fn push(&mut self, mut item: MissingPicture) {
        for name in [&mut item.part, &mut item.story] {
            if name.len() > MAX_PICTURE_NAME_CHARS {
                *name = name.chars().take(MAX_PICTURE_NAME_CHARS).collect();
            }
        }
        if self.seen.contains(&item) {
            return;
        }
        if self.list.len() >= MAX_MISSING_PICTURES {
            self.truncated = true;
            if self.unlisted.insert(picture_hash(&item)) {
                self.total = self.total.saturating_add(1);
            }
            return;
        }
        self.total = self.total.saturating_add(1);
        self.seen.insert(item.clone());
        self.list.push(item);
    }
}

/// A 64-bit hash of a record, to tell the ones past a cap apart without keeping them.
pub fn picture_hash(item: &MissingPicture) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    item.hash(&mut h);
    h.finish()
}

impl PartialEq for MissingPictures {
    /// Not part of the document's content.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

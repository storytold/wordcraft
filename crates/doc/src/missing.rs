//! Pictures a reader could not bring in or an exporter could not draw, for strict export (#6005).
//! Kept beside the document while it is open; never saved, never drawn, and not part of the
//! document's content (`==` ignores it).

use serde::{Deserialize, Serialize};

/// Most pictures remembered per document; past it [`MissingPictures::truncated`] is set.
pub const MAX_MISSING_PICTURES: usize = 256;

/// Why a picture is not in the document or not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// One picture that is missing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The main story's part name in the source file (`word/document.xml`).
    pub main: String,
    /// A header, footer, note or comment story's part id ([`crate::Document::parts`]) to the
    /// part it was read from (`word/header2.xml`). Ids freed and reused after opening may name
    /// the old part.
    pub part_names: std::collections::BTreeMap<u32, String>,
    /// A [`crate::Document::media`] key to the part its bytes were read from
    /// (`word/media/image2.bin`).
    pub media_parts: std::collections::BTreeMap<String, String>,
}

impl MissingPictures {
    /// Add one (the same part, story and reason once).
    pub fn push(&mut self, item: MissingPicture) {
        if self.list.contains(&item) {
            return;
        }
        if self.list.len() >= MAX_MISSING_PICTURES {
            self.truncated = true;
            return;
        }
        self.list.push(item);
    }
}

impl PartialEq for MissingPictures {
    /// Not part of the document's content.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

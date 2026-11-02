/// Largest zip entry we inflate.
pub(super) const MAX_ENTRY: u64 = 256 << 20;

/// Deepest element nesting handled in content.xml.
pub(super) const MAX_XML_DEPTH: usize = 512;

/// Most sections an import makes (page-style changes and column sections); later ones are ignored.
pub(super) const MAX_SECTIONS: usize = 10_000;

/// Most `text:changed-region`s read (hostile input).
pub(super) const MAX_REGIONS: usize = 10_000;

/// Most insertions open at once (`text:change-start` without its end yet).
pub(super) const MAX_OPEN_INSERTIONS: usize = 64;

/// Longest change author or date kept, in characters.
pub(super) const MAX_INFO_CHARS: usize = 256;

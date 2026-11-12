/// Largest `.tex` source imported (bytes); longer input is cut off.
pub const MAX_SOURCE: usize = 32 << 20;

/// Nesting limit for groups, environments and arguments (hostile input).
pub(super) const MAX_NEST: usize = 32;

/// Longest `tabular` column spec read (characters); the rest is ignored.
pub(super) const MAX_SPEC: usize = 4096;

/// Most bookmarks kept in a row with no text between them.
pub(super) const MAX_ANCHOR_RUN: usize = 64;

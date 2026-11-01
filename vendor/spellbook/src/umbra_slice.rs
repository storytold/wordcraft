//! Safe owned storage for dictionary words and sorted flags.
//! Retains the internal names used by the upstream engine; no custom allocation or raw UTF-8.
use crate::{
    alloc::{boxed::Box, string::String, vec::Vec},
    Flag,
};
use core::{borrow::Borrow, fmt, hash};

#[derive(Default, Clone, PartialEq, Eq)]
pub struct FlagSlice(Box<[Flag]>);
impl From<Vec<Flag>> for FlagSlice {
    fn from(flags: Vec<Flag>) -> Self {
        Self(flags.into_boxed_slice())
    }
}
impl FlagSlice {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn as_slice(&self) -> &[Flag] {
        &self.0
    }
    pub fn sorted_contains(&self, flag: &Flag) -> bool {
        self.0.binary_search(flag).is_ok()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UmbraString(Box<str>);
impl UmbraString {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for UmbraString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl Borrow<str> for UmbraString {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl hash::Hash for UmbraString {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
impl PartialEq<str> for UmbraString {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl fmt::Debug for UmbraString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
impl From<&str> for UmbraString {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}
impl From<String> for UmbraString {
    fn from(s: String) -> Self {
        Self(s.into_boxed_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::{BuildHasher, Hash};
    #[test]
    fn storage_preserves_unicode_and_hash_lookup_at_all_lengths() {
        for input in [
            String::new(),
            String::from("a"),
            String::from("árvíztűrő"),
            "ő".repeat(65536),
        ] {
            let owned = UmbraString::from(input.as_str());
            assert_eq!(owned.as_str(), input);
            assert_eq!(owned.clone(), owned);
            let hasher = crate::DefaultHashBuilder::default();
            assert_eq!(hasher.hash_one(&owned), hasher.hash_one(input.as_str()));
            let mut state = hasher.build_hasher();
            owned.hash(&mut state);
        }
    }
    #[test]
    fn empty_and_large_flag_sets_are_searchable() {
        let empty = FlagSlice::default();
        assert!(!empty.sorted_contains(&Flag::MIN));
        let flags: Vec<_> = (1..=u16::MAX).filter_map(Flag::new).collect();
        let owned = FlagSlice::from(flags.clone());
        assert_eq!(owned.as_slice(), flags);
        assert!(owned.sorted_contains(&Flag::MAX));
    }
}

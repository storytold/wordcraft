use std::sync::{Arc, OnceLock, RwLock};

/// The system locale (BCP 47 tag) the shell found at startup, for Numeral › System.
pub(super) static SYSTEM_LOCALE: OnceLock<String> = OnceLock::new();

/// Tell the engine the system locale (the shell calls this once at startup; layout never reads
/// the environment). Headless sessions without it show Western digits for Numeral › System.
pub fn set_system_locale(tag: &str) {
    let _ = SYSTEM_LOCALE.set(tag.to_string());
}

/// The system locale [`set_system_locale`] recorded, if the shell set one.
pub(crate) fn system_locale() -> Option<&'static str> {
    SYSTEM_LOCALE.get().map(String::as_str)
}

/// What output prints for a page reference to a missing bookmark, in the interface language
/// ([`set_bookmark_error`]); `None` until the shell sets it (English).
static BOOKMARK_ERROR: RwLock<Option<Arc<str>>> = RwLock::new(None);

/// Tell the engine what PDFs, prints and page images show for a page reference to a bookmark the
/// document doesn't have, in the interface language: Word prints "Error! Bookmark not defined."
/// in its own. The shell calls this when its language is set; headless sessions print English.
pub fn set_bookmark_error(text: &str) {
    let mut slot = BOOKMARK_ERROR.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.as_deref() != Some(text) {
        *slot = Some(Arc::from(text));
    }
}

/// The text [`set_bookmark_error`] recorded, for output's layout options.
pub(crate) fn bookmark_error() -> Option<Arc<str>> {
    BOOKMARK_ERROR.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// What output prints for a `REF` to a missing bookmark, in the interface language
/// ([`set_reference_error`]); `None` until the shell sets it (English).
static REFERENCE_ERROR: RwLock<Option<Arc<str>>> = RwLock::new(None);

/// Tell the engine what PDFs, prints and page images show for a `REF` to a bookmark the document
/// doesn't have, in the interface language: Word prints "Error! Reference source not found." in
/// its own. The shell calls this when its language is set; headless sessions print English.
pub fn set_reference_error(text: &str) {
    let mut slot = REFERENCE_ERROR.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.as_deref() != Some(text) {
        *slot = Some(Arc::from(text));
    }
}

/// The text [`set_reference_error`] recorded, for output's `REF` results.
pub(crate) fn reference_error() -> Option<Arc<str>> {
    REFERENCE_ERROR.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

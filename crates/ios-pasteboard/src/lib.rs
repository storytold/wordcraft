//! The general pasteboard of iPadOS and iOS, by uniform type identifier.
//!
//! Empty on every other platform.
#![cfg(target_os = "ios")]
// SAFETY (whole module): objc2 marks these UIPasteboard methods unsafe only because UIKit's
// headers don't say they may be called off the main thread or how their generic collections
// are typed. UIPasteboard is documented as thread-safe, the arguments are the Foundation types
// the headers name (NSString keys, NSData values), and nothing is kept past the call.
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use objc2_ui_kit::UIPasteboard;

fn general() -> Retained<UIPasteboard> {
    UIPasteboard::generalPasteboard()
}

/// Replaces the pasteboard's contents with one item holding every `(type, bytes)` pair; no pairs
/// empties it.
pub fn set_item(representations: &[(&str, &[u8])]) {
    let pasteboard = general();
    if representations.is_empty() {
        // SAFETY: see the module note; an empty array of items.
        unsafe { pasteboard.setItems(&NSArray::new()) };
        return;
    }
    let keys: Vec<Retained<NSString>> = representations.iter().map(|(uti, _)| NSString::from_str(uti)).collect();
    let values: Vec<Retained<AnyObject>> =
        representations.iter().map(|(_, bytes)| Retained::into_super(Retained::into_super(NSData::with_bytes(bytes)))).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let item = NSDictionary::from_retained_objects(&key_refs, &values);
    // SAFETY: see the module note; one dictionary of NSString keys and NSData values.
    unsafe { pasteboard.setItems(&NSArray::from_retained_slice(&[item])) };
}

/// The bytes of the first item's representation of type `uti`.
pub fn data(uti: &str) -> Option<Vec<u8>> {
    general().dataForPasteboardType(&NSString::from_str(uti)).map(|d| d.to_vec())
}

/// The pasteboard's text, whichever text type the copying app used.
pub fn string() -> Option<String> {
    // SAFETY: see the module note.
    unsafe { general().string() }.map(|s| s.to_string())
}

/// Whether the first item has a representation of one of `utis`. Doesn't ask the person's
/// permission to paste.
pub fn contains(utis: &[&str]) -> bool {
    let types: Vec<Retained<NSString>> = utis.iter().map(|u| NSString::from_str(u)).collect();
    general().containsPasteboardTypes(&NSArray::from_retained_slice(&types))
}

/// Whether the pasteboard holds text. Doesn't ask the person's permission to paste.
pub fn has_strings() -> bool {
    // SAFETY: see the module note.
    unsafe { general().hasStrings() }
}

/// Whether the pasteboard holds a picture. Doesn't ask the person's permission to paste.
pub fn has_images() -> bool {
    // SAFETY: see the module note.
    unsafe { general().hasImages() }
}

/// The types of the first item.
pub fn types() -> Vec<String> {
    // SAFETY: see the module note.
    let types = unsafe { general().pasteboardTypes() };
    types.iter().map(|t| t.to_string()).collect()
}

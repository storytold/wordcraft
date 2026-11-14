//! WordCraft adapter for reading Word 97-2003 binary documents (`.doc`, [MS-DOC]).
//!
//! The file-format implementation lives in `craft-doc-binary`. This crate adapts the shared
//! model to WordCraft's application model while keeping the public read and password APIs stable.
//! Stories are converted paragraph by paragraph, and embedded media buffers are moved without a
//! full-document serialization copy.
//!
//! [MS-DOC]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/

mod model_bridge;

pub use craft_doc_binary::DocbinError;
pub use model_bridge::from_shared_model;

/// Whether the input is a readable Word 97-2003 document protected by a password.
pub fn is_encrypted(bytes: &[u8]) -> bool {
    craft_doc_binary::is_encrypted(bytes)
}

/// Read a Word 97-2003 binary document into WordCraft's document model.
pub fn read(bytes: &[u8]) -> Result<wordcraft_doc::Document, DocbinError> {
    read_with_password(bytes, None)
}

/// Read a document that may be password-protected, preserving the shared reader's diagnostics.
pub fn read_with_password(bytes: &[u8], password: Option<&str>) -> Result<wordcraft_doc::Document, DocbinError> {
    let shared = craft_doc_binary::read_with_password(bytes, password)?;
    from_shared_model(shared)
}

#[cfg(test)]
mod tests;

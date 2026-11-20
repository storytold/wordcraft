//! Hancom Hangul documents: HWPX (OWPML, KS X 6101) and HWP 5.0.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//!
//! Sources (clean room, see `docs/hwp/cleanroom.md`): element and attribute names of HWPX from
//! Hancom's Apache-2.0 `hwpx-owpml-model`; HWP 5.0 records from Hancom's public "한글 문서 파일
//! 형식 5.0" (Hangul document file format 5.0) document; the compound-file container through the `cfb` crate ([MS-CFB]). No code or
//! documentation of other HWP/HWPX implementations was used. Where Hancom's viewer lays a document
//! out differently from what the specification says, the rule was found by comparing the viewer's
//! screen with ours (black box) and is noted where it is used.
//!
//! [`to_docx`] converts a document to OOXML bytes for `wordcraft-docx` to read (the engine joins
//! the two); every converted paragraph carries a hidden bookmark naming its source paragraph, so
//! [`write_back`] can put edits made in WordCraft back into a copy of the original HWPX, touching
//! only the paragraphs that changed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod convert;
mod docx;
mod drawing;
mod error;
mod fonts;
mod header;
mod hwp5;
mod hwp5_docinfo;
mod hwp5_enc;
mod hwp5_new;
mod hwp5_write;
mod hwpx_out;
mod package;
mod restyle;
mod units;
mod writer;
mod xml;

pub use convert::{Report, apply_keep_words};
pub use error::{Error, Result};
pub use fonts::notice as font_notice;
pub use hwp5_new::from_document as new_hwp;
pub use hwp5_write::write_back_hwp;
pub use hwpx_out::from_document as new_hwpx;
pub use package::Package;
pub use writer::{SaveReport, write_back, write_back_with};

/// Extensions this crate opens.
pub const EXTENSIONS: &[&str] = &["hwpx", "hwp"];

/// Is `bytes` a document this crate reads (by content, not by name)?
pub fn sniff(bytes: &[u8]) -> bool {
    package::is_hwpx(bytes) || hwp5::is_hwp(bytes)
}

/// Open an HWPX (ZIP) or HWP 5.0 (compound file) document, by its content.
pub fn open(bytes: &[u8]) -> Result<Package> {
    if bytes.starts_with(hwp5::OLE_SIG) {
        return hwp5::open(bytes);
    }
    if !bytes.starts_with(b"PK") {
        let kind = if bytes.is_empty() { "empty file" } else { "neither an HWP (compound file) nor an HWPX (ZIP) document" };
        return Err(Error::format(kind));
    }
    Package::open(bytes)
}

/// Convert an HWPX or HWP document to DOCX bytes, with a report of what was and was not
/// converted.
pub fn to_docx(bytes: &[u8]) -> Result<(Vec<u8>, Report)> {
    let pkg = open(bytes)?;
    convert::Converter::new(&pkg)?.convert()
}

#[cfg(test)]
mod tests;

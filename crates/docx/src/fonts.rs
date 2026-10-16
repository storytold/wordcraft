//! Fonts a file embeds through its font table (#1171).
//!
//! The reader keeps the font table and its font parts as opaque passthrough parts, for writing
//! back. [`embedded_fonts`] hands their font files to whoever lays the document out, undoing the
//! obfuscation Word applies to them (ECMA-376 Part 1, 17.8.1).

use wordcraft_doc::Document;

use crate::package::{EMBEDDED_PARTS, EmbeddedManifest, FONT_TABLE_PART};

/// Most font files taken from one document.
const MAX_FONTS: usize = 64;

/// The font table's references to embedded font files, one per style.
const EMBEDS: [&str; 4] = ["w:embedRegular", "w:embedBold", "w:embedItalic", "w:embedBoldItalic"];

/// A font file a document embeds, and the name its font table gives the font: the name the text
/// asks for, which a subset Word embeds doesn't carry inside (#2240).
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedFont {
    /// `w:font/@w:name`.
    pub name: String,
    /// The font file, de-obfuscated.
    pub data: Vec<u8>,
}

/// The font files `doc`'s font table embeds, de-obfuscated, in table order. Not checked to be
/// fonts: that is up to the font database. Empty for a document read from anything but a package
/// with a font table.
pub fn embedded_fonts(doc: &Document) -> Vec<EmbeddedFont> {
    let Some(table) = doc.passthrough.get(FONT_TABLE_PART).and_then(|b| crate::xml::parse(b).ok()) else {
        return Vec::new();
    };
    let manifest = doc.passthrough.get(EMBEDDED_PARTS).map(|m| EmbeddedManifest::parse(m)).unwrap_or_default();
    let Some(part) = manifest.parts.get(FONT_TABLE_PART) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let embeds = table
        .children("w:font")
        .flat_map(|f| f.els().filter(|e| EMBEDS.contains(&e.name.as_str())).map(move |e| (f.attr("w:name").unwrap_or(""), e)));
    for (name, embed) in embeds {
        if out.len() >= MAX_FONTS {
            log::warn!("docx: more than {MAX_FONTS} embedded fonts; the rest are not loaded");
            break;
        }
        let Some(rel) = embed.attr("r:id").and_then(|id| part.rels.iter().find(|r| r.id == id && !r.external)) else {
            continue;
        };
        let Some(bytes) = doc.passthrough.get(&rel.target) else {
            continue;
        };
        // No key: the file is stored as is, as with the all-zero key.
        let key = match embed.attr("w:fontKey") {
            Some(k) => match font_key(k) {
                Some(key) => key,
                None => {
                    log::warn!("docx: embedded font {} has an unreadable key {k:?}", rel.target);
                    continue;
                }
            },
            None => [0; 16],
        };
        let mut data = bytes.to_vec();
        for (i, b) in data.iter_mut().take(32).enumerate() {
            *b ^= key.get(i % 16).copied().unwrap_or(0);
        }
        out.push(EmbeddedFont { name: name.to_string(), data });
    }
    out
}

/// The obfuscation key of a `w:fontKey` GUID (`{XXXXXXXX-XXXX-…}`): its 16 bytes, reversed.
fn font_key(guid: &str) -> Option<[u8; 16]> {
    let digits: Vec<u8> = guid.bytes().filter(|c| !matches!(c, b'{' | b'}' | b'-')).collect();
    if digits.len() != 32 {
        return None;
    }
    let mut key = [0u8; 16];
    for (i, pair) in digits.chunks(2).enumerate() {
        let byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        *key.get_mut(15 - i)? = byte;
    }
    Some(key)
}

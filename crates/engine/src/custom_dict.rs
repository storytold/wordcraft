//! Custom dictionary file reading and saving (#5840).
//!
//! Microsoft Word stores custom user words in freestanding dictionary files (commonly `CUSTOM.DIC`).
//! On Windows, Word traditionally encodes `CUSTOM.DIC` as UTF-16LE with a byte order mark (BOM)
//! and CRLF line endings, but also accepts UTF-8 (with or without BOM).
//!
//! This module loads and persists custom dictionary files, preserving their detected encoding
//! across edits and merging external additions safely.

use std::collections::HashSet;
use std::path::Path;

/// Maximum size of a custom dictionary file read into memory, in bytes (4 MB).
pub const MAX_CUSTOM_FILE_BYTES: u64 = 4_000_000;

/// Maximum number of words loaded from a custom dictionary file.
pub const MAX_CUSTOM_WORDS: usize = 100_000;

/// The text encoding detected for a custom dictionary file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictEncoding {
    /// UTF-16 Little Endian with BOM (`[0xFF, 0xFE]`), standard for Word's `CUSTOM.DIC` on Windows.
    Utf16LeBom,
    /// UTF-16 Big Endian with BOM (`[0xFE, 0xFF]`).
    Utf16BeBom,
    /// UTF-8 with BOM (`[0xEF, 0xBB, 0xBF]`).
    Utf8Bom,
    /// Standard UTF-8 without BOM.
    Utf8,
}

/// Detect the text encoding of raw dictionary file bytes.
pub fn detect_file_encoding(bytes: &[u8]) -> DictEncoding {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        DictEncoding::Utf16LeBom
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        DictEncoding::Utf16BeBom
    } else if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        DictEncoding::Utf8Bom
    } else {
        DictEncoding::Utf8
    }
}

/// Decode raw dictionary bytes into a UTF-8 string, handling UTF-16LE, UTF-16BE, UTF-8 with BOM,
/// and falling back to UTF-8 lossy conversion.
pub fn decode_dictionary_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|&[b0, b1]| u16::from_le_bytes([b0, b1])).collect();
        return String::from_utf16_lossy(&u16s);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let u16s: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|&[b0, b1]| u16::from_be_bytes([b0, b1])).collect();
        return String::from_utf16_lossy(&u16s);
    }
    let slice = if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF { &bytes[3..] } else { bytes };
    match std::str::from_utf8(slice) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // Check if it is UTF-16LE without a BOM (common in headless Windows exports).
            if slice.len() >= 4 && slice.iter().skip(1).step_by(2).filter(|&&b| b == 0).count() > slice.len() / 4 {
                let u16s: Vec<u16> = slice.as_chunks::<2>().0.iter().map(|&[b0, b1]| u16::from_le_bytes([b0, b1])).collect();
                String::from_utf16_lossy(&u16s)
            } else {
                String::from_utf8_lossy(slice).into_owned()
            }
        }
    }
}

/// Read the raw words from a custom dictionary file, capped at [`MAX_CUSTOM_FILE_BYTES`] and
/// [`MAX_CUSTOM_WORDS`].
pub fn read_custom_dictionary_words(path: &Path) -> Vec<String> {
    let Ok(bytes) = crate::io::read_file_capped(&path.to_string_lossy(), MAX_CUSTOM_FILE_BYTES, "dictionary") else {
        return Vec::new();
    };
    let text = decode_dictionary_bytes(&bytes);
    text.lines()
        .map(|line| line.trim_matches(|c: char| c == '\r' || c == '\n'))
        .filter_map(wordcraft_proof::custom_word)
        .take(MAX_CUSTOM_WORDS)
        .collect()
}

/// Load custom dictionary words from `path` into proofing's user dictionary.
///
/// Returns the list of words added to the proofing dictionary.
pub fn load_custom_dictionary(path: &Path) -> Vec<String> {
    let words = read_custom_dictionary_words(path);
    let mut added = Vec::new();
    for w in words {
        wordcraft_proof::add_word(&w);
        added.push(w);
    }
    added
}

/// Encode a list of words into bytes matching the requested [`DictEncoding`].
pub fn encode_dictionary_words(words: &[String], encoding: DictEncoding) -> Vec<u8> {
    match encoding {
        DictEncoding::Utf16LeBom => {
            let mut out = Vec::with_capacity(words.len() * 20 + 2);
            out.extend_from_slice(&[0xFF, 0xFE]);
            for w in words {
                for u in w.encode_utf16() {
                    out.extend_from_slice(&u.to_le_bytes());
                }
                // CRLF line endings for Word UTF-16LE custom dictionaries.
                out.extend_from_slice(&[0x0D, 0x00, 0x0A, 0x00]);
            }
            out
        }
        DictEncoding::Utf16BeBom => {
            let mut out = Vec::with_capacity(words.len() * 20 + 2);
            out.extend_from_slice(&[0xFE, 0xFF]);
            for w in words {
                for u in w.encode_utf16() {
                    out.extend_from_slice(&u.to_be_bytes());
                }
                out.extend_from_slice(&[0x00, 0x0D, 0x00, 0x0A]);
            }
            out
        }
        DictEncoding::Utf8Bom => {
            let mut out = Vec::with_capacity(words.len() * 10 + 3);
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            for w in words {
                out.extend_from_slice(w.as_bytes());
                out.push(b'\n');
            }
            out
        }
        DictEncoding::Utf8 => {
            let mut out = Vec::with_capacity(words.len() * 10);
            for w in words {
                out.extend_from_slice(w.as_bytes());
                out.push(b'\n');
            }
            out
        }
    }
}

/// Save the current user dictionary to `path`, merging any external additions made on disk,
/// omitting `removed`, and preserving the file's encoding (UTF-16LE, UTF-8 BOM, or UTF-8).
pub fn save_custom_dictionary(path: &Path, removed: &[String]) -> Result<(), String> {
    let path_str = path.to_string_lossy();
    let encoding = crate::io::read_file_head(&path_str, 4).map(|head| detect_file_encoding(&head)).unwrap_or(DictEncoding::Utf8);

    let removed_keys: HashSet<String> = removed.iter().filter_map(|w| wordcraft_proof::custom_word(w)).collect();
    let on_disk = read_custom_dictionary_words(path);
    for w in on_disk {
        if !removed_keys.contains(&w) {
            wordcraft_proof::add_word(&w);
        }
    }
    for w in removed {
        wordcraft_proof::remove_word(w);
    }

    let all_words = wordcraft_proof::user_dictionary();
    let bytes = encode_dictionary_words(&all_words, encoding);
    crate::io::write_file(&path_str, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test decoding UTF-16LE encoded bytes with a BOM.
    #[test]
    fn decodes_utf16le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for c in "mameui\r\nfreestanding\r\n".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        let text = decode_dictionary_bytes(&bytes);
        assert_eq!(text, "mameui\r\nfreestanding\r\n");
    }

    /// Test decoding UTF-8 bytes with a BOM.
    #[test]
    fn decodes_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"retro\ncomputing\n");
        let text = decode_dictionary_bytes(&bytes);
        assert_eq!(text, "retro\ncomputing\n");
    }

    /// Test saving and loading a UTF-16LE dictionary file preserves encoding and content.
    #[test]
    fn save_and_load_utf16le_custom_dictionary() {
        let dir = std::env::temp_dir().join(format!("wc-test-utf16-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom.dic");

        // Seed file with UTF-16LE data
        let mut initial = vec![0xFF, 0xFE];
        for c in "alpha\r\nbeta\r\n".encode_utf16() {
            initial.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(&path, initial).unwrap();

        let added = load_custom_dictionary(&path);
        assert!(added.contains(&"alpha".to_string()));
        assert!(added.contains(&"beta".to_string()));

        // Add a new word and save
        wordcraft_proof::add_word("gamma");
        save_custom_dictionary(&path, &[]).unwrap();

        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw[0..2], [0xFF, 0xFE], "remains UTF-16LE with BOM");
        let decoded = decode_dictionary_bytes(&raw);
        assert!(decoded.contains("alpha"));
        assert!(decoded.contains("beta"));
        assert!(decoded.contains("gamma"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

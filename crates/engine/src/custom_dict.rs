//! Custom dictionary file reading and saving (#5840).
//!
//! Microsoft Word stores custom user words in freestanding dictionary files (commonly `CUSTOM.DIC`).
//! On Windows, Word traditionally encodes `CUSTOM.DIC` as UTF-16LE with a byte order mark (BOM)
//! and CRLF line endings, but also accepts UTF-8 (with or without BOM).
//!
//! This module loads and persists custom dictionary files, preserving their detected encoding
//! across edits and merging external additions safely. Format and encoding characteristics were
//! observed black-box from synthetic user dictionary files created and saved by Word on Windows.

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

/// Check if raw bytes appear to be BOM-less UTF-16LE using a NUL-byte heuristic on even/odd positions.
pub fn is_bomless_utf16le(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count() > bytes.len() / 4
}

/// Detect the text encoding of raw dictionary file bytes.
pub fn detect_file_encoding(bytes: &[u8]) -> DictEncoding {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        DictEncoding::Utf16LeBom
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        DictEncoding::Utf16BeBom
    } else if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        DictEncoding::Utf8Bom
    } else if is_bomless_utf16le(bytes) {
        DictEncoding::Utf16LeBom
    } else {
        DictEncoding::Utf8
    }
}

/// Decode UTF-16 `payload` bytes (no BOM) in the given byte order.
fn decode_utf16(payload: &[u8], big_endian: bool, label: &str) -> Result<String, String> {
    if !payload.len().is_multiple_of(2) {
        return Err(format!("malformed {label}: odd byte length"));
    }
    let units = payload.as_chunks::<2>().0.iter().map(|&b| if big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) });
    char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| format!("malformed {label}: unpaired surrogate"))
}

/// Decode raw dictionary bytes into a UTF-8 string, handling UTF-16LE, UTF-16BE, UTF-8 with BOM,
/// and UTF-8, returning an error on malformed encodings, odd byte lengths, or unpaired surrogates.
pub fn decode_dictionary_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return decode_utf16(&bytes[2..], false, "UTF-16LE");
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return decode_utf16(&bytes[2..], true, "UTF-16BE");
    }
    let slice = if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF { &bytes[3..] } else { bytes };
    // Check for UTF-16LE without a BOM (common in headless Windows exports) before the UTF-8
    // attempt. ASCII-only UTF-16LE is also valid UTF-8, so the UTF-8 check cannot detect it.
    if is_bomless_utf16le(slice) {
        decode_utf16(slice, false, "UTF-16LE")
    } else {
        std::str::from_utf8(slice).map(str::to_string).map_err(|e| format!("malformed UTF-8: {e}"))
    }
}

/// Parse dictionary words from decoded text lines, validating each word and capping at [`MAX_CUSTOM_WORDS`].
///
/// If `path_for_err` is `Some`, returns an error when the dictionary exceeds [`MAX_CUSTOM_WORDS`] words.
pub fn parse_dictionary_words(text: &str, path_for_err: Option<&str>) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    for line in text.lines() {
        let cleaned = line.trim_matches(|c: char| c == '\r' || c == '\n');
        if let Some(w) = wordcraft_proof::custom_word(cleaned) {
            if words.len() >= MAX_CUSTOM_WORDS {
                if let Some(p) = path_for_err {
                    return Err(format!("{p}: dictionary exceeds limit of {MAX_CUSTOM_WORDS} words"));
                }
                break;
            }
            words.push(w);
        }
    }
    Ok(words)
}

/// Read the raw words from a custom dictionary file, capped at [`MAX_CUSTOM_FILE_BYTES`] and
/// [`MAX_CUSTOM_WORDS`].
///
/// Returns `Ok(Vec::new())` if the file does not exist (treating a missing dictionary as empty).
/// Returns an error if an existing file cannot be read within limits.
pub fn read_custom_dictionary_words(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let path_str = path.to_string_lossy();
    let bytes = crate::io::read_file_capped(&path_str, MAX_CUSTOM_FILE_BYTES, "dictionary")?;
    let text = decode_dictionary_bytes(&bytes).map_err(|e| format!("{path_str}: {e}"))?;
    parse_dictionary_words(&text, Some(&path_str))
}

/// Read words from a custom dictionary file, capped at [`MAX_CUSTOM_FILE_BYTES`] and
/// [`MAX_CUSTOM_WORDS`].
///
/// Returns an error if the file cannot be read within limits or does not exist.
pub fn read_custom_dictionary_words_checked(path: &Path) -> Result<Vec<String>, String> {
    let path_str = path.to_string_lossy();
    let bytes = crate::io::read_file_capped(&path_str, MAX_CUSTOM_FILE_BYTES, "dictionary")?;
    let text = decode_dictionary_bytes(&bytes).map_err(|e| format!("{path_str}: {e}"))?;
    parse_dictionary_words(&text, Some(&path_str))
}

/// Load custom dictionary words from `path` into proofing's user dictionary.
///
/// Returns the list of words added to the proofing dictionary, or an error if an existing file
/// could not be read within limits.
pub fn load_custom_dictionary(path: &Path) -> Result<Vec<String>, String> {
    let words = read_custom_dictionary_words(path)?;
    let mut added = Vec::new();
    for w in words {
        wordcraft_proof::add_word(&w);
        added.push(w);
    }
    Ok(added)
}

/// Load custom dictionary words from `path`, replacing all existing in-memory custom words
/// only if the file is successfully read.
///
/// If `path` cannot be read within limits or does not exist, returns an error without
/// modifying the existing in-memory proofing dictionary.
///
/// # Errors
/// Returns an error string if the file does not exist, exceeds size limits, or has invalid encoding.
pub fn load_custom_dictionary_replacing(path: &Path) -> Result<Vec<String>, String> {
    let words = read_custom_dictionary_words_checked(path)?;
    clear_user_dictionary();
    for w in &words {
        wordcraft_proof::add_word(w);
    }
    Ok(words)
}

/// Clear all custom words currently held in the in-memory proofing dictionary.
pub fn clear_user_dictionary() {
    for w in wordcraft_proof::user_dictionary() {
        wordcraft_proof::remove_word(&w);
    }
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
/// omitting `removed`, and preserving the file's encoding (UTF-16LE, UTF-8 BOM, or UTF-8)
/// and existing lines (preserving exact original casing, comments, spaces, and formatting).
///
/// If an existing file cannot be read within [`MAX_CUSTOM_FILE_BYTES`] or exceeds [`MAX_CUSTOM_WORDS`],
/// an error is returned before modifying the in-memory dictionary or writing to disk, keeping
/// existing file contents intact.
pub fn save_custom_dictionary(path: &Path, removed: &[String]) -> Result<(), String> {
    let path_str = path.to_string_lossy();
    let mut encoding = DictEncoding::Utf8;

    let removed_keys: HashSet<String> = removed.iter().filter_map(|w| wordcraft_proof::custom_word(w)).collect();

    let mut out_lines: Vec<String> = Vec::new();
    let mut existing_keys: HashSet<String> = HashSet::new();

    if path.exists() {
        let bytes = crate::io::read_file_capped(&path_str, MAX_CUSTOM_FILE_BYTES, "dictionary")?;
        encoding = detect_file_encoding(&bytes);
        let text = decode_dictionary_bytes(&bytes).map_err(|e| format!("{path_str}: {e}"))?;

        let line_count = text.lines().count();
        if line_count > MAX_CUSTOM_WORDS {
            return Err(format!("{path_str}: dictionary exceeds limit of {MAX_CUSTOM_WORDS} words"));
        }

        for raw_line in text.lines() {
            let line = raw_line.trim_matches(|c: char| c == '\r' || c == '\n');
            let is_removed = removed.iter().any(|r| r == line || r.trim() == line)
                || wordcraft_proof::custom_word(line).is_some_and(|k| removed_keys.contains(&k));
            if is_removed {
                continue;
            }
            out_lines.push(line.to_string());
            if let Some(k) = wordcraft_proof::custom_word(line) {
                wordcraft_proof::add_word(&k);
                existing_keys.insert(k);
            } else {
                existing_keys.insert(line.to_lowercase());
            }
        }
    }

    for w in removed {
        wordcraft_proof::remove_word(w);
    }

    for w in wordcraft_proof::user_dictionary() {
        if !existing_keys.contains(&w) && !removed_keys.contains(&w) {
            out_lines.push(w.clone());
            existing_keys.insert(w);
        }
    }

    if out_lines.len() > MAX_CUSTOM_WORDS {
        return Err(format!("Dictionary at '{}' exceeds maximum word limit ({})", path_str, MAX_CUSTOM_WORDS));
    }
    let bytes = encode_dictionary_words(&out_lines, encoding);
    if bytes.len() as u64 > MAX_CUSTOM_FILE_BYTES {
        return Err(format!("Dictionary at '{}' exceeds maximum file size ({} bytes)", path_str, MAX_CUSTOM_FILE_BYTES));
    }
    crate::io::write_file(&path_str, &bytes)
}

#[cfg(test)]
pub(crate) use tests::{DICT_TEST_LOCK, TestDir};

#[cfg(test)]
mod tests {
    use super::*;

    /// Test lock serializing tests that access the global proofing dictionary.
    pub(crate) static DICT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard for temporary directories in dictionary tests.
    pub(crate) struct TestDir(pub(crate) std::path::PathBuf);

    impl TestDir {
        /// Create a new unique temporary directory with the given prefix.
        pub(crate) fn new(prefix: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let count = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("wc-test-{prefix}-{}-{count}", std::process::id()));
            let _ = std::fs::create_dir_all(&path);
            TestDir(path)
        }

        /// Return a reference to the temporary directory path.
        pub(crate) fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Test decoding UTF-16LE encoded bytes with a BOM.
    #[test]
    fn decodes_utf16le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for c in "mameui\r\nfreestanding\r\n".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        let text = decode_dictionary_bytes(&bytes).unwrap();
        assert_eq!(text, "mameui\r\nfreestanding\r\n");
    }

    /// Test decoding UTF-8 bytes with a BOM.
    #[test]
    fn decodes_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"retro\ncomputing\n");
        let text = decode_dictionary_bytes(&bytes).unwrap();
        assert_eq!(text, "retro\ncomputing\n");
    }

    /// Test decoding ASCII-only UTF-16LE bytes without a BOM.
    #[test]
    fn decodes_ascii_utf16le_without_bom() {
        let mut bytes = Vec::new();
        for c in "freestanding\r\nmameui\r\n".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        let text = decode_dictionary_bytes(&bytes).unwrap();
        assert_eq!(text, "freestanding\r\nmameui\r\n");
    }

    /// Test saving and loading a UTF-16LE dictionary file preserves encoding and content.
    #[test]
    fn save_and_load_utf16le_custom_dictionary() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("utf16");
        let path = dir.path().join("custom.dic");

        // Seed file with UTF-16LE data
        let mut initial = vec![0xFF, 0xFE];
        for c in "alpha\r\nbeta\r\n".encode_utf16() {
            initial.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(&path, initial).unwrap();

        let added = load_custom_dictionary(&path).unwrap();
        assert!(added.contains(&"alpha".to_string()));
        assert!(added.contains(&"beta".to_string()));

        // Add a new word and save
        wordcraft_proof::add_word("gamma");
        save_custom_dictionary(&path, &[]).unwrap();

        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw[0..2], [0xFF, 0xFE], "remains UTF-16LE with BOM");
        let decoded = decode_dictionary_bytes(&raw).unwrap();
        assert!(decoded.contains("alpha"));
        assert!(decoded.contains("beta"));
        assert!(decoded.contains("gamma"));
        wordcraft_proof::remove_word("gamma");
    }

    /// Test saving and loading a UTF-16BE dictionary file preserves big-endian BOM and content.
    #[test]
    fn save_and_load_utf16be_custom_dictionary() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("utf16be");
        let path = dir.path().join("custom_be.dic");

        let mut initial = vec![0xFE, 0xFF];
        for c in "alpha\r\nbeta\r\n".encode_utf16() {
            initial.extend_from_slice(&c.to_be_bytes());
        }
        std::fs::write(&path, initial).unwrap();

        let added = load_custom_dictionary(&path).unwrap();
        assert!(added.contains(&"alpha".to_string()));
        assert!(added.contains(&"beta".to_string()));

        wordcraft_proof::add_word("gamma");
        save_custom_dictionary(&path, &[]).unwrap();

        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw[0..2], [0xFE, 0xFF], "remains UTF-16BE with BOM");
        let decoded = decode_dictionary_bytes(&raw).unwrap();
        assert!(decoded.contains("alpha"));
        assert!(decoded.contains("beta"));
        assert!(decoded.contains("gamma"));

        wordcraft_proof::remove_word("alpha");
        wordcraft_proof::remove_word("beta");
        wordcraft_proof::remove_word("gamma");
    }

    /// Test saving and loading an ASCII-only BOM-less UTF-16LE dictionary writes Word-compatible UTF-16LE with BOM.
    #[test]
    fn save_and_load_bomless_utf16le_custom_dictionary() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("bomless-utf16");
        let path = dir.path().join("bomless.dic");

        // Seed file with BOM-less UTF-16LE data
        let mut initial = Vec::new();
        for c in "alpha\r\nbeta\r\n".encode_utf16() {
            initial.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(&path, initial).unwrap();

        let added = load_custom_dictionary(&path).unwrap();
        assert!(added.contains(&"alpha".to_string()));
        assert!(added.contains(&"beta".to_string()));

        // Add a new word and save
        wordcraft_proof::add_word("gamma");
        save_custom_dictionary(&path, &[]).unwrap();

        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw[0..2], [0xFF, 0xFE], "saved with Word-compatible UTF-16LE BOM");
        let decoded = decode_dictionary_bytes(&raw).unwrap();
        assert!(decoded.contains("alpha"));
        assert!(decoded.contains("beta"));
        assert!(decoded.contains("gamma"));
        wordcraft_proof::remove_word("gamma");
        wordcraft_proof::remove_word("alpha");
        wordcraft_proof::remove_word("beta");
    }

    /// Test that saving a custom dictionary merges external additions made on disk after loading.
    #[test]
    fn save_custom_dictionary_merges_external_additions() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("external-merge");
        let path = dir.path().join("custom.dic");

        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let loaded = load_custom_dictionary(&path).unwrap();
        assert_eq!(loaded, vec!["alpha".to_string(), "beta".to_string()]);

        // External editor appends "externalword"
        std::fs::write(&path, "alpha\nbeta\nexternalword\n").unwrap();

        // In-session addition "internalword"
        wordcraft_proof::add_word("internalword");
        save_custom_dictionary(&path, &[]).unwrap();

        let words = read_custom_dictionary_words(&path).unwrap();
        assert!(words.contains(&"alpha".to_string()));
        assert!(words.contains(&"beta".to_string()));
        assert!(words.contains(&"externalword".to_string()), "external additions merged on save");
        assert!(words.contains(&"internalword".to_string()), "internal additions preserved on save");

        wordcraft_proof::remove_word("alpha");
        wordcraft_proof::remove_word("beta");
        wordcraft_proof::remove_word("externalword");
        wordcraft_proof::remove_word("internalword");
    }

    /// Test that reading a custom dictionary rejects files exceeding the byte limit.
    #[test]
    fn read_custom_dictionary_words_rejects_file_over_byte_limit() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("read-overlimit");
        let path = dir.path().join("oversized.dic");

        let large_content = vec![b'a'; (MAX_CUSTOM_FILE_BYTES + 1024) as usize];
        std::fs::write(&path, &large_content).unwrap();

        let err1 = read_custom_dictionary_words(&path).unwrap_err();
        assert!(err1.contains("larger than"));

        let err2 = read_custom_dictionary_words_checked(&path).unwrap_err();
        assert!(err2.contains("larger than"));
    }

    /// Test that parsing custom dictionary text enforces the maximum word limit.
    #[test]
    fn parse_dictionary_words_rejects_or_truncates_over_word_limit() {
        let mut text = String::new();
        for i in 0..MAX_CUSTOM_WORDS + 10 {
            text.push_str(&format!("word{i}\n"));
        }

        // When path is provided for error reporting, returns an error.
        let err = parse_dictionary_words(&text, Some("custom.dic")).unwrap_err();
        assert!(err.contains(&format!("dictionary exceeds limit of {MAX_CUSTOM_WORDS} words")));

        // When path is None, safely truncates at MAX_CUSTOM_WORDS.
        let capped = parse_dictionary_words(&text, None).unwrap();
        assert_eq!(capped.len(), MAX_CUSTOM_WORDS);
    }

    /// Test that decoding UTF-16LE bytes with an odd byte count returns an error.
    #[test]
    fn decode_dictionary_bytes_rejects_odd_byte_length_utf16le() {
        let bytes = vec![0xFF, 0xFE, 0x61];
        let err = decode_dictionary_bytes(&bytes).unwrap_err();
        assert!(err.contains("odd byte length"));
    }

    /// Test that decoding UTF-16LE bytes with an unpaired surrogate returns an error.
    #[test]
    fn decode_dictionary_bytes_rejects_unpaired_surrogate_utf16le() {
        let bytes = vec![0xFF, 0xFE, 0x00, 0xD8];
        let err = decode_dictionary_bytes(&bytes).unwrap_err();
        assert!(err.contains("unpaired surrogate"));
    }

    /// Test that decoding UTF-16BE bytes with an odd byte count returns an error.
    #[test]
    fn decode_dictionary_bytes_rejects_odd_byte_length_utf16be() {
        let bytes = vec![0xFE, 0xFF, 0x61];
        let err = decode_dictionary_bytes(&bytes).unwrap_err();
        assert!(err.contains("odd byte length"));
    }

    /// Test that decoding UTF-16BE bytes with an unpaired surrogate returns an error.
    #[test]
    fn decode_dictionary_bytes_rejects_unpaired_surrogate_utf16be() {
        let bytes = vec![0xFE, 0xFF, 0xD8, 0x00];
        let err = decode_dictionary_bytes(&bytes).unwrap_err();
        assert!(err.contains("unpaired surrogate"));
    }

    /// Test that clearing the user dictionary removes all words from memory.
    #[test]
    fn clear_user_dictionary_removes_all_words() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_user_dictionary();
        wordcraft_proof::add_word("tempclearwordone");
        wordcraft_proof::add_word("tempclearwordtwo");
        assert!(wordcraft_proof::user_dictionary().contains(&"tempclearwordone".to_string()));
        clear_user_dictionary();
        assert!(wordcraft_proof::user_dictionary().is_empty());
    }

    /// Test that save_custom_dictionary returns an error and preserves an existing file that
    /// exceeds the maximum size limit.
    #[test]
    fn save_custom_dictionary_refuses_over_limit_file() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("overlimit");
        let path = dir.path().join("oversized.dic");

        let large_content = vec![b'a'; (MAX_CUSTOM_FILE_BYTES + 1024) as usize];
        std::fs::write(&path, &large_content).unwrap();

        wordcraft_proof::add_word("newword");
        let result = save_custom_dictionary(&path, &[]);
        assert!(result.is_err(), "should reject oversized dictionary file");

        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back.len(), large_content.len());
        wordcraft_proof::remove_word("newword");
    }

    /// Test that read_custom_dictionary_words returns Ok(empty) on a missing file.
    #[test]
    fn read_custom_dictionary_words_returns_empty_on_missing_file() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("missing");
        let path = dir.path().join("nonexistent.dic");
        let words = read_custom_dictionary_words(&path).unwrap();
        assert!(words.is_empty());
    }

    /// Test that save_custom_dictionary rejects writing when encoded output exceeds byte limit.
    #[test]
    fn save_custom_dictionary_refuses_when_output_exceeds_byte_limit() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_user_dictionary();

        struct ResetDict;
        impl Drop for ResetDict {
            fn drop(&mut self) {
                clear_user_dictionary();
            }
        }
        let _reset = ResetDict;

        let dir = TestDir::new("oversized-output");
        let path = dir.path().join("oversized.dic");

        // Populate the in-memory custom dictionary with words that exceed MAX_CUSTOM_FILE_BYTES (4MB)
        // while staying below MAX_CUSTOM_WORDS (100k) and keeping each word <= 64 chars.
        // 70,000 words * 64 bytes (63 chars + '\n') = ~4.48 MB > 4 MB limit.
        let word_count = 70_000;
        let prefix = "a".repeat(58);
        for i in 0..word_count {
            wordcraft_proof::add_word(&format!("{prefix}{:05}", i));
        }

        let res = save_custom_dictionary(&path, &[]);
        assert!(res.is_err(), "Expected save_custom_dictionary to reject oversized output");
        let err = res.unwrap_err();
        assert!(err.contains("exceeds maximum file size"), "Error message was: {err}");
        assert!(!path.exists(), "File must not be written when output exceeds byte limit");
    }

    /// Test that save_custom_dictionary preserves original lines and casing (e.g. capitalized
    /// entries like NASA, McDonald, multi-word lines, and lines exceeding 64 chars).
    #[test]
    fn save_custom_dictionary_preserves_original_lines_and_casing() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_user_dictionary();

        struct ResetDict;
        impl Drop for ResetDict {
            fn drop(&mut self) {
                clear_user_dictionary();
            }
        }
        let _reset = ResetDict;

        let dir = TestDir::new("preserve-lines");
        let path = dir.path().join("CUSTOM.DIC");

        // Create an initial UTF-16LE dictionary matching Word's format with mixed casing,
        // a multi-word line, and a line longer than 64 characters.
        let over_64_chars = "a".repeat(70);
        let content = format!("NASA\r\nMcDonald\r\ntwo words\r\n{over_64_chars}\r\n");

        let mut initial_bytes = vec![0xFF, 0xFE];
        for c in content.encode_utf16() {
            initial_bytes.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(&path, &initial_bytes).unwrap();

        // Load the dictionary into proofing. Proofing parses valid words ("nasa", "mcdonald")
        // and skips unparseable lines ("two words", 70-char line).
        let loaded = load_custom_dictionary(&path).unwrap();
        assert!(loaded.contains(&"nasa".to_string()));
        assert!(loaded.contains(&"mcdonald".to_string()));
        assert_eq!(loaded.len(), 2);

        // Add a new word in session
        wordcraft_proof::add_word("newword");

        // Save back to the file
        save_custom_dictionary(&path, &[]).unwrap();

        // Verify file remains UTF-16LE with BOM
        let saved_bytes = std::fs::read(&path).unwrap();
        assert_eq!(saved_bytes[0..2], [0xFF, 0xFE]);

        let decoded = decode_dictionary_bytes(&saved_bytes).unwrap();

        // Original lines must preserve exact casing and unparsed content
        assert!(decoded.contains("NASA\r\n"), "must preserve capitalized NASA");
        assert!(decoded.contains("McDonald\r\n"), "must preserve mixed-case McDonald");
        assert!(decoded.contains("two words\r\n"), "must preserve multi-word line");
        assert!(decoded.contains(&format!("{over_64_chars}\r\n")), "must preserve line > 64 chars");

        // Newly added word must be appended
        assert!(decoded.contains("newword\r\n"), "must append new word");

        // Now remove McDonald and save again
        save_custom_dictionary(&path, &["McDonald".to_string()]).unwrap();

        let after_remove = decode_dictionary_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert!(after_remove.contains("NASA\r\n"), "NASA still retained");
        assert!(!after_remove.contains("McDonald"), "McDonald removed");
        assert!(after_remove.contains("two words\r\n"), "multi-word line still retained");
        assert!(after_remove.contains(&format!("{over_64_chars}\r\n")), "line > 64 chars still retained");
        assert!(after_remove.contains("newword\r\n"), "newword still retained");
    }

    /// Test that load_custom_dictionary_replacing replaces stale restored words on successful load,
    /// but preserves in-memory words if the target file does not exist or fails to load.
    #[test]
    fn load_custom_dictionary_replacing_reconciles_stale_words() {
        let _guard = DICT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TestDir::new("reconcile");
        let path = dir.path().join("custom.dic");

        clear_user_dictionary();
        wordcraft_proof::add_word("stale_word");
        assert!(wordcraft_proof::user_dictionary().contains(&"stale_word".to_string()));

        // Non-existent file must return Err and not clear existing words
        let nonexistent = dir.path().join("missing.dic");
        let res = load_custom_dictionary_replacing(&nonexistent);
        assert!(res.is_err(), "missing file must error");
        assert!(wordcraft_proof::user_dictionary().contains(&"stale_word".to_string()), "must preserve words on missing file");

        // Existing file must replace stale words with its own words
        std::fs::write(&path, "fresh_word\r\n").unwrap();
        let loaded = load_custom_dictionary_replacing(&path).unwrap();
        assert_eq!(loaded, vec!["fresh_word".to_string()]);

        let in_memory = wordcraft_proof::user_dictionary();
        assert!(in_memory.contains(&"fresh_word".to_string()), "must contain fresh word");
        assert!(!in_memory.contains(&"stale_word".to_string()), "must have cleared stale word");

        clear_user_dictionary();
    }
}

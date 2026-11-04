//! Password-protected ODT (ODF 1.3 Part 2 §3.4): saved encrypted as LibreOffice encrypts
//! (AES-256-CBC, SHA-256 start key, PBKDF2), opened with the right password, refused with a
//! wrong one, and hostile encryption data never panics or hangs.

use crate::odt::tests::{NS_T, package, zip_of};
use crate::odt::{CryptError, decrypt, encrypt, export, import, is_encrypted};
use wordcraft_doc::Document;

fn secret_doc() -> Vec<u8> {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Top secret words.</text:p></office:text></office:body></office:document-content>"#
    );
    export(&import(&package(&content, None)).unwrap()).unwrap()
}

fn text(d: &Document) -> String {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn an_encrypted_package_opens_with_its_password_only() {
    let plain = secret_doc();
    assert!(!is_encrypted(&plain));
    let enc = encrypt(&plain, "pässword").unwrap();
    assert!(is_encrypted(&enc));
    // Nothing readable is left: the content is encrypted, the manifest says how.
    let z = zip::ZipArchive::new(std::io::Cursor::new(&enc)).unwrap();
    assert_eq!(z.file_names().next(), Some("mimetype"));
    assert!(!enc.windows(10).any(|w| w == b"Top secret"));
    let manifest = crate::odt::tests::entry(&enc, "META-INF/manifest.xml");
    for want in [
        r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml" manifest:size=""#,
        "http://www.w3.org/2001/04/xmlenc#aes256-cbc",
        r#"manifest:start-key-generation-name="http://www.w3.org/2000/09/xmldsig#sha256" manifest:key-size="32""#,
        r#"manifest:key-derivation-name="PBKDF2" manifest:iteration-count="100000""#,
        "#sha256-1k",
    ] {
        assert!(manifest.contains(want), "{want} in {manifest}");
    }
    assert_eq!(decrypt(&enc, "wrong"), Err(CryptError::WrongPassword));
    let opened = import(&decrypt(&enc, "pässword").unwrap()).unwrap();
    assert_eq!(text(&opened), "Top secret words.");
}

#[test]
fn unsupported_and_hostile_encryption_is_refused() {
    let manifest = |alg: &str, extra: &str| {
        format!(
            r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml" manifest:size="99999999999"><manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="AAAA"><manifest:algorithm manifest:algorithm-name="{alg}" manifest:initialisation-vector="AAAAAAAAAAA="/><manifest:key-derivation manifest:key-derivation-name="PBKDF2" {extra} manifest:salt="AAAA"/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#
        )
    };
    let pkg = |m: String, content: &[u8]| {
        zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.text"), ("META-INF/manifest.xml", m.as_bytes()), ("content.xml", content)])
    };
    // Blowfish: said, not guessed at.
    let blowfish = pkg(manifest("Blowfish CFB", r#"manifest:iteration-count="1024""#), &[0u8; 64]);
    assert!(is_encrypted(&blowfish));
    assert!(matches!(decrypt(&blowfish, "x"), Err(CryptError::Unsupported(w)) if w.contains("Blowfish")));
    // An iteration count that would hang, a vector of the wrong size, ragged data: refused.
    for (extra, data) in [
        (r#"manifest:iteration-count="4000000000""#, vec![0u8; 32]),
        (r#"manifest:iteration-count="1""#, vec![0u8; 33]),
        (r#"manifest:iteration-count="1""#, vec![]),
    ] {
        assert!(decrypt(&pkg(manifest("http://www.w3.org/2001/04/xmlenc#aes256-cbc", extra), &data), "x").is_err(), "{extra}");
    }
    // Junk that isn't a package.
    assert!(!is_encrypted(b"not a zip"));
    assert!(decrypt(b"not a zip", "x").is_err());
}

#[test]
fn a_package_declaring_too_many_entries_is_refused_before_it_is_decrypted() {
    // #3660: refused before `zip` reserves and parses them, on the password path too.
    let bomb = wordcraft_doc::zip_guard::zip64_entry_count_sample(crate::MAX_ZIP_ENTRIES + 1);
    assert!(!is_encrypted(&bomb));
    assert!(matches!(decrypt(&bomb, "x"), Err(CryptError::Unsupported(e)) if e.contains("declares")));
}

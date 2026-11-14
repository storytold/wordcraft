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

/// A package encrypted whole, the way LibreOffice 24.2+ saves with a password (ODF 1.4 Part 2):
/// `inner` deflated, AES-256-GCM (the vector, the encrypted data, the tag: XML Encryption 1.1
/// §5.2.4), the key Argon2id of the password's SHA-256. `costs` is the key derivation's
/// attributes; `data` replaces the encrypted bytes when given.
fn whole_package(inner: &[u8], password: &str, costs: &str, data: Option<Vec<u8>>) -> Vec<u8> {
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine as _;
    use sha2::Digest;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let (salt, iv) = ([7u8; 16], [9u8; 12]);
    let mut key = [0u8; 32];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, argon2::Params::new(64, 1, 4, Some(32)).unwrap())
        .hash_password_into(&sha2::Sha256::digest(password.as_bytes()), &salt, &mut key)
        .unwrap();
    let deflated = miniz_oxide::deflate::compress_to_vec(inner, 6);
    let enc = data.unwrap_or_else(|| {
        let sealed = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap().encrypt(&aes_gcm::Nonce::from(iv), deflated.as_slice()).unwrap();
        [iv.as_slice(), &sealed].concat()
    });
    let manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.4" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"><manifest:file-entry manifest:full-path="encrypted-package" manifest:media-type="application/vnd.oasis.opendocument.text" manifest:size="{}"><manifest:encryption-data><manifest:algorithm manifest:algorithm-name="http://www.w3.org/2009/xmlenc11#aes256-gcm" manifest:initialisation-vector="{}"/><manifest:start-key-generation manifest:start-key-generation-name="http://www.w3.org/2001/04/xmlenc#sha256" manifest:key-size="32"/><manifest:key-derivation manifest:key-derivation-name="urn:org:documentfoundation:names:experimental:office:manifest:argon2id" {costs} manifest:salt="{}" manifest:key-size="32"/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#,
        inner.len(),
        b64(&iv),
        b64(&salt)
    );
    zip_of(&[("mimetype", b"application/vnd.oasis.opendocument.text"), ("encrypted-package", &enc), ("META-INF/manifest.xml", manifest.as_bytes())])
}

const SMALL_COSTS: &str = r#"loext:argon2-iterations="1" loext:argon2-memory="64" loext:argon2-lanes="4""#;

#[test]
fn a_package_encrypted_whole_opens_with_its_password_only() {
    let plain = secret_doc();
    let enc = whole_package(&plain, "pässword", SMALL_COSTS, None);
    assert!(is_encrypted(&enc));
    assert!(!enc.windows(10).any(|w| w == b"Top secret"));
    assert_eq!(decrypt(&enc, "wrong"), Err(CryptError::WrongPassword));
    let opened = decrypt(&enc, "pässword").unwrap();
    assert_eq!(opened, plain, "the inner package is the plain document");
    assert_eq!(text(&import(&opened).unwrap()), "Top secret words.");
    // ODF 1.4's own attribute names work too.
    let odf14 = whole_package(&plain, "pw", &SMALL_COSTS.replace("loext:", "manifest:"), None);
    assert_eq!(decrypt(&odf14, "pw").unwrap(), plain);
}

#[test]
fn hostile_whole_package_encryption_is_refused() {
    let plain = secret_doc();
    // Costs that would take gigabytes or forever, or aren't given: refused before deriving.
    for costs in [
        r#"loext:argon2-iterations="1" loext:argon2-memory="4000000000" loext:argon2-lanes="4""#,
        r#"loext:argon2-iterations="4000000000" loext:argon2-memory="64" loext:argon2-lanes="4""#,
        r#"loext:argon2-iterations="1" loext:argon2-memory="64" loext:argon2-lanes="0""#,
        r#"loext:argon2-iterations="1" loext:argon2-memory="64""#,
    ] {
        assert!(matches!(decrypt(&whole_package(&plain, "pw", costs, None), "pw"), Err(CryptError::Unsupported(_))), "{costs}");
    }
    // Data shorter than its vector and tag, or changed: the tag doesn't match.
    assert_eq!(decrypt(&whole_package(&plain, "pw", SMALL_COSTS, Some(vec![1, 2, 3])), "pw"), Err(CryptError::WrongPassword));
    let mut sealed = Vec::new();
    let valid = whole_package(&plain, "pw", SMALL_COSTS, None);
    std::io::Read::read_to_end(&mut zip::ZipArchive::new(std::io::Cursor::new(&valid)).unwrap().by_name("encrypted-package").unwrap(), &mut sealed)
        .unwrap();
    let mid = sealed.len() / 2;
    sealed[mid] ^= 1;
    assert_eq!(decrypt(&whole_package(&plain, "pw", SMALL_COSTS, Some(sealed)), "pw"), Err(CryptError::WrongPassword));
    // Inside, something that isn't a package, or a package encrypted again.
    assert!(matches!(decrypt(&whole_package(b"just text", "pw", SMALL_COSTS, None), "pw"), Err(CryptError::Unsupported(_))));
    let nested = whole_package(&whole_package(&plain, "pw", SMALL_COSTS, None), "pw", SMALL_COSTS, None);
    assert!(matches!(decrypt(&nested, "pw"), Err(CryptError::Unsupported(_))));
}

#[test]
fn a_package_declaring_too_many_entries_is_refused_before_it_is_decrypted() {
    // #3660: refused before `zip` reserves and parses them, on the password path too.
    let bomb = wordcraft_doc::zip_guard::zip64_entry_count_sample(crate::MAX_ZIP_ENTRIES + 1);
    assert!(!is_encrypted(&bomb));
    assert!(matches!(decrypt(&bomb, "x"), Err(CryptError::Unsupported(e)) if e.contains("declares")));
}

//! Password-protected packages (ODF 1.3 Part 2 §3.4, §4.4–4.16): each encrypted file is
//! deflated, then encrypted with a key derived from the password, and described in the
//! manifest (`manifest:encryption-data`). Read: the package decrypted into a plain one, which
//! the reader opens as any other. Written as LibreOffice writes it: AES-256-CBC, a SHA-256
//! start key, PBKDF2 with HMAC-SHA-1 and a salt of its own for each file, a SHA-256 checksum of
//! the first 1024 bytes of each deflated file.
//!
//! Blowfish (the old default, §3.4.2) isn't supported: such files say so.

use std::io::{Cursor, Read, Write};

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::Mac;
use sha2::Digest;
use zeroize::Zeroizing;

use super::limits::MAX_ENTRY;
use super::xml::{Ev, events, get};

/// Why an encrypted package can't be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptError {
    /// The password doesn't decrypt it (the checksums don't match).
    WrongPassword,
    /// An encryption WordCraft doesn't support, or a broken package.
    Unsupported(String),
}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptError::WrongPassword => write!(f, "ODT: the password is incorrect"),
            CryptError::Unsupported(why) => write!(f, "ODT: {why}"),
        }
    }
}

const AES256_CBC: &str = "http://www.w3.org/2001/04/xmlenc#aes256-cbc";
const SHA256: &str = "http://www.w3.org/2000/09/xmldsig#sha256";
const SHA256_1K: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0#sha256-1k";
/// PBKDF2 iterations written (LibreOffice's), and most read (a hostile count would hang).
const ITERATIONS: u32 = 100_000;
const MAX_ITERATIONS: u32 = 10_000_000;
/// Most files in a package decrypted.
const MAX_FILES: usize = 10_000;
/// Most PBKDF2 work (iterations times the key's 20-byte blocks) for a whole package: a thousand
/// files as LibreOffice and WordCraft write them (100,000 iterations, a 32-byte key), so many
/// files at the per-file cap can't add up to a hang.
const MAX_WORK: u64 = 200_000_000;

/// `p`'s key derivation added to `spent`, an error once the package's work passes [`MAX_WORK`].
fn charge(spent: &mut u64, p: &Params) -> Result<(), CryptError> {
    let blocks = p.key_size.unwrap_or(16).clamp(16, 32).div_ceil(20) as u64;
    *spent = spent.saturating_add(u64::from(p.iterations.min(MAX_ITERATIONS)).saturating_mul(blocks));
    if *spent > MAX_WORK {
        return Err(CryptError::Unsupported("the package's key derivations would take too long".into()));
    }
    Ok(())
}

/// How one file is encrypted (a `manifest:encryption-data`).
#[derive(Default, Debug, Clone)]
struct Params {
    checksum_type: String,
    checksum: Vec<u8>,
    algorithm: String,
    iv: Vec<u8>,
    start_key: String,
    start_key_size: Option<usize>,
    derivation: String,
    iterations: u32,
    salt: Vec<u8>,
    key_size: Option<usize>,
    size: Option<usize>,
}

/// Each encrypted file's parameters, by path, from a manifest.
fn read_manifest(xml: &[u8]) -> Vec<(String, Params)> {
    let mut out: Vec<(String, Params)> = Vec::new();
    let mut path: Option<(String, Option<usize>)> = None;
    let b64 = |v: Option<&str>| v.and_then(|s| STANDARD.decode(s.trim()).ok()).unwrap_or_default();
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match q.as_str() {
            "manifest:file-entry" => {
                path = get(&a, "manifest:full-path").map(|p| (p.to_string(), get(&a, "manifest:size").and_then(|s| s.trim().parse().ok())));
            }
            "manifest:encryption-data" if out.len() < MAX_FILES => {
                if let Some((p, size)) = &path {
                    out.push((
                        p.clone(),
                        Params {
                            checksum_type: get(&a, "manifest:checksum-type").unwrap_or("SHA1/1K").to_string(),
                            checksum: b64(get(&a, "manifest:checksum")),
                            size: *size,
                            ..Default::default()
                        },
                    ));
                }
            }
            "manifest:algorithm" => {
                if let Some((_, p)) = out.last_mut() {
                    p.algorithm = get(&a, "manifest:algorithm-name").unwrap_or_default().to_string();
                    p.iv = b64(get(&a, "manifest:initialisation-vector"));
                }
            }
            "manifest:start-key-generation" => {
                if let Some((_, p)) = out.last_mut() {
                    p.start_key = get(&a, "manifest:start-key-generation-name").unwrap_or("SHA1").to_string();
                    p.start_key_size = get(&a, "manifest:key-size").and_then(|s| s.trim().parse().ok());
                }
            }
            "manifest:key-derivation" => {
                if let Some((_, p)) = out.last_mut() {
                    p.derivation = get(&a, "manifest:key-derivation-name").unwrap_or_default().to_string();
                    p.iterations = get(&a, "manifest:iteration-count").and_then(|s| s.trim().parse().ok()).unwrap_or(1024);
                    p.salt = b64(get(&a, "manifest:salt"));
                    p.key_size = get(&a, "manifest:key-size").and_then(|s| s.trim().parse().ok());
                }
            }
            _ => {}
        },
        Ev::End(q) if q == "manifest:file-entry" => path = None,
        _ => {}
    });
    out
}

/// Does this package hold encrypted files?
pub fn is_encrypted(bytes: &[u8]) -> bool {
    // A package with too many entries isn't parsed here: opening it refuses it (`zip_guard`).
    if wordcraft_doc::zip_guard::check(bytes, crate::MAX_ZIP_ENTRIES).is_err() {
        return false;
    }
    let Ok(mut z) = zip::ZipArchive::new(Cursor::new(bytes)) else { return false };
    super::import::read_entry(&mut z, "META-INF/manifest.xml").is_some_and(|m| !read_manifest(&m).is_empty())
}

/// The start key: the password's UTF-8 digest (§4.16.6), `size` bytes of it.
fn start_key(name: &str, size: Option<usize>, password: &str) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let mut k = match name {
        SHA256 | "SHA256" => sha2::Sha256::digest(password.as_bytes()).to_vec(),
        "SHA1" | "http://www.w3.org/2000/09/xmldsig#sha1" => sha1::Sha1::digest(password.as_bytes()).to_vec(),
        other => return Err(CryptError::Unsupported(format!("a start key made with {other} isn't supported"))),
    };
    if let Some(s) = size.filter(|s| *s > 0 && *s < k.len()) {
        k.truncate(s);
    }
    Ok(Zeroizing::new(k))
}

/// PBKDF2 with HMAC-SHA-1 (RFC 2898 §5.2): `len` bytes.
fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32, len: usize) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(password).map_err(|_| CryptError::Unsupported("bad key".into()))?;
    let mut out = Zeroizing::new(Vec::with_capacity(len + 20));
    let mut block = 1u32;
    while out.len() < len {
        let mut m = mac.clone();
        m.update(salt);
        m.update(&block.to_be_bytes());
        let mut u = m.finalize().into_bytes();
        let mut t = u;
        for _ in 1..iterations {
            let mut m = mac.clone();
            m.update(&u);
            u = m.finalize().into_bytes();
            t.iter_mut().zip(u.iter()).for_each(|(a, b)| *a ^= b);
        }
        out.extend_from_slice(&t);
        block = block.saturating_add(1);
    }
    out.truncate(len);
    Ok(out)
}

/// The key for a file with parameters `p`.
fn file_key(p: &Params, password: &str) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let start = start_key(&p.start_key, p.start_key_size, password)?;
    if !(p.derivation == "PBKDF2" || p.derivation.ends_with("#pbkdf2")) {
        return Err(CryptError::Unsupported(format!("key derivation {:?} isn't supported", p.derivation)));
    }
    if p.iterations == 0 || p.iterations > MAX_ITERATIONS {
        return Err(CryptError::Unsupported("the key derivation's iteration count is out of range".into()));
    }
    pbkdf2(&start, &p.salt, p.iterations, p.key_size.unwrap_or(16).clamp(16, 32))
}

/// AES-256-CBC, `data` whole blocks.
fn cbc(key: &[u8], iv: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, CryptError> {
    let aes = aes::Aes256::new_from_slice(key).map_err(|_| CryptError::Unsupported("the key isn't 256 bits".into()))?;
    let mut prev: [u8; 16] = iv.try_into().map_err(|_| CryptError::Unsupported("the initialisation vector isn't 16 bytes".into()))?;
    let (blocks, rest) = data.as_chunks::<16>();
    if !rest.is_empty() {
        return Err(CryptError::Unsupported("encrypted data isn't a whole number of blocks".into()));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        if encrypt {
            let mut x = aes::Block::from(*b);
            x.iter_mut().zip(prev.iter()).for_each(|(a, p)| *a ^= p);
            aes.encrypt_block(&mut x);
            prev = x.into();
            out.extend_from_slice(&prev);
        } else {
            let mut x = aes::Block::from(*b);
            aes.decrypt_block(&mut x);
            x.iter_mut().zip(prev.iter()).for_each(|(a, p)| *a ^= p);
            out.extend_from_slice(&x);
            prev = *b;
        }
    }
    Ok(out)
}

/// The checksum of a deflated file (§4.16.3): of its first 1024 bytes.
fn checksum(kind: &str, deflated: &[u8]) -> Result<Vec<u8>, CryptError> {
    let head = deflated.get(..deflated.len().min(1024)).unwrap_or_default();
    match kind {
        SHA256_1K => Ok(sha2::Sha256::digest(head).to_vec()),
        "SHA1/1K" | "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0#sha1-1k" => Ok(sha1::Sha1::digest(head).to_vec()),
        other => Err(CryptError::Unsupported(format!("checksum {other} isn't supported"))),
    }
}

/// One encrypted file decrypted and inflated.
fn decrypt_file(data: &[u8], p: &Params, password: &str) -> Result<Vec<u8>, CryptError> {
    if p.algorithm != AES256_CBC {
        let what = if p.algorithm.to_ascii_lowercase().contains("blowfish") { "Blowfish".to_string() } else { format!("{:?}", p.algorithm) };
        return Err(CryptError::Unsupported(format!("this file is encrypted with {what}, which WordCraft can't open")));
    }
    let key = file_key(p, password)?;
    let plain = Zeroizing::new(cbc(&key, &p.iv, data, false)?);
    // W3C XML Encryption padding: the last byte says how many bytes were added.
    let pad = usize::from(plain.last().copied().unwrap_or(0));
    let deflated = (1..=16).contains(&pad).then(|| plain.get(..plain.len().saturating_sub(pad))).flatten().ok_or(CryptError::WrongPassword)?;
    if !p.checksum.is_empty() && checksum(&p.checksum_type, deflated)? != p.checksum {
        return Err(CryptError::WrongPassword);
    }
    let limit = p.size.unwrap_or(MAX_ENTRY as usize).min(MAX_ENTRY as usize);
    miniz_oxide::inflate::decompress_to_vec_with_limit(deflated, limit).map_err(|_| CryptError::WrongPassword)
}

/// The manifest without its encryption data (and the sizes that go with it).
fn plain_manifest(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find("<manifest:encryption-data") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        let end =
            after.find("</manifest:encryption-data>").map(|e| e + "</manifest:encryption-data>".len()).or_else(|| after.find("/>").map(|e| e + 2));
        rest = end.and_then(|e| after.get(e..)).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// A password-protected package decrypted into a plain one.
pub fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, CryptError> {
    let broken = |e: zip::result::ZipError| CryptError::Unsupported(format!("not a valid package ({e})"));
    wordcraft_doc::zip_guard::check(bytes, crate::MAX_ZIP_ENTRIES).map_err(CryptError::Unsupported)?;
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(broken)?;
    let manifest = super::import::read_entry(&mut z, "META-INF/manifest.xml").unwrap_or_default();
    let encrypted = read_manifest(&manifest);
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let io = |e: std::io::Error| CryptError::Unsupported(e.to_string());
    let mut checked = false;
    let mut work = 0u64;
    for i in 0..z.len().min(MAX_FILES) {
        let Ok(mut f) = z.by_index(i) else { continue };
        let name = f.name().to_string();
        if f.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        f.by_ref().take(MAX_ENTRY).read_to_end(&mut data).map_err(io)?;
        let data = match encrypted.iter().find(|(p, _)| *p == name) {
            Some((_, p)) => {
                charge(&mut work, p)?;
                let plain = decrypt_file(&data, p, password)?;
                checked = true;
                plain
            }
            None if name == "META-INF/manifest.xml" => plain_manifest(&String::from_utf8_lossy(&data)).into_bytes(),
            None => data,
        };
        w.start_file(name.as_str(), if name == "mimetype" { stored } else { deflated }).map_err(broken)?;
        w.write_all(&data).map_err(io)?;
    }
    if !checked {
        return Err(CryptError::Unsupported("no encrypted file could be read".into()));
    }
    Ok(w.finish().map_err(broken)?.into_inner())
}

/// An attribute value's text (the manifest's own entity escapes).
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn random(n: usize) -> Result<Vec<u8>, String> {
    let mut v = vec![0u8; n];
    getrandom::fill(&mut v).map_err(|e| format!("ODT: no random numbers for encryption ({e})"))?;
    Ok(v)
}

/// A package encrypted with `password`: every file but the `mimetype` and the manifest, the
/// manifest saying how.
pub fn encrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let err = |e: zip::result::ZipError| format!("ODT: {e}");
    let io = |e: std::io::Error| format!("ODT: {e}");
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    let start = start_key(SHA256, Some(32), password).map_err(|e| e.to_string())?;
    let mut files: Vec<(String, Vec<u8>, bool)> = Vec::new();
    let mut manifest = String::new();
    // name → its encryption data and size, for the manifest.
    let mut data_of: Vec<(String, String)> = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(io)?;
        if name == "META-INF/manifest.xml" {
            manifest = String::from_utf8_lossy(&data).into_owned();
            continue;
        }
        if name == "mimetype" {
            files.push((name, data, false));
            continue;
        }
        let size = data.len();
        let mut deflated = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let sum = checksum(SHA256_1K, &deflated).map_err(|e| e.to_string())?;
        // W3C XML Encryption padding: random bytes, the last one their count.
        let pad = 16 - deflated.len() % 16;
        let mut tail = random(pad)?;
        if let Some(last) = tail.last_mut() {
            *last = pad as u8;
        }
        deflated.extend_from_slice(&tail);
        let (salt, iv) = (random(16)?, random(16)?);
        let key = pbkdf2(&start, &salt, ITERATIONS, 32).map_err(|e| e.to_string())?;
        let enc = cbc(&key, &iv, &deflated, true).map_err(|e| e.to_string())?;
        data_of.push((
            name.clone(),
            format!(
                " manifest:size=\"{size}\"><manifest:encryption-data manifest:checksum-type=\"{SHA256_1K}\" manifest:checksum=\"{}\"><manifest:algorithm manifest:algorithm-name=\"{AES256_CBC}\" manifest:initialisation-vector=\"{}\"/><manifest:start-key-generation manifest:start-key-generation-name=\"{SHA256}\" manifest:key-size=\"32\"/><manifest:key-derivation manifest:key-derivation-name=\"PBKDF2\" manifest:iteration-count=\"{ITERATIONS}\" manifest:salt=\"{}\" manifest:key-size=\"32\"/></manifest:encryption-data></manifest:file-entry>",
                STANDARD.encode(&sum),
                STANDARD.encode(&iv),
                STANDARD.encode(&salt)
            ),
        ));
        files.push((name, enc, true));
    }
    // Each encrypted file's entry gets its size and encryption data; a file the manifest
    // doesn't list gets an entry of its own.
    let mut listed = std::collections::HashSet::new();
    let mut out = String::with_capacity(manifest.len() + data_of.len() * 600);
    let mut rest = manifest.as_str();
    while let Some(i) = rest.find("<manifest:file-entry ") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        let Some(end) = after.find("/>") else { break };
        let tag = after.get(..end).unwrap_or("");
        let path = tag.split("manifest:full-path=\"").nth(1).and_then(|r| r.split('"').next()).map(unescape).unwrap_or_default();
        match data_of.iter().find(|(n, _)| *n == path) {
            Some((n, enc)) => {
                listed.insert(n.clone());
                out.push_str(tag);
                out.push_str(enc);
            }
            None => out.push_str(after.get(..end + 2).unwrap_or("")),
        }
        rest = after.get(end + 2..).unwrap_or("");
    }
    let unlisted: String = data_of
        .iter()
        .filter(|(n, _)| !listed.contains(n))
        .map(|(n, enc)| format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"\"{enc}\n", super::writer::x(n)))
        .collect();
    match rest.rfind("</manifest:manifest>") {
        Some(e) => {
            out.push_str(rest.get(..e).unwrap_or(""));
            out.push_str(&unlisted);
            out.push_str(rest.get(e..).unwrap_or(""));
        }
        None => out.push_str(rest),
    }
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data, _) in files.iter().filter(|(n, _, _)| n == "mimetype") {
        w.start_file(name.as_str(), stored).map_err(err)?;
        w.write_all(data).map_err(io)?;
    }
    w.start_file("META-INF/manifest.xml", deflated).map_err(err)?;
    w.write_all(out.as_bytes()).map_err(io)?;
    // Encrypted files are stored: they're deflated already (§3.4.1).
    for (name, data, _) in files.iter().filter(|(n, _, _)| n != "mimetype") {
        w.start_file(name.as_str(), stored).map_err(err)?;
        w.write_all(data).map_err(io)?;
    }
    Ok(w.finish().map_err(err)?.into_inner())
}

#[cfg(test)]
mod work_tests {
    use super::*;

    /// Key derivations are budgeted over the whole package, not only per file: a thousand of
    /// LibreOffice's fit, twenty-one at the per-file cap don't.
    #[test]
    fn a_packages_key_derivations_share_one_budget() {
        let ours = Params { iterations: ITERATIONS, key_size: Some(32), ..Default::default() };
        let mut spent = 0;
        for _ in 0..1000 {
            assert!(charge(&mut spent, &ours).is_ok());
        }
        assert!(charge(&mut spent, &ours).is_err());
        let hostile = Params { iterations: MAX_ITERATIONS, key_size: Some(16), ..Default::default() };
        let mut spent = 0;
        let ok = (0..30).take_while(|_| charge(&mut spent, &hostile).is_ok()).count();
        assert_eq!(ok, 20);
    }
}

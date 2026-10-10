//! Password-protected Word documents: ECMA-376 document encryption with the *agile* method
//! (Word 2010 and later), written from Microsoft's public Open Specification [MS-OFFCRYPTO]
//! §2.3.4.4 and §2.3.4.10–§2.3.4.15, with the data-spaces streams of §2.1 / §2.3.4.1–§2.3.4.3.
//!
//! An encrypted package is not a zip but an OLE compound file ([MS-CFB]) holding
//! `EncryptionInfo` (how the key is derived from the password, as XML) and `EncryptedPackage`
//! (the original zip, AES-CBC encrypted in 4096-byte segments). A random *intermediate key*
//! encrypts the package; the password, hashed `spinCount` times with a salt, encrypts that key and
//! a verifier that tells a wrong password apart; an HMAC over the encrypted stream checks that it
//! hasn't been damaged or tampered with.
//!
//! Files are hostile: every stream read is capped, every length is checked before slicing, and the
//! spin count is bounded by the specification's maximum. Passwords and keys are wiped from memory
//! when dropped ([`Zeroizing`]) and never logged.

use std::io::{Cursor, Read, Write};

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use base64::Engine as _;
use hmac::Mac;
use sha2::Digest;
use zeroize::Zeroizing;

use crate::DocxError;
use crate::xml;

/// The spin count Word uses, and WordCraft writes.
pub const DEFAULT_SPIN_COUNT: u32 = 100_000;
/// The largest spin count the specification allows (§2.3.4.10 `ST_SpinCount`): it bounds the
/// work a hostile file can ask for.
const MAX_SPIN_COUNT: u32 = 10_000_000;
/// Word's limit on password length, in characters.
pub const MAX_PASSWORD_CHARS: usize = 255;
/// `EncryptedPackage` is encrypted in segments of this many bytes (§2.3.4.15).
const SEGMENT: usize = 4096;
/// Largest `EncryptionInfo` stream read (the XML is a few kilobytes).
const MAX_INFO: u64 = 1 << 20;
/// AES's block size.
const BLOCK: usize = 16;
/// Salt length written.
const SALT: usize = 16;

/// Block keys (§2.3.4.13, §2.3.4.14).
const BK_VERIFIER_INPUT: [u8; 8] = [0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79];
const BK_VERIFIER_VALUE: [u8; 8] = [0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e];
const BK_KEY_VALUE: [u8; 8] = [0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6];
const BK_HMAC_KEY: [u8; 8] = [0x5f, 0xb2, 0xad, 0x01, 0x0c, 0xb9, 0xe1, 0xf6];
const BK_HMAC_VALUE: [u8; 8] = [0xa0, 0x67, 0x7f, 0x02, 0xb2, 0x2c, 0x84, 0x33];

/// An OLE compound file starts with these bytes ([MS-CFB] §2.2).
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

const NS_ENCRYPTION: &str = "http://schemas.microsoft.com/office/2006/encryption";
const NS_PASSWORD: &str = "http://schemas.microsoft.com/office/2006/keyEncryptor/password";

/// Is this an encrypted Office package (a compound file with `EncryptionInfo` and
/// `EncryptedPackage` streams) rather than a zip?
pub fn is_encrypted(bytes: &[u8]) -> bool {
    if bytes.get(..8) != Some(&CFB_MAGIC[..]) {
        return false;
    }
    match cfb::CompoundFile::open(Cursor::new(bytes)) {
        Ok(comp) => comp.is_stream("/EncryptionInfo") && comp.is_stream("/EncryptedPackage"),
        Err(_) => false,
    }
}

/// The zip package inside an encrypted Office file. Without a password (`None`) this says
/// [`DocxError::PasswordRequired`] once it knows the file is one it can decrypt; a password that
/// doesn't fit gives [`DocxError::WrongPassword`].
pub fn decrypt(bytes: &[u8], password: Option<&str>) -> Result<Vec<u8>, DocxError> {
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e| bad(format!("not a readable compound file ({e})")))?;
    let info = stream(&mut comp, "/EncryptionInfo", MAX_INFO)?;
    let package = stream(&mut comp, "/EncryptedPackage", bytes.len() as u64)?;
    let info = parse_info(&info)?;
    let Some(password) = password else { return Err(DocxError::PasswordRequired) };
    let pe = &info.password;
    let hn = iterate(pe.hash, &pe.salt, password, pe.spin_count);
    // The verifier (§2.3.4.13): a random value and its hash, both encrypted with keys derived
    // from the password. They match only for the right password.
    let key_input = derive_key(pe.hash, &hn, &BK_VERIFIER_INPUT, pe.key_bytes);
    let key_value = derive_key(pe.hash, &hn, &BK_VERIFIER_VALUE, pe.key_bytes);
    let iv = make_iv(pe.hash, &pe.salt, None, pe.block_size);
    let input = Zeroizing::new(cbc_decrypt(&key_input, &iv, &pe.verifier_input)?);
    let input = input.get(..pe.salt_size).ok_or_else(|| bad("the password verifier is too short"))?;
    let value = Zeroizing::new(cbc_decrypt(&key_value, &iv, &pe.verifier_hash)?);
    let value = value.get(..pe.hash.size()).ok_or_else(|| bad("the password verifier hash is too short"))?;
    if !same(&pe.hash.hash(&[input]), value) {
        return Err(DocxError::WrongPassword);
    }
    // The intermediate key that encrypts the package.
    let key_key = derive_key(pe.hash, &hn, &BK_KEY_VALUE, pe.key_bytes);
    let key = Zeroizing::new(cbc_decrypt(&key_key, &iv, &pe.key_value)?);
    let key = Zeroizing::new(key.get(..info.key_bytes).ok_or_else(|| bad("the encrypted key is too short"))?.to_vec());
    // Data integrity (§2.3.4.14): an HMAC of the whole EncryptedPackage stream.
    if let Some((enc_key, enc_value)) = &info.integrity {
        let hmac_key = Zeroizing::new(cbc_decrypt(&key, &make_iv(info.hash, &info.salt, Some(&BK_HMAC_KEY), info.block_size), enc_key)?);
        // The key is the salt, zero-padded to the block size; HMAC pads short keys with zeros
        // anyway, so the padded key gives the same HMAC.
        let hmac_key = hmac_key.get(..hmac_key.len().min(info.hash.size())).unwrap_or(&[]);
        let expected = cbc_decrypt(&key, &make_iv(info.hash, &info.salt, Some(&BK_HMAC_VALUE), info.block_size), enc_value)?;
        let expected = expected.get(..info.hash.size()).ok_or_else(|| bad("the integrity check value is too short"))?;
        if !same(&info.hash.hmac(hmac_key, &package)?, expected) {
            return Err(bad("the document is damaged (its integrity check failed)"));
        }
    }
    // The package itself (§2.3.4.15): StreamSize, then 4096-byte segments, each with its own IV.
    let size = package
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| bad("the encrypted package is truncated"))?;
    let data = package.get(8..).unwrap_or(&[]);
    let size = usize::try_from(size).ok().filter(|s| *s <= data.len()).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let needed = size.div_ceil(BLOCK).saturating_mul(BLOCK);
    let data = data.get(..needed).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let mut out = Vec::with_capacity(needed);
    for (i, seg) in data.chunks(SEGMENT).enumerate() {
        let i = u32::try_from(i).map_err(|_| bad("the encrypted package is too large"))?;
        let iv = make_iv(info.hash, &info.salt, Some(&i.to_le_bytes()), info.block_size);
        out.extend_from_slice(&cbc_decrypt(&key, &iv, seg)?);
    }
    out.truncate(size);
    Ok(out)
}

/// Encrypt a zip package with `password` (agile encryption: AES-256-CBC, SHA-512, Word's spin
/// count), as Word's "Encrypt with Password" does.
pub fn encrypt(package: &[u8], password: &str) -> Result<Vec<u8>, DocxError> {
    encrypt_with_spin_count(package, password, DEFAULT_SPIN_COUNT)
}

/// [`encrypt`] with a chosen spin count (tests use a small one; files should use the default).
pub fn encrypt_with_spin_count(package: &[u8], password: &str, spin_count: u32) -> Result<Vec<u8>, DocxError> {
    check_password(password)?;
    let spin_count = spin_count.min(MAX_SPIN_COUNT);
    let hash = Hash::Sha512;
    let key_bytes = 32;
    let key_data_salt = random(SALT)?;
    let password_salt = random(SALT)?;
    let key = random(key_bytes)?;
    let verifier = random(SALT)?;
    // The HMAC key is a salt as long as the hash (MS-OFFCRYPTO §2.3.4.14).
    let hmac_salt = random(hash.size())?;

    // The package, segment by segment; the last segment is zero-padded to the block size.
    let mut stream = Vec::with_capacity(package.len().saturating_add(8 + BLOCK));
    stream.extend_from_slice(&(package.len() as u64).to_le_bytes());
    for (i, seg) in package.chunks(SEGMENT).enumerate() {
        let i = u32::try_from(i).map_err(|_| bad("the document is too large to encrypt"))?;
        let iv = make_iv(hash, &key_data_salt, Some(&i.to_le_bytes()), BLOCK);
        stream.extend_from_slice(&cbc_encrypt(&key, &iv, &padded(seg))?);
    }

    // The password key encryptor (§2.3.4.13).
    let hn = iterate(hash, &password_salt, password, spin_count);
    let iv = make_iv(hash, &password_salt, None, BLOCK);
    let enc_verifier_input = cbc_encrypt(&derive_key(hash, &hn, &BK_VERIFIER_INPUT, key_bytes), &iv, &padded(&verifier))?;
    let enc_verifier_hash = cbc_encrypt(&derive_key(hash, &hn, &BK_VERIFIER_VALUE, key_bytes), &iv, &padded(&hash.hash(&[&verifier])))?;
    let enc_key_value = cbc_encrypt(&derive_key(hash, &hn, &BK_KEY_VALUE, key_bytes), &iv, &padded(&key))?;

    // Data integrity (§2.3.4.14).
    let enc_hmac_key = cbc_encrypt(&key, &make_iv(hash, &key_data_salt, Some(&BK_HMAC_KEY), BLOCK), &padded(&hmac_salt))?;
    let hmac = hash.hmac(&hmac_salt, &stream)?;
    let enc_hmac_value = cbc_encrypt(&key, &make_iv(hash, &key_data_salt, Some(&BK_HMAC_VALUE), BLOCK), &padded(&hmac))?;

    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let params = format!(
        r#"saltSize="{SALT}" blockSize="{BLOCK}" keyBits="{}" hashSize="{}" cipherAlgorithm="AES" cipherChaining="ChainingModeCBC" hashAlgorithm="{}""#,
        key_bytes * 8,
        hash.size(),
        hash.name(),
    );
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <encryption xmlns=\"{NS_ENCRYPTION}\" xmlns:p=\"{NS_PASSWORD}\">\
         <keyData {params} saltValue=\"{}\"/>\
         <dataIntegrity encryptedHmacKey=\"{}\" encryptedHmacValue=\"{}\"/>\
         <keyEncryptors><keyEncryptor uri=\"{NS_PASSWORD}\">\
         <p:encryptedKey spinCount=\"{spin_count}\" {params} saltValue=\"{}\" encryptedVerifierHashInput=\"{}\" encryptedVerifierHashValue=\"{}\" encryptedKeyValue=\"{}\"/>\
         </keyEncryptor></keyEncryptors></encryption>",
        b64(&key_data_salt),
        b64(&enc_hmac_key),
        b64(&enc_hmac_value),
        b64(&password_salt),
        b64(&enc_verifier_input),
        b64(&enc_verifier_hash),
        b64(&enc_key_value),
    );
    // EncryptionInfo (§2.3.4.10): version 4.4, reserved 0x40, the XML.
    let mut info = vec![4, 0, 4, 0, 0x40, 0, 0, 0];
    info.extend_from_slice(xml.as_bytes());

    let io = |e: std::io::Error| bad(format!("couldn't write the compound file ({e})"));
    let mut comp = cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).map_err(io)?;
    for (path, data) in [
        ("/\u{6}DataSpaces/Version", data_space_version()),
        ("/\u{6}DataSpaces/DataSpaceMap", data_space_map()),
        ("/\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace", data_space_definition()),
        ("/\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary", primary_transform()),
        ("/EncryptionInfo", info),
        ("/EncryptedPackage", stream),
    ] {
        if let Some((parent, _)) = path.rsplit_once('/')
            && !parent.is_empty()
        {
            comp.create_storage_all(parent).map_err(io)?;
        }
        comp.create_stream(path).map_err(io)?.write_all(&data).map_err(io)?;
    }
    comp.flush().map_err(io)?;
    Ok(comp.into_inner().into_inner())
}

/// Refuse a password Word wouldn't accept: empty, or longer than [`MAX_PASSWORD_CHARS`].
pub fn check_password(password: &str) -> Result<(), DocxError> {
    if password.is_empty() {
        return Err(bad("the password is empty"));
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err(bad(format!("passwords are at most {MAX_PASSWORD_CHARS} characters")));
    }
    Ok(())
}

fn bad(msg: impl Into<String>) -> DocxError {
    DocxError::Encryption(msg.into())
}

/// A root-level stream, at most `cap` bytes.
fn stream<F: Read + std::io::Seek>(comp: &mut cfb::CompoundFile<F>, path: &str, cap: u64) -> Result<Vec<u8>, DocxError> {
    let s = comp.open_stream(path).map_err(|e| bad(format!("can't read {} ({e})", path.trim_start_matches('/'))))?;
    let mut v = Vec::new();
    s.take(cap.saturating_add(1)).read_to_end(&mut v).map_err(|e| bad(format!("can't read {} ({e})", path.trim_start_matches('/'))))?;
    if v.len() as u64 > cap {
        return Err(DocxError::Limit(format!("{} is too large", path.trim_start_matches('/'))));
    }
    Ok(v)
}

/// The hash algorithms agile encryption names that WordCraft supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn parse(s: &str) -> Option<Hash> {
        match s.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Some(Hash::Sha1),
            "SHA256" => Some(Hash::Sha256),
            "SHA384" => Some(Hash::Sha384),
            "SHA512" => Some(Hash::Sha512),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Hash::Sha1 => "SHA-1",
            Hash::Sha256 => "SHA256",
            Hash::Sha384 => "SHA384",
            Hash::Sha512 => "SHA512",
        }
    }

    fn size(self) -> usize {
        match self {
            Hash::Sha1 => 20,
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
            Hash::Sha512 => 64,
        }
    }

    /// The hash of the concatenated `parts`.
    fn hash(self, parts: &[&[u8]]) -> Vec<u8> {
        fn go<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => go::<sha1::Sha1>(parts),
            Hash::Sha256 => go::<sha2::Sha256>(parts),
            Hash::Sha384 => go::<sha2::Sha384>(parts),
            Hash::Sha512 => go::<sha2::Sha512>(parts),
        }
    }

    fn hmac(self, key: &[u8], msg: &[u8]) -> Result<Vec<u8>, DocxError> {
        fn go<M: Mac + KeyInit>(key: &[u8], msg: &[u8]) -> Result<Vec<u8>, DocxError> {
            let mut m = <M as KeyInit>::new_from_slice(key).map_err(|_| bad("bad integrity key"))?;
            m.update(msg);
            Ok(m.finalize().into_bytes().to_vec())
        }
        match self {
            Hash::Sha1 => go::<hmac::Hmac<sha1::Sha1>>(key, msg),
            Hash::Sha256 => go::<hmac::Hmac<sha2::Sha256>>(key, msg),
            Hash::Sha384 => go::<hmac::Hmac<sha2::Sha384>>(key, msg),
            Hash::Sha512 => go::<hmac::Hmac<sha2::Sha512>>(key, msg),
        }
    }
}

/// The password hash after `spin_count` iterations (§2.3.4.11): H0 = H(salt + password as
/// UTF-16LE), Hn = H(iterator + Hn-1).
fn iterate(hash: Hash, salt: &[u8], password: &str, spin_count: u32) -> Zeroizing<Vec<u8>> {
    let pw: Zeroizing<Vec<u8>> = Zeroizing::new(password.encode_utf16().flat_map(u16::to_le_bytes).collect());
    let mut h = Zeroizing::new(hash.hash(&[salt, &pw]));
    for i in 0..spin_count.min(MAX_SPIN_COUNT) {
        *h = hash.hash(&[&i.to_le_bytes(), &h]);
    }
    h
}

/// A key from the iterated password hash and a block key (§2.3.4.11): H(Hn + blockKey),
/// truncated to `len`, or padded with 0x36.
fn derive_key(hash: Hash, hn: &[u8], block_key: &[u8], len: usize) -> Zeroizing<Vec<u8>> {
    let mut k = Zeroizing::new(hash.hash(&[hn, block_key]));
    k.resize(len, 0x36);
    k
}

/// An initialization vector (§2.3.4.12): H(salt + blockKey), or the salt itself, truncated to
/// the block size or padded with 0x36.
fn make_iv(hash: Hash, salt: &[u8], block_key: Option<&[u8]>, block_size: usize) -> Vec<u8> {
    let mut iv = match block_key {
        Some(b) => hash.hash(&[salt, b]),
        None => salt.to_vec(),
    };
    iv.resize(block_size, 0x36);
    iv
}

/// `data` zero-padded to a whole number of blocks.
fn padded(data: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(data.to_vec());
    v.resize(data.len().div_ceil(BLOCK).saturating_mul(BLOCK), 0);
    v
}

fn random(n: usize) -> Result<Zeroizing<Vec<u8>>, DocxError> {
    let mut v = Zeroizing::new(vec![0u8; n]);
    getrandom::fill(&mut v).map_err(|e| bad(format!("no secure random numbers available ({e})")))?;
    Ok(v)
}

/// Compare without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// AES with a 128-, 192- or 256-bit key.
enum Aes {
    A128(aes::Aes128),
    A192(aes::Aes192),
    A256(aes::Aes256),
}

impl Aes {
    fn new(key: &[u8]) -> Result<Aes, DocxError> {
        let err = |_| bad("bad AES key");
        Ok(match key.len() {
            16 => Aes::A128(aes::Aes128::new_from_slice(key).map_err(err)?),
            24 => Aes::A192(aes::Aes192::new_from_slice(key).map_err(err)?),
            32 => Aes::A256(aes::Aes256::new_from_slice(key).map_err(err)?),
            n => return Err(bad(format!("unsupported AES key length {} bits", n * 8))),
        })
    }

    fn encrypt(&self, b: &mut aes::Block) {
        match self {
            Aes::A128(c) => c.encrypt_block(b),
            Aes::A192(c) => c.encrypt_block(b),
            Aes::A256(c) => c.encrypt_block(b),
        }
    }

    fn decrypt(&self, b: &mut aes::Block) {
        match self {
            Aes::A128(c) => c.decrypt_block(b),
            Aes::A192(c) => c.decrypt_block(b),
            Aes::A256(c) => c.decrypt_block(b),
        }
    }
}

/// AES-CBC without padding: `data` must be whole blocks.
fn cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, DocxError> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    let mut prev: [u8; BLOCK] = iv.try_into().map_err(|_| bad("bad initialization vector"))?;
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::default();
        for ((o, p), v) in x.iter_mut().zip(b).zip(&prev) {
            *o = p ^ v;
        }
        aes.encrypt(&mut x);
        prev.copy_from_slice(&x);
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// AES-CBC decryption without padding.
fn cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, DocxError> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut prev: [u8; BLOCK] = iv.try_into().map_err(|_| bad("bad initialization vector"))?;
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::default();
        x.copy_from_slice(b);
        aes.decrypt(&mut x);
        for (o, v) in x.iter_mut().zip(&prev) {
            *o ^= v;
        }
        prev = *b;
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// What `EncryptionInfo` says.
struct Info {
    hash: Hash,
    salt: Vec<u8>,
    block_size: usize,
    key_bytes: usize,
    /// encryptedHmacKey, encryptedHmacValue.
    integrity: Option<(Vec<u8>, Vec<u8>)>,
    password: PasswordEncryptor,
}

struct PasswordEncryptor {
    hash: Hash,
    salt: Vec<u8>,
    salt_size: usize,
    block_size: usize,
    key_bytes: usize,
    spin_count: u32,
    verifier_input: Vec<u8>,
    verifier_hash: Vec<u8>,
    key_value: Vec<u8>,
}

/// Cipher parameters shared by `keyData` and `encryptedKey`.
struct Params {
    hash: Hash,
    salt: Vec<u8>,
    salt_size: usize,
    block_size: usize,
    key_bytes: usize,
}

fn parse_info(info: &[u8]) -> Result<Info, DocxError> {
    let (major, minor) = match info.get(..4) {
        Some([a, b, c, d]) => (u16::from_le_bytes([*a, *b]), u16::from_le_bytes([*c, *d])),
        _ => return Err(bad("EncryptionInfo is truncated")),
    };
    match (major, minor) {
        (4, 4) => {}
        (2..=4, 2) => return Err(bad("this file uses Standard encryption (Office 2007), which WordCraft can't open yet")),
        (3 | 4, 3) => return Err(bad("this file uses extensible encryption, which WordCraft can't open")),
        _ => return Err(bad(format!("unknown encryption version {major}.{minor}"))),
    }
    let root = xml::parse(info.get(8..).unwrap_or(&[]))?;
    if root.local() != "encryption" {
        return Err(bad("EncryptionInfo has no <encryption> element"));
    }
    let key_data = root.els().find(|e| e.local() == "keyData").ok_or_else(|| bad("EncryptionInfo has no keyData"))?;
    let kd = params(key_data)?;
    let integrity = match root.els().find(|e| e.local() == "dataIntegrity") {
        Some(di) => Some((b64_attr(di, "encryptedHmacKey")?, b64_attr(di, "encryptedHmacValue")?)),
        None => None,
    };
    // The password key encryptor is the `encryptedKey` with a spin count (a certificate key
    // encryptor has none).
    let encryptors = root.els().find(|e| e.local() == "keyEncryptors").ok_or_else(|| bad("EncryptionInfo has no keyEncryptors"))?;
    let ek = encryptors
        .els()
        .filter(|e| e.local() == "keyEncryptor")
        .flat_map(|e| e.els())
        .find(|e| e.local() == "encryptedKey" && e.attr("spinCount").is_some())
        .ok_or_else(|| bad("this file isn't protected with a password (only certificate encryption is listed)"))?;
    let pe = params(ek)?;
    let spin_count: u32 = ek.attr("spinCount").and_then(|s| s.trim().parse().ok()).ok_or_else(|| bad("bad spinCount"))?;
    if spin_count > MAX_SPIN_COUNT {
        return Err(bad(format!("spinCount {spin_count} is over the allowed maximum")));
    }
    Ok(Info {
        hash: kd.hash,
        salt: kd.salt,
        block_size: kd.block_size,
        key_bytes: kd.key_bytes,
        integrity,
        password: PasswordEncryptor {
            hash: pe.hash,
            salt: pe.salt,
            salt_size: pe.salt_size,
            block_size: pe.block_size,
            key_bytes: pe.key_bytes,
            spin_count,
            verifier_input: b64_attr(ek, "encryptedVerifierHashInput")?,
            verifier_hash: b64_attr(ek, "encryptedVerifierHashValue")?,
            key_value: b64_attr(ek, "encryptedKeyValue")?,
        },
    })
}

fn params(e: &xml::El) -> Result<Params, DocxError> {
    let num = |k: &str| -> Result<usize, DocxError> { e.attr(k).and_then(|s| s.trim().parse::<usize>().ok()).ok_or_else(|| bad(format!("bad {k}"))) };
    let cipher = e.attr("cipherAlgorithm").unwrap_or("");
    if !cipher.eq_ignore_ascii_case("AES") {
        return Err(bad(format!("the {cipher:?} cipher isn't supported")));
    }
    let chaining = e.attr("cipherChaining").unwrap_or("");
    if chaining != "ChainingModeCBC" {
        return Err(bad(format!("the {chaining:?} chaining mode isn't supported")));
    }
    let name = e.attr("hashAlgorithm").unwrap_or("");
    let hash = Hash::parse(name).ok_or_else(|| bad(format!("the {name:?} hash algorithm isn't supported")))?;
    if num("hashSize")? != hash.size() {
        return Err(bad("hashSize doesn't match the hash algorithm"));
    }
    let block_size = num("blockSize")?;
    if block_size != BLOCK {
        return Err(bad(format!("AES needs a block size of {BLOCK}, not {block_size}")));
    }
    let key_bits = num("keyBits")?;
    if !matches!(key_bits, 128 | 192 | 256) {
        return Err(bad(format!("unsupported AES key size {key_bits}")));
    }
    let salt = b64_attr(e, "saltValue")?;
    let salt_size = num("saltSize")?;
    if salt.is_empty() || salt.len() != salt_size {
        return Err(bad("the salt doesn't match saltSize"));
    }
    Ok(Params { hash, salt, salt_size, block_size, key_bytes: key_bits / 8 })
}

fn b64_attr(e: &xml::El, k: &str) -> Result<Vec<u8>, DocxError> {
    let s = e.attr(k).ok_or_else(|| bad(format!("{k} is missing")))?;
    let s: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| bad(format!("{k} isn't valid base64")))
}

/// A UNICODE-LP-P4 string (§2.1.2): byte length, UTF-16LE, zero-padded to 4 bytes.
fn lp_p4(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out.extend_from_slice(&(units.len() as u32).to_le_bytes());
    out.extend_from_slice(&units);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// `\x06DataSpaces\Version` (§2.1.5).
fn data_space_version() -> Vec<u8> {
    let mut v = Vec::new();
    lp_p4(&mut v, "Microsoft.Container.DataSpaces");
    for _ in 0..3 {
        v.extend_from_slice(&[1, 0, 0, 0]);
    }
    v
}

/// `\x06DataSpaces\DataSpaceMap` (§2.1.6, §2.3.4.1): EncryptedPackage → StrongEncryptionDataSpace.
fn data_space_map() -> Vec<u8> {
    let mut entry = Vec::new();
    entry.extend_from_slice(&1u32.to_le_bytes());
    entry.extend_from_slice(&0u32.to_le_bytes());
    lp_p4(&mut entry, "EncryptedPackage");
    lp_p4(&mut entry, "StrongEncryptionDataSpace");
    let mut v = Vec::new();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&((entry.len() + 4) as u32).to_le_bytes());
    v.extend_from_slice(&entry);
    v
}

/// `\x06DataSpaces\DataSpaceInfo\StrongEncryptionDataSpace` (§2.1.7, §2.3.4.2).
fn data_space_definition() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    lp_p4(&mut v, "StrongEncryptionTransform");
    v
}

/// `\x06DataSpaces\TransformInfo\StrongEncryptionTransform\x06Primary` (§2.1.8, §2.1.9,
/// §2.3.4.3): the encryption transform, with a null EncryptionName as agile encryption requires.
fn primary_transform() -> Vec<u8> {
    let mut id = Vec::new();
    lp_p4(&mut id, "{FF9A3F03-56EF-4613-BDD5-5A41C1D07246}");
    let mut v = Vec::new();
    // TransformLength: the bytes before TransformName.
    v.extend_from_slice(&((8 + id.len()) as u32).to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&id);
    lp_p4(&mut v, "Microsoft.Container.EncryptionTransform");
    for _ in 0..3 {
        v.extend_from_slice(&[1, 0, 0, 0]);
    }
    // EncryptionTransformInfo: null name, block size, cipher mode 0, reserved 4.
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&(BLOCK as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&4u32.to_le_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_package() -> Vec<u8> {
        let mut doc = wordcraft_doc::Document::from_text("Top secret plans\nSecond paragraph");
        doc.core.title = "Secret".into();
        crate::write(&doc).unwrap()
    }

    #[test]
    fn round_trip_with_a_password() {
        let pkg = zip_package();
        let enc = encrypt_with_spin_count(&pkg, "pässwörd 🔑", 16).unwrap();
        assert!(is_encrypted(&enc));
        assert!(!is_encrypted(&pkg));
        // The compound file carries the data spaces Word expects.
        let comp = cfb::CompoundFile::open(Cursor::new(&enc)).unwrap();
        assert!(comp.is_stream("/\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary"));
        assert_eq!(decrypt(&enc, Some("pässwörd 🔑")).unwrap(), pkg);
        let doc = crate::read_with_password(&enc, Some("pässwörd 🔑")).unwrap();
        assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Top secret plans"));
        assert_eq!(doc.core.title, "Secret");
        // Without a password, or with the wrong one, it says so instead of opening blank.
        assert_eq!(crate::read(&enc), Err(DocxError::PasswordRequired));
        assert_eq!(crate::read_with_password(&enc, None), Err(DocxError::PasswordRequired));
        assert_eq!(decrypt(&enc, Some("Pässwörd 🔑")), Err(DocxError::WrongPassword));
        // A package that is a whole number of segments, and an empty one.
        for n in [0usize, SEGMENT, SEGMENT * 2 + 1] {
            let data: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            let enc = encrypt_with_spin_count(&data, "x", 1).unwrap();
            assert_eq!(decrypt(&enc, Some("x")).unwrap(), data);
        }
        assert!(encrypt(&pkg, "").is_err());
        assert!(encrypt(&pkg, &"x".repeat(MAX_PASSWORD_CHARS + 1)).is_err());
    }

    #[test]
    fn tampered_package_fails_the_integrity_check() {
        let pkg = zip_package();
        let enc = encrypt_with_spin_count(&pkg, "pw", 4).unwrap();
        let mut comp = cfb::CompoundFile::open(Cursor::new(enc)).unwrap();
        let mut data = Vec::new();
        comp.open_stream("/EncryptedPackage").unwrap().read_to_end(&mut data).unwrap();
        let last = data.len() - 1;
        data[last] ^= 1;
        comp.create_stream("/EncryptedPackage").unwrap().write_all(&data).unwrap();
        comp.flush().unwrap();
        let enc = comp.into_inner().into_inner();
        assert!(matches!(decrypt(&enc, Some("pw")), Err(DocxError::Encryption(m)) if m.contains("integrity")));
    }

    /// A compound file with the given EncryptionInfo and EncryptedPackage streams.
    fn cfb_with(info: &[u8], package: &[u8]) -> Vec<u8> {
        let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        comp.create_stream("/EncryptionInfo").unwrap().write_all(info).unwrap();
        comp.create_stream("/EncryptedPackage").unwrap().write_all(package).unwrap();
        comp.flush().unwrap();
        comp.into_inner().into_inner()
    }

    #[test]
    fn hostile_encryption_info_errors_without_panicking() {
        let good = encrypt_with_spin_count(&zip_package(), "pw", 2).unwrap();
        let mut comp = cfb::CompoundFile::open(Cursor::new(&good)).unwrap();
        let mut info = Vec::new();
        comp.open_stream("/EncryptionInfo").unwrap().read_to_end(&mut info).unwrap();
        let mut package = Vec::new();
        comp.open_stream("/EncryptedPackage").unwrap().read_to_end(&mut package).unwrap();
        let xml = String::from_utf8(info[8..].to_vec()).unwrap();
        // (EncryptionInfo, EncryptedPackage, must fail). Truncations only must not panic: the
        // XML reader is lenient, so cutting the closing tags still reads.
        let mut cases: Vec<(Vec<u8>, Vec<u8>, bool)> = Vec::new();
        for n in (0..info.len()).step_by(7) {
            cases.push((info[..n].to_vec(), package.clone(), n < info.len() / 2));
        }
        for n in [0, 4, 8, 9, 100, package.len() - 1] {
            cases.push((info.clone(), package[..n].to_vec(), true));
        }
        // Standard and unknown versions, absurd numbers, bad base64, a huge StreamSize.
        let with = |from: &str, to: &str| {
            let mut v = info[..8].to_vec();
            v.extend_from_slice(xml.replacen(from, to, 2).as_bytes());
            v
        };
        cases.push(([3u8, 0, 2, 0, 0x24, 0, 0, 0].to_vec(), package.clone(), true));
        cases.push(([9u8, 0, 9, 0].to_vec(), package.clone(), true));
        cases.push((with("spinCount=\"2\"", "spinCount=\"4294967295\""), package.clone(), true));
        cases.push((with("keyBits=\"256\"", "keyBits=\"8\""), package.clone(), true));
        cases.push((with("blockSize=\"16\"", "blockSize=\"4096\""), package.clone(), true));
        cases.push((with("hashSize=\"64\"", "hashSize=\"0\""), package.clone(), true));
        cases.push((with("saltSize=\"16\"", "saltSize=\"99999999999999999999\""), package.clone(), true));
        cases.push((with("SHA512", "MD5"), package.clone(), true));
        cases.push((with("ChainingModeCBC", "ChainingModeCFB"), package.clone(), true));
        cases.push((with("saltValue=\"", "saltValue=\"!!"), package.clone(), true));
        cases.push((with("encryptedKeyValue=\"", "encryptedKeyValue=\"AAAA"), package.clone(), true));
        // Data integrity is optional; without it a good file still opens.
        cases.push((with("<dataIntegrity", "<nothing"), package.clone(), false));
        let mut huge = package.clone();
        huge[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        cases.push((with("<dataIntegrity", "<nothing"), huge, true));
        for (i, p, must_fail) in cases {
            let file = cfb_with(&i, &p);
            let r = decrypt(&file, Some("pw"));
            assert!(!must_fail || r.is_err());
            let _ = crate::read_with_password(&file, Some("pw"));
            assert!(crate::read(&file).is_err());
        }
        // A file that only looks like a compound file.
        let mut fake = CFB_MAGIC.to_vec();
        fake.extend_from_slice(&[0u8; 600]);
        assert!(!is_encrypted(&fake));
        assert!(decrypt(&fake, Some("pw")).is_err());
    }
}

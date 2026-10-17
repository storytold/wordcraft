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
//! Older packages (Word 2007) use the *Standard* method (§2.3.4.5–§2.3.4.9): a binary
//! `EncryptionInfo`, one AES key derived from the password with SHA-1 and 50,000 spins, and the
//! whole package AES-ECB encrypted. [`decrypt`] reads both; [`encrypt`] writes agile only.
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

/// Standard encryption (§2.3.2, §2.3.4.5–§2.3.4.7): `EncryptionHeader.Flags` bits, the AES and
/// SHA-1 algorithm ids, the provider type, the fixed spin count and the SHA-1 size.
const F_CRYPTO_API: u32 = 0x04;
const F_EXTERNAL: u32 = 0x10;
const F_AES: u32 = 0x20;
const STANDARD_FLAGS: u32 = F_CRYPTO_API | F_AES;
const ALG_AES_128: u32 = 0x660E;
const ALG_AES_192: u32 = 0x660F;
const ALG_AES_256: u32 = 0x6610;
const ALG_SHA1: u32 = 0x8004;
const PROV_RSA_AES: u32 = 0x18;
const STANDARD_SPIN_COUNT: u32 = 50_000;
const SHA1_SIZE: usize = 20;

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
    if matches!(info.get(..4), Some([2..=4, 0, 2, 0])) {
        return decrypt_standard(&info, &package, password);
    }
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

/// [`decrypt`] for Standard encryption (§2.3.4.5–§2.3.4.9): one AES key from the password,
/// checked with the verifier, and the whole package AES-ECB encrypted after its 8-byte size.
fn decrypt_standard(info: &[u8], package: &[u8], password: Option<&str>) -> Result<Vec<u8>, DocxError> {
    let s = parse_standard(info)?;
    let Some(password) = password else { return Err(DocxError::PasswordRequired) };
    let key = standard_key(s.salt, password, s.key_bytes);
    // The verifier (§2.3.4.9): a random value and its SHA-1, both encrypted with the key.
    let verifier = Zeroizing::new(ecb(&key, s.verifier, false)?);
    let hash = Zeroizing::new(ecb(&key, s.verifier_hash, false)?);
    let hash = hash.get(..SHA1_SIZE).ok_or_else(|| bad("the password verifier hash is too short"))?;
    if !same(&Hash::Sha1.hash(&[&verifier]), hash) {
        return Err(DocxError::WrongPassword);
    }
    // The package (§2.3.4.4): StreamSize, then the data; bytes past the last block are ignored.
    let size = package
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| bad("the encrypted package is truncated"))?;
    let data = package.get(8..).unwrap_or(&[]);
    let size = usize::try_from(size).ok().filter(|s| *s <= data.len()).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let needed = size.div_ceil(BLOCK).saturating_mul(BLOCK);
    let data = data.get(..needed).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let mut out = ecb(&key, data, false)?;
    out.truncate(size);
    Ok(out)
}

/// What a Standard `EncryptionInfo` stream says (§2.3.4.5): the key size, and the verifier's salt
/// and encrypted values.
struct Standard<'a> {
    key_bytes: usize,
    salt: &'a [u8],
    verifier: &'a [u8],
    verifier_hash: &'a [u8],
}

fn parse_standard(info: &[u8]) -> Result<Standard<'_>, DocxError> {
    let short = || bad("EncryptionInfo is truncated");
    let u32_at = |at: usize| -> Result<u32, DocxError> {
        let b = at.checked_add(4).and_then(|end| info.get(at..end)).ok_or_else(short)?;
        <[u8; 4]>::try_from(b).map(u32::from_le_bytes).map_err(|_| short())
    };
    let flags = u32_at(4)?;
    if flags & F_EXTERNAL != 0 {
        return Err(bad("this file uses extensible encryption, which WordCraft can't open"));
    }
    if flags & STANDARD_FLAGS != STANDARD_FLAGS {
        return Err(bad("this file's encryption isn't AES, which WordCraft can't open"));
    }
    // EncryptionHeader (§2.3.2): Flags, SizeExtra, AlgID, AlgIDHash, KeySize, ProviderType, two
    // reserved words, then the CSP name, which isn't needed.
    let header_size = usize::try_from(u32_at(8)?).map_err(|_| short())?;
    if header_size < 32 {
        return Err(bad("the encryption header is too short"));
    }
    let verifier_at = header_size.checked_add(12).filter(|&v| v <= info.len()).ok_or_else(short)?;
    // AlgID 0 and AlgIDHash 0 mean "set by the flags": AES-128 and SHA-1.
    let bits = match u32_at(20)? {
        0 | ALG_AES_128 => 128,
        ALG_AES_192 => 192,
        ALG_AES_256 => 256,
        id => return Err(bad(format!("encryption algorithm 0x{id:X} isn't supported"))),
    };
    let hash_id = u32_at(24)?;
    if !matches!(hash_id, 0 | ALG_SHA1) {
        return Err(bad(format!("hash algorithm 0x{hash_id:X} isn't supported")));
    }
    if u32_at(28)? != bits {
        return Err(bad("the key size doesn't match the encryption algorithm"));
    }
    // EncryptionVerifier (§2.3.3): SaltSize (16), Salt, EncryptedVerifier, VerifierHashSize (20
    // for SHA-1), EncryptedVerifierHash (32 bytes for AES).
    if u32_at(verifier_at)? != SALT as u32 {
        return Err(bad("the salt isn't 16 bytes"));
    }
    let field = |at: usize, len: usize| verifier_at.checked_add(at).and_then(|a| info.get(a..a.checked_add(len)?)).ok_or_else(short);
    let salt = field(4, SALT)?;
    let verifier = field(4 + SALT, BLOCK)?;
    if u32_at(verifier_at.saturating_add(4 + SALT + BLOCK))? != SHA1_SIZE as u32 {
        return Err(bad("the password verifier hash isn't SHA-1"));
    }
    let verifier_hash = field(8 + SALT + BLOCK, 2 * BLOCK)?;
    Ok(Standard { key_bytes: bits as usize / 8, salt, verifier, verifier_hash })
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

/// A package encrypted with the *Standard* method (§2.3.4.5–§2.3.4.9: AES-ECB, SHA-1, 50,000
/// spins), as Word 2007 wrote it, for tests only: WordCraft saves with the agile method
/// ([`encrypt`]). `key_bits` is 128, 192 or 256. The compound file holds just the two streams
/// [`decrypt`] reads.
#[doc(hidden)]
pub fn encrypt_standard_for_tests(package: &[u8], password: &str, key_bits: u32) -> Result<Vec<u8>, DocxError> {
    check_password(password)?;
    let alg_id = match key_bits {
        128 => ALG_AES_128,
        192 => ALG_AES_192,
        256 => ALG_AES_256,
        _ => return Err(bad(format!("unsupported AES key size {key_bits}"))),
    };
    let salt = random(SALT)?;
    let verifier = random(SALT)?;
    let key = standard_key(&salt, password, key_bits as usize / 8);
    let mut header = Vec::new();
    for v in [STANDARD_FLAGS, 0, alg_id, ALG_SHA1, key_bits, PROV_RSA_AES, 0, 0] {
        header.extend_from_slice(&v.to_le_bytes());
    }
    // CSPName: an empty null-terminated UTF-16 string.
    header.extend_from_slice(&[0, 0]);
    let mut info = vec![3, 0, 2, 0];
    info.extend_from_slice(&STANDARD_FLAGS.to_le_bytes());
    info.extend_from_slice(&(header.len() as u32).to_le_bytes());
    info.extend_from_slice(&header);
    info.extend_from_slice(&(SALT as u32).to_le_bytes());
    info.extend_from_slice(&salt);
    info.extend_from_slice(&ecb(&key, &verifier, true)?);
    info.extend_from_slice(&(SHA1_SIZE as u32).to_le_bytes());
    info.extend_from_slice(&ecb(&key, &padded(&Hash::Sha1.hash(&[&verifier])), true)?);
    let mut stream = (package.len() as u64).to_le_bytes().to_vec();
    stream.extend_from_slice(&ecb(&key, &padded(package), true)?);
    let io = |e: std::io::Error| bad(format!("couldn't write the compound file ({e})"));
    let mut comp = cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).map_err(io)?;
    comp.create_stream("/EncryptionInfo").map_err(io)?.write_all(&info).map_err(io)?;
    comp.create_stream("/EncryptedPackage").map_err(io)?.write_all(&stream).map_err(io)?;
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

/// Restrict Editing's password (`w:documentProtection`, ECMA-376 Part 1 §17.15.1.29): the
/// attributes that store its hash — SHA-512 of a random 16-byte salt and the password as UTF-16LE,
/// hashed again `spin_count` times with the iteration number appended ([MS-OFFCRYPTO] §2.4.2.4,
/// the ISO write-protection method). Protection isn't encryption: the document stays readable,
/// the password only guards turning protection off.
pub fn protection_hash(password: &str, spin_count: u32) -> Result<Vec<(String, String)>, DocxError> {
    check_password(password)?;
    let spin_count = spin_count.min(MAX_SPIN_COUNT);
    let salt = random(SALT)?;
    let hash = iterate_appended(Hash::Sha512, &salt, password, spin_count);
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    Ok(vec![
        ("w:algorithmName".into(), "SHA-512".into()),
        ("w:hashValue".into(), b64(&hash)),
        ("w:saltValue".into(), b64(&salt)),
        ("w:spinCount".into(), spin_count.to_string()),
    ])
}

/// What checking a Restrict Editing password found ([`check_protection_password`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    /// The protection has no password: anyone may stop it.
    NoPassword,
    /// The password matches the stored hash.
    Correct,
    /// The password doesn't match.
    Incorrect,
    /// The hash uses an algorithm or form WordCraft can't check, or is damaged. The protection
    /// must stay on: guessing would let any password through.
    Unsupported,
}

/// Does `password` match a Restrict Editing hash (attributes as [`protection_hash`] makes them, or
/// as read from a file)? Two forms are checked:
///
/// - `w:algorithmName` / `w:hashValue` / `w:saltValue` / `w:spinCount`: the password itself is
///   hashed ([`iterate_appended`]).
/// - `w:cryptAlgorithmSid` / `w:hash` / `w:salt` / `w:cryptSpinCount` (ECMA-376 Part 4 §14.8.1):
///   the password first becomes Word's legacy 32-bit key ([`legacy_key`]), whose bytes, reversed
///   and written as an upper-case hex string, are hashed the same way.
///
/// Anything else — an algorithm other than SHA-1/SHA-256/SHA-384/SHA-512, an application-defined
/// algorithm (`w:algIdExt`), a class other than `hash`, a spin count above the 10,000,000 Word
/// accepts, damaged values — is [`PasswordCheck::Unsupported`], never a match.
pub fn check_protection_password(attrs: &[(String, String)], password: &str) -> PasswordCheck {
    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim());
    if get("w:hashValue").is_some() {
        let Some(hash) = get("w:algorithmName").and_then(Hash::parse) else { return PasswordCheck::Unsupported };
        return verify(hash, get("w:hashValue"), get("w:saltValue"), get("w:spinCount"), password);
    }
    if get("w:hash").is_none() {
        // §17.15.1.29: without a hash no password is associated with the protection.
        return PasswordCheck::NoPassword;
    }
    let class_ok = get("w:cryptAlgorithmClass").is_none_or(|c| c == "hash");
    let type_ok = get("w:cryptAlgorithmType").is_none_or(|t| t == "typeAny");
    if !class_ok || !type_ok || get("w:algIdExt").is_some() {
        return PasswordCheck::Unsupported;
    }
    // Part 4 §14.8.1 lists 1 MD2, 2 MD4, 3 MD5, 4 SHA-1, 12 SHA-256, 13 SHA-384, 14 SHA-512.
    let hash = match get("w:cryptAlgorithmSid").and_then(|v| v.parse::<u32>().ok()) {
        Some(4) => Hash::Sha1,
        Some(12) => Hash::Sha256,
        Some(13) => Hash::Sha384,
        Some(14) => Hash::Sha512,
        _ => return PasswordCheck::Unsupported,
    };
    verify(hash, get("w:hash"), get("w:salt"), get("w:cryptSpinCount"), &legacy_key_hex(password))
}

/// Hash `input` with the stored salt and spin count and compare with the stored hash. Damaged or
/// out-of-range values are [`PasswordCheck::Unsupported`].
fn verify(hash: Hash, want: Option<&str>, salt: Option<&str>, spin_count: Option<&str>, input: &str) -> PasswordCheck {
    let dec = |v: &str| base64::engine::general_purpose::STANDARD.decode(v).ok();
    let Some(want) = want.and_then(dec) else { return PasswordCheck::Unsupported };
    // §17.15.1.29: without a salt nothing is prepended.
    let salt = match salt {
        Some(v) => match dec(v) {
            Some(s) => s,
            None => return PasswordCheck::Unsupported,
        },
        None => Vec::new(),
    };
    // Word accepts spin counts from 0 to 10,000,000 ([MS-OI29500] 2.1.402 d); a missing one is 0.
    let spin_count = match spin_count.map(str::parse::<u32>) {
        None => 0,
        Some(Ok(n)) if n <= MAX_SPIN_COUNT => n,
        Some(_) => return PasswordCheck::Unsupported,
    };
    if want.len() != hash.size() || salt.len() > 1024 {
        return PasswordCheck::Unsupported;
    }
    if same(&iterate_appended(hash, &salt, input, spin_count), &want) { PasswordCheck::Correct } else { PasswordCheck::Incorrect }
}

/// The initial value of the legacy key's high-order word, by password length 1–15 (ECMA-376
/// Part 4 §14.8.1, "Initial code array").
const LEGACY_INITIAL_CODE: [u16; 15] =
    [0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F, 0x84F9, 0x280C, 0xA96A, 0x4EC3];

/// The legacy key's encryption matrix (Part 4 §14.8.1): one row per character position, rows
/// "Last-14" to "Last", one column per bit 0–6. The password's last character uses the last row.
const LEGACY_MATRIX: [[u16; 7]; 15] = [
    [0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09],
    [0x7B61, 0xF6C2, 0xFDA5, 0xEB6B, 0xC6F7, 0x9DCF, 0x2BBF],
    [0x4563, 0x8AC6, 0x05AD, 0x0B5A, 0x16B4, 0x2D68, 0x5AD0],
    [0x0375, 0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40],
    [0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D, 0xAA7A, 0x44D5],
    [0x6F45, 0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A],
    [0xEB23, 0xC667, 0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9],
    [0x47D3, 0x8FA6, 0x0F6D, 0x1EDA, 0x3DB4, 0x7B68, 0xF6D0],
    [0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC],
    [0x45A0, 0x8B40, 0x06A1, 0x0D42, 0x1A84, 0x3508, 0x6A10],
    [0xAA51, 0x4483, 0x8906, 0x022D, 0x045A, 0x08B4, 0x1168],
    [0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C],
    [0x3730, 0x6E60, 0xDCC0, 0xA9A1, 0x4363, 0x86C6, 0x1DAD],
    [0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC],
    [0x1021, 0x2042, 0x4084, 0x8108, 0x1231, 0x2462, 0x48C4],
];

/// Word's legacy 32-bit password key (ECMA-376 Part 4 §14.8.1): the password as UTF-16 without a
/// leading BOM, cut to 15 characters, each character reduced to one byte (its low byte, or its
/// high byte when the low byte is 0); the high-order word comes from the initial code array and
/// the encryption matrix, the low-order word from rotating and XOR-ing the bytes backwards.
fn legacy_key(password: &str) -> u32 {
    let mut units = password.encode_utf16().peekable();
    units.next_if_eq(&0xFEFF);
    let bytes: Zeroizing<Vec<u8>> =
        Zeroizing::new(units.take(LEGACY_INITIAL_CODE.len()).map(|u| if u & 0xFF != 0 { (u & 0xFF) as u8 } else { (u >> 8) as u8 }).collect());
    let Some(&initial) = bytes.len().checked_sub(1).and_then(|i| LEGACY_INITIAL_CODE.get(i)) else { return 0 };
    let mut high = initial;
    let first_row = LEGACY_MATRIX.len() - bytes.len();
    for (row, &c) in LEGACY_MATRIX.iter().skip(first_row).zip(bytes.iter()) {
        for (bit, &code) in row.iter().enumerate() {
            if c & (1 << bit) != 0 {
                high ^= code;
            }
        }
    }
    let rotate = |w: u16| ((w >> 14) & 0x0001) | ((w << 1) & 0x7FFF);
    let mut low = 0u16;
    for &c in bytes.iter().rev() {
        low = rotate(low) ^ u16::from(c);
    }
    // `bytes` holds at most 15 values, so its length fits a word.
    low = rotate(low) ^ (bytes.len() as u16) ^ 0xCE4B;
    (u32::from(high) << 16) | u32::from(low)
}

/// What the legacy form hashes: the key's bytes in reversed order (Part 4 §14.8.1: 0x64CEED7E
/// becomes 7EEDCE64), as an upper-case hex string — hashed as UTF-16LE like a password, as Word
/// does ([MS-OE376] 2.1.410 a).
fn legacy_key_hex(password: &str) -> Zeroizing<String> {
    let key = legacy_key(password);
    Zeroizing::new(key.to_le_bytes().iter().map(|b| format!("{b:02X}")).collect())
}

/// The ISO write-protection hash ([MS-OFFCRYPTO] §2.4.2.4): H0 = H(salt + password as UTF-16LE),
/// Hn = H(Hn-1 + iterator) — the iterator after the hash, unlike encryption's [`iterate`].
fn iterate_appended(hash: Hash, salt: &[u8], password: &str, spin_count: u32) -> Zeroizing<Vec<u8>> {
    let pw: Zeroizing<Vec<u8>> = Zeroizing::new(password.encode_utf16().flat_map(u16::to_le_bytes).collect());
    let mut h = Zeroizing::new(hash.hash(&[salt, &pw]));
    for i in 0..spin_count.min(MAX_SPIN_COUNT) {
        *h = hash.hash(&[&h, &i.to_le_bytes()]);
    }
    h
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

/// Standard encryption's key (§2.3.4.7): the SHA-1 password hash after 50,000 iterations, hashed
/// with block number 0; then X1 = SHA-1 of that hash XOR-ed into 64 bytes of 0x36, X2 the same
/// with 0x5C, and the key is the first `len` bytes of X1 + X2.
fn standard_key(salt: &[u8], password: &str, len: usize) -> Zeroizing<Vec<u8>> {
    let hn = iterate(Hash::Sha1, salt, password, STANDARD_SPIN_COUNT);
    let hfinal = Zeroizing::new(Hash::Sha1.hash(&[&hn, &0u32.to_le_bytes()]));
    let x = |fill: u8| {
        let mut buf = Zeroizing::new([fill; 64]);
        for (b, h) in buf.iter_mut().zip(hfinal.iter()) {
            *b ^= h;
        }
        Zeroizing::new(Hash::Sha1.hash(&[&buf[..]]))
    };
    let mut key = Zeroizing::new(x(0x36).to_vec());
    key.extend_from_slice(&x(0x5C));
    key.truncate(len);
    key
}

/// AES-ECB without padding (Standard encryption, §2.3.4.7–§2.3.4.9): `data` must be whole blocks.
fn ecb(key: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, DocxError> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::from(*b);
        if encrypt {
            aes.encrypt(&mut x);
        } else {
            aes.decrypt(&mut x);
        }
        out.extend_from_slice(&x);
    }
    Ok(out)
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
    #[test]
    fn protection_password_round_trip() {
        use super::PasswordCheck::*;
        let attrs = super::protection_hash("s3cret", 1000).unwrap();
        assert_eq!(super::check_protection_password(&attrs, "s3cret"), Correct);
        assert_eq!(super::check_protection_password(&attrs, "S3cret"), Incorrect);
        // Damaged attributes are refused, never accepted.
        let mut bad = attrs.clone();
        bad.retain(|(k, _)| k != "w:hashValue");
        bad.push(("w:hashValue".into(), "not base64!".into()));
        assert_eq!(super::check_protection_password(&bad, "s3cret"), Unsupported);
        let mut odd = attrs.clone();
        odd.retain(|(k, _)| k != "w:algorithmName");
        odd.push(("w:algorithmName".into(), "WHIRLPOOL".into()));
        assert_eq!(super::check_protection_password(&odd, "s3cret"), Unsupported);
        assert_eq!(super::check_protection_password(&[("w:cryptProviderType".into(), "rsaAES".into())], "x"), NoPassword);
    }

    /// The worked example of ECMA-376 Part 4 §14.8.1: "Example" gives the key 0x64CEED7E, hashed
    /// as "7EEDCE64".
    #[test]
    fn legacy_key_matches_the_specification_example() {
        assert_eq!(super::legacy_key("Example"), 0x64CE_ED7E);
        assert_eq!(super::legacy_key_hex("Example").as_str(), "7EEDCE64");
        assert_eq!(super::legacy_key(""), 0);
        // A leading BOM is dropped; only the first 15 characters count.
        assert_eq!(super::legacy_key("\u{FEFF}Example"), 0x64CE_ED7E);
        assert_eq!(super::legacy_key("abcdefghijklmnopq"), super::legacy_key("abcdefghijklmno"));
        assert_ne!(super::legacy_key("abcdefghijklmno"), super::legacy_key("abcdefghijklmn"));
    }

    /// The legacy form (`w:cryptAlgorithmSid`, `w:hash`, `w:salt`, `w:cryptSpinCount`), as Word
    /// writes it. The expected hash was computed independently from the specification's stages.
    #[test]
    fn legacy_protection_hash_is_verified() {
        use super::PasswordCheck::*;
        let legacy = |sid: &str, hash: &str, spin: &str| -> Vec<(String, String)> {
            vec![
                ("w:cryptProviderType".into(), "rsaFull".into()),
                ("w:cryptAlgorithmClass".into(), "hash".into()),
                ("w:cryptAlgorithmType".into(), "typeAny".into()),
                ("w:cryptAlgorithmSid".into(), sid.into()),
                ("w:cryptSpinCount".into(), spin.into()),
                ("w:hash".into(), hash.into()),
                ("w:salt".into(), "AAECAwQFBgcICQoLDA0ODw==".into()),
            ]
        };
        let attrs = legacy("4", KNOWN_LEGACY_SHA1, "100000");
        assert_eq!(super::check_protection_password(&attrs, "Example"), Correct);
        assert_eq!(super::check_protection_password(&attrs, "example"), Incorrect);
        assert_eq!(super::check_protection_password(&attrs, ""), Incorrect);
        // MD5 (and the other algorithms WordCraft lacks), hostile spin counts: refused.
        assert_eq!(super::check_protection_password(&legacy("3", KNOWN_LEGACY_SHA1, "100000"), "Example"), Unsupported);
        assert_eq!(super::check_protection_password(&legacy("4", KNOWN_LEGACY_SHA1, "4294967295"), "Example"), Unsupported);
        assert_eq!(super::check_protection_password(&legacy("4", KNOWN_LEGACY_SHA1, "-1"), "Example"), Unsupported);
        let mut ext = attrs.clone();
        ext.push(("w:algIdExt".into(), "0000000A".into()));
        assert_eq!(super::check_protection_password(&ext, "Example"), Unsupported);
        // No algorithm named: the very old bare-hash form isn't defined, so it's refused.
        let bare = vec![("w:hash".to_string(), KNOWN_LEGACY_SHA1.to_string())];
        assert_eq!(super::check_protection_password(&bare, "Example"), Unsupported);
    }

    /// SHA-1 of the salt 00 01 … 0F and "7EEDCE64" as UTF-16LE, iterated 100,000 times.
    const KNOWN_LEGACY_SHA1: &str = "pEWelKoH6SVpvMmyPAJ8bHA/g/s=";

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

    /// Standard encryption's key derivation (§2.3.4.7) for the password "Password1234_" and the
    /// salt 00 01 … 0F. The expected keys were computed outside WordCraft (Python's hashlib)
    /// following the specification's steps, so the encryptor and decryptor can't share a mistake.
    #[test]
    fn standard_key_matches_an_independent_derivation() {
        let salt: Vec<u8> = (0..16).collect();
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(&standard_key(&salt, "Password1234_", 16)), "340fc93a8e2154db8304d055df81cebc");
        assert_eq!(hex(&standard_key(&salt, "Password1234_", 32)), "340fc93a8e2154db8304d055df81cebcef5e89fb8e5eb520efdca76f3330e032");
        assert_eq!(standard_key(&salt, "Password1234_", 24).len(), 24);
        // AES-ECB against FIPS-197 Appendix C.1 and C.3.
        let pt: Vec<u8> = (0..16u8).map(|i| i * 0x11).collect();
        let k128: Vec<u8> = (0..16).collect();
        let k256: Vec<u8> = (0..32).collect();
        assert_eq!(hex(&ecb(&k128, &pt, true).unwrap()), "69c4e0d86a7b0430d8cdb78070b4c55a");
        assert_eq!(hex(&ecb(&k256, &pt, true).unwrap()), "8ea2b7ca516745bfeafc49904b496089");
        assert_eq!(ecb(&k256, &ecb(&k256, &pt, true).unwrap(), false).unwrap(), pt);
        assert!(ecb(&k128, &pt[..15], false).is_err());
    }

    /// Standard encryption (Word 2007's AES-ECB method) opens with the right password, for each
    /// AES key size, and says when the password is missing or wrong — as agile files do.
    #[test]
    fn standard_encryption_round_trips() {
        let pkg = zip_package();
        for bits in [128, 192, 256] {
            let enc = encrypt_standard_for_tests(&pkg, "pässwörd 🔑", bits).unwrap();
            assert!(is_encrypted(&enc));
            assert_eq!(decrypt(&enc, Some("pässwörd 🔑")), Ok(pkg.clone()), "AES-{bits}");
            assert_eq!(decrypt(&enc, None), Err(DocxError::PasswordRequired));
            assert_eq!(decrypt(&enc, Some("Pässwörd 🔑")), Err(DocxError::WrongPassword));
            assert_eq!(crate::read(&enc), Err(DocxError::PasswordRequired));
        }
        let enc = encrypt_standard_for_tests(&pkg, "pw", 128).unwrap();
        let doc = crate::read_with_password(&enc, Some("pw")).unwrap();
        assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Top secret plans"));
        // Sizes on and off the block boundary, and empty.
        for n in [0usize, 1, 15, 16, 17, SEGMENT + 3] {
            let data: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            assert_eq!(decrypt(&encrypt_standard_for_tests(&data, "x", 256).unwrap(), Some("x")).unwrap(), data);
        }
        assert!(encrypt_standard_for_tests(&pkg, "pw", 64).is_err());
        assert!(encrypt_standard_for_tests(&pkg, "", 128).is_err());
    }

    /// The two streams of an encrypted package.
    fn streams(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut comp = cfb::CompoundFile::open(Cursor::new(file)).unwrap();
        let mut info = Vec::new();
        comp.open_stream("/EncryptionInfo").unwrap().read_to_end(&mut info).unwrap();
        let mut package = Vec::new();
        comp.open_stream("/EncryptedPackage").unwrap().read_to_end(&mut package).unwrap();
        (info, package)
    }

    #[test]
    fn hostile_standard_encryption_info_errors_without_panicking() {
        let data = b"a small package".to_vec();
        let (info, package) = streams(&encrypt_standard_for_tests(&data, "pw", 128).unwrap());
        // Layout: version 0, flags 4, header size 8, header 12 (flags, sizeExtra, algId 20,
        // algIdHash 24, keySize 28, provider, 2 reserved, an empty CSP name), then the verifier at
        // 46: saltSize, salt 50, encrypted verifier 66, verifierHashSize 82, its hash 86..118.
        assert_eq!(info.len(), 118);
        let put = |at: usize, v: u32| {
            let mut i = info.clone();
            i[at..at + 4].copy_from_slice(&v.to_le_bytes());
            i
        };
        let mut cases: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for n in 0..info.len() {
            cases.push((info[..n].to_vec(), package.clone()));
        }
        for v in [0, 4, 31, 0x1000, u32::MAX] {
            cases.push((put(8, v), package.clone()));
        }
        // No AES, no CryptoAPI, an external (extensible) provider.
        for v in [F_CRYPTO_API, F_AES, STANDARD_FLAGS | F_EXTERNAL, 0] {
            cases.push((put(4, v), package.clone()));
        }
        // RC4, MD5, key sizes that don't fit the algorithm.
        cases.push((put(20, 0x6801), package.clone()));
        cases.push((put(24, 0x8003), package.clone()));
        for v in [0, 64, 192, 256, u32::MAX] {
            cases.push((put(28, v), package.clone()));
        }
        for v in [0, 15, 17, u32::MAX] {
            cases.push((put(46, v), package.clone()));
        }
        for v in [0, 16, 32, u32::MAX] {
            cases.push((put(82, v), package.clone()));
        }
        // The package: truncated, an absurd StreamSize, a StreamSize past the data, data that
        // isn't whole blocks.
        for n in [0, 4, 8, 9, package.len() - 1] {
            cases.push((info.clone(), package[..n].to_vec()));
        }
        for size in [u64::MAX, 1 << 40, package.len() as u64] {
            let mut p = package.clone();
            p[..8].copy_from_slice(&size.to_le_bytes());
            cases.push((info.clone(), p));
        }
        for (i, p) in cases {
            let file = cfb_with(&i, &p);
            assert!(decrypt(&file, Some("pw")).is_err());
            let _ = crate::read_with_password(&file, Some("pw"));
            assert!(crate::read(&file).is_err());
        }
        // Version 2.2 and 4.2 are Standard too; trailing bytes after the last block are ignored.
        for v in [2u8, 4] {
            let mut i = info.clone();
            i[0] = v;
            assert_eq!(decrypt(&cfb_with(&i, &package), Some("pw")).unwrap(), data);
        }
        let mut p = package.clone();
        p.extend_from_slice(&[1, 2, 3]);
        assert_eq!(decrypt(&cfb_with(&info, &p), Some("pw")).unwrap(), data);
        // AlgID and AlgIDHash 0 mean "from the flags": AES-128 and SHA-1.
        assert_eq!(decrypt(&cfb_with(&put(20, 0), &package), Some("pw")).unwrap(), data);
        assert_eq!(decrypt(&cfb_with(&put(24, 0), &package), Some("pw")).unwrap(), data);
    }
}

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
    let Some(password) = password else { return Err(DocxError::PasswordRequired) };
    // Standard (Office 2007) shares the container and the segmentation with agile, but its
    // `EncryptionInfo` is binary, its key derivation is a fixed SHA-1 loop, its cipher is ECB
    // and it carries no integrity value.
    if matches!(version(&info)?, (2..=4, 2)) {
        return standard_decrypt(&info, &package, password);
    }
    let info = parse_info(&info)?;
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

/// The `EncryptionInfo` version pair (§2.3.4.2).
fn version(info: &[u8]) -> Result<(u16, u16), DocxError> {
    let Some([a, b, c, d]) = info.get(..4) else { return Err(bad("EncryptionInfo is truncated")) };
    Ok((u16::from_le_bytes([*a, *b]), u16::from_le_bytes([*c, *d])))
}

/// The rounds Standard encryption iterates the password hash; fixed by the specification
/// (§2.3.4.7), unlike agile's per-file `spinCount`.
const STANDARD_SPIN: u32 = 50_000;

/// What Standard encryption needs from its `EncryptionInfo`: the key length, the salt and the
/// two verifier blocks (§2.3.4.5, §2.3.4.7).
#[derive(Debug)]
struct Standard {
    key_bytes: usize,
    salt: Vec<u8>,
    verifier: Vec<u8>,
    verifier_hash: Vec<u8>,
}

/// Parse the binary `EncryptionInfo` of Standard encryption (§2.3.4.5). The stream is
/// `EncryptionVersionInfo`, a copy of the header's `Flags`, the header's size, the header itself
/// and then the verifier — the header is not read at the offset the copy of `Flags` sits at.
fn parse_standard(info: &[u8]) -> Result<Standard, DocxError> {
    let word = |at: usize| -> Result<u32, DocxError> {
        info.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes).ok_or_else(|| bad("EncryptionInfo is truncated"))
    };
    let header_size = usize::try_from(word(8)?).map_err(|_| bad("EncryptionInfo is truncated"))?;
    let header = info.get(12..12 + header_size).ok_or_else(|| bad("EncryptionInfo is truncated"))?;
    let head = |at: usize| -> Result<u32, DocxError> {
        header
            .get(at..at + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| bad("the encryption header is truncated"))
    };
    // AlgID names the cipher; only AES is implemented, and it fixes the key length.
    let key_bytes = match head(8)? {
        0x660e => 16,
        0x660f => 24,
        0x6610 => 32,
        id => return Err(bad(format!("this file uses cipher {id:#x}, which WordCraft can't open"))),
    };
    if head(12)? != 0x8004 {
        return Err(bad("this file's password hash isn't SHA-1, which WordCraft can't open"));
    }
    // The verifier follows the header: salt size, salt, the encrypted verifier, the verifier
    // hash's length, and the hash itself in as many cipher blocks as AES needs (two, 32 bytes).
    let at = 12 + header_size;
    let word_at = |i: usize| -> Result<u32, DocxError> {
        info.get(at + i..at + i + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| bad("the password verifier is truncated"))
    };
    let salt_size = usize::try_from(word_at(0)?).map_err(|_| bad("the salt size is absurd"))?;
    let salt = info.get(at + 4..at + 4 + salt_size).ok_or_else(|| bad("the salt is truncated"))?.to_vec();
    let v = at + 4 + salt_size;
    let verifier = info.get(v..v + BLOCK).ok_or_else(|| bad("the password verifier is truncated"))?.to_vec();
    let hash_at = v + BLOCK + 4;
    let verifier_hash = info.get(hash_at..hash_at + BLOCK * 2).ok_or_else(|| bad("the verifier hash is truncated"))?.to_vec();
    Ok(Standard { key_bytes, salt, verifier, verifier_hash })
}

/// The key of Standard encryption (§2.3.4.7): the password hash iterated `STANDARD_SPIN` times,
/// then the PKCS #5 pair over a 64-byte buffer of 0x36 and 0x5C xored with the hash.
fn standard_key(password: &str, salt: &[u8], key_bytes: usize) -> Zeroizing<Vec<u8>> {
    let hn = iterate(Hash::Sha1, salt, password, STANDARD_SPIN);
    let final_hash = derive_key(Hash::Sha1, &hn, &[0, 0, 0, 0], Hash::Sha1.size());
    let x1 = fill_and_xor(&final_hash, 0x36);
    let x2 = fill_and_xor(&final_hash, 0x5c);
    let mut key = Zeroizing::new(Vec::with_capacity(x1.len() + x2.len()));
    key.extend_from_slice(&x1);
    key.extend_from_slice(&x2);
    key.truncate(key_bytes);
    key
}

/// X1 and X2 of PKCS #5: SHA-1 of a 64-byte buffer of `fill`, xored with `hash`.
fn fill_and_xor(hash: &[u8], fill: u8) -> Vec<u8> {
    let mut buf = [fill; 64];
    for (b, h) in buf.iter_mut().zip(hash) {
        *b ^= h;
    }
    Hash::Sha1.hash(&[&buf])
}

/// AES-ECB without padding: `data` must be whole blocks. Standard encryption has no per-segment
/// initialization vector (§2.3.4.9), every block standing on its own.
fn ecb_decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>, DocxError> {
    let aes = Aes::new(key)?;
    if !data.len().is_multiple_of(BLOCK) {
        return Err(bad("the encrypted package isn't a whole number of blocks"));
    }
    let (blocks, _) = data.as_chunks::<BLOCK>();
    let mut out = Vec::with_capacity(data.len());
    for block in blocks {
        let mut b: aes::Block = (*block).into();
        aes.decrypt(&mut b);
        out.extend_from_slice(&b);
    }
    Ok(out)
}

/// Decrypt a Standard-encrypted package (§2.3.4.9): the verifier tells a wrong password apart,
/// then the segments come off in ECB.
fn standard_decrypt(info: &[u8], package: &[u8], password: &str) -> Result<Vec<u8>, DocxError> {
    let s = parse_standard(info)?;
    let key = standard_key(password, &s.salt, s.key_bytes);
    let verifier = Zeroizing::new(ecb_decrypt(&key, &s.verifier)?);
    let hash = Zeroizing::new(ecb_decrypt(&key, &s.verifier_hash)?);
    let want = Hash::Sha1.hash(&[&verifier]);
    let got = hash.get(..want.len()).ok_or_else(|| bad("the verifier hash is too short"))?;
    if !same(&want, got) {
        return Err(DocxError::WrongPassword);
    }
    let size = package
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| bad("the encrypted package is truncated"))?;
    let data = package.get(8..).unwrap_or(&[]);
    let size = usize::try_from(size).ok().filter(|s| *s <= data.len()).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let needed = size.div_ceil(BLOCK).saturating_mul(BLOCK);
    let data = data.get(..needed).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let mut out = ecb_decrypt(&key, data)?;
    out.truncate(size);
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// The key `protected_passtika.xlsx` (Apache POI test data, Apache-2.0) derives with the
    /// password "tika". Computed from [MS-OFFCRYPTO] 2.3.4.7 independently of this code: the
    /// file decrypts to a valid package under it and nothing else opens that file.
    const KNOWN_STANDARD_SALT: [u8; 16] = [0x19, 0xd0, 0xc7, 0xf3, 0x95, 0x28, 0x78, 0x43, 0x6a, 0x49, 0x49, 0xd7, 0xf0, 0xa2, 0xcf, 0x1e];
    const KNOWN_STANDARD_KEY: [u8; 16] = [0x8d, 0x4d, 0x00, 0x14, 0x3a, 0x5f, 0x2e, 0x49, 0x18, 0xaf, 0x7e, 0x6e, 0x74, 0x10, 0xd4, 0x6c];

    #[test]
    fn standard_key_matches_the_known_package() {
        let key = super::standard_key("tika", &KNOWN_STANDARD_SALT, 16);
        assert_eq!(&*key, &KNOWN_STANDARD_KEY);
        // A different password, salt or length gives a different key.
        assert_ne!(super::standard_key("tika2", &KNOWN_STANDARD_SALT, 16).as_slice(), &KNOWN_STANDARD_KEY);
        assert_ne!(super::standard_key("tika", &[0; 16], 16).as_slice(), &KNOWN_STANDARD_KEY);
        // The key is exactly the length asked for, for each AES width.
        assert_eq!(super::standard_key("tika", &KNOWN_STANDARD_SALT, 24).len(), 24);
        assert_eq!(super::standard_key("tika", &KNOWN_STANDARD_SALT, 32).len(), 32);
    }

    /// A real `EncryptionInfo` from that package, header included: `protected_passtika.xlsx`
    /// carries a 164-byte header, so reading the fields at the header's own offset would read
    /// the wrapper's fields instead and land on nonsense.
    fn known_standard_info() -> Vec<u8> {
        let mut info = Vec::new();
        info.extend_from_slice(&4u16.to_le_bytes());
        info.extend_from_slice(&2u16.to_le_bytes());
        info.extend_from_slice(&0x24u32.to_le_bytes());
        info.extend_from_slice(&164u32.to_le_bytes());
        info.extend_from_slice(&0x24u32.to_le_bytes());
        info.extend_from_slice(&0u32.to_le_bytes());
        info.extend_from_slice(&0x660eu32.to_le_bytes());
        info.extend_from_slice(&0x8004u32.to_le_bytes());
        info.extend_from_slice(&128u32.to_le_bytes());
        info.extend_from_slice(&0x18u32.to_le_bytes());
        info.extend_from_slice(&[0u8; 8]);
        info.extend_from_slice(&[0u8; 132]);
        info.extend_from_slice(&16u32.to_le_bytes());
        info.extend_from_slice(&KNOWN_STANDARD_SALT);
        info.extend_from_slice(&[0u8; 16]);
        info.extend_from_slice(&20u32.to_le_bytes());
        info.extend_from_slice(&[0u8; 32]);
        info
    }

    #[test]
    fn standard_info_is_read_past_the_wrapper() {
        let s = super::parse_standard(&known_standard_info()).unwrap();
        assert_eq!(s.key_bytes, 16);
        assert_eq!(s.salt, KNOWN_STANDARD_SALT);
        assert_eq!(s.verifier.len(), 16);
        assert_eq!(s.verifier_hash.len(), 32);
    }

    /// Every truncation of a real `EncryptionInfo` must be refused, never panic: the lengths in
    /// it come from the file and are as hostile as any other input.
    #[test]
    fn a_truncated_standard_info_is_refused() {
        let info = known_standard_info();
        for n in 0..info.len() {
            let _ = super::parse_standard(&info[..n]);
        }
        // Absurd sizes and a header that claims more than the stream holds.
        let mut huge = known_standard_info();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(super::parse_standard(&huge).is_err());
        let mut big_salt = known_standard_info();
        big_salt[12 + 164..12 + 168].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        assert!(super::parse_standard(&big_salt).is_err());
    }

    /// Only AES is implemented: RC4 and any other cipher is named in the error, not misread.
    #[test]
    fn an_unsupported_cipher_is_named() {
        let mut info = known_standard_info();
        info[12 + 8..12 + 12].copy_from_slice(&0x6801u32.to_le_bytes());
        let err = super::parse_standard(&info).unwrap_err().to_string();
        assert!(err.contains("0x6801"), "{err}");
        // A password hash that isn't SHA-1 is refused too.
        let mut sha256 = known_standard_info();
        sha256[12 + 12..12 + 16].copy_from_slice(&0x800cu32.to_le_bytes());
        assert!(super::parse_standard(&sha256).is_err());
    }

    /// AES-ECB without padding: whole blocks only, and what goes in comes back out.
    #[test]
    fn ecb_decrypt_round_trips_and_refuses_a_ragged_block() {
        let key = &KNOWN_STANDARD_KEY[..];
        let block: Vec<u8> = (0..16u8).collect();
        let data: Vec<u8> = block.iter().chain(&block).copied().collect();
        assert!(super::ecb_decrypt(key, &data[1..]).is_err());
        let out = super::ecb_decrypt(key, &data).unwrap();
        assert_eq!(out.len(), data.len());
        // AES-ECB has no per-block chaining: the same input block gives the same output twice.
        assert_eq!(&out[..16], &out[16..]);
        assert_ne!(&out[..16], &block[..]);
        // A different key gives a different result.
        let other = super::ecb_decrypt(&[0u8; 16], &data).unwrap();
        assert_ne!(&out[..16], &other[..16]);
    }

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
}

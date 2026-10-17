//! Password-protected Word 97-2003 files ([MS-DOC] §2.2.6, [MS-OFFCRYPTO] §2.3.5-2.3.7).
//!
//! Three schemes exist for `.doc`:
//!
//! * **XOR obfuscation** (`fObfuscated`): a 32-bit password verifier sits in `FibBase.lKey` and
//!   the streams are XORed with a 16-byte array derived from the password (method 2).
//! * **RC4** (header version 1.1): `EncryptionHeader` at the start of the Table stream; the key
//!   per 512-byte block comes from MD5.
//! * **RC4 CryptoAPI** (header version 2/3/4 . 2): like RC4 but with a SHA-1 key derivation, a
//!   key size of 40-128 bits and a variable-size header.
//!
//! In all three the WordDocument stream (past its first 68 bytes), the Table stream (past its
//! header, for RC4) and the whole Data stream are transformed. The password is checked against
//! the stored verifier before anything is decrypted, so a wrong password is reported and never
//! yields a garbled document. Everything is written from the public specifications.

use sha1::{Digest, Sha1};

use crate::DocbinError;

/// Bytes at the start of the WordDocument stream that are stored untransformed.
pub(crate) const FIB_PLAIN: usize = 68;
/// RC4 re-keys every this many bytes of a stream.
const BLOCK: usize = 512;
/// Longest password considered (the specifications' limit for the RC4 schemes).
const MAX_PASSWORD_CHARS: usize = 255;
/// XOR obfuscation uses at most this many single-byte password characters.
const MAX_XOR_PASSWORD: usize = 15;
/// `FibBase.flags`: the document is encrypted (`fEncrypted`) / obfuscated (`fObfuscated`).
pub(crate) const F_ENCRYPTED: u16 = 0x0100;
pub(crate) const F_OBFUSCATED: u16 = 0x8000;
/// `EncryptionHeaderFlags`: `fCryptoAPI`, `fExternal`, `fAES`.
const H_CRYPTO_API: u32 = 0x04;
const H_EXTERNAL: u32 = 0x10;
const H_AES: u32 = 0x20;

fn unsupported(what: &str) -> DocbinError {
    DocbinError::Encryption(what.into())
}

// ---------------------------------------------------------------------------------------------
// Primitives

/// MD5 ([RFC 1321]); the RC4 scheme is defined on it.
pub(crate) fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 * abs(sin(i + 1))), as the RFC defines it.
    let k: Vec<u32> = (0..64u32).map(|i| (f64::from(i + 1).sin().abs() * 4_294_967_296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x6745_2301u32, 0xefcd_ab89u32, 0x98ba_dcfeu32, 0x1032_5476u32);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&(data.len() as u64).wrapping_mul(8).to_le_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        let words = chunk.as_chunks::<4>().0;
        for (w, b) in m.iter_mut().zip(words) {
            *w = u32::from_le_bytes(*b);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64usize {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    let (cells, _) = out.as_chunks_mut::<4>();
    for (o, v) in cells.iter_mut().zip([a0, b0, c0, d0]) {
        *o = v.to_le_bytes();
    }
    out
}

fn sha1(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha1::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

/// The RC4 stream cipher.
pub(crate) struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub(crate) fn new(key: &[u8]) -> Rc4 {
        let mut s = [0u8; 256];
        for (n, v) in s.iter_mut().enumerate() {
            *v = n as u8;
        }
        let kl = key.len().max(1);
        let mut j = 0u8;
        for i in 0..256usize {
            let k = key.get(i % kl).copied().unwrap_or(0);
            j = j.wrapping_add(s[i]).wrapping_add(k);
            s.swap(i, usize::from(j));
        }
        Rc4 { s, i: 0, j: 0 }
    }

    fn next(&mut self) -> u8 {
        self.i = self.i.wrapping_add(1);
        self.j = self.j.wrapping_add(self.s[usize::from(self.i)]);
        self.s.swap(usize::from(self.i), usize::from(self.j));
        self.s[usize::from(self.s[usize::from(self.i)].wrapping_add(self.s[usize::from(self.j)]))]
    }

    pub(crate) fn apply(&mut self, data: &mut [u8]) {
        for b in data {
            *b ^= self.next();
        }
    }
}

fn utf16le(password: &str) -> Vec<u8> {
    password.encode_utf16().take(MAX_PASSWORD_CHARS).flat_map(u16::to_le_bytes).collect()
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

// ---------------------------------------------------------------------------------------------
// XOR obfuscation (method 1 key/verifier pieces, method 2 array and transform)

const PAD_ARRAY: [u8; 15] = [0xBB, 0xFF, 0xFF, 0xBA, 0xFF, 0xFF, 0xB9, 0x80, 0x00, 0xBE, 0x0F, 0x00, 0xBF, 0x0F, 0x00];
const INITIAL_CODE: [u16; 15] =
    [0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F, 0x84F9, 0x280C, 0xA96A, 0x4EC3];
const XOR_MATRIX: [u16; 105] = [
    0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09, 0x7B61, 0xF6C2, 0xFDA5, 0xEB6B, 0xC6F7, 0x9DCF, 0x2BBF, 0x4563, 0x8AC6, 0x05AD, 0x0B5A,
    0x16B4, 0x2D68, 0x5AD0, 0x0375, 0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40, 0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D, 0xAA7A, 0x44D5, 0x6F45,
    0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A, 0xEB23, 0xC667, 0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9, 0x47D3, 0x8FA6, 0x0F6D, 0x1EDA, 0x3DB4,
    0x7B68, 0xF6D0, 0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC, 0x45A0, 0x8B40, 0x06A1, 0x0D42, 0x1A84, 0x3508, 0x6A10, 0xAA51, 0x4483,
    0x8906, 0x022D, 0x045A, 0x08B4, 0x1168, 0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C, 0x3730, 0x6E60, 0xDCC0, 0xA9A1, 0x4363, 0x86C6,
    0x1DAD, 0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC, 0x1021, 0x2042, 0x4084, 0x8108, 0x1231, 0x2462, 0x48C4,
];

/// `CreateXorKey_Method1`. `pw` is 1 to 15 bytes.
fn xor_key(pw: &[u8]) -> u16 {
    let mut key = INITIAL_CODE.get(pw.len().wrapping_sub(1)).copied().unwrap_or(0);
    let mut current = 0x68i32;
    for &ch in pw.iter().rev() {
        let mut ch = ch;
        for _ in 0..7 {
            if ch & 0x40 != 0
                && let Some(m) = usize::try_from(current).ok().and_then(|c| XOR_MATRIX.get(c))
            {
                key ^= m;
            }
            ch <<= 1;
            current -= 1;
        }
    }
    key
}

/// `CreatePasswordVerifier_Method1`.
fn verifier_method1(pw: &[u8]) -> u16 {
    let mut v = 0u16;
    for &b in pw.iter().rev().chain(std::iter::once(&(pw.len() as u8))) {
        let i1 = u16::from(v & 0x4000 != 0);
        let i2 = v.wrapping_mul(2) & 0x7FFF;
        v = (i1 | i2) ^ u16::from(b);
    }
    v ^ 0xCE4B
}

/// `CreatePasswordVerifier_Method2`: the 32-bit value stored in `FibBase.lKey`.
pub(crate) fn xor_verifier(pw: &[u8]) -> u32 {
    (u32::from(xor_key(pw)) << 16) | u32::from(verifier_method1(pw))
}

/// `CreateXorArray_Method2`.
pub(crate) fn xor_array(pw: &[u8]) -> [u8; 16] {
    let key = (xor_verifier(pw) >> 16) as u16;
    let (high, low) = ((key >> 8) as u8, key as u8);
    let mut arr = [0u8; 16];
    for (i, slot) in arr.iter_mut().enumerate() {
        *slot = pw.get(i).copied().unwrap_or_else(|| PAD_ARRAY.get(i.saturating_sub(pw.len())).copied().unwrap_or(0));
    }
    let (pairs, _) = arr.as_chunks_mut::<2>();
    for pair in pairs {
        pair[0] = (pair[0] ^ low).rotate_right(1);
        pair[1] = (pair[1] ^ high).rotate_right(1);
    }
    arr
}

/// The single-byte forms of a password the XOR scheme tries: the system-codepage form (Latin-1
/// here) and the "low byte, or high byte when the low one is zero" form. Each is cut to 15.
fn xor_candidates(password: &str) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let latin: Option<Vec<u8>> = password.chars().take(MAX_XOR_PASSWORD).map(|c| u8::try_from(u32::from(c)).ok()).collect();
    if let Some(l) = latin {
        out.push(l);
    }
    let folded: Vec<u8> = password.encode_utf16().take(MAX_XOR_PASSWORD).map(|u| if u & 0xFF == 0 { (u >> 8) as u8 } else { u as u8 }).collect();
    if !out.contains(&folded) {
        out.push(folded);
    }
    out.retain(|p| !p.is_empty());
    out
}

// ---------------------------------------------------------------------------------------------
// The cipher

/// How the streams of one file are transformed, once the password has been verified.
pub(crate) enum Cipher {
    Xor { array: [u8; 16] },
    Rc4 { base: Vec<u8>, crypto_api: bool, key_len: usize, table_skip: usize },
}

impl Cipher {
    /// Check `password` against the file's verifier and set up the cipher. `flags` and `lkey` come
    /// from the FIB base, `table` is the (still encrypted) Table stream.
    pub(crate) fn open(flags: u16, lkey: u32, table: &[u8], password: &str) -> Result<Cipher, DocbinError> {
        if flags & F_OBFUSCATED != 0 {
            for pw in xor_candidates(password) {
                if xor_verifier(&pw) == lkey {
                    return Ok(Cipher::Xor { array: xor_array(&pw) });
                }
            }
            return Err(DocbinError::WrongPassword);
        }
        let short = || DocbinError::Malformed("the encryption header is truncated".into());
        let major = u16_at(table, 0).ok_or_else(short)?;
        let minor = u16_at(table, 2).ok_or_else(short)?;
        match (major, minor) {
            (1, 1) => Self::open_rc4(table, password),
            (2..=4, 2) => Self::open_crypto_api(table, password),
            _ => Err(unsupported(&format!("unknown encryption version {major}.{minor}"))),
        }
    }

    fn open_rc4(table: &[u8], password: &str) -> Result<Cipher, DocbinError> {
        let short = || DocbinError::Malformed("the encryption header is truncated".into());
        let salt = table.get(4..20).ok_or_else(short)?;
        let enc_verifier = table.get(20..36).ok_or_else(short)?;
        let enc_hash = table.get(36..52).ok_or_else(short)?;
        let h0 = md5(&utf16le(password));
        let mut buf = Vec::with_capacity(336);
        for _ in 0..16 {
            buf.extend_from_slice(&h0[..5]);
            buf.extend_from_slice(salt);
        }
        let base = md5(&buf)[..5].to_vec();
        let cipher = Cipher::Rc4 { base, crypto_api: false, key_len: 16, table_skip: 52 };
        let mut rc4 = Rc4::new(&cipher.block_key(0));
        let mut verifier = [0u8; 16];
        verifier.copy_from_slice(enc_verifier);
        let mut hash = [0u8; 16];
        hash.copy_from_slice(enc_hash);
        rc4.apply(&mut verifier);
        rc4.apply(&mut hash);
        if md5(&verifier) != hash {
            return Err(DocbinError::WrongPassword);
        }
        Ok(cipher)
    }

    fn open_crypto_api(table: &[u8], password: &str) -> Result<Cipher, DocbinError> {
        let short = || DocbinError::Malformed("the encryption header is truncated".into());
        let hflags = u32_at(table, 4).ok_or_else(short)?;
        if hflags & H_EXTERNAL != 0 {
            return Err(unsupported("the document uses an external encryption provider"));
        }
        // [MS-OFFCRYPTO] defines AES only for the encrypted packages of Word 2007 and later
        // (which open through the .docx reader whatever their extension); a Word 97-2003 file
        // may only use XOR, RC4 or RC4 CryptoAPI, so there is no public scheme to follow here.
        if hflags & H_AES != 0 {
            return Err(unsupported(
                "this Word 97-2003 file claims AES encryption, which is only defined for the newer encrypted-package format; it can't be opened",
            ));
        }
        if hflags & H_CRYPTO_API == 0 {
            return Err(unsupported("unknown encryption header"));
        }
        let hsize = usize::try_from(u32_at(table, 8).ok_or_else(short)?).unwrap_or(usize::MAX);
        // EncryptionHeader: flags, sizeExtra, algId, algIdHash, keySize, providerType, 2 reserved
        // words and the CSP name; it starts at byte 12.
        if hsize < 32 || hsize > table.len() {
            return Err(DocbinError::Malformed("the encryption header has a bad size".into()));
        }
        let alg_id = u32_at(table, 20).ok_or_else(short)?;
        let alg_hash = u32_at(table, 24).ok_or_else(short)?;
        if !matches!(alg_id, 0 | 0x6801) {
            return Err(unsupported(&format!("encryption algorithm 0x{alg_id:X}")));
        }
        if !matches!(alg_hash, 0 | 0x8004) {
            return Err(unsupported(&format!("hash algorithm 0x{alg_hash:X}")));
        }
        let mut bits = u32_at(table, 28).ok_or_else(short)?;
        if bits == 0 {
            bits = 40;
        }
        if !(40..=128).contains(&bits) || bits % 8 != 0 {
            return Err(unsupported(&format!("a {bits}-bit key")));
        }
        let key_len = (bits / 8) as usize;
        // EncryptionVerifier: saltSize, salt[16], encryptedVerifier[16], verifierHashSize,
        // encryptedVerifierHash[20].
        let v = 12 + hsize;
        if u32_at(table, v) != Some(16) {
            return Err(DocbinError::Malformed("the encryption salt has a bad size".into()));
        }
        let salt = table.get(v + 4..v + 20).ok_or_else(short)?;
        let enc_verifier = table.get(v + 20..v + 36).ok_or_else(short)?;
        if u32_at(table, v + 36) != Some(20) {
            return Err(unsupported("a verifier hash that is not SHA-1"));
        }
        let enc_hash = table.get(v + 40..v + 60).ok_or_else(short)?;
        let mut pw = utf16le(password);
        let base = sha1(&[salt, &pw]);
        pw.fill(0);
        let cipher = Cipher::Rc4 { base, crypto_api: true, key_len, table_skip: v + 60 };
        let mut rc4 = Rc4::new(&cipher.block_key(0));
        let mut verifier = enc_verifier.to_vec();
        let mut hash = enc_hash.to_vec();
        rc4.apply(&mut verifier);
        rc4.apply(&mut hash);
        if sha1(&[&verifier]) != hash {
            return Err(DocbinError::WrongPassword);
        }
        Ok(cipher)
    }

    /// The RC4 key of stream block `block`.
    fn block_key(&self, block: u32) -> Vec<u8> {
        match self {
            Cipher::Xor { .. } => Vec::new(),
            Cipher::Rc4 { base, crypto_api: false, .. } => md5(&[base.as_slice(), &block.to_le_bytes()].concat()).to_vec(),
            Cipher::Rc4 { base, crypto_api: true, key_len, .. } => {
                let mut k = sha1(&[base, &block.to_le_bytes()]);
                k.truncate(*key_len);
                // A 40-bit key is zero-padded to 128 bits.
                if *key_len == 5 {
                    k.resize(16, 0);
                }
                k
            }
        }
    }

    /// Decrypt `data` in place; its first `skip` bytes are stored untransformed.
    fn decrypt(&self, data: &mut [u8], skip: usize) {
        match self {
            Cipher::Xor { array } => {
                for (i, b) in data.iter_mut().enumerate().skip(skip) {
                    let a = array[i % 16];
                    // Zero bytes, and bytes equal to the array element, were left as they were.
                    if *b != 0 && *b != a {
                        *b ^= a;
                    }
                }
            }
            Cipher::Rc4 { .. } => {
                for (n, chunk) in data.chunks_mut(BLOCK).enumerate() {
                    let Ok(block) = u32::try_from(n) else { return };
                    let mut rc4 = Rc4::new(&self.block_key(block));
                    let start = n * BLOCK;
                    // The keystream runs over the untransformed bytes too.
                    let plain = skip.saturating_sub(start).min(chunk.len());
                    let head = chunk.get(..plain).map(<[u8]>::to_vec).unwrap_or_default();
                    rc4.apply(chunk);
                    if let Some(dst) = chunk.get_mut(..plain) {
                        dst.copy_from_slice(&head);
                    }
                }
            }
        }
    }

    pub(crate) fn decrypt_word(&self, data: &mut [u8]) {
        self.decrypt(data, FIB_PLAIN);
    }

    pub(crate) fn decrypt_table(&self, data: &mut [u8]) {
        let skip = match self {
            Cipher::Xor { .. } => 0,
            Cipher::Rc4 { table_skip, .. } => *table_skip,
        };
        self.decrypt(data, skip);
    }

    pub(crate) fn decrypt_data(&self, data: &mut [u8]) {
        self.decrypt(data, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(
            hex(&md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn rc4_matches_a_published_vector() {
        let mut d = b"Plaintext".to_vec();
        Rc4::new(b"Key").apply(&mut d);
        assert_eq!(hex(&d), "bbf316e8d940af0ad3");
    }

    #[test]
    fn xor_candidates_cut_to_fifteen_and_skip_empty() {
        assert!(xor_candidates("").is_empty());
        assert_eq!(xor_candidates("abcdefghijklmnopqrst")[0].len(), 15);
        // Not representable as single bytes: only the folded form remains.
        assert_eq!(xor_candidates("\u{4e2d}").len(), 1);
    }
}

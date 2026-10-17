//! Password-protected files. The fixtures are encrypted here with a small encryptor written from
//! [MS-DOC] §2.2.6 and [MS-OFFCRYPTO] §2.3.5-2.3.7; no file produced by Word is used.

use std::io::Read;

use sha1::{Digest, Sha1};

use super::{Piece, body_texts, cfb_file, text_doc};
use crate::DocbinError;
use crate::crypt::{F_ENCRYPTED, F_OBFUSCATED, Rc4, md5, xor_array, xor_verifier};
use crate::{is_encrypted, read, read_with_password};

const TEXT: &str = "Secret \u{20ac} plans";
/// Where the piece table's pair lives in the FIB: `fcClx` of pair 33.
const CLX_PAIR: usize = 0x9A + 33 * 8;

fn plain() -> (Vec<u8>, Vec<u8>) {
    let f = text_doc(&[Piece { compressed: false, text: format!("{TEXT}\r") }], &[], 0);
    let mut comp = cfb::CompoundFile::open(std::io::Cursor::new(f)).unwrap();
    let mut get = |n: &str| {
        let mut v = Vec::new();
        comp.open_stream(format!("/{n}")).unwrap().read_to_end(&mut v).unwrap();
        v
    };
    (get("WordDocument"), get("0Table"))
}

fn rewrap(word: &[u8], table: &[u8]) -> Vec<u8> {
    cfb_file(&[("WordDocument", word), ("0Table", table)])
}

fn put16(v: &mut [u8], at: usize, x: u16) {
    v[at..at + 2].copy_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn sha1(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha1::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

/// Encrypt (= RC4 is symmetric) `data` in 512-byte blocks; the first `skip` bytes stay as they are
/// but the keystream still runs over them.
fn rc4_stream(data: &mut [u8], skip: usize, key: &dyn Fn(u32) -> Vec<u8>) {
    for (n, chunk) in data.chunks_mut(512).enumerate() {
        let orig = chunk.to_vec();
        Rc4::new(&key(n as u32)).apply(chunk);
        let keep = skip.saturating_sub(n * 512).min(chunk.len());
        chunk[..keep].copy_from_slice(&orig[..keep]);
    }
}

/// Put `header` in front of the table and set the FIB's encryption fields.
fn assemble(mut word: Vec<u8>, table: &[u8], header: &[u8], flags: u16, lkey: u32) -> (Vec<u8>, Vec<u8>) {
    let fl = u16::from_le_bytes([word[0x0A], word[0x0B]]) | flags;
    put16(&mut word, 0x0A, fl);
    put32(&mut word, 0x0E, lkey);
    let fc = u32::from_le_bytes(word[CLX_PAIR..CLX_PAIR + 4].try_into().unwrap());
    put32(&mut word, CLX_PAIR, fc + header.len() as u32);
    ([word].concat(), [header, table].concat())
}

fn xor_file(pw: &[u8]) -> Vec<u8> {
    let (word, table) = plain();
    let (mut word, mut table) = assemble(word, &table, &[], F_ENCRYPTED | F_OBFUSCATED, xor_verifier(pw));
    let arr = xor_array(pw);
    for (data, skip) in [(&mut word, 68), (&mut table, 0)] {
        for (i, b) in data.iter_mut().enumerate().skip(skip) {
            if *b != 0 && *b ^ arr[i % 16] != 0 {
                *b ^= arr[i % 16];
            }
        }
    }
    rewrap(&word, &table)
}

fn rc4_header(pw: &str) -> (Vec<u8>, Box<dyn Fn(u32) -> Vec<u8>>) {
    let salt = [7u8; 16];
    let h0 = md5(&utf16(pw));
    let h1 = md5(&(0..16).flat_map(|_| [&h0[..5], &salt[..]].concat()).collect::<Vec<u8>>());
    let trunc = h1[..5].to_vec();
    let key: Box<dyn Fn(u32) -> Vec<u8>> = Box::new(move |b| md5(&[trunc.as_slice(), &b.to_le_bytes()].concat()).to_vec());
    let verifier = *b"0123456789abcdef";
    let mut enc = [verifier.to_vec(), md5(&verifier).to_vec()].concat();
    Rc4::new(&key(0)).apply(&mut enc);
    ([&[1, 0, 1, 0][..], &salt, &enc].concat(), key)
}

fn capi_header(pw: &str, bits: u32, hflags: u32) -> (Vec<u8>, Box<dyn Fn(u32) -> Vec<u8>>) {
    let salt = [9u8; 16];
    let h0 = sha1(&[&salt, &utf16(pw)]);
    let n = (bits / 8) as usize;
    let key: Box<dyn Fn(u32) -> Vec<u8>> = Box::new(move |b| {
        let mut k = sha1(&[&h0, &b.to_le_bytes()])[..n].to_vec();
        if n == 5 {
            k.resize(16, 0);
        }
        k
    });
    let verifier = *b"fedcba9876543210";
    let mut enc = [verifier.to_vec(), sha1(&[&verifier])].concat();
    Rc4::new(&key(0)).apply(&mut enc);
    let mut body = Vec::new();
    for x in [hflags, 0, 0x6801, 0x8004, bits, 1, 0, 0] {
        body.extend_from_slice(&x.to_le_bytes());
    }
    body.extend_from_slice(&utf16("Test RC4 provider\0"));
    let mut h = vec![3, 0, 2, 0];
    h.extend_from_slice(&hflags.to_le_bytes());
    h.extend_from_slice(&(body.len() as u32).to_le_bytes());
    h.extend_from_slice(&body);
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&salt);
    h.extend_from_slice(&enc[..16]);
    h.extend_from_slice(&20u32.to_le_bytes());
    h.extend_from_slice(&enc[16..]);
    (h, key)
}

fn rc4_file_with(header: Vec<u8>, key: &dyn Fn(u32) -> Vec<u8>) -> Vec<u8> {
    let (word, table) = plain();
    let (mut word, mut table) = assemble(word, &table, &header, F_ENCRYPTED, header.len() as u32);
    rc4_stream(&mut word, 68, key);
    rc4_stream(&mut table, header.len(), key);
    rewrap(&word, &table)
}

fn rc4_file(pw: &str) -> Vec<u8> {
    let (h, k) = rc4_header(pw);
    rc4_file_with(h, &*k)
}

fn capi_file(pw: &str, bits: u32) -> Vec<u8> {
    let (h, k) = capi_header(pw, bits, 0x04);
    rc4_file_with(h, &*k)
}

fn check(f: &[u8], pw: &str, wrong: &str) {
    assert!(is_encrypted(f));
    assert_eq!(read(f).unwrap_err(), DocbinError::PasswordRequired);
    assert_eq!(read_with_password(f, Some(wrong)).unwrap_err(), DocbinError::WrongPassword);
    assert_eq!(read_with_password(f, Some("")).unwrap_err(), DocbinError::WrongPassword);
    let doc = read_with_password(f, Some(pw)).unwrap();
    assert_eq!(body_texts(&doc), [TEXT]);
}

#[test]
fn xor_obfuscation_round_trip() {
    check(&xor_file(b"hunter2"), "hunter2", "hunter3");
    // Longer than 15 characters: only the first 15 count, as in the format.
    let long = xor_file(b"abcdefghijklmno");
    assert_eq!(body_texts(&read_with_password(&long, Some("abcdefghijklmnopqrst")).unwrap()), [TEXT]);
    // A non-ASCII (Latin-1) password.
    check(&xor_file(&[b'p', 0xE4, b's', b's']), "p\u{e4}ss", "pass");
}

#[test]
fn rc4_round_trip() {
    check(&rc4_file("correct horse"), "correct horse", "Correct horse");
    check(&rc4_file("p\u{e4}ssw\u{f6}rd \u{1f511}"), "p\u{e4}ssw\u{f6}rd \u{1f511}", "password");
}

#[test]
fn rc4_crypto_api_round_trip() {
    check(&capi_file("battery staple", 128), "battery staple", "battery stapl");
    // 40-bit keys are zero-padded to 128 bits.
    check(&capi_file("battery staple", 40), "battery staple", "x");
}

#[test]
fn unencrypted_files_ignore_the_password() {
    let f = text_doc(&[Piece { compressed: false, text: "Plain\r".into() }], &[], 0);
    assert!(!is_encrypted(&f));
    assert_eq!(body_texts(&read_with_password(&f, Some("whatever")).unwrap()), ["Plain"]);
}

#[test]
fn a_tampered_verifier_is_a_wrong_password_never_a_garbled_document() {
    let (h, k) = rc4_header("pw");
    let mut bad = h.clone();
    bad[40] ^= 1; // inside the encrypted verifier hash
    assert_eq!(read_with_password(&rc4_file_with(bad, &*k), Some("pw")).unwrap_err(), DocbinError::WrongPassword);
}

#[test]
fn unsupported_variants_say_so() {
    // AES and external providers, an unknown header version.
    for hflags in [0x04 | 0x20, 0x10] {
        let (h, k) = capi_header("pw", 128, hflags);
        let e = read_with_password(&rc4_file_with(h, &*k), Some("pw")).unwrap_err();
        assert!(matches!(e, DocbinError::Encryption(_)), "{e:?}");
        if hflags & 0x20 != 0 {
            assert!(e.to_string().contains("only defined for the newer encrypted-package format"), "{e}");
        }
    }
    let (mut h, k) = rc4_header("pw");
    h[0] = 9;
    assert!(matches!(read_with_password(&rc4_file_with(h, &*k), Some("pw")), Err(DocbinError::Encryption(_))));
}

#[test]
fn damaged_headers_are_errors_never_panics() {
    for f in [rc4_header("pw"), capi_header("pw", 128, 0x04)] {
        let (h, k) = f;
        let full = rc4_file_with(h.clone(), &*k);
        let (word, table) = {
            let mut comp = cfb::CompoundFile::open(std::io::Cursor::new(&full)).unwrap();
            let mut get = |n: &str| {
                let mut v = Vec::new();
                comp.open_stream(format!("/{n}")).unwrap().read_to_end(&mut v).unwrap();
                v
            };
            (get("WordDocument"), get("0Table"))
        };
        // Truncated everywhere in and around the header: an error, not a panic.
        for n in 0..h.len() + 8 {
            let r = read_with_password(&rewrap(&word, &table[..n.min(table.len())]), Some("pw"));
            assert!(r.is_err(), "length {n}");
        }
        // Every header byte garbled.
        for at in 0..h.len() {
            let mut t = table.clone();
            t[at] ^= 0xFF;
            let _ = read_with_password(&rewrap(&word, &t), Some("pw"));
        }
    }
    // A header size claiming more than the stream holds.
    let (mut h, k) = capi_header("pw", 128, 0x04);
    h[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_with_password(&rc4_file_with(h, &*k), Some("pw")).is_err());
}

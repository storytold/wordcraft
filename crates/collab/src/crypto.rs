//! End-to-end encryption (protocol 2, section 4): AES-256-GCM.
//!
//! payload = `nonce (12 random bytes) || ciphertext || tag (16 bytes)`; associated data = UTF-8 of
//! `room + "\n" + method`, so a message can't be replayed into another room or as another kind.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};

use crate::CollabError;

/// A room key: 32 bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(…)")
    }
}

impl Key {
    pub fn from_bytes(b: &[u8]) -> Result<Key, CollabError> {
        let k: [u8; 32] = b.try_into().map_err(|_| CollabError::Other(format!("a room key is 32 bytes, not {}", b.len())))?;
        Ok(Key(k))
    }
    /// From base64 (standard alphabet; URL-safe accepted too, as invitation links carry it).
    pub fn from_base64(text: &str) -> Result<Key, CollabError> {
        let t: String = text
            .trim()
            .chars()
            .map(|c| match c {
                '-' => '+',
                '_' => '/',
                c => c,
            })
            .collect();
        Key::from_bytes(&crate::b64_decode(&t)?)
    }
    /// A new random key.
    pub fn generate() -> Result<Key, CollabError> {
        let mut k = [0u8; 32];
        crate::random_bytes(&mut k)?;
        Ok(Key(k))
    }
    pub fn to_base64(&self) -> String {
        crate::b64_encode(&self.0)
    }
}

fn aad(room: &str, method: &str) -> Vec<u8> {
    let mut a = Vec::with_capacity(room.len() + method.len() + 1);
    a.extend_from_slice(room.as_bytes());
    a.push(b'\n');
    a.extend_from_slice(method.as_bytes());
    a
}

/// Encrypts `plain` for `method` (`collab/update`…) in `room`.
pub fn encrypt(key: &Key, room: &str, method: &str, plain: &[u8]) -> Result<Vec<u8>, CollabError> {
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| CollabError::Other("bad key".into()))?;
    let mut nonce = [0u8; 12];
    crate::random_bytes(&mut nonce)?;
    let aad = aad(room, method);
    let ct =
        cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: &aad }).map_err(|_| CollabError::Other("encryption failed".into()))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts a payload [`encrypt`] made. Any failure (wrong key, other room or method,
/// tampering, too short) is [`CollabError::Decrypt`].
pub fn decrypt(key: &Key, room: &str, method: &str, data: &[u8]) -> Result<Vec<u8>, CollabError> {
    if data.len() < 12 + 16 {
        return Err(CollabError::Decrypt);
    }
    let (nonce, ct) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| CollabError::Decrypt)?;
    let aad = aad(room, method);
    cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: &aad }).map_err(|_| CollabError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tampering() {
        let key = Key::generate().unwrap_or(Key([7; 32]));
        let msg = b"a yjs update";
        let sealed = encrypt(&key, "mqtt:h:r", "collab/update", msg).expect("encrypt");
        assert_eq!(sealed.len(), 12 + msg.len() + 16);
        assert_eq!(decrypt(&key, "mqtt:h:r", "collab/update", &sealed).expect("decrypt"), msg);
        // Another room, another method, another key, a flipped bit, a truncation: all refused.
        assert_eq!(decrypt(&key, "mqtt:h:other", "collab/update", &sealed), Err(CollabError::Decrypt));
        assert_eq!(decrypt(&key, "mqtt:h:r", "collab/awareness", &sealed), Err(CollabError::Decrypt));
        assert_eq!(decrypt(&Key([1; 32]), "mqtt:h:r", "collab/update", &sealed), Err(CollabError::Decrypt));
        for i in [0, 12, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert_eq!(decrypt(&key, "mqtt:h:r", "collab/update", &bad), Err(CollabError::Decrypt));
        }
        assert_eq!(decrypt(&key, "mqtt:h:r", "collab/update", &sealed[..20]), Err(CollabError::Decrypt));
        // Two encryptions of one message differ (random nonce).
        assert_ne!(encrypt(&key, "mqtt:h:r", "collab/update", msg).expect("encrypt"), sealed);
    }

    #[test]
    fn keys_from_base64() {
        let k = Key([9; 32]);
        assert_eq!(Key::from_base64(&k.to_base64()).expect("key"), k);
        let url = k.to_base64().replace('+', "-").replace('/', "_");
        assert_eq!(Key::from_base64(&url).expect("key"), k);
        assert!(Key::from_base64("c2hvcnQ=").is_err());
    }
}

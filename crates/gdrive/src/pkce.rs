//! Proof Key for Code Exchange (RFC 7636), `S256` method only.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// A code verifier and its `S256` challenge.
#[derive(Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl std::fmt::Debug for Pkce {
    // The verifier is a secret for the length of a sign-in; keep it out of logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce").field("challenge", &self.challenge).finish_non_exhaustive()
    }
}

impl Pkce {
    /// A fresh verifier from 32 random bytes (43 characters, the RFC's recommendation).
    pub fn generate() -> Result<Self> {
        Ok(Self::from_verifier(random_token(32)?))
    }

    /// The pair for a given verifier.
    pub fn from_verifier(verifier: String) -> Self {
        let challenge = challenge(&verifier);
        Self { verifier, challenge }
    }
}

/// `BASE64URL(SHA256(verifier))`, unpadded.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `n` random bytes as unpadded base64url: used for verifiers, `state` and multipart boundaries.
pub fn random_token(n: usize) -> Result<String> {
    let mut bytes = vec![0u8; n.clamp(1, 96)];
    getrandom::fill(&mut bytes).map_err(|e| Error::SignIn(format!("no system randomness: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

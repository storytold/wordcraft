//! Where the refresh token lives: the operating system's keychain (macOS Keychain, Windows
//! Credential Manager, Secret Service on Linux and the BSDs). Never a file.

use std::sync::Mutex;

use crate::{Error, Result};

pub trait TokenStore: Send {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, token: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// Keeps the token for this run only: used in tests, on the web, and where no keychain works.
#[derive(Default)]
pub struct Memory(Mutex<Option<String>>);

impl TokenStore for Memory {
    fn load(&self) -> Result<Option<String>> {
        Ok(self.0.lock().map_err(|_| Error::Keychain("lock poisoned".into()))?.clone())
    }
    fn save(&self, token: &str) -> Result<()> {
        *self.0.lock().map_err(|_| Error::Keychain("lock poisoned".into()))? = Some(token.to_owned());
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        *self.0.lock().map_err(|_| Error::Keychain("lock poisoned".into()))? = None;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
mod keychain {
    use super::{Error, Result, TokenStore};

    pub const SERVICE: &str = "WordCraft Google Drive";

    /// One entry per OS user and OAuth client: WordCraft signs in to one Google account at a
    /// time, and a token belongs to the client that got it, so another registration (or another
    /// profile with its own) never picks up, or deletes, this one's sign-in.
    pub struct Keychain {
        account: String,
    }

    impl Keychain {
        pub fn for_client(client_id: &str) -> Self {
            Self { account: super::account(client_id) }
        }
        fn entry(&self) -> Result<keyring::Entry> {
            keyring::Entry::new(SERVICE, &self.account).map_err(|e| Error::Keychain(e.to_string()))
        }
    }

    impl TokenStore for Keychain {
        fn load(&self) -> Result<Option<String>> {
            match self.entry()?.get_password() {
                Ok(t) if !t.is_empty() => Ok(Some(t)),
                Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(Error::Keychain(e.to_string())),
            }
        }
        fn save(&self, token: &str) -> Result<()> {
            self.entry()?.set_password(token).map_err(|e| Error::Keychain(e.to_string()))
        }
        fn clear(&self) -> Result<()> {
            match self.entry()?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(Error::Keychain(e.to_string())),
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub use keychain::Keychain;

/// The keychain account name for a client's refresh token.
pub fn account(client_id: &str) -> String {
    format!("refresh-token:{}", client_id.trim())
}

/// The keychain where the platform has one (an entry for `client_id`), otherwise [`Memory`].
pub fn system(client_id: &str) -> Box<dyn TokenStore> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    {
        Box::new(Keychain::for_client(client_id))
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )))]
    {
        let _ = client_id;
        Box::new(Memory::default())
    }
}

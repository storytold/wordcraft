//! Talking to cloud storage from this application.
//!
//! No backend ships here. The previous implementation called `craft_cloud_client`/
//! `collabcraft-share`, both from a personal repository (`wdunn001/collab-craft`), which is
//! exactly the structural problem a storytold maintainer flagged on sibling app PRs. This file now
//! only keeps [`CloudToken`]'s shape (it implements [`crate::collab::TokenSource`]) so
//! `main.rs`'s call site does not need to change again once a real backend lands; `from_env`
//! always returns `None`.

/// A [`crate::collab::TokenSource`] that would be backed by a cloud service. Always empty: there
/// is no service to ask.
pub struct CloudToken;

impl CloudToken {
    pub fn from_env() -> Option<CloudToken> {
        None
    }
}

impl crate::collab::TokenSource for CloudToken {
    fn join(&self) -> Option<(String, String)> {
        None
    }
}

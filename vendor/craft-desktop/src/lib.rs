#![forbid(unsafe_code)]
#![allow(clippy::type_complexity)] // DBusMenu's specified wire tuple signatures.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! Toolkit-independent snapshots; native transports are opt-in and Linux-only.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub command: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    /// Stable within its parent, independent of label/state/translation.
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub shortcut: String,
    pub action: Option<Action>,
    pub children: Vec<Item>,
    pub separator: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    /// Change whenever the active document/session is replaced. Retires stale IDs.
    pub document: u64,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Click {
    pub document: u64,
    pub action: Action,
}

pub mod appearance;
#[cfg(all(target_os = "linux", feature = "linux"))]
pub mod appmenu;
#[cfg(all(target_os = "linux", feature = "linux"))]
mod wayland;

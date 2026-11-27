//! Keyboard: typing, editing keys and command shortcuts (the user's custom keys from Customize
//! Keyboard first, then the registry's `shortcut` fields).

use egui::{Key, Modifiers};
use serde_json::json;

use crate::WordApp;
use wordcraft_engine::ImeEditOutcome;

mod canvas;
mod ime;
mod shortcuts;

pub(crate) use canvas::{page_input, window_cancel};
pub(crate) use ime::report_editor;
#[cfg(test)]
use ime::{ImeProof, MAX_IME_COMMIT_BYTES, MAX_IME_PREEDIT_BYTES, classify_ime_proof};
pub use shortcuts::{combo, key_name};

#[cfg(test)]
mod tests;

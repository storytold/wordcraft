use egui::{Key, Modifiers};
use serde_json::{Value, json};

use crate::WordApp;
use wordcraft_engine::ImeEditOutcome;

mod alt_code;
mod canvas;
mod ime;
mod shortcuts;

pub use alt_code::{AltCodeEcho, AltCodeState};
pub use canvas::canvas_events;
use ime::{ImeProof, MAX_IME_COMMIT_BYTES, MAX_IME_PREEDIT_BYTES, classify_ime_proof};
pub use shortcuts::{combo, global_shortcuts, key_name};
#[cfg(test)]
use shortcuts::{
    dispatch, is_context_menu_key, is_global_shortcut, is_paste_release, is_paste_special_release, is_shift_delete, paste_release_modifiers,
    shift_variant_params, ui_shortcut,
};

#[cfg(test)]
mod tests;

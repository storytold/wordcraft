//! Tools and the long tail: Repeat, macros, AutoCorrect, Compare, accessibility checker,
//! document inspector, restrict editing, versions, templates, Quick Parts, and more.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, NumRef};

use wordcraft_doc::{Block, Document, Paragraph, Pos, RevisionKind, StoryRef, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod actions;
mod autocorrect;
mod compare;
mod inspect;
mod specs;
mod typing;

pub use autocorrect::{AUTOCORRECT, AutoCap, AutoCorrectPrefs, lenient_autocorrect};
#[cfg(test)]
use compare::{Op, collapse_noise_matches, lcs_ops};
pub use specs::specs;
pub use typing::{INITIAL_CAPS_EXCEPTIONS, autocorrect, autocorrect_word, emphasis_autoformat, has_emphasis};
pub(crate) use typing::{is_german, is_turkic, lower_char, text_lang, upper_char};

#[cfg(test)]
mod tests;

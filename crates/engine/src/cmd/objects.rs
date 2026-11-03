//! Picture Format, Shape Format and Arrange: commands on the selected inline object.
//!
//! The selected object is the first picture/shape inside the selection (or right before the
//! caret). Picture adjustments re-encode the bitmap (originals are kept for Reset Picture).

use serde_json::{Value, json};
use wordcraft_doc::graphic::GraphicKind;
use wordcraft_doc::para::{Float, InlineObject, Wrap, WrapText};
use wordcraft_doc::props::{Rgb, TextVert, VAlign};
use wordcraft_doc::{Pos, StoryRef, TextBody};
use wordcraft_geom::Spin;

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod arrange;
mod group;
mod picture;
mod select;
mod shape;
mod specs;

pub use arrange::moves_with_text;
pub(crate) use arrange::{anchor_paragraph, nearest_line};
#[cfg(test)]
use group::ungroup;
pub use picture::{OriginalSize, PictureLook, cropped_original_size, fit_size, original_size};
#[cfg(test)]
use picture::{clear_background, grown, picture_style};
#[cfg(test)]
pub(crate) use select::all_objects;
pub(crate) use select::{edit_obj, has_shape_or_canvas};
#[cfg(test)]
use select::{has_group, holds_ink, obj_size};
pub use select::{min_size, object_selection, picked_objects, selected, selected_all};
#[cfg(test)]
use shape::can_link;
pub(crate) use shape::set_text_body;
pub use shape::text_box_target;
pub use specs::specs;

#[cfg(test)]
mod tests;

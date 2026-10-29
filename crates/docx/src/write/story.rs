//! Blocks, paragraphs, runs, objects and tables → WordprocessingML.

use wordcraft_doc::effects::ShapeEffects;
use wordcraft_doc::fill::{Fill, GradientFlip, GradientKind, PathShape};
use wordcraft_doc::para::{AltText, Anchor, Float, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, PropChange, Rgb, TextColor};
use wordcraft_doc::section::{LineNumberRestart, NoteProps, SectionProps, SectionStart};
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, RevisionKind};

use super::props::{ChangeAttrs, borders, open_change, ppr_inner, rpr, rpr_inner, tblpr, tcpr, trpr};
use super::{PartRels, Writer};
use crate::package::rt;
use crate::units::{emu, n, twips};
use crate::xml::W;

mod blocks;
mod drawing;
mod field;
mod object;
mod para;
mod section;
mod toc;

pub(super) use drawing::spin_attrs;
pub(super) use section::note_pr_inner;
pub(crate) use toc::written_toc_span;

const MAX_DEPTH: usize = 24;

enum Piece<'a> {
    Text(String),
    Tab,
    Br(Option<&'static str>),
    NbHyphen,
    SoftHyphen,
    /// A symbol character in its own font (`w:sym`, ECMA-376 §17.3.3.30).
    Sym(&'a str, char),
    /// An object and its byte offset in the paragraph.
    Obj(&'a InlineObject, usize),
}

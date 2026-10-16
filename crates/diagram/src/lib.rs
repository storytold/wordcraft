//! Editable diagrams: the draw.io XML riding inside a PNG.
//!
//! A picture exported from draw.io (`File › Export as › PNG`, *Include a copy of my diagram*)
//! is still a diagram: the PNG carries the `mxfile` XML in a text chunk, so it can be opened and
//! edited again, in place, the way a Visio object is inside Word.
//!
//! * [`png`] reads that chunk (and only reads: nothing here rewrites a picture's pixels).
//! * The editor that opens one is in the front end (`wordcraft_ui_egui::diagram`): it needs a
//!   native window, which belongs to the UI layer, not to this one.
//!
//! Nothing is re-encoded on the way in or out of a `.docx`, so the chunk survives a save. What
//! loses it is a command that rewrites the pixels (Compress Pictures, Corrections, Color…),
//! which writes a fresh PNG without it.

pub mod png;

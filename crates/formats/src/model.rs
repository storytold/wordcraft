//! A small "flow" model every text format maps to and from: paragraphs with a kind (heading,
//! quote, code…), list membership, alignment and formatted inline spans; tables of cells holding
//! blocks. [`to_doc`] and [`from_doc`] convert between it and the full [`Document`].

use std::sync::Arc;

use wordcraft_doc::numbering::{AbstractNum, Level, ListKind, Num, levels_for};
use wordcraft_doc::para::{InlineObject, OBJ, PAGE_BREAK, RubyAlign, Run, ruby_raise};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, CellProps, CharProps, NumRef, ParaProps, PropChange, RowProps, TextColor, Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Align, Block, Blocks, Document, Paragraph, Revision, RevisionKind, Rgb, Table, para_block};

mod color;
mod from_doc;
mod media;
mod to_doc;
mod types;

pub use color::parse_color;
pub use from_doc::{ListCounter, flow_paras, from_doc, from_doc_tracked, is_mono, para_kind};
pub use media::{base64_decode, base64_encode, data_uri, decode_bounded, image_px, make_img, mime_of, percent_decode, sniff_image};
pub use to_doc::{char_props, part_blocks, to_doc, to_doc_in};
pub use types::{
    CODE_STYLE, Cell, FBlock, FTable, Flow, Fmt, FmtChange, Img, Inline, Kind, ListInfo, MAX_DEPTH, MAX_FONT_NAME, MAX_LINK, MONO_FONT, Meta, Para,
    RUBY_BASE_SIZE, RevMark, Spacing, clean_text, equations_as_text, kept_font, kept_link, ruby_as_text, ruby_object,
};
pub(crate) use types::{Dropped, begin_import, dropped};

thread_local! {
    /// The kinds of [`Dropped`] already reported for the document being imported on this thread.
    /// Per thread because a document is parsed and built on one, and the parsers drop things
    /// from places that have no log of their own to hand.
    static REPORTED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_requires_hex_digits() {
        assert_eq!(percent_decode("%+A"), b"%+A".to_vec());
        assert_eq!(percent_decode("%-A"), b"%-A".to_vec());
        assert_eq!(percent_decode("%G1"), b"%G1".to_vec());
        assert_eq!(percent_decode("%41"), b"A".to_vec());
    }

    #[test]
    fn insert_covered_is_bounded_on_hostile_rowspan() {
        // Many rows each carrying the maximum rowspan would be O(rows × rowspan) covered cells;
        // the work cap must keep insertion bounded (and quick) instead of exhausting memory.
        let mut t =
            FTable { rows: (0..20_000).map(|_| vec![Cell { rowspan: 1000, ..Default::default() }]).collect(), widths: Vec::new(), borderless: false };
        t.insert_covered();
        assert!(t.rows.iter().all(|r| r.len() <= 64), "a row grew past the column cap");
    }

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let e = base64_encode(data);
            assert_eq!(base64_decode(&e).unwrap(), data, "{e}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert!(base64_decode("@@").is_none());
        assert_eq!(data_uri("data:text/plain,a%20b").unwrap(), b"a b");
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00f"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("#0080FF"), Some(Rgb(0, 128, 255)));
        assert_eq!(parse_color("#0080ff80"), Some(Rgb(0, 128, 255)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("rgb(1 2 3)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("rgb(1 2 3 / 50%)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("hsl(120deg 100% 50%)"), Some(Rgb(0, 255, 0)));
        assert_eq!(parse_color("hsla(240, 100%, 50%, 0.8)"), Some(Rgb(0, 0, 255)));
        assert_eq!(parse_color("navy"), Some(Rgb(0, 0, 128)));
        assert_eq!(parse_color("#zz"), None);
        assert_eq!(parse_color("rgb(1)"), None);
    }

    #[test]
    fn list_labels() {
        let mut c = ListCounter::default();
        let o = |level| ListInfo { ordered: true, level, start: 1 };
        assert_eq!(c.label(o(0)), "1.");
        assert_eq!(c.label(o(1)), "a.");
        assert_eq!(c.label(o(1)), "b.");
        assert_eq!(c.label(o(0)), "2.");
        assert_eq!(c.label(ListInfo { ordered: false, level: 0, start: 1 }), "•");
    }

    #[test]
    fn covered_cells_inserted() {
        let mut t = FTable {
            rows: vec![vec![Cell { rowspan: 2, ..Default::default() }, Cell::default()], vec![Cell::default()]],
            widths: vec![],
            borderless: false,
        };
        t.insert_covered();
        assert_eq!(t.rows[1].len(), 2);
        assert!(t.rows[1][0].covered);
    }

    #[test]
    fn trim_drops_a_long_run_of_blank_spans_in_linear_time() {
        // Separately formatted line breaks don't merge; each used to be removed with
        // `Vec::remove(0)`, shifting all the rest (quadratic).
        let mut p = Para::default();
        let (b, i) = (Fmt { bold: true, ..Default::default() }, Fmt { italic: true, ..Default::default() });
        for _ in 0..200_000 {
            p.push_text("\n", &b);
            p.push_text(" ", &i);
        }
        p.push_text("  x ", &Fmt::default());
        let t = std::time::Instant::now();
        p.trim();
        // A fraction of a second; `Vec::remove(0)` per span ran past 6 minutes.
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
        assert_eq!(p.inlines, vec![Inline::Text("x".into(), Fmt::default())]);
        // The first span left keeps its formatting and loses only its leading blanks.
        let mut p = Para::default();
        p.push_text(" \t", &b);
        p.push_text(" a ", &i);
        p.push_text("b ", &b);
        p.trim();
        assert_eq!(p.inlines, vec![Inline::Text("a ".into(), i), Inline::Text("b".into(), b)]);
    }
}

//! Character, paragraph, table and cell properties.
//!
//! Every field is an `Option`: `None` means "inherit" (from the style chain, then document
//! defaults). [`CharProps::overlay`] and friends apply a patch on top of a base.

use serde::{Deserialize, Serialize};

macro_rules! overlay_fields {
    ($dst:ident, $src:ident; $($f:ident),*) => {
        $( if $src.$f.is_some() { $dst.$f = $src.$f.clone(); } )*
    };
}

macro_rules! drop_same_fields {
    ($dst:ident, $base:ident; $($f:ident),*) => {
        $( if $dst.$f == $base.$f { $dst.$f = None; } )*
    };
}

mod char_props;
mod color;
mod para_props;
mod table_props;

pub use char_props::{
    CharProps, CombineBrackets, DEFAULT_FONT_SIZE, EastAsianLayout, Highlight, Ligatures, NumChange, PropChange, Underline, VertAlign,
};
pub use color::{Rgb, TextColor};
pub use para_props::{Align, Border, BorderStyle, Borders, LineSpacing, NumRef, ParaProps, TAB_POS_EPS, TabAlign, TabLeader, TabStop, TextFrame};
pub use table_props::{
    CellProps, CellSpacing, CellSpacingKind, HeightRule, RowProps, TableFloat, TableLook, TableProps, TextDirection, TextVert, VAlign, VMerge,
    anchor_ooxml, valign_from_anchor,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex() {
        assert_eq!(Rgb(255, 0, 16).hex(), "FF0010");
        assert_eq!(Rgb::parse("#FF0010"), Some(Rgb(255, 0, 16)));
        assert_eq!(Rgb::parse("zzzzzz"), None);
        assert_eq!(Rgb::parse("é1234"), None);
    }

    #[test]
    fn rgb_parse_rejects_non_hex() {
        assert_eq!(Rgb::parse("+1+2+3"), None);
        assert_eq!(Rgb::parse("-1-2-3"), None);
        assert_eq!(Rgb::parse("##ff0000"), None);
        assert_eq!(Rgb::parse("ff00 0"), None);
        assert_eq!(Rgb::parse("#ff0000"), Some(Rgb(255, 0, 0)));
        assert_eq!(Rgb::parse("ff0000"), Some(Rgb(255, 0, 0)));
    }

    #[test]
    fn overlay_keeps_unset() {
        let base = CharProps { bold: Some(true), size: Some(12.0), ..Default::default() };
        let patch = CharProps { size: Some(14.0), italic: Some(true), ..Default::default() };
        let r = base.overlaid(&patch);
        assert_eq!(r.bold, Some(true));
        assert_eq!(r.size, Some(14.0));
        assert_eq!(r.italic, Some(true));
    }

    #[test]
    fn borders_overlay_per_side() {
        let thin = Border::single(0.5);
        let thick = Border::single(2.0);
        let nil = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
        let mut b = Borders::all(thin);
        b.overlay(&Borders { top: Some(thick), ..Default::default() });
        assert_eq!(b.top, Some(thick));
        assert_eq!(b.bottom, Some(thin));
        b.overlay(&Borders::box_(nil));
        assert_eq!(b.left, Some(nil)); // explicit nil overrides that side
        assert_eq!(b.between, Some(thin)); // untouched sides survive
        b.overlay(&Borders::default());
        assert_eq!(b.left, Some(nil)); // empty patch changes nothing
    }

    #[test]
    fn ooxml_names_round_trip() {
        for u in Underline::ALL {
            assert_eq!(Underline::from_ooxml(u.ooxml()), u);
        }
        for h in Highlight::ALL {
            assert_eq!(Highlight::from_ooxml(h.ooxml()), h);
        }
    }

    /// The 16 `w14:ligatures` values of [MS-DOCX], each to its flags and back.
    #[test]
    fn ligature_names() {
        let l = |standard, contextual, historical, discretional| Ligatures { standard, contextual, historical, discretional };
        for (name, flags) in [
            ("none", l(false, false, false, false)),
            ("standard", l(true, false, false, false)),
            ("contextual", l(false, true, false, false)),
            ("historical", l(false, false, true, false)),
            ("discretional", l(false, false, false, true)),
            ("standardContextual", l(true, true, false, false)),
            ("standardHistorical", l(true, false, true, false)),
            ("contextualHistorical", l(false, true, true, false)),
            ("standardDiscretional", l(true, false, false, true)),
            ("contextualDiscretional", l(false, true, false, true)),
            ("historicalDiscretional", l(false, false, true, true)),
            ("standardContextualHistorical", l(true, true, true, false)),
            ("standardContextualDiscretional", l(true, true, false, true)),
            ("standardHistoricalDiscretional", l(true, false, true, true)),
            ("contextualHistoricalDiscretional", l(false, true, true, true)),
            ("all", l(true, true, true, true)),
        ] {
            assert_eq!(Ligatures::from_ooxml(name), Some(flags), "{name}");
            assert_eq!(flags.ooxml(), name);
        }
        assert_eq!(Ligatures::all().count(), 16);
        for bad in ["", "Standard", "contextualStandard", "standardstandard", "allx"] {
            assert_eq!(Ligatures::from_ooxml(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn text_direction_names() {
        let mut d = TextDirection::Horizontal;
        for _ in 0..3 {
            assert_eq!(TextDirection::from_ooxml(d.ooxml()), d);
            d = d.next();
        }
        assert_eq!(d, TextDirection::Horizontal, "the button cycles through three directions");
        // Strict names, East Asian vertical layouts and junk.
        assert_eq!(TextDirection::from_ooxml("rl"), TextDirection::Down);
        assert_eq!(TextDirection::from_ooxml("lr"), TextDirection::Up);
        assert_eq!(TextDirection::from_ooxml("tbRlV"), TextDirection::Down);
        assert_eq!(TextDirection::from_ooxml("tb"), TextDirection::Horizontal);
        assert_eq!(TextDirection::from_ooxml("lrTbV"), TextDirection::Horizontal);
        assert_eq!(TextDirection::from_ooxml("é?"), TextDirection::Horizontal);
    }

    /// #865: a colour reads as `format.color` takes it (`RRGGBB`, `#RRGGBB`, `auto`), while what
    /// documents and settings already hold reads and writes exactly as before.
    #[test]
    fn text_color_reads_hex_and_keeps_its_serialized_forms() {
        let read = |j: &str| serde_json::from_str::<TextColor>(j);
        let ink = TextColor::Rgb(Rgb(0x1B, 0x16, 0x11));
        assert_eq!(read(r#""1B1611""#).unwrap(), ink);
        assert_eq!(read(r##""#1b1611""##).unwrap(), ink);
        assert_eq!(read(r#""auto""#).unwrap(), TextColor::Auto);
        assert_eq!(read(r#""Automatic""#).unwrap(), TextColor::Auto);
        for (c, j) in [(TextColor::Auto, r#""Auto""#), (ink, r#"{"Rgb":[27,22,17]}"#)] {
            assert_eq!(serde_json::to_string(&c).unwrap(), j, "written as before");
            assert_eq!(read(j).unwrap(), c);
        }
        for bad in [r#""1B161""#, r#""zzzzzz""#, r#""""#, r#"{"Rgb":"1B16"}"#, r#"{"Rgb":[1,2]}"#, r#"{"Hex":[1,2,3]}"#, "7", "[27,22,17]"] {
            assert!(read(bad).is_err(), "{bad}");
        }
        assert_eq!(read(r#"{"Rgb":"1B1611"}"#).unwrap(), ink);
        let p: CharProps = serde_json::from_value(serde_json::json!({"color": "#1B1611", "bold": true})).unwrap();
        assert_eq!((p.color, p.bold), (Some(ink), Some(true)));
    }

    /// #865: an [`Rgb`] reads `RRGGBB` / `#RRGGBB` as well as the `[r, g, b]` it still writes.
    #[test]
    fn rgb_reads_hex_and_keeps_its_serialized_form() {
        let read = |j: &str| serde_json::from_str::<Rgb>(j);
        let ink = Rgb(0x1B, 0x16, 0x11);
        assert_eq!(serde_json::to_string(&ink).unwrap(), "[27,22,17]", "written as before");
        for j in ["[27,22,17]", r#""1B1611""#, r##""#1b1611""##] {
            assert_eq!(read(j).unwrap(), ink, "{j}");
        }
        for bad in ["[27,22]", "[27,22,17,0]", "[27,22,300]", "[-1,0,0]", r#""auto""#, r#""1B161""#, r#""""#, "7", "null", r#"{"r":1}"#] {
            assert!(read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn props_serde_skip_none() {
        let p = CharProps { bold: Some(true), ..Default::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"bold":true}"#);
        let back: CharProps = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn symbol_font_is_optional_in_json_and_survives_font_changes() {
        // Older documents (no `symFont`) still load.
        let old: CharProps = serde_json::from_str(r#"{"font":"Arial"}"#).unwrap();
        assert_eq!(old.sym_font, None);
        let p = CharProps { font: Some("Arial".into()), sym_font: Some("Wingdings".into()), ..Default::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains(r#""symFont":"Wingdings""#), "{s}");
        assert_eq!(serde_json::from_str::<CharProps>(&s).unwrap(), p);
        // A font change keeps the symbol's own font; Clear Formatting too.
        let changed = p.clone().overlaid(&CharProps { font: Some("Times New Roman".into()), ..Default::default() });
        assert_eq!(changed.sym_font.as_deref(), Some("Wingdings"));
        assert_eq!(p.cleared().sym_font.as_deref(), Some("Wingdings"));
    }
}

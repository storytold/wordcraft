//! Document themes (ECMA-376 Part 1 §20.1.6, `a:theme`): the 12 theme colours, the heading and
//! body fonts and the theme's name, as a `.docx` keeps them (`word/theme/theme1.xml`) and as an
//! Office Theme file (`.thmx`) carries them: a package whose main part (the `officeDocument`
//! relationship of `_rels/.rels`) is the theme, `theme/theme/theme1.xml` (#1869).
//!
//! The format scheme (`a:fmtScheme`: fills, lines, effects) is our own minimal valid one; theme
//! effects are not edited.

use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Document, THEME_COLORS};

use crate::DocxError;
use crate::package::{Package, rel_is, rt, zip_entries};
use crate::xml::{El, W};

/// The colour scheme's slots, in [`Theme::colors`] order (as `Settings::theme_colors`).
pub const COLOR_SLOTS: [&str; 12] =
    ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];

/// Content type of a theme part.
pub const THEME_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

/// Largest theme file read (a `.thmx` is a few kilobytes; a `.docx` may carry pictures).
pub const MAX_THEME_FILE: usize = 64 * 1024 * 1024;
/// Largest theme part (or relationships part on the way to it) unpacked: a theme is a few
/// kilobytes, so anything near this is hostile.
pub const MAX_THEME_PART: usize = 4 * 1024 * 1024;
/// Longest theme or font name kept.
const MAX_NAME: usize = 256;

/// A document theme: its name, the 12 theme colours (dark 1, light 1, dark 2, light 2,
/// accents 1–6, hyperlink, followed hyperlink) and the heading (major) and body (minor) fonts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub colors: [Rgb; 12],
    pub major_font: String,
    pub minor_font: String,
}

impl Theme {
    /// The theme a document uses now.
    pub fn of(doc: &Document) -> Theme {
        let s = &doc.settings;
        let mut colors = THEME_COLORS;
        for (slot, c) in colors.iter_mut().zip(&s.theme_colors) {
            *slot = *c;
        }
        Theme { name: s.theme_name.clone(), colors, major_font: s.major_font.clone(), minor_font: s.minor_font.clone() }
    }

    /// Make it the document's theme (the colours, fonts and name only: styles are the caller's).
    pub fn apply_settings(&self, doc: &mut Document) {
        let s = &mut doc.settings;
        s.theme_colors = self.colors.to_vec();
        s.major_font = self.major_font.clone();
        s.minor_font = self.minor_font.clone();
        s.theme_name = self.name.clone();
    }
}

/// The theme part (`a:theme`).
pub(crate) fn theme_xml(t: &Theme) -> Vec<u8> {
    let mut w = W::new();
    let name = if t.name.is_empty() { "Craft" } else { t.name.as_str() };
    w.open("a:theme", &[("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main"), ("name", name)]);
    w.open("a:themeElements", &[]);
    w.open("a:clrScheme", &[("name", name)]);
    for (n, c) in COLOR_SLOTS.iter().zip(&t.colors) {
        w.open(&format!("a:{n}"), &[]);
        w.empty("a:srgbClr", &[("val", &c.hex())]);
        w.close(&format!("a:{n}"));
    }
    w.close("a:clrScheme");
    w.open("a:fontScheme", &[("name", name)]);
    for (tag, face) in [("a:majorFont", &t.major_font), ("a:minorFont", &t.minor_font)] {
        w.open(tag, &[]);
        w.empty("a:latin", &[("typeface", face)]);
        w.empty("a:ea", &[("typeface", "")]);
        w.empty("a:cs", &[("typeface", "")]);
        w.close(tag);
    }
    w.close("a:fontScheme");
    // A minimal format scheme (three entries per list, as the schema requires).
    w.open("a:fmtScheme", &[("name", name)]);
    w.open("a:fillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:fillStyleLst");
    w.open("a:lnStyleLst", &[]);
    for width in ["6350", "12700", "19050"] {
        w.raw(&format!(
            "<a:ln w=\"{width}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>"
        ));
    }
    w.close("a:lnStyleLst");
    w.open("a:effectStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:effectStyle><a:effectLst/></a:effectStyle>");
    }
    w.close("a:effectStyleLst");
    w.open("a:bgFillStyleLst", &[]);
    for _ in 0..3 {
        w.raw("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    }
    w.close("a:bgFillStyleLst");
    w.close("a:fmtScheme");
    w.close("a:themeElements");
    w.empty("a:objectDefaults", &[]);
    w.empty("a:extraClrSchemeLst", &[]);
    w.close("a:theme");
    w.into_bytes()
}

/// Write an Office Theme file (`.thmx`).
pub fn write_thmx(t: &Theme) -> Result<Vec<u8>, DocxError> {
    const PART: &str = "theme/theme/theme1.xml";
    let types = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/{PART}\" ContentType=\"{THEME_CONTENT_TYPE}\"/></Types>"
    );
    let rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{}\" Target=\"{PART}\"/></Relationships>",
        rt::OFFICE_DOC
    );
    zip_entries(&[("[Content_Types].xml".into(), types.into_bytes()), ("_rels/.rels".into(), rels.into_bytes()), (PART.into(), theme_xml(t))])
}

/// Read the theme of an Office Theme file (`.thmx`), or the theme part of a Word document or
/// template (`.docx`, `.dotx`…). Missing colours and fonts take WordCraft's defaults; a package
/// with no theme is an error.
pub fn read_theme(bytes: &[u8]) -> Result<Theme, DocxError> {
    if bytes.len() > MAX_THEME_FILE {
        return Err(DocxError::Limit(format!("theme file of {} bytes", bytes.len())));
    }
    // Only the relationship parts and then the theme part are unpacked, each capped at a theme's
    // size: a document's pictures cost nothing, and a zip bomb in place of a theme stops early.
    let cap = MAX_THEME_PART as u64;
    let rels = Package::open_filtered(bytes, &|n| n.to_ascii_lowercase().ends_with(".rels"), cap, cap.saturating_mul(4))?;
    let main = rels.rels("").by_type(rt::OFFICE_DOC).filter(|r| !r.external).map(|r| r.target.clone());
    let main = main.ok_or_else(|| DocxError::MissingPart("main part".into()))?;
    // A document names its theme part; a theme file's main part is the theme.
    let part = rels.rels(&main).list.into_iter().find(|r| rel_is(&r.kind, rt::THEME) && !r.external).map(|r| r.target).unwrap_or(main);
    let want = part.trim_start_matches('/').to_string();
    let pkg = Package::open_filtered(bytes, &|n| n.eq_ignore_ascii_case(&want), cap, cap)?;
    let root = pkg.xml(&part)?.ok_or_else(|| DocxError::MissingPart(part.clone()))?;
    if root.local() != "theme" {
        return Err(DocxError::MissingPart("theme".into()));
    }
    parse_theme(&root)
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_NAME).collect()
}

/// A theme from its `a:theme` element.
pub(crate) fn parse_theme(t: &El) -> Result<Theme, DocxError> {
    let mut theme = Theme { name: clip(t.attr("name").unwrap_or("")), colors: THEME_COLORS, major_font: String::new(), minor_font: String::new() };
    let el = t.child("a:themeElements");
    if let Some(cs) = el.and_then(|e| e.child("a:clrScheme")) {
        for (n, slot) in COLOR_SLOTS.iter().zip(theme.colors.iter_mut()) {
            if let Some(rgb) = cs.child(&format!("a:{n}")).and_then(scheme_color) {
                *slot = rgb;
            }
        }
    }
    let face = |n: &str| {
        el.and_then(|e| e.child("a:fontScheme"))
            .and_then(|fs| fs.child(n))
            .and_then(|f| f.child("a:latin"))
            .and_then(|l| l.attr("typeface"))
            .filter(|t| !t.is_empty())
            .map(clip)
    };
    theme.major_font = face("a:majorFont").unwrap_or_else(|| wordcraft_doc::styles::HEADING_FONT.into());
    theme.minor_font = face("a:minorFont").unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.into());
    Ok(theme)
}

/// A scheme colour slot's colour: `a:srgbClr`, or a system colour's last value.
pub(crate) fn scheme_color(c: &El) -> Option<Rgb> {
    c.child("a:srgbClr")
        .and_then(|x| x.attr("val"))
        .and_then(Rgb::parse)
        .or_else(|| c.child("a:sysClr").and_then(|x| x.attr("lastClr")).and_then(Rgb::parse))
}

//! The document's text watermark as a VML text shape in the headers (ECMA-376 Part 4, VML:
//! `shape`, `shapetype`, `path`, `textpath`, `fill`), which is how a WordprocessingML consumer
//! draws text behind every page. The reader (`read::story::vml_watermark`) maps it back.

use wordcraft_doc::{SectionProps, Watermark};

use crate::xml::esc;

/// The shape type the watermark shape uses: two straight subpaths, the box's top and bottom
/// edges, between which the `v:textpath` text is stretched to fit the shape (`fitshape`). A single
/// path gives the text no height, and Word draws nothing.
const SHAPETYPE_ID: &str = "_x0000_t136";
const SHAPETYPE_PATH: &str = "m0,0l21600,0m0,21600l21600,21600e";

/// The watermark text as written: what the reader accepts (non-blank, at most 200 characters).
pub(super) fn text(wm: &Watermark) -> Option<String> {
    let t: String = wm.text.chars().take(200).collect();
    (!t.trim().is_empty()).then_some(t)
}

/// A `w:r` holding the watermark shape (and its shape type), for the first paragraph of header
/// part number `n`, in its box ([`Watermark::frame`]: its own, or Word's default for the page).
pub(super) fn run(wm: &Watermark, n: usize, page: &SectionProps) -> Option<String> {
    let text = text(wm)?;
    let [width, height] = wm.frame(page.page_w, page.page_h);
    // A font name can't break out of the textpath's CSS-like style or make it malformed (#3994):
    // the characters a quoted CSS string can't carry raw (`\22 ` is `"`, `\3b ` `;`, `\3a ` `:`,
    // `\5c ` `\`) are escaped and control characters are dropped (no font name carries one); the
    // reader (`read::story::css_string`) maps the escapes back. An apostrophe is legal inside a
    // double-quoted string, so it is kept.
    let mut font = String::with_capacity(wm.font.len());
    for c in wm.font.chars().filter(|c| !c.is_control()).take(100) {
        match c {
            '"' => font.push_str(r"\22 "),
            ';' => font.push_str(r"\3b "),
            ':' => font.push_str(r"\3a "),
            '\\' => font.push_str(r"\5c "),
            c => font.push(c),
        }
    }
    let font = if font.trim().is_empty() { Watermark::default().font } else { font };
    let rotation = if wm.diagonal { "rotation:315;" } else { "" };
    let style = format!(
        "position:absolute;margin-left:0;margin-top:0;width:{width:.1}pt;height:{height:.1}pt;{rotation}z-index:-251654144;\
         mso-position-horizontal:center;mso-position-horizontal-relative:margin;\
         mso-position-vertical:center;mso-position-vertical-relative:margin"
    );
    let fill = if wm.semitransparent { r#"<v:fill opacity=".5"/>"# } else { "" };
    Some(format!(
        concat!(
            r#"<w:r><w:rPr><w:noProof/></w:rPr><w:pict>"#,
            r#"<v:shapetype id="{st}" coordsize="21600,21600" o:spt="136" path="{path}">"#,
            r#"<v:path textpathok="t"/><v:textpath on="t" fitshape="t"/></v:shapetype>"#,
            r##"<v:shape id="PowerPlusWaterMarkObject{n}" type="#{st}" style="{style}" o:allowincell="f" fillcolor="#{color}" stroked="f">"##,
            r#"{fill}<v:textpath style="{tstyle}" string="{text}"/></v:shape></w:pict></w:r>"#
        ),
        st = SHAPETYPE_ID,
        path = SHAPETYPE_PATH,
        n = n,
        style = esc(&style),
        color = wm.color.hex(),
        fill = fill,
        tstyle = esc(&format!("font-family:\"{font}\";font-size:1pt")),
        text = esc(&text),
    ))
}

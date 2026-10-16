//! Diagrams: a draw.io PNG that is still a diagram, because its source rides inside it.
//!
//! `File › Export as › PNG` in draw.io (with *Include a copy of my diagram*, the default for
//! PNG) puts the `mxfile` XML into a PNG text chunk, so the picture can be opened and edited
//! again — the embedded editor does that in place, the way Word activates a Visio object.
//!
//! The document keeps the PNG's bytes as they are (nothing re-encodes them on the way in or out
//! of a `.docx`), so the chunk survives a save. What loses it is a command that rewrites the
//! pixels (Compress Pictures, Corrections, Color…), which writes a fresh PNG without it.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;

use super::objects::{has_picture, selected};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("diagram.info", "Diagram", "Picture Format › Adjust", info)
            .params("{} (the selected picture: is it a draw.io diagram, and how big is it)")
            .when(has_picture),
        CommandSpec::new("diagram.update", "Update Diagram", "Picture Format › Adjust", update)
            .params(
                r#"{"data"?: base64 PNG, "path"?: string, "fit"?: "width"|"height"|"none" (the new picture, a draw.io PNG; the side it is fitted to keeps its size and the other follows the new picture's proportions)}"#,
            )
            .when(has_picture),
    ]
}

/// The selected picture as a draw.io PNG: does it carry a diagram, and how big is it?
fn info(s: &mut Session, _: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, w, h, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let bytes = s.doc.media.get(&media).ok_or_else(|| CmdError::Failed("picture data missing".into()))?;
    let (px, py) = wordcraft_diagram::png::size(bytes).unwrap_or((0, 0));
    Ok(json!({
        "isDiagram": wordcraft_diagram::png::is_drawio(bytes),
        "pixels": {"width": px, "height": py},
        "display": {"width": w, "height": h},
        "media": media,
    }))
}

/// Put an edited draw.io PNG back where the selected picture is.
///
/// The bytes are stored as they are: re-encoding would drop the `mxfile` chunk that makes the
/// picture editable, along with whatever else the PNG carries.
fn update(s: &mut Session, v: &Value) -> CmdResult {
    let bytes = if let Some(d) = p::str(v, "data") {
        super::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64".into()))?
    } else if let Some(path) = p::str(v, "path") {
        crate::io::read_file(path).map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`data` or `path` required".into()));
    };
    let (nw, nh) = wordcraft_diagram::png::size(&bytes).ok_or_else(|| CmdError::Failed("not a PNG".into()))?;
    if nw == 0 || nh == 0 {
        return Err(CmdError::Failed("the PNG has no size".into()));
    }
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, w, h, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    // Keep the side it is fitted to and take the new picture's proportions on the other: a
    // diagram redrawn taller shouldn't come back squashed.
    let (w, h) = match p::str(v, "fit").unwrap_or("width") {
        "height" => (w * nw as f32 / nh as f32, h),
        "none" => (w, h),
        _ => (w, w * nh as f32 / nw as f32),
    };
    // Reset Picture goes back to what was there before the editor was opened.
    let original = s.originals.get(&media).cloned().unwrap_or(media);
    let key = s.doc.add_media(bytes, "png");
    s.originals.entry(key.clone()).or_insert(original);
    let is_diagram = s.doc.media.get(&key).is_some_and(|b| wordcraft_diagram::png::is_drawio(b));
    let mut out = super::objects::with_obj(s, move |o| {
        if let InlineObject::Image { media, w: ow, h: oh, .. } = o {
            *media = key.clone();
            *ow = w;
            *oh = h;
        }
    })?;
    out["diagram"] = json!(is_diagram);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::Document;

    use super::*;
    use crate::{Session, cmd::objects::selected};

    /// A real PNG of `w × h` with one text chunk `keyword` holding `text`: what draw.io writes
    /// (its own text is base64 of deflated XML; a chunk is a chunk for these commands).
    fn png_with_text(w: u32, h: u32, keyword: &str, text: &str) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            if x > 2 && x < w - 3 && y > 2 && y < h - 3 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut b = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        // Put the chunk where a PNG reader keeps looking: right before `IEND`.
        let mut data = keyword.as_bytes().to_vec();
        data.push(0);
        data.extend_from_slice(text.as_bytes());
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(&data);
        chunk.extend_from_slice(&0u32.to_be_bytes());
        let at = (8..b.len()).step_by(1).find(|&i| b[i..].starts_with(b"IEND")).map_or(b.len(), |i| i - 4);
        b.splice(at..at, chunk);
        b
    }
    fn drawio_png(w: u32, h: u32, xml: &str) -> Vec<u8> {
        png_with_text(w, h, "mxfile", xml)
    }

    /// A session with one draw.io picture, selected (inserting one selects it).
    fn session_with(png: &[u8]) -> Session {
        let mut s = Session::new(Document::new());
        let data = super::super::insert::base64_encode(png);
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s
    }

    #[test]
    fn a_drawio_png_is_recognised() {
        let mut s = session_with(&drawio_png(20, 10, "<mxfile><diagram/></mxfile>"));
        let v = s.run("diagram.info", &json!({})).unwrap();
        assert_eq!(v["isDiagram"], json!(true), "{v}");
        assert_eq!(v["pixels"], json!({"width": 20, "height": 10}));
    }

    #[test]
    fn a_plain_png_is_not_a_diagram() {
        let mut s = session_with(&png_with_text(20, 10, "Title", "holiday"));
        let v = s.run("diagram.info", &json!({})).unwrap();
        assert_eq!(v["isDiagram"], json!(false), "{v}");
        assert_eq!(v["pixels"], json!({"width": 20, "height": 10}));
    }

    /// The edited picture comes back as the picture, and the frame takes its new proportions.
    #[test]
    fn an_edit_comes_back_and_the_frame_follows_it() {
        let mut s = session_with(&drawio_png(20, 10, "<mxfile><diagram/></mxfile>"));
        let (_, o) = selected(&s).unwrap();
        let InlineObject::Image { media: before, w, h, .. } = o else { panic!("a picture") };
        assert!((h - w / 2.0).abs() < 0.01, "{w}×{h}");
        // Redrawn twice as tall.
        let png = drawio_png(20, 40, "<mxfile><diagram name=\"edited\"/></mxfile>");
        let v = s.run("diagram.update", &json!({"data": super::super::insert::base64_encode(&png)})).unwrap();
        assert_eq!(v["diagram"], json!(true), "{v}");
        let (_, o) = selected(&s).unwrap();
        let InlineObject::Image { media, w: nw, h: nh, .. } = o else { panic!("a picture") };
        assert_ne!(media, before, "the picture's data is replaced");
        assert!((nw - w).abs() < 0.01, "the width is kept: {nw} != {w}");
        assert!((nh - w * 2.0).abs() < 0.01, "the height follows: {nh}");
        // Reset Picture goes back to the diagram as it was before the edit.
        s.run("picture.reset", &json!({})).unwrap();
        let (_, o) = selected(&s).unwrap();
        let InlineObject::Image { media, .. } = o else { panic!("a picture") };
        assert_eq!(media, before);
    }

    /// Only a PNG goes back in: anything else would lose the diagram tucked inside it.
    #[test]
    fn something_other_than_a_png_is_refused() {
        let mut s = session_with(&drawio_png(20, 10, "<mxfile/>"));
        assert!(s.run("diagram.update", &json!({"data": super::super::insert::base64_encode(b"not a png")})).is_err());
    }
}

//! Pictures a layout draws that the exporters cannot decode, for strict export (#6005).
//!
//! The PDF export, the raster renderer and this check decide whether a picture is drawn the same
//! way (`wordcraft_geom::picture::decode_for_drawing`, and the metafile parser): what fails here
//! is what shows as a grey box there.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;

use wordcraft_doc::{Document, MAX_MISSING_PICTURES, MissingPicture, MissingPictureReason, StoryRef, picture_hash};
use wordcraft_geom::picture::DrawFailure;

use crate::display::{DisplayOptions, Draw, ItemDisplay};
use crate::{DocLayout, Placed};

/// The widest or tallest picture the PDF export draws, pixels (its decoder limit).
pub const MAX_PDF_PICTURE_SIDE: u32 = 30_000;

/// The widest or tallest picture the page raster draws, pixels (what a `Pixmap` holds).
pub const MAX_PNG_PICTURE_SIDE: u32 = u16::MAX as u32;

/// The output a strict export checks, which decides what its exporter can draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictOutput {
    Pdf,
    Png,
}

impl StrictOutput {
    /// The widest or tallest picture this output decodes.
    pub fn max_picture_side(self) -> u32 {
        match self {
            StrictOutput::Pdf => MAX_PDF_PICTURE_SIDE,
            StrictOutput::Png => MAX_PNG_PICTURE_SIDE,
        }
    }
}

/// Whether `bytes` are a picture `output` draws: a metafile that parses, or a raster picture that
/// decodes within the decode budget. The same decision `wordcraft-pdf` and `wordcraft-render`
/// take, so a picture that fails here is drawn as a grey box (or not at all) by that output.
pub fn media_check(bytes: &[u8], output: StrictOutput) -> Result<(), DrawFailure> {
    if wordcraft_metafile::is_metafile(bytes) && wordcraft_metafile::parse(bytes).is_ok_and(|p| p.has_size()) {
        return Ok(());
    }
    wordcraft_geom::picture::check_for_drawing(bytes, output.max_picture_side())
}

/// The pictures drawn on `pages` (0-based, `None` for every page) that fail [`media_check`] for
/// `output`, the page's background picture included (named `background`), one
/// per picture and story, at most [`MAX_MISSING_PICTURES`]; the flag says more were found, and the
/// count is how many distinct ones there were in all (saturating).
///
/// `part` is the part the bytes were read from when the document knows it (a DOCX opened by the
/// reader), else the media key. `story` is the source part of the header, footer, note or comment
/// the picture is in (`word/header2.xml`), `word/document.xml` for the body; where the document
/// holds no source names (a document not opened from a DOCX) it is `header`, `footer`,
/// `footnote`, `endnote`, `comment`, `text box` or `body`. A picture in a text box or a shape is
/// named for the header, footer or body area it is placed in, not for the box.
pub fn undecodable_pictures(
    doc: &Document,
    layout: &DocLayout,
    pages: Option<RangeInclusive<usize>>,
    opts: &DisplayOptions,
    output: StrictOutput,
) -> (Vec<MissingPicture>, bool, u64) {
    let mut out: Vec<MissingPicture> = Vec::new();
    let mut truncated = false;
    let mut total: u64 = 0;
    // Hashes of the distinct ones past the cap, so the total counts each once.
    let mut unlisted: HashSet<u64> = HashSet::new();
    // What `out` holds, for an O(1) test of "already listed".
    let mut listed: HashSet<MissingPicture> = HashSet::new();
    let mut verdicts: HashMap<String, Option<DrawFailure>> = HashMap::new();
    let range = pages.unwrap_or(0..=usize::MAX);
    for (n, page) in layout.pages.iter().enumerate() {
        if !range.contains(&n) {
            continue;
        }
        let shown = ItemDisplay::new(doc, page);
        let regions: [(&[Placed], Region); 3] =
            [(&page.header, Region::Header(page.header_story)), (&page.footer, Region::Footer(page.footer_story)), (&page.items, Region::Body)];
        for (items, region) in regions {
            for it in items {
                let mut media: Vec<String> = Vec::new();
                let draws = shown.draws(it, opts);
                collect_media(&draws, &mut media);
                for key in media {
                    let verdict = *verdicts.entry(key.clone()).or_insert_with(|| match doc.media.get(&key) {
                        Some(bytes) => media_check(bytes, output).err(),
                        None => Some(DrawFailure::Undecodable),
                    });
                    let Some(failure) = verdict else { continue };
                    let reason = match failure {
                        DrawFailure::Undecodable => MissingPictureReason::Undecodable,
                        DrawFailure::UnsupportedFormat => MissingPictureReason::UnsupportedFormat,
                    };
                    let part = doc.missing_pictures.media_parts.get(&key).cloned().unwrap_or(key);
                    let item = MissingPicture { part, story: story_name(doc, region, it), reason };
                    if listed.contains(&item) {
                        continue;
                    }
                    if out.len() >= MAX_MISSING_PICTURES {
                        truncated = true;
                        if unlisted.insert(picture_hash(&item)) {
                            total = total.saturating_add(1);
                        }
                        continue;
                    }
                    total = total.saturating_add(1);
                    listed.insert(item.clone());
                    out.push(item);
                }
            }
        }
    }
    // The page picture is drawn behind every page, outside the display list.
    if let Some(pic) = &doc.settings.page_picture
        && layout.pages.iter().enumerate().any(|(n, _)| range.contains(&n))
    {
        // The page raster leaves out a background it can't read as a raster picture (a metafile),
        // which the PDF draws as vector.
        let failure = match doc.media.get(&pic.media) {
            Some(bytes) => media_check(bytes, output).err().or_else(|| {
                (output == StrictOutput::Png && wordcraft_geom::picture::background_size(bytes).is_none()).then_some(DrawFailure::UnsupportedFormat)
            }),
            None => Some(DrawFailure::Undecodable),
        };
        if let Some(failure) = failure {
            let reason = match failure {
                DrawFailure::Undecodable => MissingPictureReason::Undecodable,
                DrawFailure::UnsupportedFormat => MissingPictureReason::UnsupportedFormat,
            };
            let part = doc.missing_pictures.media_parts.get(&pic.media).cloned().unwrap_or_else(|| pic.media.clone());
            let item = MissingPicture { part, story: "background".into(), reason };
            if !listed.contains(&item) {
                if out.len() >= MAX_MISSING_PICTURES {
                    truncated = true;
                    if unlisted.insert(picture_hash(&item)) {
                        total = total.saturating_add(1);
                    }
                } else {
                    total = total.saturating_add(1);
                    listed.insert(item.clone());
                    out.push(item);
                }
            }
        }
    }
    (out, truncated, total)
}

#[derive(Clone, Copy)]
enum Region {
    Header(Option<u32>),
    Footer(Option<u32>),
    Body,
}

fn collect_media(draws: &[Draw], out: &mut Vec<String>) {
    for d in draws {
        match d {
            Draw::Image { media, .. } => out.push(media.clone()),
            Draw::Figure { draws, .. } => collect_media(draws, out),
            Draw::Turned { items, .. } | Draw::Rotated { items, .. } | Draw::Clip { items, .. } => collect_media(items, out),
            _ => {}
        }
    }
}

/// The story a placed item is in, named as [`undecodable_pictures`] says.
fn story_name(doc: &Document, region: Region, it: &Placed) -> String {
    let names = &doc.missing_pictures;
    let known = |id: Option<u32>| id.and_then(|id| names.part_names.get(&id)).map(|n| n.to_string());
    match region {
        Region::Header(id) => known(id).unwrap_or_else(|| "header".into()),
        Region::Footer(id) => known(id).unwrap_or_else(|| "footer".into()),
        Region::Body => {
            let story = match it {
                Placed::Lines { story, .. } | Placed::Image { story, .. } | Placed::Graphic { story, .. } => *story,
                _ => StoryRef::Body,
            };
            match story {
                StoryRef::Body if names.main.is_empty() => "body".into(),
                StoryRef::Body => names.main.clone(),
                StoryRef::Part(id) => known(Some(id)).unwrap_or_else(|| {
                    use wordcraft_doc::PartKind::*;
                    match doc.parts.get(&id).map(|p| p.kind) {
                        Some(Header) => "header",
                        Some(Footer) => "footer",
                        Some(Footnote) => "footnote",
                        Some(Endnote) => "endnote",
                        Some(Comment) => "comment",
                        Some(TextBox) | None => "text box",
                    }
                    .into()
                }),
            }
        }
    }
}

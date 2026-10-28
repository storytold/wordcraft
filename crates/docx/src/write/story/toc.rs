use super::{Block, Blocks, InlineObject, W, Writer};

impl Writer<'_> {
    pub(super) fn update_toc_before_block(&mut self, i: usize, toc: Option<(usize, usize)>, toc_field_open: bool, w: &mut W) {
        if let Some((start, end)) = toc {
            if i == start {
                w.raw(TOC_SDT_OPEN);
                self.toc_hold_end = true;
            }
            self.toc_begin_here = toc_field_open && i == start + 1;
            self.toc_end_here = toc_field_open && i == end;
        }
    }

    pub(super) fn update_toc_after_block(&mut self, i: usize, toc: Option<(usize, usize)>, toc_field_open: &mut bool, w: &mut W) {
        if let Some((start, end)) = toc {
            if i == start {
                // `field` holds the heading's TOC field back for the first entry.
                self.toc_hold_end = false;
                *toc_field_open = self.toc_field.is_some();
            }
            if i == end {
                w.raw("</w:sdtContent></w:sdt>");
            }
        }
    }
}

/// Word's Table of Contents content control, which gives the TOC its frame and Update Table.
const TOC_SDT_OPEN: &str =
    r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>"#;

pub(super) fn is_toc(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("TOC"))
}

pub(super) fn is_toa(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("TOA"))
}

/// The tables of authorities the engine builds: a paragraph holding an empty `TOA` field, then
/// its entries (Word's Table of Authorities style; nothing else is taken as one), then, as Word saves
/// one, maybe an empty paragraph its field ends in. Returns (heading, last paragraph) for each,
/// so the field can be written spanning the heading's text and the entries, as Word does.
pub(super) fn toa_spans(bl: &Blocks) -> Vec<(usize, usize)> {
    let entry = |b: &Block| b.as_para().is_some_and(|p| p.section.is_none() && matches!(p.props.style.as_deref(), Some("TableofAuthorities")));
    let mut out = Vec::new();
    for (start, b) in bl.iter().enumerate() {
        let heading = b.as_para().is_some_and(|p| {
            p.section.is_none()
                && p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, result, .. } if is_toa(instr) && result.is_empty()))
        });
        if !heading {
            continue;
        }
        let entries = bl[start + 1..].iter().take_while(|b| entry(b)).count();
        let mut end = start + entries;
        if let Some(p) = bl.get(end + 1).and_then(|b| b.as_para())
            && p.section.is_none()
            && p.text.is_empty()
            && p.objects.is_empty()
        {
            end += 1;
        }
        if end > start {
            out.push((start, end));
        }
    }
    out
}

/// The body TOC the writer wraps in Word's Table of Contents control (`toc_span`): not one whose
/// heading is inside a content control already (read from a file).
pub(crate) fn written_toc_span(bl: &Blocks) -> Option<(usize, usize)> {
    toc_span(bl).filter(|(start, _)| !crate::write::sdt::plan(bl).covers(*start))
}

/// The engine's TOC: a paragraph holding an empty `TOC` field, then its TOC-styled entries.
/// Returns (heading, last entry) so the field can be written spanning the entries, as Word does.
/// The heading may end a section: entries in columns follow a continuous break (#3889).
fn toc_span(bl: &Blocks) -> Option<(usize, usize)> {
    let start = bl.iter().position(|b| {
        b.as_para()
            .is_some_and(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, result, .. } if is_toc(instr) && result.is_empty())))
    })?;
    let entries = bl[start + 1..]
        .iter()
        .take_while(|b| {
            b.as_para().is_some_and(|p| p.section.is_none() && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        })
        .count();
    (entries > 0).then_some((start, start + entries))
}

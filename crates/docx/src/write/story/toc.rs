use super::{Blocks, InlineObject, W, Writer};

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

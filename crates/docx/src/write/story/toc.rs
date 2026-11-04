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
    toc_span(bl).or_else(|| range_toc_span(bl)).filter(|(start, _)| !crate::write::sdt::plan(bl).covers(*start))
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

/// A TOC read from Word's Table of Contents control whose `TOC` field is still a range over the
/// entries (`FieldStart`…`FieldEnd`) because Update Table hasn't converted it yet: either the
/// field's end sits alone in a paragraph after the entries (Word's layout) or the heading is an
/// empty paragraph (#2182, #2427).
fn range_toc_span(bl: &Blocks) -> Option<(usize, usize)> {
    use wordcraft_doc::para::OBJ;
    let entry =
        |p: &wordcraft_doc::Paragraph| p.section.is_none() && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading");
    let first = (1..bl.len()).find(|&i| {
        bl[i].as_para().is_some_and(|p| entry(p) && matches!(p.object_at(0), Some(InlineObject::FieldStart { instr, .. }) if is_toc(instr)))
    })?;
    let start = first - 1;
    let heading_ok = bl[start].as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOCHeading") || p.is_empty());
    if !heading_ok {
        return None;
    }
    let mut depth = 0_usize;
    for (i, b) in bl.iter().enumerate().skip(first) {
        let Some(p) = b.as_para() else { break };
        if !entry(p) {
            let end_only =
                i > first && depth == 1 && p.section.is_none() && p.len() == OBJ.len_utf8() && matches!(p.object_at(0), Some(InlineObject::FieldEnd));
            return end_only.then_some((start, i));
        }
        for (off, o) in p.object_offsets().into_iter().zip(&p.objects) {
            match o {
                InlineObject::FieldStart { .. } => depth = depth.checked_add(1)?,
                InlineObject::FieldEnd => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let following_entry = bl.get(i + 1).and_then(|b| b.as_para()).is_some_and(entry);
                        return (off.checked_add(OBJ.len_utf8()) == Some(p.len()) && !following_entry).then_some((start, i));
                    }
                }
                _ => {}
            }
        }
    }
    None
}

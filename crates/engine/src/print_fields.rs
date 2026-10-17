//! Cross-reference results in a transient print copy.

use std::collections::{HashMap, HashSet};

use wordcraft_doc::{Document, InlineObject, Paragraph, Pos, StoryRef};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions};

use crate::cmd::references::FieldCode;

/// Bound page-number feedback separately from REF dependency depth.
const MAX_PAGE_REF_PASSES: usize = 16;
/// Bound generated text work, including cyclic fields that duplicate their own results.
const MAX_REF_WORK_BYTES: usize = 64 * 1024 * 1024;
const REF_NAME: &str = "REF";
const PAGE_REF_NAME: &str = "PAGEREF";
const MISSING_TARGET: &str = "Missing reference target.";
const REF_INSPECTION_LIMIT: &str = "Print cross-reference inspection exceeds the work limit";

pub(crate) struct PrintCopy {
    pub doc: Document,
    pub layout: DocLayout,
}

#[derive(Clone, Copy)]
enum RefPass<'a> {
    Text,
    Pages(&'a DocLayout),
}

/// Evaluate a copy: exporting never changes editor caches, saved results or TOC structure.
pub(crate) fn prepare(doc: &Document, opts: &LayoutOptions) -> Result<PrintCopy, String> {
    let mut doc = doc.clone();
    let mut cache = LayoutCache::new();
    if doc.settings.protection.is_some() {
        let layout = wordcraft_layout::layout(&doc, &mut cache, opts);
        return Ok(PrintCopy { doc, layout });
    }
    let held = held_ref_targets(&doc, MAX_REF_WORK_BYTES)?;
    let mut budget = MAX_REF_WORK_BYTES;
    settle_text(&mut doc, &mut budget, &held)?;
    for _ in 0..MAX_PAGE_REF_PASSES {
        let layout = wordcraft_layout::layout(&doc, &mut cache, opts);
        if !refresh(&mut doc, RefPass::Pages(&layout), &mut budget, &held)? {
            return Ok(PrintCopy { doc, layout });
        }
        settle_text(&mut doc, &mut budget, &held)?;
    }
    Err("Print page-reference results did not converge".into())
}

fn settle_text(doc: &mut Document, budget: &mut usize, held: &HashSet<String>) -> Result<(), String> {
    let count: usize = std::iter::once(StoryRef::Body)
        .chain(doc.parts.keys().copied().map(StoryRef::Part))
        .map(|story| {
            doc.para_paths(story)
                .iter()
                .filter_map(|path| doc.para(story, path))
                .map(|p| p.objects.iter().filter_map(print_field_code).filter(|code| code.name == REF_NAME).count())
                .sum::<usize>()
        })
        .sum();
    // An acyclic chain propagates through at most its number of REF fields. One extra
    // pass checks stability without laying out every intermediate dependency value.
    for _ in 0..=count {
        if !refresh(doc, RefPass::Text, budget, held)? {
            return Ok(());
        }
    }
    Err("Print REF dependencies did not converge".into())
}

fn refresh(doc: &mut Document, pass: RefPass<'_>, budget: &mut usize, held: &HashSet<String>) -> Result<bool, String> {
    let bookmarks: std::collections::HashMap<_, _> = doc.bookmarks().into_iter().collect();
    let nonbody_targets = nonbody_bookmarks(doc);
    let sections = doc.sections();
    let pages: std::collections::HashMap<_, _> = match pass {
        RefPass::Text => Default::default(),
        RefPass::Pages(layout) => {
            let mut ends = std::collections::HashMap::new();
            for path in doc.para_paths(StoryRef::Body) {
                let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
                for (off, object) in p.object_offsets().into_iter().zip(&p.objects) {
                    if let InlineObject::BookmarkEnd { name } = object {
                        ends.entry(name.clone()).or_insert_with(|| Pos::new(StoryRef::Body, path.clone(), off));
                    }
                }
            }
            bookmarks
                .iter()
                .filter_map(|(name, pos)| {
                    let end = ends.get(name)?;
                    let first = layout.caret(pos)?;
                    let last = layout.caret(end)?;
                    if end < pos {
                        return None;
                    }
                    let anchor = if first.page == last.page { first } else { layout.caret(&range_page_pos(doc, pos, end)?)? };
                    let page = layout.pages.get(anchor.page)?;
                    let format = sections.get(page.section).map(|(_, s)| s.page_num_format).unwrap_or_default();
                    Some((name.clone(), (page.number, format)))
                })
                .collect()
        }
    };
    let wanted = match pass {
        RefPass::Text => REF_NAME,
        RefPass::Pages(_) => PAGE_REF_NAME,
    };
    let stories: Vec<_> = std::iter::once(StoryRef::Body).chain(doc.parts.keys().copied().map(StoryRef::Part)).collect();
    let mut changed = false;
    for story in stories {
        let locks: Vec<_> = doc.field_ranges(story).into_iter().filter(|r| r.locked).collect();
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            let mut updates = Vec::new();
            for (index, (off, object)) in para.object_offsets().into_iter().zip(&para.objects).enumerate() {
                let InlineObject::Field { instr, result, locked: false, .. } = object else { continue };
                let pos = Pos::new(story, path.clone(), off);
                if locks.iter().any(|r| r.start < pos && pos < r.end) {
                    continue;
                }
                let Some(code) = print_field_code(object) else { continue };
                if code.name != wanted {
                    continue;
                }
                let Some(target) = code.args.first() else { continue };
                if wanted == REF_NAME && held.contains(target) {
                    continue;
                }
                let text = match bookmarks.get(target) {
                    None if nonbody_targets.contains(target) => None,
                    None => Some(MISSING_TARGET.to_string()),
                    Some(start) if wanted == REF_NAME => inline_bookmark(doc, target, start, *budget)?,
                    Some(_) => {
                        pages.get(target).map(|(number, format)| wordcraft_layout::fields::format_switch(instr).unwrap_or(*format).format(*number))
                    }
                };
                if let Some(text) = text.filter(|text| text != result) {
                    *budget = budget.checked_sub(text.len()).ok_or("Print cross-reference text exceeds the work limit")?;
                    updates.push((index, text));
                }
            }
            if updates.is_empty() {
                continue;
            }
            let para = doc.para_mut(story, &path).map_err(|e| e.to_string())?;
            for (index, text) in updates {
                if let Some(InlineObject::Field { result, dirty, .. }) = para.objects.get_mut(index) {
                    *result = text;
                    *dirty = false;
                }
            }
            para.touch();
            changed = true;
        }
    }
    Ok(changed)
}

fn print_field_code(object: &InlineObject) -> Option<FieldCode> {
    let InlineObject::Field { instr, locked: false, .. } = object else { return None };
    print_ref_code(instr)
}

// Keep one eligibility rule for exported fields and fields copied through a REF.
fn print_ref_code(instr: &str) -> Option<FieldCode> {
    let code = FieldCode::parse(instr);
    if !matches!(code.name.as_str(), REF_NAME | PAGE_REF_NAME) || code.args.len() != 1 {
        return None;
    }
    // Numbered, relative and note references need their own evaluators.
    if code.switches.iter().any(|(s, arg)| match s {
        'h' => false,
        '*' => !arg.as_deref().is_some_and(|a| {
            a.eq_ignore_ascii_case("MERGEFORMAT")
                || a.eq_ignore_ascii_case("CHARFORMAT")
                || (code.name == PAGE_REF_NAME && wordcraft_layout::fields::format_switch(&format!(r"PAGEREF target \* {a}")).is_some())
        }),
        _ => true,
    }) {
        return None;
    }
    // Adjacent targets or switches are not separated by the shared parser.
    if code.args.first()?.contains('\\') || instr.split_whitespace().any(|word| word.matches('\\').count() > 1) {
        return None;
    }
    Some(code)
}

fn nonbody_bookmarks(doc: &Document) -> HashSet<String> {
    let mut nonbody_targets = std::collections::HashSet::new();
    for part in doc.parts.keys().copied() {
        let story = StoryRef::Part(part);
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            for object in &para.objects {
                if let InlineObject::BookmarkStart { name } = object {
                    nonbody_targets.insert(name.clone());
                }
            }
        }
    }
    nonbody_targets
}

fn bookmark_end(para: &Paragraph, name: &str, start: usize) -> Option<usize> {
    para.object_offsets()
        .into_iter()
        .zip(&para.objects)
        .find_map(|(off, object)| (off > start && matches!(object, InlineObject::BookmarkEnd { name: n } if n == name)).then_some(off))
}

/// Layout-only or unsupported fields cannot supply their stored text to a REF.
/// Propagate this restriction through REF dependencies once, without recursion.
/// Charge selected inspection work; initial scans and total heap use are not capped.
fn held_ref_targets(doc: &Document, mut budget: usize) -> Result<HashSet<String>, String> {
    let has_refs = std::iter::once(StoryRef::Body).chain(doc.parts.keys().copied().map(StoryRef::Part)).any(|story| {
        doc.para_paths(story)
            .iter()
            .filter_map(|path| doc.para(story, path))
            .any(|para| para.objects.iter().filter_map(print_field_code).any(|code| code.name == REF_NAME))
    });
    if !has_refs {
        return Ok(HashSet::new());
    }
    let bookmarks: HashMap<_, _> = doc.bookmarks().into_iter().collect();
    let nonbody = nonbody_bookmarks(doc);
    let mut held: HashSet<_> = nonbody.iter().filter(|name| !bookmarks.contains_key(*name)).cloned().collect();
    let ranges = doc.field_ranges(StoryRef::Body);
    let mut parents: HashMap<String, Vec<String>> = HashMap::new();
    for (name, start) in &bookmarks {
        let Some(para) = doc.para(start.story, &start.path) else {
            held.insert(name.clone());
            continue;
        };
        budget = budget.checked_sub(para.text.len()).ok_or(REF_INSPECTION_LIMIT)?;
        let Some(end) = bookmark_end(para, name, start.off) else {
            held.insert(name.clone());
            continue;
        };
        let stop = Pos { off: end, ..start.clone() };
        let mut in_field = false;
        for range in &ranges {
            budget = budget.checked_sub(std::mem::size_of::<wordcraft_doc::FieldRange>()).ok_or(REF_INSPECTION_LIMIT)?;
            if range.start < stop && range.end > *start {
                in_field = true;
                break;
            }
        }
        if in_field {
            held.insert(name.clone());
            continue;
        }
        for (off, object) in para.object_offsets().into_iter().zip(&para.objects) {
            if off < start.off || off >= end {
                continue;
            }
            let instr = match object {
                InlineObject::Field { instr, locked: false, code: false, .. } => instr,
                InlineObject::Field { .. } | InlineObject::FieldStart { .. } | InlineObject::FieldEnd => {
                    held.insert(name.clone());
                    break;
                }
                _ => continue,
            };
            budget = budget.checked_sub(instr.len()).ok_or(REF_INSPECTION_LIMIT)?;
            let Some(code) = print_ref_code(instr) else {
                held.insert(name.clone());
                break;
            };
            let Some(target) = code.args.first() else { continue };
            if code.name == REF_NAME {
                budget = budget
                    .checked_sub(name.len())
                    .and_then(|left| left.checked_sub(target.len()))
                    .and_then(|left| left.checked_sub(std::mem::size_of::<(String, String)>()))
                    .ok_or(REF_INSPECTION_LIMIT)?;
                parents.entry(target.clone()).or_default().push(name.clone());
            } else if !bookmarks.contains_key(target) && nonbody.contains(target) {
                held.insert(name.clone());
                break;
            }
        }
    }
    let mut pending: Vec<_> = held.iter().cloned().collect();
    while let Some(target) = pending.pop() {
        let Some(dependents) = parents.remove(&target) else { continue };
        for name in dependents {
            if held.insert(name.clone()) {
                pending.push(name);
            }
        }
    }
    Ok(held)
}

/// A spanning range labels its first text. Leading hard page breaks alone move that
/// anchor forward; blank paragraphs and unsupported objects retain the saved cache.
fn range_page_pos(doc: &Document, start: &Pos, end: &Pos) -> Option<Pos> {
    if start.story != end.story || end < start {
        return None;
    }
    let mut after_break = false;
    for path in doc.para_paths(start.story).into_iter().filter(|p| *p >= start.path && *p <= end.path) {
        let para = doc.para(start.story, &path)?;
        let from = if path == start.path { start.off } else { 0 };
        let to = if path == end.path { end.off } else { para.len() };
        let mut has_break = false;
        for (offset, ch) in para.text.get(from..to)?.char_indices() {
            let off = from + offset;
            if ch == wordcraft_doc::para::OBJ {
                if para.object_at(off).is_some_and(InlineObject::is_marker) {
                    continue;
                }
                return None;
            }
            if ch == '\u{c}' {
                has_break = true;
                after_break = true;
                continue;
            }
            if ch.is_control() || ch.is_whitespace() {
                return None;
            }
            return Some(if after_break { Pos::new(start.story, path, off) } else { start.clone() });
        }
        if !has_break {
            return None;
        }
    }
    None
}

/// Buffered fields cannot represent copied paragraph formatting. Preserve their complete
/// cache unless the bookmark ends in its starting paragraph; never truncate at that boundary.
fn inline_bookmark(doc: &Document, name: &str, start: &Pos, budget: usize) -> Result<Option<String>, String> {
    let Some(para) = doc.para(start.story, &start.path) else { return Ok(None) };
    let Some(end) = bookmark_end(para, name, start.off) else { return Ok(None) };
    let bytes = para.objects.iter().try_fold(para.text.len(), |n, o| n.checked_add(o.plain_text().len()));
    if bytes.is_none_or(|n| n > budget) {
        return Err("Print cross-reference text exceeds the work limit".into());
    }
    Ok(Some(doc.copy_range(start, &Pos { off: end, ..start.clone() }).plain_text()))
}

#[cfg(test)]
mod tests {
    use wordcraft_doc::{CharProps, Document, InlineObject, Paragraph, Path, StoryRef, para_block};

    enum FieldFlags {
        Clean,
        Dirty,
        LockedDirty,
    }

    fn field(instr: &str, result: &str, flags: FieldFlags) -> InlineObject {
        InlineObject::Field {
            instr: instr.into(),
            result: result.into(),
            locked: matches!(flags, FieldFlags::LockedDirty),
            dirty: !matches!(flags, FieldFlags::Clean),
            code: false,
            form: String::new(),
        }
    }

    fn control() -> Document {
        let props = CharProps::default();
        let mut doc = Document::new();
        let mut toc = Paragraph::with_text("Cached old heading ", props.clone());
        toc.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-1" \h"#.into(), locked: false, dirty: false }, &props).unwrap();
        toc.insert_object(toc.len(), field(r"PAGEREF PublicTarget \h", "99", FieldFlags::Clean), &props).unwrap();
        toc.insert_object(toc.len(), InlineObject::FieldEnd, &props).unwrap();
        doc.body = vec![para_block(toc)];
        for (instr, cache, flags) in [
            (r"REF PublicTarget \h", "Cached clean text", FieldFlags::Clean),
            (r"REF PublicTarget \h", "Cached dirty text", FieldFlags::Dirty),
            (r"REF PublicTarget \h", "Cached locked text", FieldFlags::LockedDirty),
            (r"PAGEREF PublicTarget \h", "99", FieldFlags::Clean),
        ] {
            let mut p = Paragraph::new();
            p.insert_object(0, field(instr, cache, flags), &props).unwrap();
            doc.body.push(para_block(p));
        }
        let mut target = Paragraph::with_text("Current target heading", props.clone());
        target.props.page_break_before = Some(true);
        target.insert_object(0, InlineObject::BookmarkStart { name: "PublicTarget".into() }, &props).unwrap();
        target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "PublicTarget".into() }, &props).unwrap();
        doc.body.push(para_block(target));
        doc
    }

    fn shown(layout: &wordcraft_layout::DocLayout, index: usize) -> Vec<String> {
        layout
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| match i {
                wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, para, .. } if *path == Path::top(index) => {
                    Some(para.shown.iter().map(|(_, text, _)| text.as_str()).collect::<String>())
                }
                _ => None,
            })
            .collect()
    }

    fn pdf_text(bytes: &[u8]) -> Vec<String> {
        use hayro_interpret::font::Glyph;
        use hayro_interpret::{
            BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
            interpret_page,
        };
        use hayro_syntax::Pdf;
        struct Ex(String);
        impl Device<'_> for Ex {
            fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
            fn set_blend_mode(&mut self, _: BlendMode) {}
            fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
            fn push_clip_path(&mut self, _: &ClipPath) {}
            fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
            fn draw_glyph(&mut self, g: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
                // Synthetic bold fills and then strokes each glyph: one character, two draws.
                if matches!(mode, GlyphDrawMode::Stroke(_)) {
                    return;
                }
                match g.as_unicode() {
                    Some(hayro_cmap::BfString::Char(c)) => self.0.push(c),
                    Some(hayro_cmap::BfString::String(s)) => self.0.push_str(&s),
                    None => self.0.push('\u{FFFD}'),
                }
            }
            fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
            fn pop_clip_path(&mut self) {}
            fn pop_transparency_group(&mut self) {}
        }
        let pdf = Pdf::new(bytes.to_vec()).expect("parse");
        let cache = InterpreterCache::new();
        pdf.pages()
            .iter()
            .map(|page| {
                let mut ctx =
                    Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
                let mut ex = Ex(String::new());
                interpret_page(page, &mut ctx, &mut ex);
                ex.0
            })
            .collect()
    }

    #[test]
    fn compact_refs_keep_cache() {
        let mut doc = control();
        let props = CharProps::default();
        let cases = [
            (r"REF PublicTarget\h", FieldFlags::Dirty),
            (r"PAGEREF PublicTarget\h", FieldFlags::Dirty),
            (r"PAGEREF PublicTarget\p\h", FieldFlags::Dirty),
            (r"REF PublicTarget\h", FieldFlags::LockedDirty),
            (r"PAGEREF PublicTarget \p\h", FieldFlags::Dirty),
            (r"REF PublicTarget \h\p", FieldFlags::Dirty),
            (r"PAGEREF PublicTarget \h\p", FieldFlags::Dirty),
        ];
        let first = doc.body.len();
        for (instr, flags) in cases {
            let mut p = Paragraph::new();
            p.insert_object(0, field(instr, "Complete saved cache", flags), &props).unwrap();
            doc.body.push(para_block(p));
        }
        let before = serde_json::to_value(&doc).unwrap();
        let print = super::prepare(&doc, &Default::default()).unwrap();
        for index in first..doc.body.len() {
            assert_eq!(print.doc.para(StoryRef::Body, &Path::top(index)), doc.para(StoryRef::Body, &Path::top(index)));
        }
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap()).join("");
        assert_eq!(text.matches("Complete saved cache").count(), doc.body.len() - first);
        assert!(!text.contains(super::MISSING_TARGET));
    }

    #[test]
    fn nonbody_refs_keep_cache() {
        use wordcraft_doc::PartKind;
        for kind in [PartKind::Header, PartKind::Footer, PartKind::TextBox, PartKind::Footnote, PartKind::Endnote, PartKind::Comment] {
            let mut doc = control();
            let props = CharProps::default();
            let mut target = Paragraph::with_text("Original nonbody target", props.clone());
            target.insert_object(0, InlineObject::BookmarkStart { name: "OtherTarget".into() }, &props).unwrap();
            target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "OtherTarget".into() }, &props).unwrap();
            let part = doc.add_part(kind, vec![para_block(target)]);
            let first = doc.body.len();
            for instr in ["REF OtherTarget", "PAGEREF OtherTarget"] {
                let mut p = Paragraph::new();
                p.insert_object(0, field(instr, "Complete nonbody cache", FieldFlags::Dirty), &props).unwrap();
                doc.body.push(para_block(p));
            }
            let print = super::prepare(&doc, &Default::default()).unwrap();
            for index in first..doc.body.len() {
                assert_eq!(print.doc.para(StoryRef::Body, &Path::top(index)), doc.para(StoryRef::Body, &Path::top(index)), "{kind:?}");
            }
            let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap()).join("");
            assert_eq!(text.matches("Complete nonbody cache").count(), 2, "{kind:?}");
            // A same-named nonbody target must not shadow the supported body target.
            for object in &mut doc.para_mut(StoryRef::Part(part), &Path::top(0)).unwrap().objects {
                if let InlineObject::BookmarkStart { name } | InlineObject::BookmarkEnd { name } = object {
                    *name = "PublicTarget".into();
                }
            }
            let print = super::prepare(&doc, &Default::default()).unwrap();
            assert_eq!(shown(&print.layout, 1), ["Current target heading"]);
        }
    }

    #[test]
    fn absent_refs_keep_error_policy() {
        let mut doc = control();
        for instr in ["REF AbsentTarget", "PAGEREF AbsentTarget"] {
            let mut p = Paragraph::new();
            p.insert_object(0, field(instr, "Old cache", FieldFlags::Dirty), &CharProps::default()).unwrap();
            doc.body.push(para_block(p));
        }
        let print = super::prepare(&doc, &Default::default()).unwrap();
        for index in doc.body.len() - 2..doc.body.len() {
            assert_eq!(print.doc.para(StoryRef::Body, &Path::top(index)).unwrap().plain_text(), super::MISSING_TARGET);
        }
    }

    #[test]
    fn quoted_refs_converge() {
        for instr in [r#"REF"Target""#, r#""REF" Target"#, r#"REF "Target""#] {
            let props = CharProps::default();
            let mut doc = Document::new();
            let mut lookup = Paragraph::new();
            lookup.insert_object(0, field(&instr.replace("Target", "Middle"), "Old heading", FieldFlags::Dirty), &props).unwrap();
            let mut middle = Paragraph::new();
            middle.insert_object(0, InlineObject::BookmarkStart { name: "Middle".into() }, &props).unwrap();
            middle.insert_object(middle.len(), field(instr, "Old middle", FieldFlags::Clean), &props).unwrap();
            middle.insert_object(middle.len(), InlineObject::BookmarkEnd { name: "Middle".into() }, &props).unwrap();
            let mut target = Paragraph::with_text("Current heading", props.clone());
            target.insert_object(0, InlineObject::BookmarkStart { name: "Target".into() }, &props).unwrap();
            target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "Target".into() }, &props).unwrap();
            doc.body = vec![para_block(lookup), para_block(middle), para_block(target)];
            let doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
            let before = serde_json::to_value(&doc).unwrap();
            let print = super::prepare(&doc, &Default::default()).unwrap();
            assert_eq!(shown(&print.layout, 0), ["Current heading"], "{instr}");
            let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap()).concat();
            assert_eq!(text.matches("Current heading").count(), 3, "{instr}: {text}");
            assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        }
    }

    #[test]
    fn quoted_refs_hold_cache() {
        for instr in [r#"REF"Target""#, r#""REF" Target"#] {
            for nested in ["PAGE", "NUMPAGES", "SEQ Figure", "MERGEFIELD Caption"] {
                let props = CharProps::default();
                let mut doc = Document::new();
                let mut lookup = Paragraph::new();
                lookup.insert_object(0, field(instr, "Complete saved cache", FieldFlags::Dirty), &props).unwrap();
                for unsupported in [r#""REF" Target\h"#, r#"REF"Target" \n"#, r#"REF"Target" \h\p"#] {
                    lookup.insert_object(lookup.len(), field(unsupported, "Unsupported complete cache", FieldFlags::Dirty), &props).unwrap();
                }
                let mut target = Paragraph::new();
                target.insert_object(0, InlineObject::BookmarkStart { name: "Target".into() }, &props).unwrap();
                target.insert_object(target.len(), field(nested, "99", FieldFlags::Dirty), &props).unwrap();
                target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "Target".into() }, &props).unwrap();
                doc.body = vec![para_block(lookup), para_block(target)];
                let doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
                let before = serde_json::to_value(&doc).unwrap();
                let print = super::prepare(&doc, &Default::default()).unwrap();
                assert_eq!(print.doc.para(StoryRef::Body, &Path::top(0)), doc.para(StoryRef::Body, &Path::top(0)), "{instr}: {nested}");
                let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap()).concat();
                assert!(text.contains("Complete saved cache"), "{instr}: {nested}: {text}");
                assert_eq!(text.matches("Unsupported complete cache").count(), 3, "{instr}: {nested}: {text}");
                assert_eq!(serde_json::to_value(&doc).unwrap(), before);
            }
        }
    }

    #[test]
    fn ref_inspection_charges_work() {
        assert!(super::held_ref_targets(&control(), 0).unwrap_err().contains("work limit"));
    }

    #[test]
    fn print_ref_holds_nested_fields() {
        for (instr, painted) in [("PAGE", "1"), ("NUMPAGES", "1"), ("SEQ Figure", "99"), ("MERGEFIELD Caption", "99"), (r"REF Other \n", "99")] {
            let props = CharProps::default();
            let mut doc = Document::new();
            let mut lookup = Paragraph::with_text("Refs ", props.clone());
            lookup.insert_object(lookup.len(), field("REF Nested", "1", FieldFlags::Dirty), &props).unwrap();
            lookup.insert_text(lookup.len(), " / ", &props).unwrap();
            lookup.insert_object(lookup.len(), field("REF Nested", "1", FieldFlags::Clean), &props).unwrap();
            let mut target = Paragraph::new();
            target.insert_object(0, InlineObject::BookmarkStart { name: "Nested".into() }, &props).unwrap();
            target.insert_object(target.len(), field(instr, "99", FieldFlags::Dirty), &props).unwrap();
            target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "Nested".into() }, &props).unwrap();
            let mut indirect = Paragraph::new();
            indirect.insert_object(0, InlineObject::BookmarkStart { name: "Indirect".into() }, &props).unwrap();
            indirect.insert_object(indirect.len(), field("REF Nested", "Stale indirect", FieldFlags::Clean), &props).unwrap();
            indirect.insert_object(indirect.len(), InlineObject::BookmarkEnd { name: "Indirect".into() }, &props).unwrap();
            let mut outer = Paragraph::with_text("Indirect ref ", props.clone());
            outer.insert_object(outer.len(), field("REF Indirect", "1", FieldFlags::Dirty), &props).unwrap();
            doc.body = vec![para_block(lookup), para_block(target), para_block(indirect), para_block(outer)];
            let doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
            let before = serde_json::to_value(&doc).unwrap();
            let print = super::prepare(&doc, &Default::default()).unwrap();
            for index in [0, 2, 3] {
                assert_eq!(print.doc.para(StoryRef::Body, &Path::top(index)), doc.para(StoryRef::Body, &Path::top(index)), "{instr}");
            }
            assert_eq!(shown(&print.layout, 1), [painted], "{instr}");
            let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap()).concat();
            assert!(text.contains("Refs 1 / 1"), "{instr}: {text}");
            assert!(text.contains("Indirect ref 1"), "{instr}: {text}");
            assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        }
    }

    #[test]
    fn print_ref_page_dependency() {
        let mut doc = control();
        let props = CharProps::default();
        let target = doc.para_mut(StoryRef::Body, &Path::top(4)).unwrap();
        target.insert_object(0, InlineObject::BookmarkStart { name: "Label".into() }, &props).unwrap();
        target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "Label".into() }, &props).unwrap();
        let mut lookup = Paragraph::new();
        lookup.insert_object(0, field("REF Label", "99", FieldFlags::Clean), &props).unwrap();
        doc.body.push(para_block(lookup));
        let print = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(shown(&print.layout, 6), ["2"]);
    }

    #[test]
    fn print_ref_holds_range_field() {
        let props = CharProps::default();
        let mut doc = Document::new();
        let mut lookup = Paragraph::new();
        lookup.insert_object(0, field("REF Nested", "Complete REF cache", FieldFlags::Dirty), &props).unwrap();
        let mut target = Paragraph::new();
        target.insert_object(0, InlineObject::FieldStart { instr: "SEQ Figure".into(), locked: false, dirty: true }, &props).unwrap();
        target.insert_object(target.len(), InlineObject::BookmarkStart { name: "Nested".into() }, &props).unwrap();
        target.insert_text(target.len(), "99", &props).unwrap();
        target.insert_object(target.len(), InlineObject::BookmarkEnd { name: "Nested".into() }, &props).unwrap();
        target.insert_object(target.len(), InlineObject::FieldEnd, &props).unwrap();
        doc.body = vec![para_block(lookup), para_block(target)];
        let print = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(print.doc.para(StoryRef::Body, &Path::top(0)), doc.para(StoryRef::Body, &Path::top(0)));
    }

    #[test]
    fn print_ref_plain_heading() {
        use wordcraft_doc::{numbering::ListKind, props::NumRef};
        let mut doc = control();
        let num = doc.numbering.add_list(ListKind::Numbered);
        let target = doc.para_mut(StoryRef::Body, &Path::top(5)).unwrap();
        target.props.numbering = Some(NumRef { num, level: 0 });
        // Page fields outside the bookmark must not disable plain-text refresh.
        target.insert_object(0, field("PAGE", "99", FieldFlags::Clean), &Default::default()).unwrap();
        target.insert_object(target.len(), field("NUMPAGES", "99", FieldFlags::Clean), &Default::default()).unwrap();
        let doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        let print = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(shown(&print.layout, 1), ["Current target heading"]);
        assert_eq!(shown(&print.layout, 4), ["2"]);
    }

    #[test]
    fn print_refs_keep_toc_cache() {
        // Original model survives DOCX import/save; exporting changes only displayed results.
        let bytes = wordcraft_docx::write(&control()).unwrap();
        let doc = crate::io::open_bytes("control.docx", &bytes).unwrap();
        let before = serde_json::to_value(&doc).unwrap();
        let session = crate::Session::new(doc);
        let output = super::prepare(&session.doc, &Default::default()).unwrap();
        assert_eq!(output.doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), "Cached old heading 2");
        let layout = output.layout;
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(shown(&layout, 1), ["Current target heading"]);
        assert_eq!(shown(&layout, 2), ["Current target heading"]);
        assert_eq!(shown(&layout, 3), ["Cached locked text"]);
        assert_eq!(shown(&layout, 4), ["2"]);
        assert!(shown(&layout, 0).join("").contains("2"));
        assert!(!shown(&layout, 0).join("").contains("Current target heading"));
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
        let reopened = wordcraft_docx::read(&crate::io::save_bytes("again.docx", &session.doc).unwrap()).unwrap();
        assert_eq!(reopened.plain_text(StoryRef::Body), session.doc.plain_text(StoryRef::Body));
    }

    #[test]
    fn print_toc_replaces_field_cache() {
        let mut doc = control();
        let cached = "Cached nested title99";
        for object in &mut doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().objects {
            if let InlineObject::Field { result, .. } = object {
                *result = cached.into();
            }
        }
        let doc = crate::io::open_bytes("nested.docx", &wordcraft_docx::write(&doc).unwrap()).unwrap();
        let before = doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text();
        assert_eq!(before, format!("Cached old heading {cached}"));
        let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap());
        assert!(text[0].contains("Cached old heading 2"));
        assert!(!text[0].contains("Cached nested title"));
        assert_eq!(doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), before);
    }

    fn reflow_control() -> Document {
        let mut doc = control();
        doc.last_section.page_w = 220.0;
        doc.last_section.page_h = 220.0;
        doc.last_section.margin_top = 20.0;
        doc.last_section.margin_bottom = 20.0;
        doc.last_section.margin_left = 20.0;
        doc.last_section.margin_right = 20.0;
        let target = doc.para_mut(StoryRef::Body, &Path::top(5)).unwrap();
        target.props.page_break_before = Some(false);
        target.props.keep_lines = Some(true);
        target.insert_text(target.len() - wordcraft_doc::para::OBJ.len_utf8(), &" target detail".repeat(12), &CharProps::default()).unwrap();
        doc
    }

    #[test]
    fn print_crossrefs_follow_reflow() {
        let doc = reflow_control();
        let before = wordcraft_layout::layout(&doc, &mut Default::default(), &Default::default());
        let pos = doc.bookmarks().into_iter().find(|(name, _)| name == "PublicTarget").unwrap().1;
        let old_page = before.caret(&pos).unwrap().page;
        let print = super::prepare(&doc, &Default::default()).unwrap();
        let new_page = print.layout.caret(&pos).unwrap().page;
        assert!(new_page > old_page, "expanded REF must move the target");
        assert_eq!(shown(&print.layout, 4), [(new_page + 1).to_string()]);
        assert_eq!(doc.para(StoryRef::Body, &Path::top(4)).unwrap().plain_text(), "99");
    }

    #[test]
    fn print_ref_keeps_block_cache() {
        let props = CharProps::default();
        let mut doc = Document::new();
        let mut result = Paragraph::new();
        result.insert_object(0, field("REF span", "First paragraph\nSecond paragraph", FieldFlags::Clean), &props).unwrap();
        let mut first = Paragraph::with_text("First paragraph", props.clone());
        first.props.page_break_before = Some(true);
        first.insert_object(0, InlineObject::BookmarkStart { name: "span".into() }, &props).unwrap();
        let mut last = Paragraph::with_text("Second paragraph", props.clone());
        last.insert_object(last.len(), InlineObject::BookmarkEnd { name: "span".into() }, &props).unwrap();
        doc.body = vec![para_block(result), para_block(first), para_block(last)];
        let output = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(output.doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), "First paragraph\nSecond paragraph");
        let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap());
        assert!(text[0].contains("Second paragraph"));
    }

    #[derive(Clone, Copy)]
    enum RangeContext {
        Standalone,
        Toc,
    }

    fn break_range_control(context: RangeContext) -> Document {
        let props = CharProps::default();
        let mut doc = Document::new();
        doc.body.clear();
        for name in ["BreakFirst", "TextFirst"] {
            let mut p = Paragraph::with_text(&format!("{name} page "), props.clone());
            p.insert_object(p.len(), field(&format!(r"PAGEREF {name} \h"), "99", FieldFlags::Clean), &props).unwrap();
            doc.body.push(para_block(p));
        }
        if matches!(context, RangeContext::Toc) {
            for (index, name) in ["BreakFirst", "TextFirst"].into_iter().enumerate() {
                let mut p = Paragraph::with_text(&format!("Cached TOC {name} page "), props.clone());
                if index == 0 {
                    p.insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-3" \h"#.into(), locked: false, dirty: false }, &props).unwrap();
                }
                p.insert_object(p.len(), field(&format!(r"PAGEREF {name} \h"), "99", FieldFlags::Clean), &props).unwrap();
                if index == 1 {
                    p.insert_object(p.len(), InlineObject::FieldEnd, &props).unwrap();
                }
                doc.body.push(para_block(p));
            }
        }
        doc.body.push(para_block(Paragraph::with_text("\u{c}", props.clone())));
        let mut first = Paragraph::with_text("\u{c}", props.clone());
        first.insert_object(0, InlineObject::BookmarkStart { name: "BreakFirst".into() }, &props).unwrap();
        doc.body.push(para_block(first));
        let mut last = Paragraph::with_text("First visible range text", props.clone());
        last.insert_object(last.len(), InlineObject::BookmarkEnd { name: "BreakFirst".into() }, &props).unwrap();
        doc.body.push(para_block(last));
        doc.body.push(para_block(Paragraph::with_text("\u{c}", props.clone())));
        let mut text = Paragraph::with_text("Visible range prefix\u{c}Visible range suffix", props.clone());
        text.insert_object(0, InlineObject::BookmarkStart { name: "TextFirst".into() }, &props).unwrap();
        text.insert_object(text.len(), InlineObject::BookmarkEnd { name: "TextFirst".into() }, &props).unwrap();
        doc.body.push(para_block(text));
        doc
    }

    #[test]
    fn print_range_break_labels() {
        // Word's unchanged default-export controls show 3/4, including cached TOC entries.
        for context in [RangeContext::Standalone, RangeContext::Toc] {
            let doc = break_range_control(context);
            let before = serde_json::to_value(&doc).unwrap();
            let print = super::prepare(&doc, &Default::default()).unwrap();
            assert_eq!(shown(&print.layout, 0), ["3"]);
            assert_eq!(shown(&print.layout, 1), ["4"]);
            if matches!(context, RangeContext::Toc) {
                assert_eq!(shown(&print.layout, 2), ["3"]);
                assert_eq!(shown(&print.layout, 3), ["4"]);
            }
            let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap());
            assert_eq!(text.len(), 5);
            assert!(text[0].contains("BreakFirst page 3"));
            assert!(text[0].contains("TextFirst page 4"));
            assert!(text[2].contains("First visible range text"));
            assert!(text[3].contains("Visible range prefix"));
            assert!(text[4].contains("Visible range suffix"));
            assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        }
    }

    #[test]
    fn print_page_span_keeps_cache() {
        let props = CharProps::default();
        let mut doc = Document::new();
        let mut lookup = Paragraph::new();
        lookup.insert_object(0, field("PAGEREF span", "Saved range label 99", FieldFlags::Clean), &props).unwrap();
        let mut first = Paragraph::new();
        first.props.page_break_before = Some(true);
        first.insert_object(0, InlineObject::BookmarkStart { name: "span".into() }, &props).unwrap();
        first.insert_text(first.len(), " \u{c}", &props).unwrap();
        let mut last = Paragraph::with_text("Range text", props.clone());
        last.insert_object(last.len(), InlineObject::BookmarkEnd { name: "span".into() }, &props).unwrap();
        doc.body = vec![para_block(lookup), para_block(first), para_block(last)];
        let output = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(output.doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), "Saved range label 99");
        let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap());
        assert_eq!(text[0], "Saved range label 99", "whitespace before a break retains the complete cache");
    }

    #[test]
    fn print_refs_follow_long_chain() {
        let props = CharProps::default();
        let mut doc = Document::new();
        doc.body.clear();
        for i in 0..=24 {
            let mut p = Paragraph::new();
            p.insert_object(0, InlineObject::BookmarkStart { name: format!("B{i}") }, &props).unwrap();
            if i == 24 {
                p.insert_text(p.len(), "Final value", &props).unwrap();
            } else {
                p.insert_object(p.len(), field(&format!("REF B{}", i + 1), "Cached", FieldFlags::Clean), &props).unwrap();
            }
            p.insert_object(p.len(), InlineObject::BookmarkEnd { name: format!("B{i}") }, &props).unwrap();
            doc.body.push(para_block(p));
        }
        let output = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(output.doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), "Final value");
        assert_eq!(doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text(), "Cached");
    }

    #[test]
    fn print_refs_mark_missing() {
        let mut doc = control();
        for index in [1, 4] {
            let p = doc.para_mut(StoryRef::Body, &Path::top(index)).unwrap();
            let InlineObject::Field { instr, .. } = &mut p.objects[0] else { panic!("field") };
            *instr = if index == 1 { "REF absent" } else { "PAGEREF absent" }.into();
        }
        let output = super::prepare(&doc, &Default::default()).unwrap();
        for index in [1, 4] {
            assert_eq!(output.doc.para(StoryRef::Body, &Path::top(index)).unwrap().plain_text(), "Missing reference target.");
        }
    }

    #[test]
    fn print_refs_keep_range_locks() {
        let mut doc = control();
        let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        let end = p.object_offsets().last().copied().unwrap();
        p.delete(end, p.len()).unwrap();
        if let InlineObject::FieldStart { locked, .. } = &mut p.objects[0] {
            *locked = true;
        }
        let p = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
        p.insert_object(p.len(), InlineObject::FieldEnd, &Default::default()).unwrap();
        let text = pdf_text(&crate::io::save_bytes("out.pdf", &doc).unwrap());
        assert!(text[0].contains("Cached clean text"));
        assert!(text[0].contains("Cached dirty text"));
        assert!(text[0].contains("Cached locked text"));
        let output = super::prepare(&doc, &Default::default()).unwrap();
        assert_eq!(shown(&output.layout, 4), ["2"], "unlocked field outside the range refreshes");
    }

    #[test]
    fn print_pdf_labels_and_targets() {
        use hayro_syntax::object::{Array, Dict};
        let mut doc = reflow_control();
        // Only the lookup entry links, so its annotation cannot be masked by another REF.
        for path in doc.para_paths(StoryRef::Body) {
            for object in &mut doc.para_mut(StoryRef::Body, &path).unwrap().objects {
                if let InlineObject::Field { instr, .. } = object {
                    *instr = instr.replace(r" \h", "");
                }
            }
        }
        let linked = CharProps { link: Some("#PublicTarget".into()), ..Default::default() };
        let mut p = Paragraph::with_text("Lookup marker ", linked.clone());
        p.insert_object(p.len(), field(r"PAGEREF PublicTarget \h", "99", FieldFlags::Clean), &linked).unwrap();
        doc.body[4] = para_block(p);
        let before = serde_json::to_value(&doc).unwrap();
        let pos = doc.bookmarks().into_iter().find(|(name, _)| name == "PublicTarget").unwrap().1;
        let output = super::prepare(&doc, &Default::default()).unwrap();
        let target = output.layout.caret(&pos).unwrap().page;
        assert!(
            output.layout.pages.iter().flat_map(|p| &p.items).any(|item| matches!(item,
                wordcraft_layout::Placed::Lines { path, para, .. } if *path == Path::top(4)
                    && para.styles.iter().any(|r| r.rc.link.as_deref() == Some("#PublicTarget"))
            )),
            "linked field style survived preparation: {:?}",
            doc.para(StoryRef::Body, &Path::top(4)).unwrap().runs
        );
        let session = crate::Session::new(doc.clone());
        for bytes in [crate::io::save_bytes("out.pdf", &doc).unwrap(), session.print_pdf(&serde_json::json!({})).unwrap().0] {
            let text = pdf_text(&bytes).concat();
            assert!(text.contains(&format!("Lookup marker {}", target + 1)), "updated visible page label");
            let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
            let targets: Vec<_> = pdf
                .pages()
                .iter()
                .flat_map(|page| {
                    page.raw()
                        .get::<Array>(b"Annots")
                        .into_iter()
                        .flat_map(|a| a.iter::<Dict>().collect::<Vec<_>>())
                        .filter_map(|a| a.get::<Array>(b"Dest").and_then(|a| a.raw_iter().next()).and_then(|r| r.as_obj_ref()))
                        .collect::<Vec<_>>()
                })
                .collect();
            assert_eq!(targets, [pdf.pages()[target].raw().obj_id().unwrap().into()], "link follows the target moved by REF expansion");
        }
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }

    #[test]
    fn print_refs_section_format() {
        for (instruction, expected) in [(r#"PAGEREF "PublicTarget" \h"#, "x"), (r"PAGEREF PublicTarget \* Arabic", "10")] {
            let mut doc = control();
            doc.last_section.page_num_start = Some(9);
            doc.last_section.page_num_format = wordcraft_doc::section::NumFormat::LowerRoman;
            let p = doc.para_mut(StoryRef::Body, &Path::top(4)).unwrap();
            if let InlineObject::Field { instr, .. } = &mut p.objects[0] {
                *instr = instruction.into();
            }
            let output = super::prepare(&doc, &Default::default()).unwrap();
            assert_eq!(shown(&output.layout, 4), [expected]);
        }
    }

    #[test]
    fn print_crossrefs_respect_scope() {
        for (instruction, locked_toc, protected) in [
            (r"REF PublicTarget \n", false, false),
            (r"PAGEREF PublicTarget \p", false, false),
            (r"PAGEREF PublicTarget \* roman \* invalid", false, false),
            (r"INCLUDETEXT remote", false, false),
            (r"PAGEREF PublicTarget \h", true, false),
            (r"REF PublicTarget \h", false, true),
        ] {
            let mut doc = control();
            let toc = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
            for object in &mut toc.objects {
                match object {
                    InlineObject::FieldStart { locked, .. } => *locked = locked_toc,
                    InlineObject::Field { instr, .. } => *instr = instruction.into(),
                    _ => {}
                }
            }
            if protected {
                doc.settings.protection = Some("readOnly".into());
            }
            let print = super::prepare(&doc, &Default::default()).unwrap();
            assert!(print.doc.para(StoryRef::Body, &Path::top(0)).unwrap().plain_text().contains("99"), "{instruction}");
        }
    }

    #[test]
    fn print_crossrefs_reject_cycles() {
        let mut doc = control();
        let target = doc.para_mut(StoryRef::Body, &Path::top(5)).unwrap();
        target.insert_object(wordcraft_doc::para::OBJ.len_utf8(), field("REF PublicTarget", "loop", FieldFlags::Clean), &Default::default()).unwrap();
        let before = serde_json::to_value(&doc).unwrap();
        assert!(super::prepare(&doc, &Default::default()).err().unwrap().contains("did not converge"));
        for name in ["out.pdf", "out.png"] {
            assert!(crate::io::save_bytes(name, &doc).unwrap_err().contains("did not converge"));
        }
        let mut session = crate::Session::new(doc.clone());
        for command in ["file.exportPdf", "file.exportPng", "file.printPdf"] {
            let error = session.run(command, &serde_json::json!({"path": "/tmp/wordcraft-crossref-cycle"})).unwrap_err();
            assert!(error.to_string().contains("did not converge"), "{command}: {error}");
        }
        assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
}

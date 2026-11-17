//! Bounded conversion between the shared Word model and WordCraft's app model, both ways: from
//! the shared reader's document, and to the shared writer's.
//!
//! The model crates intentionally have independent Rust types. Convert document stories a
//! paragraph at a time, recursively through tables, so a large document never needs a second
//! serialized copy of its complete body in memory. Embedded media and opaque streams are moved
//! (or, writing, shared) directly because both models use the same `Arc<Vec<u8>>` representation.

use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::DocbinError;

fn convert<T, U>(value: T) -> Result<U, DocbinError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let bytes = serde_json::to_vec(&value).map_err(|error| DocbinError::Malformed(format!("shared model conversion: {error}")))?;
    drop(value);
    serde_json::from_slice(&bytes).map_err(|error| DocbinError::Malformed(format!("shared model conversion: {error}")))
}

fn convert_ref<T, U>(value: &T) -> Result<U, DocbinError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(|error| DocbinError::Malformed(format!("shared model conversion: {error}")))?;
    serde_json::from_slice(&bytes).map_err(|error| DocbinError::Malformed(format!("shared model conversion: {error}")))
}

fn blocks(source: craft_wordmodel::Blocks) -> Result<wordcraft_doc::Blocks, DocbinError> {
    source
        .into_iter()
        .map(|block| {
            let block = Arc::try_unwrap(block).unwrap_or_else(|shared| (*shared).clone());
            block_value(block).map(Arc::new)
        })
        .collect()
}

fn block_value(source: craft_wordmodel::Block) -> Result<wordcraft_doc::Block, DocbinError> {
    match source {
        craft_wordmodel::Block::Para(paragraph) => paragraph_value(paragraph).map(wordcraft_doc::Block::Para),
        craft_wordmodel::Block::Table(table) => table_value(table).map(wordcraft_doc::Block::Table),
    }
}

fn paragraph_value(source: craft_wordmodel::para::Paragraph) -> Result<wordcraft_doc::para::Paragraph, DocbinError> {
    let craft_wordmodel::para::Paragraph { text, runs, props, mark, objects, section, .. } = source;
    let mut paragraph = wordcraft_doc::para::Paragraph::new();
    paragraph.text = text;
    paragraph.runs =
        runs.into_iter().map(|run| Ok(wordcraft_doc::para::Run { len: run.len, props: convert(run.props)? })).collect::<Result<_, DocbinError>>()?;
    paragraph.props = convert(props)?;
    paragraph.mark = convert(mark)?;
    paragraph.objects = objects.into_iter().map(object_value).collect::<Result<_, _>>()?;
    paragraph.section = section.map(|section| convert(*section).map(Box::new)).transpose()?;
    Ok(paragraph)
}

fn object_value(source: craft_wordmodel::para::InlineObject) -> Result<wordcraft_doc::para::InlineObject, DocbinError> {
    use craft_wordmodel::para::InlineObject as Source;
    use wordcraft_doc::para::InlineObject as Target;

    match source {
        Source::Group { w, h, float, ch_w, ch_h, children, canvas } => Ok(Target::Group {
            w,
            h,
            float: convert(float)?,
            ch_w,
            ch_h,
            children: children.into_iter().map(group_child_value).collect::<Result<_, _>>()?,
            canvas: canvas.map(convert).transpose()?,
        }),
        other => convert(other),
    }
}

fn group_child_value(source: craft_wordmodel::para::GroupChild) -> Result<wordcraft_doc::para::GroupChild, DocbinError> {
    Ok(wordcraft_doc::para::GroupChild { x: source.x, y: source.y, obj: object_value(source.obj)? })
}

fn table_value(source: craft_wordmodel::table::Table) -> Result<wordcraft_doc::table::Table, DocbinError> {
    Ok(wordcraft_doc::table::Table {
        props: convert(source.props)?,
        grid: source.grid,
        rows: source.rows.into_iter().map(row_value).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn row_value(source: craft_wordmodel::table::Row) -> Result<wordcraft_doc::table::Row, DocbinError> {
    Ok(wordcraft_doc::table::Row {
        props: convert(source.props)?,
        cells: source.cells.into_iter().map(cell_value).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn cell_value(source: craft_wordmodel::table::Cell) -> Result<wordcraft_doc::table::Cell, DocbinError> {
    Ok(wordcraft_doc::table::Cell { props: convert(source.props)?, blocks: blocks(source.blocks)?, controls: convert(source.controls)? })
}

fn part_value(source: craft_wordmodel::Part) -> Result<wordcraft_doc::Part, DocbinError> {
    Ok(wordcraft_doc::Part { kind: convert(source.kind)?, blocks: blocks(source.blocks)?, body: convert(source.body)? })
}

fn glossary_value(source: craft_wordmodel::BuildingBlock) -> Result<wordcraft_doc::BuildingBlock, DocbinError> {
    Ok(wordcraft_doc::BuildingBlock {
        name: source.name,
        gallery: source.gallery,
        category: source.category,
        description: source.description,
        guid: source.guid,
        style: source.style,
        types: source.types,
        behaviors: source.behaviors,
        blocks: blocks(source.blocks)?,
    })
}

/// Converts the shared reader's model without copying the full document or its embedded media.
///
/// The DOC reader leaves WordCraft-only transient fit state, editor fragments, and style change
/// annotations at their defaults; the persistent document fields are transferred here. Body and
/// table-cell stories are mapped one paragraph at a time, while shared media buffers are moved.
pub fn from_shared_model(source: craft_wordmodel::Document) -> Result<wordcraft_doc::Document, DocbinError> {
    let craft_wordmodel::Document {
        body,
        last_section,
        parts,
        styles,
        numbering,
        comments,
        revisions,
        settings,
        core,
        sources,
        custom_props,
        footnote_separators,
        endnote_separators,
        media,
        passthrough,
        glossary,
        ..
    } = source;

    let mut document = wordcraft_doc::Document::new();
    document.body = blocks(body)?;
    document.last_section = convert(last_section)?;
    document.parts = parts.into_iter().map(|(id, part)| Ok((id, part_value(part)?))).collect::<Result<_, DocbinError>>()?;
    document.styles = Arc::new(convert((*styles).clone())?);
    document.numbering = Arc::new(convert((*numbering).clone())?);
    document.comments = convert(comments)?;
    document.revisions = convert(revisions)?;
    document.settings = convert(settings)?;
    document.core = convert(core)?;
    document.sources = convert(sources)?;
    document.custom_props = convert(custom_props)?;
    document.footnote_separators = convert(footnote_separators)?;
    document.endnote_separators = convert(endnote_separators)?;
    document.media = media;
    document.passthrough = passthrough;
    document.glossary = glossary.into_iter().map(glossary_value).collect::<Result<_, _>>()?;
    Ok(document)
}

fn blocks_to_shared(source: &wordcraft_doc::Blocks) -> Result<craft_wordmodel::Blocks, DocbinError> {
    source.iter().map(|block| block_to_shared(block).map(Arc::new)).collect()
}

fn block_to_shared(source: &wordcraft_doc::Block) -> Result<craft_wordmodel::Block, DocbinError> {
    match source {
        wordcraft_doc::Block::Para(paragraph) => paragraph_to_shared(paragraph).map(craft_wordmodel::Block::Para),
        wordcraft_doc::Block::Table(table) => table_to_shared(table).map(craft_wordmodel::Block::Table),
    }
}

/// A non-finite number as 0: JSON has no NaN or infinity, so one would fail the conversion.
fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn finite_para_props(mut props: wordcraft_doc::ParaProps) -> wordcraft_doc::ParaProps {
    if let Some(ls) = &mut props.line_spacing {
        match ls {
            wordcraft_doc::props::LineSpacing::Multiple(v)
            | wordcraft_doc::props::LineSpacing::Exactly(v)
            | wordcraft_doc::props::LineSpacing::AtLeast(v) => {
                *v = finite(*v);
            }
        }
    }
    for v in [&mut props.indent_left, &mut props.indent_right, &mut props.indent_first, &mut props.space_before, &mut props.space_after]
        .into_iter()
        .flatten()
    {
        *v = finite(*v);
    }
    props
}

fn finite_char_props(mut props: wordcraft_doc::CharProps) -> wordcraft_doc::CharProps {
    for v in [&mut props.size, &mut props.spacing].into_iter().flatten() {
        *v = finite(*v);
    }
    props
}

fn paragraph_to_shared(source: &wordcraft_doc::para::Paragraph) -> Result<craft_wordmodel::para::Paragraph, DocbinError> {
    let mut paragraph = craft_wordmodel::para::Paragraph::new();
    paragraph.text = source.text.clone();
    paragraph.runs = source
        .runs
        .iter()
        .map(|run| Ok(craft_wordmodel::para::Run { len: run.len, props: convert(finite_char_props(run.props.clone()))? }))
        .collect::<Result<_, DocbinError>>()?;
    paragraph.props = convert(finite_para_props(source.props.clone()))?;
    paragraph.mark = convert(finite_char_props(source.mark.clone()))?;
    paragraph.objects = source.objects.iter().map(object_to_shared).collect::<Result<_, _>>()?;
    paragraph.section = source.section.as_ref().map(|section| convert_ref(section.as_ref()).map(Box::new)).transpose()?;
    Ok(paragraph)
}

fn object_to_shared(source: &wordcraft_doc::para::InlineObject) -> Result<craft_wordmodel::para::InlineObject, DocbinError> {
    use craft_wordmodel::para::InlineObject as Target;
    use wordcraft_doc::para::InlineObject as Source;

    match source {
        Source::Image { media, w, h, alt, float, crop, fx, ole } => Ok(Target::Image {
            media: media.clone(),
            w: finite(*w),
            h: finite(*h),
            alt: alt.clone(),
            float: convert_ref(float)?,
            crop: crop.map(finite),
            fx: convert_ref(fx)?,
            ole: ole.as_ref().map(|o| convert_ref(o.as_ref()).map(Arc::new)).transpose()?,
        }),
        Source::Group { w, h, float, ch_w, ch_h, children, canvas } => Ok(Target::Group {
            w: *w,
            h: *h,
            float: convert_ref(float)?,
            ch_w: *ch_w,
            ch_h: *ch_h,
            children: children
                .iter()
                .map(|child| Ok(craft_wordmodel::para::GroupChild { x: child.x, y: child.y, obj: object_to_shared(&child.obj)? }))
                .collect::<Result<_, DocbinError>>()?,
            canvas: canvas.as_ref().map(convert_ref).transpose()?,
        }),
        other => convert_ref(other),
    }
}

fn table_to_shared(source: &wordcraft_doc::table::Table) -> Result<craft_wordmodel::table::Table, DocbinError> {
    Ok(craft_wordmodel::table::Table {
        props: convert_ref(&source.props)?,
        grid: source.grid.clone(),
        rows: source
            .rows
            .iter()
            .map(|row| {
                Ok(craft_wordmodel::table::Row {
                    props: convert_ref(&row.props)?,
                    cells: row
                        .cells
                        .iter()
                        .map(|cell| {
                            Ok(craft_wordmodel::table::Cell {
                                props: convert_ref(&cell.props)?,
                                blocks: blocks_to_shared(&cell.blocks)?,
                                controls: convert_ref(&cell.controls)?,
                            })
                        })
                        .collect::<Result<_, DocbinError>>()?,
                    controls: convert_ref(&row.controls)?,
                })
            })
            .collect::<Result<_, DocbinError>>()?,
        controls: convert_ref(&source.controls)?,
    })
}

/// The shared model of a document, for the `.doc` writer: the stories (body, headers, footers,
/// notes), sections, styles, lists, tracked changes, settings, document properties and media.
/// Comments, sources, the glossary and WordCraft's own state stay out: the writer has no place
/// for them.
pub fn to_shared_model(source: &wordcraft_doc::Document) -> Result<craft_wordmodel::Document, DocbinError> {
    let mut document = craft_wordmodel::Document::new();
    document.body = blocks_to_shared(&source.body)?;
    document.last_section = convert_ref(&source.last_section)?;
    document.parts = source
        .parts
        .iter()
        .map(|(&id, part)| {
            Ok((
                id,
                craft_wordmodel::Part { kind: convert_ref(&part.kind)?, blocks: blocks_to_shared(&part.blocks)?, body: convert_ref(&part.body)? },
            ))
        })
        .collect::<Result<_, DocbinError>>()?;
    document.styles = Arc::new(convert_ref(source.styles.as_ref())?);
    document.numbering = Arc::new(convert_ref(source.numbering.as_ref())?);
    document.revisions = convert_ref(&source.revisions)?;
    document.settings = convert_ref(&source.settings)?;
    document.core = convert_ref(&source.core)?;
    document.custom_props = convert_ref(&source.custom_props)?;
    document.media = source.media.clone();
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridges_nested_stories_and_moves_media_without_copying() {
        let mut source = craft_wordmodel::Document::new();
        let long_text = "x".repeat(4 << 20);
        let mut body_para = craft_wordmodel::para::Paragraph::with_text(&long_text, Default::default());
        body_para.objects.push(craft_wordmodel::para::InlineObject::Group {
            w: 32.0,
            h: 24.0,
            float: Default::default(),
            ch_w: 64.0,
            ch_h: 48.0,
            children: vec![craft_wordmodel::para::GroupChild {
                x: 4.0,
                y: 6.0,
                obj: craft_wordmodel::para::InlineObject::Field {
                    instr: "PAGE".into(),
                    result: "7".into(),
                    locked: true,
                    dirty: false,
                    code: false,
                    form: String::new(),
                },
            }],
            canvas: None,
        });
        let cell = craft_wordmodel::table::Cell::with_text("nested cell");
        let table = craft_wordmodel::table::Table {
            rows: vec![craft_wordmodel::table::Row { cells: vec![cell], ..Default::default() }],
            ..Default::default()
        };
        source.body = vec![Arc::new(craft_wordmodel::Block::Para(body_para)), Arc::new(craft_wordmodel::Block::Table(table))];
        source.parts.insert(
            12,
            craft_wordmodel::Part {
                kind: craft_wordmodel::PartKind::TextBox,
                blocks: vec![craft_wordmodel::para_block(craft_wordmodel::para::Paragraph::with_text("part story", Default::default()))],
                ..Default::default()
            },
        );
        source.settings.compat_mode = 11;
        let image = Arc::new(vec![0x5a; 1 << 20]);
        source.media.insert("image1.png".into(), image.clone());

        let document = from_shared_model(source).expect("model conversion");
        assert_eq!(document.body.len(), 2);
        assert_eq!(document.body[0].as_para().map(|p| p.text.as_str()), Some(long_text.as_str()));
        let group = match &document.body[0].as_para().expect("paragraph").objects[0] {
            wordcraft_doc::para::InlineObject::Group { children, ch_w, .. } => {
                assert_eq!(*ch_w, 64.0);
                children
            }
            other => panic!("expected grouped object, got {other:?}"),
        };
        assert_eq!((group[0].x, group[0].y), (4.0, 6.0));
        assert!(
            matches!(&group[0].obj, wordcraft_doc::para::InlineObject::Field { instr, result, locked: true, .. } if instr == "PAGE" && result == "7")
        );
        let table = document.body[1].as_table().expect("table block");
        assert_eq!(table.rows[0].cells[0].blocks[0].as_para().map(|p| p.text.as_str()), Some("nested cell"));
        assert_eq!(document.parts[&12].blocks[0].as_para().map(|p| p.text.as_str()), Some("part story"));
        assert_eq!(document.settings.compat_mode, 11);
        assert!(Arc::ptr_eq(document.media.get("image1.png").expect("moved media"), &image));
        assert!(document.fit_state.is_none());
    }

    /// A WordCraft document saved as a `.doc` goes through the shared model and reads back: body
    /// text, a table cell, a tracked insertion with its author and time, and the document title.
    #[test]
    fn writes_through_the_shared_model_and_reads_back() {
        use wordcraft_doc::{Revision, RevisionKind};
        let mut doc = wordcraft_doc::Document::new();
        doc.revisions = vec![Revision::new(RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z")];
        let mut para = wordcraft_doc::para::Paragraph::with_text("kept added", Default::default());
        para.format(5, 10, &|c| c.ins = Some(0)).expect("format");
        let table = wordcraft_doc::table::Table {
            grid: vec![144.0],
            rows: vec![wordcraft_doc::table::Row { cells: vec![wordcraft_doc::table::Cell::with_text("in a cell")], ..Default::default() }],
            ..Default::default()
        };
        doc.body = vec![
            Arc::new(wordcraft_doc::Block::Para(para)),
            Arc::new(wordcraft_doc::Block::Table(table)),
            Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::para::Paragraph::new())),
        ];
        doc.core.title = "Bridged".into();

        let bytes = crate::write(&doc).expect("writes");
        let back = crate::read(&bytes).expect("reads");
        let para = back.body[0].as_para().expect("paragraph");
        assert_eq!(para.text, "kept added");
        let ins = para.props_at(6).ins.and_then(|i| back.revisions.get(i as usize)).expect("a tracked insertion");
        assert_eq!((ins.kind, ins.author.as_str(), ins.date.as_str()), (RevisionKind::Insert, "Ada", "2025-03-04T05:06:00Z"));
        assert_eq!(para.props_at(1).ins, None);
        let table = back.body[1].as_table().expect("table");
        assert_eq!(table.rows[0].cells[0].blocks[0].as_para().map(|p| p.text.as_str()), Some("in a cell"));
        assert_eq!(back.core.title, "Bridged");
    }

    /// A non-finite number in WordCraft's model (JSON has none) is written as 0 instead of
    /// failing the save.
    #[test]
    fn non_finite_sizes_do_not_fail_a_save() {
        let mut doc = wordcraft_doc::Document::new();
        let mut para = wordcraft_doc::para::Paragraph::with_text("odd", Default::default());
        // Not an Option: JSON's `null` for NaN would fail to convert back to a number.
        para.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(f32::NAN));
        para.format(0, 3, &|c| c.size = Some(f32::INFINITY)).expect("format");
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(para))];
        let back = crate::read(&crate::write(&doc).expect("writes")).expect("reads");
        assert_eq!(back.body[0].as_para().map(|p| p.text.as_str()), Some("odd"));
    }
}

//! Bounded conversion between craft-wordmodel and WordCraft's document model.

use std::sync::Arc;

use craft_docx::DocxError;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub(crate) fn convert<T, U>(value: T) -> Result<U, DocxError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let bytes = serde_json::to_vec(&value).map_err(|error| DocxError::Xml(format!("shared model conversion: {error}")))?;
    drop(value);
    serde_json::from_slice(&bytes).map_err(|error| DocxError::Xml(format!("shared model conversion: {error}")))
}

pub(crate) fn convert_ref<T, U>(value: &T) -> Result<U, DocxError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(|error| DocxError::Xml(format!("shared model conversion: {error}")))?;
    serde_json::from_slice(&bytes).map_err(|error| DocxError::Xml(format!("shared model conversion: {error}")))
}

// ---------------------------------------------------------------------------
// From Shared Model -> WordCraft Document
// ---------------------------------------------------------------------------

fn blocks_from_shared(source: craft_wordmodel::Blocks) -> Result<wordcraft_doc::Blocks, DocxError> {
    source
        .into_iter()
        .map(|block| {
            let block = Arc::try_unwrap(block).unwrap_or_else(|shared| (*shared).clone());
            block_value_from_shared(block).map(Arc::new)
        })
        .collect()
}

fn block_value_from_shared(source: craft_wordmodel::Block) -> Result<wordcraft_doc::Block, DocxError> {
    match source {
        craft_wordmodel::Block::Para(paragraph) => paragraph_value_from_shared(paragraph).map(wordcraft_doc::Block::Para),
        craft_wordmodel::Block::Table(table) => table_value_from_shared(table).map(wordcraft_doc::Block::Table),
    }
}

fn paragraph_value_from_shared(source: craft_wordmodel::para::Paragraph) -> Result<wordcraft_doc::para::Paragraph, DocxError> {
    let craft_wordmodel::para::Paragraph { text, runs, props, mark, objects, section, .. } = source;
    let mut paragraph = wordcraft_doc::para::Paragraph::new();
    paragraph.text = text;
    paragraph.runs =
        runs.into_iter().map(|run| Ok(wordcraft_doc::para::Run { len: run.len, props: convert(run.props)? })).collect::<Result<_, DocxError>>()?;
    paragraph.props = convert(props)?;
    paragraph.mark = convert(mark)?;
    paragraph.objects = objects.into_iter().map(object_value_from_shared).collect::<Result<_, _>>()?;
    paragraph.section = section.map(|section| convert(*section).map(Box::new)).transpose()?;
    Ok(paragraph)
}

fn object_value_from_shared(source: craft_wordmodel::para::InlineObject) -> Result<wordcraft_doc::para::InlineObject, DocxError> {
    use craft_wordmodel::para::InlineObject as Source;
    use wordcraft_doc::para::InlineObject as Target;

    match source {
        Source::Image { media, w, h, alt, float, crop, fx, ole } => Ok(Target::Image {
            media,
            w,
            h,
            alt,
            float: convert(float)?,
            crop,
            fx: convert(fx)?,
            ole: ole.map(|o| convert(Arc::try_unwrap(o).unwrap_or_else(|s| (*s).clone())).map(Arc::new)).transpose()?,
        }),
        Source::Group { w, h, float, ch_w, ch_h, children, canvas } => Ok(Target::Group {
            w,
            h,
            float: convert(float)?,
            ch_w,
            ch_h,
            children: children.into_iter().map(group_child_value_from_shared).collect::<Result<_, _>>()?,
            canvas: canvas.map(convert).transpose()?,
        }),
        other => convert(other),
    }
}

fn group_child_value_from_shared(source: craft_wordmodel::para::GroupChild) -> Result<wordcraft_doc::para::GroupChild, DocxError> {
    Ok(wordcraft_doc::para::GroupChild { x: source.x, y: source.y, obj: object_value_from_shared(source.obj)? })
}

fn table_value_from_shared(source: craft_wordmodel::table::Table) -> Result<wordcraft_doc::table::Table, DocxError> {
    Ok(wordcraft_doc::table::Table {
        props: convert(source.props)?,
        grid: source.grid,
        rows: source.rows.into_iter().map(row_value_from_shared).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn row_value_from_shared(source: craft_wordmodel::table::Row) -> Result<wordcraft_doc::table::Row, DocxError> {
    Ok(wordcraft_doc::table::Row {
        props: convert(source.props)?,
        cells: source.cells.into_iter().map(cell_value_from_shared).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn cell_value_from_shared(source: craft_wordmodel::table::Cell) -> Result<wordcraft_doc::table::Cell, DocxError> {
    Ok(wordcraft_doc::table::Cell { props: convert(source.props)?, blocks: blocks_from_shared(source.blocks)?, controls: convert(source.controls)? })
}

fn part_value_from_shared(source: craft_wordmodel::Part) -> Result<wordcraft_doc::Part, DocxError> {
    Ok(wordcraft_doc::Part { kind: convert(source.kind)?, blocks: blocks_from_shared(source.blocks)?, body: convert(source.body)? })
}

fn glossary_value_from_shared(source: craft_wordmodel::BuildingBlock) -> Result<wordcraft_doc::BuildingBlock, DocxError> {
    Ok(wordcraft_doc::BuildingBlock {
        name: source.name,
        gallery: source.gallery,
        category: source.category,
        description: source.description,
        guid: source.guid,
        style: source.style,
        types: source.types,
        behaviors: source.behaviors,
        blocks: blocks_from_shared(source.blocks)?,
    })
}

fn missing_pictures_from_shared(source: craft_wordmodel::MissingPictures) -> Result<wordcraft_doc::MissingPictures, DocxError> {
    let mut target = wordcraft_doc::MissingPictures::default();
    target.truncated = source.truncated;
    target.total = source.total;
    target.main = source.main;
    target.part_names = source.part_names;
    target.media_parts = source.media_parts;
    for item in source.list {
        target.push(convert(item)?);
    }
    Ok(target)
}

/// Converts the shared reader's model without copying the full document or its embedded media.
pub fn from_shared_model(mut source: craft_wordmodel::Document) -> Result<wordcraft_doc::Document, DocxError> {
    let mut fit_state = None;
    let fit_key = source.passthrough.keys().find(|k| k.eq_ignore_ascii_case(crate::fit::PART)).cloned();
    if let Some(key) = fit_key {
        if let Some(xml_bytes) = source.passthrough.get(&key) {
            if let Some(state) = crate::fit::read_from_bytes(xml_bytes) {
                fit_state = Some(state);
                source.passthrough.remove(&key);
            }
        }
    }

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
        missing_pictures,
        ..
    } = source;

    let mut document = wordcraft_doc::Document::new();
    document.body = blocks_from_shared(body)?;
    document.last_section = convert(last_section)?;
    document.parts = parts.into_iter().map(|(id, part)| Ok((id, part_value_from_shared(part)?))).collect::<Result<_, DocxError>>()?;
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
    document.glossary = glossary.into_iter().map(glossary_value_from_shared).collect::<Result<_, _>>()?;
    document.missing_pictures = missing_pictures_from_shared(missing_pictures)?;
    document.fit_state = fit_state;
    Ok(document)
}

// ---------------------------------------------------------------------------
// To Shared Model (from WordCraft Document)
// ---------------------------------------------------------------------------

fn blocks_to_shared(source: &wordcraft_doc::Blocks) -> Result<craft_wordmodel::Blocks, DocxError> {
    source.iter().map(|block| block_value_to_shared(block).map(Arc::new)).collect()
}

fn block_value_to_shared(source: &wordcraft_doc::Block) -> Result<craft_wordmodel::Block, DocxError> {
    match source {
        wordcraft_doc::Block::Para(paragraph) => paragraph_value_to_shared(paragraph).map(craft_wordmodel::Block::Para),
        wordcraft_doc::Block::Table(table) => table_value_to_shared(table).map(craft_wordmodel::Block::Table),
    }
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn sanitize_para_props(mut props: wordcraft_doc::ParaProps) -> wordcraft_doc::ParaProps {
    if let Some(ls) = &mut props.line_spacing {
        match ls {
            wordcraft_doc::props::LineSpacing::Multiple(v) => *v = finite(*v),
            wordcraft_doc::props::LineSpacing::Exactly(v) => *v = finite(*v),
            wordcraft_doc::props::LineSpacing::AtLeast(v) => *v = finite(*v),
        }
    }
    if let Some(v) = &mut props.indent_left {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    if let Some(v) = &mut props.indent_right {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    if let Some(v) = &mut props.indent_first {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    if let Some(v) = &mut props.space_before {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    if let Some(v) = &mut props.space_after {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    props
}

fn sanitize_char_props(mut props: wordcraft_doc::CharProps) -> wordcraft_doc::CharProps {
    if let Some(v) = &mut props.size {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    if let Some(v) = &mut props.spacing {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    props
}

fn paragraph_value_to_shared(source: &wordcraft_doc::para::Paragraph) -> Result<craft_wordmodel::para::Paragraph, DocxError> {
    let mut paragraph = craft_wordmodel::para::Paragraph::new();
    paragraph.text = source.text.clone();
    paragraph.runs = source
        .runs
        .iter()
        .map(|run| Ok(craft_wordmodel::para::Run { len: run.len, props: convert(sanitize_char_props(run.props.clone()))? }))
        .collect::<Result<_, DocxError>>()?;
    paragraph.props = convert(sanitize_para_props(source.props.clone()))?;
    paragraph.mark = convert(sanitize_char_props(source.mark.clone()))?;
    paragraph.objects = source.objects.iter().map(object_value_to_shared).collect::<Result<_, _>>()?;
    paragraph.section = source.section.as_ref().map(|section| convert_ref(section.as_ref()).map(Box::new)).transpose()?;
    Ok(paragraph)
}

fn object_value_to_shared(source: &wordcraft_doc::para::InlineObject) -> Result<craft_wordmodel::para::InlineObject, DocxError> {
    use craft_wordmodel::para::InlineObject as Target;
    use wordcraft_doc::para::InlineObject as Source;

    match source {
        Source::Image { media, w, h, alt, float, crop, fx, ole } => Ok(Target::Image {
            media: media.clone(),
            w: finite(*w),
            h: finite(*h),
            alt: alt.clone(),
            float: convert_ref(float)?,
            crop: [finite(crop[0]), finite(crop[1]), finite(crop[2]), finite(crop[3])],
            fx: convert_ref(fx)?,
            ole: ole.as_ref().map(|o| convert_ref(o.as_ref()).map(Arc::new)).transpose()?,
        }),
        Source::Group { w, h, float, ch_w, ch_h, children, canvas } => Ok(Target::Group {
            w: *w,
            h: *h,
            float: convert_ref(float)?,
            ch_w: *ch_w,
            ch_h: *ch_h,
            children: children.iter().map(group_child_value_to_shared).collect::<Result<_, _>>()?,
            canvas: canvas.as_ref().map(convert_ref).transpose()?,
        }),
        other => convert_ref(other),
    }
}

fn group_child_value_to_shared(source: &wordcraft_doc::para::GroupChild) -> Result<craft_wordmodel::para::GroupChild, DocxError> {
    Ok(craft_wordmodel::para::GroupChild { x: source.x, y: source.y, obj: object_value_to_shared(&source.obj)? })
}

fn table_value_to_shared(source: &wordcraft_doc::table::Table) -> Result<craft_wordmodel::table::Table, DocxError> {
    Ok(craft_wordmodel::table::Table {
        props: convert_ref(&source.props)?,
        grid: source.grid.clone(),
        rows: source.rows.iter().map(row_value_to_shared).collect::<Result<_, _>>()?,
        controls: convert_ref(&source.controls)?,
    })
}

fn row_value_to_shared(source: &wordcraft_doc::table::Row) -> Result<craft_wordmodel::table::Row, DocxError> {
    Ok(craft_wordmodel::table::Row {
        props: convert_ref(&source.props)?,
        cells: source.cells.iter().map(cell_value_to_shared).collect::<Result<_, _>>()?,
        controls: convert_ref(&source.controls)?,
    })
}

fn cell_value_to_shared(source: &wordcraft_doc::table::Cell) -> Result<craft_wordmodel::table::Cell, DocxError> {
    Ok(craft_wordmodel::table::Cell {
        props: convert_ref(&source.props)?,
        blocks: blocks_to_shared(&source.blocks)?,
        controls: convert_ref(&source.controls)?,
    })
}

fn part_value_to_shared(source: &wordcraft_doc::Part) -> Result<craft_wordmodel::Part, DocxError> {
    Ok(craft_wordmodel::Part { kind: convert_ref(&source.kind)?, blocks: blocks_to_shared(&source.blocks)?, body: convert_ref(&source.body)? })
}

fn glossary_value_to_shared(source: &wordcraft_doc::BuildingBlock) -> Result<craft_wordmodel::BuildingBlock, DocxError> {
    Ok(craft_wordmodel::BuildingBlock {
        name: source.name.clone(),
        gallery: source.gallery.clone(),
        category: source.category.clone(),
        description: source.description.clone(),
        guid: source.guid.clone(),
        style: source.style.clone(),
        types: source.types.clone(),
        behaviors: source.behaviors.clone(),
        blocks: blocks_to_shared(&source.blocks)?,
    })
}

fn missing_pictures_to_shared(source: &wordcraft_doc::MissingPictures) -> Result<craft_wordmodel::MissingPictures, DocxError> {
    let mut target = craft_wordmodel::MissingPictures::default();
    target.truncated = source.truncated;
    target.total = source.total;
    target.main = source.main.clone();
    target.part_names = source.part_names.clone();
    target.media_parts = source.media_parts.clone();
    for item in &source.list {
        target.push(convert_ref(item)?);
    }
    Ok(target)
}

/// Converts a WordCraft document to the shared model for serialization.
pub fn to_shared_model(source: &wordcraft_doc::Document) -> Result<craft_wordmodel::Document, DocxError> {
    let mut document = craft_wordmodel::Document::new();
    document.body = blocks_to_shared(&source.body)?;
    document.last_section = convert_ref(&source.last_section)?;
    document.parts = source.parts.iter().map(|(&id, part)| Ok((id, part_value_to_shared(part)?))).collect::<Result<_, DocxError>>()?;
    document.styles = Arc::new(convert_ref(source.styles.as_ref())?);
    document.numbering = Arc::new(convert_ref(source.numbering.as_ref())?);
    document.comments = convert_ref(&source.comments)?;
    document.revisions = convert_ref(&source.revisions)?;
    document.settings = convert_ref(&source.settings)?;
    document.core = convert_ref(&source.core)?;
    document.sources = convert_ref(&source.sources)?;
    document.custom_props = convert_ref(&source.custom_props)?;
    document.footnote_separators = convert_ref(&source.footnote_separators)?;
    document.endnote_separators = convert_ref(&source.endnote_separators)?;
    document.media = source.media.clone();
    document.passthrough = source.passthrough.clone();
    document.glossary = source.glossary.iter().map(glossary_value_to_shared).collect::<Result<_, _>>()?;
    document.missing_pictures = missing_pictures_to_shared(&source.missing_pictures)?;

    // Handle fit_state preservation through custom XML passthrough
    if let Some(fit) = &source.fit_state {
        let xml = crate::fit::write(fit)?;
        document.passthrough.insert(crate::fit::PART.into(), Arc::new(xml));
        const CUSTOM_XML_ROOTS: &str = "wordcraft:customXml.roots";
        const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";

        let manifest_entry = format!("P\t{}\tapplication/xml\n", crate::fit::PART);
        let manifest = match document.passthrough.get_mut(EMBEDDED_PARTS) {
            Some(existing) => {
                let mut text = String::from_utf8_lossy(existing).to_string();
                if !text.contains(crate::fit::PART) {
                    text.push_str(&manifest_entry);
                }
                Arc::new(text.into_bytes())
            }
            None => Arc::new(manifest_entry.into_bytes()),
        };
        document.passthrough.insert(EMBEDDED_PARTS.into(), manifest);

        let root_entry = format!("D\thttp://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\t{}\n", crate::fit::PART);
        let roots = match document.passthrough.get_mut(CUSTOM_XML_ROOTS) {
            Some(existing) => {
                let mut text = String::from_utf8_lossy(existing).to_string();
                if !text.contains(crate::fit::PART) {
                    text.push_str(&root_entry);
                }
                Arc::new(text.into_bytes())
            }
            None => Arc::new(root_entry.into_bytes()),
        };
        document.passthrough.insert(CUSTOM_XML_ROOTS.into(), roots);
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridges_nested_stories_and_moves_media_both_directions() {
        let mut source = craft_wordmodel::Document::new();
        let long_text = "x".repeat(1 << 16);
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
        let image = Arc::new(vec![0x5a; 1 << 16]);
        source.media.insert("image1.png".into(), image.clone());

        let doc = from_shared_model(source).expect("from_shared_model");
        assert_eq!(doc.body.len(), 2);
        assert_eq!(doc.body[0].as_para().map(|p| p.text.as_str()), Some(long_text.as_str()));
        assert!(Arc::ptr_eq(doc.media.get("image1.png").expect("media"), &image));

        let back = to_shared_model(&doc).expect("to_shared_model");
        assert_eq!(back.body.len(), 2);
        assert_eq!(back.body[0].as_para().map(|p| p.text.as_str()), Some(long_text.as_str()));
        assert!(Arc::ptr_eq(back.media.get("image1.png").expect("media"), &image));
    }
}

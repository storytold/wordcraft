//! Bounded conversion between craft-wordmodel and WordCraft's document model.

use std::collections::{BTreeMap, HashMap};
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

type GraphicCache = HashMap<usize, Arc<wordcraft_doc::graphic::Graphic>>;

struct ReadCtx<'a> {
    graphics: GraphicCache,
    numbering: &'a wordcraft_doc::numbering::Numbering,
    originals: Option<BTreeMap<String, wordcraft_doc::props::NumRef>>,
    overrides: HashMap<String, crate::num_change::SnapshotEntry>,
    /// When `false`, the store was truncated by size/entry limits on write. Any
    /// `numberingChange` without a matching store entry is treated as unresolved rather than
    /// falling back to cache inference, to avoid wrong-list attachment.
    store_complete: bool,
}

impl<'a> ReadCtx<'a> {
    fn new(
        numbering: &'a wordcraft_doc::numbering::Numbering,
        overrides: HashMap<String, crate::num_change::SnapshotEntry>,
        store_complete: bool,
    ) -> Self {
        Self { graphics: GraphicCache::new(), numbering, originals: None, overrides, store_complete }
    }

    fn snapshot_for(&mut self, loc: &str, original: &str) -> Option<wordcraft_doc::props::NumRef> {
        if let Some(entry) = self.overrides.get(loc)
            && entry.original == original
        {
            return match entry.snapshot {
                Some(nr) if nr.level <= 8 && (nr.num == 0 || self.numbering.num(nr.num).is_some()) => Some(nr),
                _ => None,
            };
        }
        // When the store existed but was incomplete (truncated), treat a missing entry as
        // unresolved rather than falling back to cache inference, which could resolve an
        // edited-list paragraph to the sole remaining cache match in the wrong list.
        if !self.store_complete {
            return None;
        }
        self.previous_numbering(original)
    }

    fn previous_numbering(&mut self, original: &str) -> Option<wordcraft_doc::props::NumRef> {
        let original = original.trim();
        if original.is_empty() || original.eq_ignore_ascii_case("none") {
            return None;
        }
        let originals = self.originals.get_or_insert_with(|| self.numbering.change_originals());
        originals.get(original).copied()
    }
}

fn blocks_from_shared(source: craft_wordmodel::Blocks, prefix: &str, ctx: &mut ReadCtx<'_>) -> Result<wordcraft_doc::Blocks, DocxError> {
    source
        .into_iter()
        .enumerate()
        .map(|(i, block)| {
            let block = Arc::try_unwrap(block).unwrap_or_else(|shared| (*shared).clone());
            block_value_from_shared(block, &format!("{prefix}:{i}"), ctx).map(Arc::new)
        })
        .collect()
}

fn block_value_from_shared(source: craft_wordmodel::Block, loc: &str, ctx: &mut ReadCtx<'_>) -> Result<wordcraft_doc::Block, DocxError> {
    match source {
        craft_wordmodel::Block::Para(paragraph) => paragraph_value_from_shared(paragraph, loc, ctx).map(wordcraft_doc::Block::Para),
        craft_wordmodel::Block::Table(table) => table_value_from_shared(table, loc, ctx).map(wordcraft_doc::Block::Table),
    }
}

fn paragraph_value_from_shared(
    source: craft_wordmodel::para::Paragraph,
    loc: &str,
    ctx: &mut ReadCtx<'_>,
) -> Result<wordcraft_doc::para::Paragraph, DocxError> {
    let craft_wordmodel::para::Paragraph { text, runs, props, mark, objects, section, .. } = source;
    let mut paragraph = wordcraft_doc::para::Paragraph::new();
    paragraph.text = text;
    paragraph.runs =
        runs.into_iter().map(|run| Ok(wordcraft_doc::para::Run { len: run.len, props: convert(run.props)? })).collect::<Result<_, DocxError>>()?;
    paragraph.props = convert(props)?;
    if let Some(nc) = paragraph.props.num_change.as_deref_mut()
        && nc.snapshot.is_none()
    {
        nc.snapshot = ctx.snapshot_for(loc, &nc.original);
    }
    paragraph.mark = convert(mark)?;
    paragraph.objects = objects.into_iter().map(|o| object_value_from_shared(o, &mut ctx.graphics)).collect::<Result<_, _>>()?;
    paragraph.section = section.map(|section| convert(*section).map(Box::new)).transpose()?;
    Ok(paragraph)
}

fn object_value_from_shared(
    source: craft_wordmodel::para::InlineObject,
    graphics: &mut GraphicCache,
) -> Result<wordcraft_doc::para::InlineObject, DocxError> {
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
        Source::Graphic { w, h, alt, float, graphic } => {
            let key = Arc::as_ptr(&graphic) as usize;
            let graphic = if let Some(cached) = graphics.get(&key) {
                Arc::clone(cached)
            } else {
                let converted: Arc<wordcraft_doc::graphic::Graphic> = Arc::new(convert(Arc::try_unwrap(graphic).unwrap_or_else(|s| (*s).clone()))?);
                graphics.insert(key, Arc::clone(&converted));
                converted
            };
            Ok(Target::Graphic { w, h, alt, float: convert(float)?, graphic })
        }
        Source::Group { w, h, float, ch_w, ch_h, children, canvas } => Ok(Target::Group {
            w,
            h,
            float: convert(float)?,
            ch_w,
            ch_h,
            children: children.into_iter().map(|c| group_child_value_from_shared(c, graphics)).collect::<Result<_, _>>()?,
            canvas: canvas.map(convert).transpose()?,
        }),
        other => convert(other),
    }
}

fn group_child_value_from_shared(
    source: craft_wordmodel::para::GroupChild,
    graphics: &mut GraphicCache,
) -> Result<wordcraft_doc::para::GroupChild, DocxError> {
    Ok(wordcraft_doc::para::GroupChild { x: source.x, y: source.y, obj: object_value_from_shared(source.obj, graphics)? })
}

fn table_value_from_shared(
    source: craft_wordmodel::table::Table,
    loc: &str,
    ctx: &mut ReadCtx<'_>,
) -> Result<wordcraft_doc::table::Table, DocxError> {
    Ok(wordcraft_doc::table::Table {
        props: convert(source.props)?,
        grid: source.grid,
        rows: source.rows.into_iter().enumerate().map(|(r, row)| row_value_from_shared(row, &format!("{loc}/{r}"), ctx)).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn row_value_from_shared(source: craft_wordmodel::table::Row, loc: &str, ctx: &mut ReadCtx<'_>) -> Result<wordcraft_doc::table::Row, DocxError> {
    Ok(wordcraft_doc::table::Row {
        props: convert(source.props)?,
        cells: source
            .cells
            .into_iter()
            .enumerate()
            .map(|(c, cell)| cell_value_from_shared(cell, &format!("{loc}/{c}"), ctx))
            .collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn cell_value_from_shared(
    source: craft_wordmodel::table::Cell,
    prefix: &str,
    ctx: &mut ReadCtx<'_>,
) -> Result<wordcraft_doc::table::Cell, DocxError> {
    Ok(wordcraft_doc::table::Cell {
        props: convert(source.props)?,
        blocks: blocks_from_shared(source.blocks, prefix, ctx)?,
        controls: convert(source.controls)?,
    })
}

fn part_value_from_shared(source: craft_wordmodel::Part, prefix: &str, ctx: &mut ReadCtx<'_>) -> Result<wordcraft_doc::Part, DocxError> {
    Ok(wordcraft_doc::Part { kind: convert(source.kind)?, blocks: blocks_from_shared(source.blocks, prefix, ctx)?, body: convert(source.body)? })
}

fn glossary_value_from_shared(
    source: craft_wordmodel::BuildingBlock,
    prefix: &str,
    ctx: &mut ReadCtx<'_>,
) -> Result<wordcraft_doc::BuildingBlock, DocxError> {
    Ok(wordcraft_doc::BuildingBlock {
        name: source.name,
        gallery: source.gallery,
        category: source.category,
        description: source.description,
        guid: source.guid,
        style: source.style,
        types: source.types,
        behaviors: source.behaviors,
        blocks: blocks_from_shared(source.blocks, prefix, ctx)?,
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
        target.list.push(convert(item)?);
    }
    Ok(target)
}

/// Converts the shared reader's model without copying the full document or its embedded media.
pub fn from_shared_model(mut source: craft_wordmodel::Document) -> Result<wordcraft_doc::Document, DocxError> {
    let mut fit_state = None;
    let fit_key = source.passthrough.keys().find(|k| k.eq_ignore_ascii_case(crate::fit::PART)).cloned();
    if let Some(key) = fit_key
        && let Some(xml_bytes) = source.passthrough.get(&key)
        && let Some(state) = crate::fit::read_from_bytes(xml_bytes)
    {
        fit_state = Some(state);
        source.passthrough.remove(&key);
    }

    let mut num_overrides = HashMap::new();
    let mut store_complete = true;
    let parsed_store = source
        .passthrough
        .iter()
        .find_map(|(k, b)| {
            k.eq_ignore_ascii_case(crate::num_change::PART).then(|| crate::num_change::read_from_bytes(b).map(|s| (k.clone(), s))).flatten()
        })
        .or_else(|| {
            source.passthrough.iter().find_map(|(k, b)| {
                (k.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("customxml/")) && crate::num_change::is_store(b))
                    .then(|| crate::num_change::read_from_bytes(b).map(|s| (k.clone(), s)))
                    .flatten()
            })
        });
    if let Some((key, store)) = parsed_store {
        if !store.complete {
            // Truncated/incomplete store: fail closed rather than falling back to cache inference.
            store_complete = false;
        } else {
            // Verify the document fingerprint: if another editor changed the structure
            // (e.g. inserted a paragraph or renumbered lists), block positions or list
            // identities shifted and all overrides are invalid.
            // On mismatch, treat every numberingChange as unresolved instead of falling
            // back to cache inference, which would attach paragraphs to the wrong lists.
            let doc_fp = crate::num_change::fingerprint_shared_model(&source, store.entries.values());
            if store.fingerprint != Some(doc_fp) {
                store_complete = false;
            } else {
                num_overrides = store.entries;
                store_complete = true;
            }
        }
        // Only remove a part once it has actually parsed as our store, to avoid deleting
        // another editor's custom XML part that merely contains the marker string.
        crate::num_change::remove_custom_xml_part(&mut source.passthrough, &key);
    } else if source
        .passthrough
        .iter()
        .any(|(k, b)| k.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("customxml/")) && crate::num_change::has_store_root(b))
    {
        // A damaged/unparseable store root still existed: fail closed rather than guessing via cache.
        store_complete = false;
    }
    let part_labels = if num_overrides.is_empty() { HashMap::new() } else { crate::num_change::part_labels(&source) };

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
    let doc_numbering = Arc::new(convert::<_, wordcraft_doc::numbering::Numbering>((*numbering).clone())?);
    let mut ctx = ReadCtx::new(&doc_numbering, num_overrides, store_complete);
    document.body = blocks_from_shared(body, "b", &mut ctx)?;
    document.last_section = convert(last_section)?;
    document.parts = parts
        .into_iter()
        .map(|(id, part)| {
            let prefix = part_labels.get(&id).map_or("", String::as_str);
            Ok((id, part_value_from_shared(part, prefix, &mut ctx)?))
        })
        .collect::<Result<_, DocxError>>()?;
    document.styles = Arc::new(convert((*styles).clone())?);
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
    document.glossary =
        glossary.into_iter().enumerate().map(|(i, g)| glossary_value_from_shared(g, &format!("g{i}"), &mut ctx)).collect::<Result<_, _>>()?;
    drop(ctx);
    document.numbering = doc_numbering;
    document.missing_pictures = missing_pictures_from_shared(missing_pictures)?;
    document.fit_state = fit_state;
    Ok(document)
}

// ---------------------------------------------------------------------------
// To Shared Model (from WordCraft Document)
// ---------------------------------------------------------------------------

fn blocks_to_shared(source: &wordcraft_doc::Blocks, numbering: &wordcraft_doc::numbering::Numbering) -> Result<craft_wordmodel::Blocks, DocxError> {
    source.iter().map(|block| block_value_to_shared(block, numbering).map(Arc::new)).collect()
}

fn block_value_to_shared(
    source: &wordcraft_doc::Block,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::Block, DocxError> {
    match source {
        wordcraft_doc::Block::Para(paragraph) => paragraph_value_to_shared(paragraph, numbering).map(craft_wordmodel::Block::Para),
        wordcraft_doc::Block::Table(table) => table_value_to_shared(table, numbering).map(craft_wordmodel::Block::Table),
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
    for field in [&mut props.indent_left, &mut props.indent_right, &mut props.indent_first, &mut props.space_before, &mut props.space_after] {
        if let Some(v) = field
            && !v.is_finite()
        {
            *v = 0.0;
        }
    }
    props
}

fn sanitize_char_props(mut props: wordcraft_doc::CharProps) -> wordcraft_doc::CharProps {
    for field in [&mut props.size, &mut props.spacing] {
        if let Some(v) = field
            && !v.is_finite()
        {
            *v = 0.0;
        }
    }
    props
}

fn paragraph_value_to_shared(
    source: &wordcraft_doc::para::Paragraph,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::para::Paragraph, DocxError> {
    let mut paragraph = craft_wordmodel::para::Paragraph::new();
    paragraph.text = source.text.clone();
    paragraph.runs = source
        .runs
        .iter()
        .map(|run| Ok(craft_wordmodel::para::Run { len: run.len, props: convert(sanitize_char_props(run.props.clone()))? }))
        .collect::<Result<_, DocxError>>()?;
    let mut para_props = sanitize_para_props(source.props.clone());
    // An imported value is kept as written; one this app recorded is cached from the
    // previous list's definitions, in the format ECMA-376 Part 1 §17.3.1.19 / Part 4 §14.7.1.2 gives.
    if let Some(nc) = para_props.num_change.as_deref_mut()
        && nc.original.is_empty()
        && let Some(nr) = nc.snapshot
    {
        nc.original = numbering.change_original(nr.num, nr.level).unwrap_or_default();
    }
    paragraph.props = convert(para_props)?;
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

fn table_value_to_shared(
    source: &wordcraft_doc::table::Table,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::table::Table, DocxError> {
    Ok(craft_wordmodel::table::Table {
        props: convert_ref(&source.props)?,
        grid: source.grid.clone(),
        rows: source.rows.iter().map(|r| row_value_to_shared(r, numbering)).collect::<Result<_, _>>()?,
        controls: convert_ref(&source.controls)?,
    })
}

fn row_value_to_shared(
    source: &wordcraft_doc::table::Row,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::table::Row, DocxError> {
    Ok(craft_wordmodel::table::Row {
        props: convert_ref(&source.props)?,
        cells: source.cells.iter().map(|c| cell_value_to_shared(c, numbering)).collect::<Result<_, _>>()?,
        controls: convert_ref(&source.controls)?,
    })
}

fn cell_value_to_shared(
    source: &wordcraft_doc::table::Cell,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::table::Cell, DocxError> {
    Ok(craft_wordmodel::table::Cell {
        props: convert_ref(&source.props)?,
        blocks: blocks_to_shared(&source.blocks, numbering)?,
        controls: convert_ref(&source.controls)?,
    })
}

fn part_value_to_shared(source: &wordcraft_doc::Part, numbering: &wordcraft_doc::numbering::Numbering) -> Result<craft_wordmodel::Part, DocxError> {
    Ok(craft_wordmodel::Part {
        kind: convert_ref(&source.kind)?,
        blocks: blocks_to_shared(&source.blocks, numbering)?,
        body: convert_ref(&source.body)?,
    })
}

fn glossary_value_to_shared(
    source: &wordcraft_doc::BuildingBlock,
    numbering: &wordcraft_doc::numbering::Numbering,
) -> Result<craft_wordmodel::BuildingBlock, DocxError> {
    Ok(craft_wordmodel::BuildingBlock {
        name: source.name.clone(),
        gallery: source.gallery.clone(),
        category: source.category.clone(),
        description: source.description.clone(),
        guid: source.guid.clone(),
        style: source.style.clone(),
        types: source.types.clone(),
        behaviors: source.behaviors.clone(),
        blocks: blocks_to_shared(&source.blocks, numbering)?,
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
        target.list.push(convert_ref(item)?);
    }
    Ok(target)
}

/// Converts a WordCraft document to the shared model for serialization.
pub fn to_shared_model(source: &wordcraft_doc::Document) -> Result<craft_wordmodel::Document, DocxError> {
    let mut document = craft_wordmodel::Document::new();
    document.body = blocks_to_shared(&source.body, source.numbering.as_ref())?;
    document.last_section = convert_ref(&source.last_section)?;
    document.parts =
        source.parts.iter().map(|(&id, part)| Ok((id, part_value_to_shared(part, source.numbering.as_ref())?))).collect::<Result<_, DocxError>>()?;
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
    document.glossary = source.glossary.iter().map(|g| glossary_value_to_shared(g, source.numbering.as_ref())).collect::<Result<_, _>>()?;
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

    let stale_store_keys: Vec<String> = document
        .passthrough
        .iter()
        .filter(|(k, b)| {
            k.eq_ignore_ascii_case(crate::num_change::PART)
                && (crate::num_change::read_from_bytes(b).is_some() || crate::num_change::has_store_root(b))
        })
        .map(|(k, _)| k.clone())
        .collect();
    for key in stale_store_keys {
        crate::num_change::remove_custom_xml_part(&mut document.passthrough, &key);
    }
    let (overrides, complete) = crate::num_change::collect_overrides(source, &document);
    if !overrides.is_empty() || !complete {
        let fingerprint = crate::num_change::fingerprint_shared_model(&document, &overrides);
        if let Some(xml) = crate::num_change::write(&overrides, fingerprint, complete)? {
            let part = if !document.passthrough.keys().any(|k| k.eq_ignore_ascii_case(crate::num_change::PART)) {
                crate::num_change::PART.to_string()
            } else {
                (1..=1000)
                    .map(|i| format!("customxml/wordcraftnumchange-{i}.xml"))
                    .find(|cand| !document.passthrough.keys().any(|k| k.eq_ignore_ascii_case(cand)))
                    .ok_or_else(|| DocxError::Xml("no free customxml path for numbering-change snapshot store".into()))?
            };
            crate::num_change::insert_custom_xml_part(&mut document.passthrough, &part, xml);
        }
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(objects: Vec<craft_wordmodel::para::InlineObject>) -> craft_wordmodel::Document {
        let mut source = craft_wordmodel::Document::new();
        let mut para = craft_wordmodel::para::Paragraph::new();
        para.objects = objects;
        source.body = vec![Arc::new(craft_wordmodel::Block::Para(para))];
        source
    }

    fn objects(doc: &wordcraft_doc::Document) -> Vec<wordcraft_doc::para::InlineObject> {
        doc.body.first().and_then(|b| b.as_para()).map(|p| p.objects.clone()).unwrap_or_default()
    }

    fn chart_object(spec: craft_wordmodel::chart::ChartSpec) -> craft_wordmodel::para::InlineObject {
        let graphic = craft_wordmodel::graphic::Graphic { chart: Some(Arc::new(spec)), ..Default::default() };
        craft_wordmodel::para::InlineObject::Graphic { w: 100.0, h: 60.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) }
    }

    fn charts(doc: &wordcraft_doc::Document) -> Vec<Option<wordcraft_doc::chart::ChartSpec>> {
        objects(doc)
            .iter()
            .filter_map(|o| match o {
                wordcraft_doc::para::InlineObject::Graphic { graphic, .. } => Some(graphic.chart.as_deref().cloned()),
                _ => None,
            })
            .collect()
    }

    /// A picture frame shape opens as a frame and keeps its own border width.
    #[test]
    fn a_frame_shape_keeps_its_kind_and_border_width() {
        let shape = |kind| craft_wordmodel::para::InlineObject::Shape {
            kind,
            w: 40.0,
            h: 30.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: craft_wordmodel::connector::ShapeExtra { corner: Some(2604), ..Default::default() },
        };
        let doc = from_shared_model(doc_with(vec![shape(craft_wordmodel::para::ShapeKind::Frame)])).expect("from_shared_model");
        let found = objects(&doc);
        let [wordcraft_doc::para::InlineObject::Shape { kind, w, extra, .. }] = found.as_slice() else { panic!("{found:?}") };
        assert_eq!((*kind, *w, extra.corner), (wordcraft_doc::para::ShapeKind::Frame, 40.0, Some(2604)));
    }

    /// A chart's series formatting and axis settings survive the trip through this model, so
    /// saving writes the chart part the reader read.
    #[test]
    fn chart_formatting_survives_the_bridge() {
        use craft_wordmodel::chart::{ChartSpec, ChartType};
        let mut formatted = ChartSpec::sample(ChartType::Line);
        formatted.series[0].fill = Some(craft_wordmodel::props::Rgb(0x12, 0x34, 0x56));
        formatted.series[0].line_w = Some(2.5);
        formatted.series[1].smooth = true;
        formatted.series[1].markers = Some(true);
        formatted.series[2].labels = Some(true);
        formatted.series[2].fmt = Some("0.0".into());
        formatted.val_axis.title = Some("Sales".into());
        formatted.val_axis.min = Some(0.0);
        formatted.cat_axis.gridlines = true;
        let mut scatter_lines = ChartSpec::sample(ChartType::Scatter);
        scatter_lines.series[0].no_line = false;
        let all = [formatted, ChartSpec::sample(ChartType::Scatter), ChartSpec::sample(ChartType::Pie), scatter_lines];
        let doc = from_shared_model(doc_with(all.iter().cloned().map(chart_object).collect())).expect("from_shared_model");
        assert!(charts(&doc).iter().all(Option::is_some));
        let back = to_shared_model(&doc).expect("to_shared_model");
        let back: Vec<ChartSpec> = back
            .body
            .first()
            .and_then(|b| b.as_para())
            .map(|p| {
                p.objects
                    .iter()
                    .filter_map(|o| match o {
                        craft_wordmodel::para::InlineObject::Graphic { graphic, .. } => graphic.chart.as_deref().cloned(),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(back, all);
    }

    /// A scatter chart this model makes is written as markers without lines, the same part the
    /// shared model writes for its own new scatter chart.
    #[test]
    fn a_new_scatter_chart_is_written_without_lines() {
        let local = wordcraft_doc::chart::ChartSpec::sample(wordcraft_doc::chart::ChartType::Scatter);
        assert!(!local.series.is_empty() && local.series.iter().all(|s| s.no_line));
        assert_eq!(
            crate::chart_xml(&local),
            craft_docx::chart_xml(&craft_wordmodel::chart::ChartSpec::sample(craft_wordmodel::chart::ChartType::Scatter)).into_bytes()
        );
        for kind in wordcraft_doc::chart::ChartType::ALL {
            let local = wordcraft_doc::chart::ChartSpec::sample(kind);
            let shared: craft_wordmodel::chart::ChartSpec = convert_ref(&local).expect("convert");
            assert_eq!(shared, craft_wordmodel::chart::ChartSpec::sample(shared.kind), "{kind:?}");
        }
    }

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

    #[test]
    fn bridges_num_change_original_and_snapshot() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum {
            id: 20,
            levels: vec![Level { format: wordcraft_doc::section::NumFormat::UpperRoman, text: "%1.".into(), ..Default::default() }],
            ..Default::default()
        });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut para_with_active = wordcraft_doc::para::Paragraph::with_text("List changed", Default::default());
        para_with_active.props.numbering = Some(NumRef { num: 2, level: 0 });
        para_with_active.props.num_change =
            Some(Box::new(NumChange { rev: 1, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut para_removed = wordcraft_doc::para::Paragraph::with_text("List removed", Default::default());
        para_removed.props.num_change = Some(Box::new(NumChange { rev: 2, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(para_with_active)), Arc::new(wordcraft_doc::Block::Para(para_removed))];

        let shared = to_shared_model(&doc).expect("to_shared_model");
        let expected_original = doc.numbering.change_original(1, 0).expect("change_original");
        assert!(!expected_original.is_empty());
        assert_eq!(
            shared.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).map(|nc| nc.original.as_str()),
            Some(expected_original.as_str())
        );
        assert_eq!(
            shared.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).map(|nc| nc.original.as_str()),
            Some(expected_original.as_str())
        );

        let roundtripped = from_shared_model(shared).expect("from_shared_model");
        assert_eq!(
            roundtripped.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).and_then(|nc| nc.snapshot),
            Some(NumRef { num: 1, level: 0 })
        );
        assert_eq!(
            roundtripped.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).and_then(|nc| nc.snapshot),
            Some(NumRef { num: 1, level: 0 })
        );
    }

    #[test]
    fn bridges_num_change_when_edited_list_leaves_another_sole_cache_match() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        // List 1 is Decimal ("%1:1:0:."); List 2 was edited in place to UpperRoman ("%1:1:1:.").
        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum {
            id: 20,
            levels: vec![Level { format: wordcraft_doc::section::NumFormat::UpperRoman, text: "%1.".into(), ..Default::default() }],
            ..Default::default()
        });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Paragraph 0 retains its historical "%1:1:0:." original while referencing edited List 2.
        let mut p_known = wordcraft_doc::para::Paragraph::with_text("Known snapshot on edited list", Default::default());
        p_known.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        // Paragraph 1 had an unresolved ambiguous "%1:1:0:." original before List 2 was edited.
        let mut p_unresolved = wordcraft_doc::para::Paragraph::with_text("Unresolved foreign collision", Default::default());
        p_unresolved.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: None }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p_known)), Arc::new(wordcraft_doc::Block::Para(p_unresolved))];

        let bytes = crate::write(&doc).expect("write docx");
        let reopened = crate::read(&bytes).expect("read docx");
        assert!(
            !reopened.passthrough.keys().any(|k| k.eq_ignore_ascii_case(crate::num_change::PART)),
            "custom XML part removed from passthrough on read"
        );
        for bk in ["wordcraft:embedded.parts", "wordcraft:customXml.roots"] {
            assert!(
                !reopened.passthrough.get(bk).is_some_and(|b| String::from_utf8_lossy(b).contains(crate::num_change::PART)),
                "{bk} cleaned up on read"
            );
        }

        let nc0 = reopened.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc0");
        assert_eq!(nc0.original, "%1:1:0:.", "historical w:original preserved as written");
        assert_eq!(nc0.snapshot, Some(NumRef { num: 2, level: 0 }), "must restore edited list 2, not sole cache match list 1");

        let nc1 = reopened.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc1");
        assert_eq!(nc1.original, "%1:1:0:.");
        assert_eq!(nc1.snapshot, None, "previously unresolved collision must not attach to sole remaining match list 1");
    }

    /// When another editor inserts a paragraph before two colliding entries, the fingerprint
    /// changes and both overrides are invalidated to prevent wrong-list attachment.
    #[test]
    fn fingerprint_mismatch_invalidates_all_overrides() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        // Two lists with identical Decimal definitions ("%1:1:0:.").
        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Para 0: list A item, changed from list 1 → list 2 (snapshot list 1).
        let mut p0 = wordcraft_doc::para::Paragraph::with_text("Item in list A", Default::default());
        p0.props.numbering = Some(NumRef { num: 2, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        // Para 1: list B item, changed from list 2 → list 1 (snapshot list 2).
        let mut p1 = wordcraft_doc::para::Paragraph::with_text("Item in list B", Default::default());
        p1.props.numbering = Some(NumRef { num: 1, level: 0 });
        p1.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0)), Arc::new(wordcraft_doc::Block::Para(p1))];

        // Save to DOCX (store is written with two overrides and a fingerprint).
        let bytes = crate::write(&doc).expect("write docx");

        // Read back as shared model and insert a paragraph at the front to simulate another editor.
        let mut shared = craft_docx::read(&bytes).expect("read shared model");
        let inserted = craft_wordmodel::para_block(craft_wordmodel::para::Paragraph::with_text("Inserted by another editor", Default::default()));
        shared.body.insert(0, inserted);

        // Convert to WordCraft: the fingerprint should mismatch and all overrides are dropped.
        let reopened = from_shared_model(shared).expect("from_shared_model");

        // With overrides invalidated, the reader sets store_complete = false and fails closed
        // to None rather than falling back to cache inference.
        let nc0 = reopened.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc0 (now at index 1)");
        assert_eq!(nc0.snapshot, None, "fingerprint mismatch must invalidate overrides and fail closed to None");
        let nc1 = reopened.body[2].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc1 (now at index 2)");
        assert_eq!(nc1.snapshot, None, "fingerprint mismatch must invalidate overrides and fail closed to None");
    }

    /// Snapshot overrides for a header part round-trip through save/reopen.
    #[test]
    fn num_change_override_in_header_roundtrips() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Header part with two colliding numberingChange entries.
        let header_id = 100u32;
        let mut hp0 = wordcraft_doc::para::Paragraph::with_text("Header list A", Default::default());
        hp0.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));
        let mut hp1 = wordcraft_doc::para::Paragraph::with_text("Header list B", Default::default());
        hp1.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let header_part = wordcraft_doc::Part {
            kind: wordcraft_doc::PartKind::Header,
            blocks: vec![Arc::new(wordcraft_doc::Block::Para(hp0)), Arc::new(wordcraft_doc::Block::Para(hp1))],
            ..Default::default()
        };

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.last_section.headers.default = Some(header_id);
        doc.parts.insert(header_id, header_part);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(wordcraft_doc::para::Paragraph::with_text("Body", Default::default())))];

        let bytes = crate::write(&doc).expect("write docx");
        let reopened = crate::read(&bytes).expect("read docx");

        let reopened_header_id = reopened.last_section.headers.default.expect("reopened header id");
        let hdr = reopened.parts.get(&reopened_header_id).expect("header part");
        let nc0 = hdr.blocks[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("header nc0");
        assert_eq!(nc0.snapshot, Some(NumRef { num: 1, level: 0 }), "header override for list 1 must round-trip");
        let nc1 = hdr.blocks[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("header nc1");
        assert_eq!(nc1.snapshot, Some(NumRef { num: 2, level: 0 }), "header override for list 2 must round-trip");
    }

    /// Snapshot overrides for a table cell round-trip through save/reopen.
    #[test]
    fn num_change_override_in_table_cell_roundtrips() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Table with two colliding numberingChange entries in a cell.
        let mut tp0 = wordcraft_doc::para::Paragraph::with_text("Cell list A", Default::default());
        tp0.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));
        let mut tp1 = wordcraft_doc::para::Paragraph::with_text("Cell list B", Default::default());
        tp1.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let cell = wordcraft_doc::table::Cell {
            blocks: vec![Arc::new(wordcraft_doc::Block::Para(tp0)), Arc::new(wordcraft_doc::Block::Para(tp1))],
            ..Default::default()
        };
        let table =
            wordcraft_doc::table::Table { rows: vec![wordcraft_doc::table::Row { cells: vec![cell], ..Default::default() }], ..Default::default() };

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Table(table))];

        let bytes = crate::write(&doc).expect("write docx");
        let reopened = crate::read(&bytes).expect("read docx");

        let table = reopened.body[0].as_table().expect("table");
        let cell_blocks = &table.rows[0].cells[0].blocks;
        let nc0 = cell_blocks[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("cell nc0");
        assert_eq!(nc0.snapshot, Some(NumRef { num: 1, level: 0 }), "table cell override for list 1 must round-trip");
        let nc1 = cell_blocks[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("cell nc1");
        assert_eq!(nc1.snapshot, Some(NumRef { num: 2, level: 0 }), "table cell override for list 2 must round-trip");
    }

    /// When the custom XML store is marked `complete="false"` (truncated due to size/entry
    /// limits), any `numberingChange` without an entry must remain unresolved (`None`) rather
    /// than falling back to cache inference and attaching to a sole remaining match.
    #[test]
    fn incomplete_store_fails_closed_for_missing_entries() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        // List 1 is Decimal ("%1:1:0:."); List 2 was edited in place to UpperRoman.
        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum {
            id: 20,
            levels: vec![Level { format: wordcraft_doc::section::NumFormat::UpperRoman, text: "%1.".into(), ..Default::default() }],
            ..Default::default()
        });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.revisions.push(wordcraft_doc::Revision {
            kind: wordcraft_doc::RevisionKind::Format,
            author: "Alice".into(),
            date: "2026-01-01T00:00:00Z".into(),
            move_name: None,
        });
        doc.body = (0..10_001)
            .map(|i| {
                let mut p = wordcraft_doc::para::Paragraph::with_text(&format!("Item {i}"), Default::default());
                p.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));
                Arc::new(wordcraft_doc::Block::Para(p))
            })
            .collect();

        // Exceeding MAX_ENTRIES (10,000) writes a complete="false" stub.
        // On read, missing entries must fail closed to None and never fall back to List 1.
        let bytes = crate::write(&doc).expect("write docx");
        let reopened = crate::read(&bytes).expect("read docx");
        assert_eq!(reopened.body.len(), 10_001);
        let nc = reopened.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
        assert_eq!(nc.snapshot, None, "incomplete store must fail closed and not infer List 1");
    }

    /// When another editor inserts a paragraph above an item with an override, the fingerprint
    /// mismatch invalidates all overrides and fails closed to None rather than inferring the wrong list.
    #[test]
    fn paragraph_insertion_causes_fingerprint_mismatch_and_fails_closed() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum {
            id: 20,
            levels: vec![Level { format: wordcraft_doc::section::NumFormat::UpperRoman, text: "%1.".into(), ..Default::default() }],
            ..Default::default()
        });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut p = wordcraft_doc::para::Paragraph::with_text("Item with override", Default::default());
        p.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p))];

        let bytes = crate::write(&doc).expect("write docx");
        let mut shared = craft_docx::read(&bytes).expect("read shared model");

        // External edit: insert a new paragraph above the item
        shared
            .body
            .insert(0, Arc::new(craft_wordmodel::Block::Para(craft_wordmodel::para::Paragraph::with_text("Inserted above", Default::default()))));

        let reopened = from_shared_model(shared).expect("from_shared_model");
        let nc = reopened.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
        assert_eq!(nc.snapshot, None, "fingerprint mismatch must fail closed to None rather than inferring List 1");
    }

    /// Two identical lists sharing an nsid preserve overrides across save and reopen.
    #[test]
    fn two_lists_sharing_nsid_preserves_overrides_across_save_and_reopen() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        // Both abstracts share the same nsid "ABCD1234"
        numbering.abstracts.push(AbstractNum { id: 10, nsid: Some("ABCD1234".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, nsid: Some("ABCD1234".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut p0 = wordcraft_doc::para::Paragraph::with_text("Item on list 1", Default::default());
        p0.props.numbering = Some(NumRef { num: 2, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut p1 = wordcraft_doc::para::Paragraph::with_text("Item on list 2", Default::default());
        p1.props.numbering = Some(NumRef { num: 1, level: 0 });
        p1.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0)), Arc::new(wordcraft_doc::Block::Para(p1))];

        let bytes = crate::write(&doc).expect("write docx");
        let shared = craft_docx::read(&bytes).expect("read shared model");
        let reopened = from_shared_model(shared).expect("from_shared_model");

        let nc0 = reopened.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc0");
        assert_eq!(nc0.snapshot, Some(NumRef { num: 1, level: 0 }), "snapshot list 1 must roundtrip despite shared nsid");
        let nc1 = reopened.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc1");
        assert_eq!(nc1.snapshot, Some(NumRef { num: 2, level: 0 }), "snapshot list 2 must roundtrip despite shared nsid");
    }

    /// A store entry with num == 0 and level > 8 must be rejected by snapshot_for.
    #[test]
    fn store_entry_with_num_zero_and_level_above_eight_is_rejected() {
        use wordcraft_doc::numbering::Numbering;
        use wordcraft_doc::props::NumRef;

        let numbering = Numbering::default();
        let mut overrides = HashMap::new();
        overrides.insert(
            "b:0".into(),
            crate::num_change::SnapshotEntry { loc: "b:0".into(), original: "none".into(), snapshot: Some(NumRef { num: 0, level: 9 }) },
        );
        let mut ctx = ReadCtx::new(&numbering, overrides, true);
        assert_eq!(ctx.snapshot_for("b:0", "none"), None, "level 9 with num 0 must be rejected");
    }

    /// When another editor swaps w:numIds without moving any paragraph, the fingerprint
    /// mismatch invalidates all overrides so paragraphs do not attach to the wrong list.
    #[test]
    fn swapping_num_ids_in_shared_model_invalidates_all_overrides() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Para 0: list A item, changed from list 1 → list 2 (snapshot list 1).
        let mut p0 = wordcraft_doc::para::Paragraph::with_text("Item in list A", Default::default());
        p0.props.numbering = Some(NumRef { num: 2, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        // Para 1: list B item, changed from list 2 → list 1 (snapshot list 2).
        let mut p1 = wordcraft_doc::para::Paragraph::with_text("Item in list B", Default::default());
        p1.props.numbering = Some(NumRef { num: 1, level: 0 });
        p1.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0)), Arc::new(wordcraft_doc::Block::Para(p1))];

        let bytes = crate::write(&doc).expect("write docx");

        // Read back as shared model and swap the two nums' IDs without moving paragraphs.
        let mut shared = craft_docx::read(&bytes).expect("read shared model");
        assert_eq!(shared.numbering.nums.len(), 2);
        let id0 = shared.numbering.nums[0].id;
        let id1 = shared.numbering.nums[1].id;
        let numbering = Arc::make_mut(&mut shared.numbering);
        numbering.nums[0].id = id1;
        numbering.nums[1].id = id0;

        let reopened = from_shared_model(shared).expect("from_shared_model");
        let nc0 = reopened.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc0");
        assert_eq!(nc0.snapshot, None, "swapping num IDs must invalidate overrides and fail closed to None");
        let nc1 = reopened.body[1].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc1");
        assert_eq!(nc1.snapshot, None, "swapping num IDs must invalidate overrides and fail closed to None");
    }

    /// On an unedited file, the custom XML store fingerprint exactly matches the shared model
    /// fingerprint computed on read.
    #[test]
    fn fingerprint_matches_on_unedited_roundtrip() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut p0 = wordcraft_doc::para::Paragraph::with_text("Item 1", Default::default());
        p0.props.numbering = Some(NumRef { num: 2, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0))];

        let bytes = crate::write(&doc).expect("write docx");
        let shared = craft_docx::read(&bytes).expect("read shared model");

        // Extract the store from passthrough and verify the written fingerprint matches
        // the reader's recomputed fingerprint.
        let raw = shared.passthrough.get(crate::num_change::PART).expect("store part exists");
        let store = crate::num_change::read_from_bytes(raw).expect("store parses");
        let doc_fp = crate::num_change::fingerprint_shared_model(&shared, store.entries.values());
        assert_eq!(store.fingerprint, Some(doc_fp), "written fingerprint must match read-side recomputed fingerprint");

        // Verify full reopen restores snapshot properly
        let reopened = from_shared_model(shared).expect("from_shared_model");
        let nc = reopened.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
        assert_eq!(nc.snapshot, Some(NumRef { num: 1, level: 0 }));
    }

    /// A foreign `customxml/` part that happens to contain `wordcraft-numchange-v1` in its
    /// bytes but does not parse as our store must not be deleted on read.
    #[test]
    fn foreign_custom_xml_containing_marker_is_preserved() {
        let mut shared = craft_wordmodel::Document::new();
        shared.body = vec![craft_wordmodel::para_block(craft_wordmodel::para::Paragraph::with_text("Hello", Default::default()))];
        let foreign = br#"<?xml version="1.0" encoding="UTF-8"?><other kind="wordcraft-numchange-v1"/>"#.to_vec();
        shared.passthrough.insert("customxml/item1.xml".into(), Arc::new(foreign.clone()));

        let reopened = from_shared_model(shared).expect("from_shared_model");
        assert_eq!(
            reopened.passthrough.get("customxml/item1.xml").map(|b| b.as_slice()),
            Some(foreign.as_slice()),
            "foreign customXml part must not be removed"
        );
    }

    #[test]
    fn swapping_colliding_paragraphs_invalidates_overrides() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, nsid: Some("1111AAAA".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, nsid: Some("2222BBBB".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        // Both paragraphs have identical current numbering and identical w:original strings; only text & snapshots differ.
        let mut p0 = wordcraft_doc::para::Paragraph::with_text("First item", Default::default());
        p0.props.numbering = Some(NumRef { num: 2, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut p1 = wordcraft_doc::para::Paragraph::with_text("Second item", Default::default());
        p1.props.numbering = Some(NumRef { num: 2, level: 0 });
        p1.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0)), Arc::new(wordcraft_doc::Block::Para(p1))];

        let bytes = crate::write(&doc).expect("write docx");
        let mut shared = craft_docx::read(&bytes).expect("read shared model");
        shared.body.swap(0, 1);

        let reopened = from_shared_model(shared).expect("from_shared_model");
        for i in [0, 1] {
            let nc = reopened.body[i].as_para().and_then(|p| p.props.num_change.as_deref()).expect("num_change preserved");
            assert_eq!(nc.snapshot, None, "swapping colliding paragraphs must invalidate overrides for paragraph {i}");
        }
    }

    #[test]
    fn replacing_inferred_list_invalidates_store_and_inference() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        // Unique list 1 (inferred by original)
        numbering.abstracts.push(AbstractNum {
            id: 10,
            nsid: Some("1111AAAA".into()),
            levels: vec![Level { text: "%1)".into(), ..Level::default() }],
            ..Default::default()
        });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        // Colliding lists 2 and 3 (requires override)
        numbering.abstracts.push(AbstractNum { id: 20, nsid: Some("2222BBBB".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 30, nsid: Some("3333CCCC".into()), levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 3, abstract_id: 30, ..Default::default() });

        let mut p_inferred = wordcraft_doc::para::Paragraph::with_text("Inferred", Default::default());
        p_inferred.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:)".into(), snapshot: Some(NumRef { num: 1, level: 0 }) }));

        let mut p_override = wordcraft_doc::para::Paragraph::with_text("Override", Default::default());
        p_override.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 3, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering);
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p_inferred)), Arc::new(wordcraft_doc::Block::Para(p_override))];

        let bytes = crate::write(&doc).expect("write docx");
        let mut shared = craft_docx::read(&bytes).expect("read shared model");

        // Replace List 1's abstract level text while keeping override lists intact: even if the
        // modified abstract were replaced with another list matching "%1:1:0:)", the fingerprint must fail closed.
        let mut new_numbering = (*shared.numbering).clone();
        new_numbering.abstracts[0].levels[0].text = "[%1]".into();
        shared.numbering = Arc::new(new_numbering);

        let reopened = from_shared_model(shared).expect("from_shared_model");
        for i in [0, 1] {
            let nc = reopened.body[i].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
            assert_eq!(nc.snapshot, None, "modifying an inferred list invalidates store and disables inference");
        }
    }

    #[test]
    fn foreign_and_corrupted_custom_xml_edge_cases() {
        use wordcraft_doc::numbering::{AbstractNum, Level, Num, Numbering};
        use wordcraft_doc::props::{NumChange, NumRef};

        let mut numbering = Numbering::default();
        numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        numbering.abstracts.push(AbstractNum { id: 20, levels: vec![Level::default()], ..Default::default() });
        numbering.nums.push(Num { id: 2, abstract_id: 20, ..Default::default() });

        let mut p0 = wordcraft_doc::para::Paragraph::with_text("Item", Default::default());
        p0.props.numbering = Some(NumRef { num: 1, level: 0 });
        p0.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 2, level: 0 }) }));

        let mut doc = wordcraft_doc::Document::new();
        doc.numbering = Arc::new(numbering.clone());
        doc.body = vec![Arc::new(wordcraft_doc::Block::Para(p0.clone()))];

        // 1. Renamed store placed after a foreign part carrying our KIND marker.
        let bytes = crate::write(&doc).expect("write docx");
        let mut shared_renamed = craft_docx::read(&bytes).expect("read shared model");
        let store_bytes = shared_renamed.passthrough.remove(crate::num_change::PART).expect("store part");
        let foreign_marker = br#"<?xml version="1.0" encoding="UTF-8"?><other kind="wordcraft-numchange-v1"/>"#.to_vec();
        shared_renamed.passthrough.insert("customxml/item1.xml".into(), Arc::new(foreign_marker.clone()));
        shared_renamed.passthrough.insert("customxml/item2.xml".into(), store_bytes);

        let reopened_renamed = from_shared_model(shared_renamed).expect("reopen renamed");
        assert_eq!(reopened_renamed.passthrough.get("customxml/item1.xml").map(|b| b.as_slice()), Some(foreign_marker.as_slice()));
        assert!(!reopened_renamed.passthrough.contains_key("customxml/item2.xml"));
        let nc = reopened_renamed.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
        assert_eq!(nc.snapshot, Some(NumRef { num: 2, level: 0 }));

        // 2. Foreign custom XML at canonical PART path is preserved on export, while our store writes to an unused path.
        let mut doc_foreign_at_part = doc.clone();
        let foreign_at_part = br#"<?xml version="1.0" encoding="UTF-8"?><foreign/>"#.to_vec();
        crate::num_change::insert_custom_xml_part(&mut doc_foreign_at_part.passthrough, crate::num_change::PART, foreign_at_part.clone());
        let bytes_foreign = crate::write(&doc_foreign_at_part).expect("write with occupied PART");
        let shared_foreign = craft_docx::read(&bytes_foreign).expect("read shared model");
        assert_eq!(shared_foreign.passthrough.get(crate::num_change::PART).map(|b| b.as_slice()), Some(foreign_at_part.as_slice()));
        assert!(shared_foreign.passthrough.contains_key("customxml/wordcraftnumchange-1.xml"), "fallback customxml path written");
        let reopened_foreign = from_shared_model(shared_foreign).expect("reopen foreign PART");
        assert_eq!(reopened_foreign.passthrough.get(crate::num_change::PART).map(|b| b.as_slice()), Some(foreign_at_part.as_slice()));
        assert_eq!(
            reopened_foreign.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).and_then(|nc| nc.snapshot),
            Some(NumRef { num: 2, level: 0 })
        );

        // 3. Damaged <wordcraftNumChange part disables cache inference even when list is otherwise unique.
        let mut uniq_numbering = Numbering::default();
        uniq_numbering.abstracts.push(AbstractNum { id: 10, levels: vec![Level::default()], ..Default::default() });
        uniq_numbering.nums.push(Num { id: 1, abstract_id: 10, ..Default::default() });
        let mut p_uniq = wordcraft_doc::para::Paragraph::with_text("Unique", Default::default());
        p_uniq.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: Some(NumRef { num: 1, level: 0 }) }));
        let mut doc_uniq = wordcraft_doc::Document::new();
        doc_uniq.numbering = Arc::new(uniq_numbering);
        doc_uniq.body = vec![Arc::new(wordcraft_doc::Block::Para(p_uniq))];
        let bytes_uniq = crate::write(&doc_uniq).expect("write uniq");
        let mut shared_damaged = craft_docx::read(&bytes_uniq).expect("read uniq");
        shared_damaged.passthrough.insert("customxml/item1.xml".into(), Arc::new(b"<wordcraftNumChange broken".to_vec()));
        let reopened_damaged = from_shared_model(shared_damaged).expect("reopen damaged");
        let nc_damaged = reopened_damaged.body[0].as_para().and_then(|p| p.props.num_change.as_deref()).expect("nc");
        assert_eq!(nc_damaged.snapshot, None, "damaged store root must disable cache inference");
    }
}

//! Bounded conversion between craft-wordmodel and WordCraft's document model.

use std::collections::HashMap;
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

fn blocks_from_shared(source: craft_wordmodel::Blocks, graphics: &mut GraphicCache) -> Result<wordcraft_doc::Blocks, DocxError> {
    source
        .into_iter()
        .map(|block| {
            let block = Arc::try_unwrap(block).unwrap_or_else(|shared| (*shared).clone());
            block_value_from_shared(block, graphics).map(Arc::new)
        })
        .collect()
}

fn block_value_from_shared(source: craft_wordmodel::Block, graphics: &mut GraphicCache) -> Result<wordcraft_doc::Block, DocxError> {
    match source {
        craft_wordmodel::Block::Para(paragraph) => paragraph_value_from_shared(paragraph, graphics).map(wordcraft_doc::Block::Para),
        craft_wordmodel::Block::Table(table) => table_value_from_shared(table, graphics).map(wordcraft_doc::Block::Table),
    }
}

fn paragraph_value_from_shared(
    source: craft_wordmodel::para::Paragraph,
    graphics: &mut GraphicCache,
) -> Result<wordcraft_doc::para::Paragraph, DocxError> {
    let craft_wordmodel::para::Paragraph { text, runs, props, mark, objects, section, .. } = source;
    let mut paragraph = wordcraft_doc::para::Paragraph::new();
    paragraph.text = text;
    paragraph.runs =
        runs.into_iter().map(|run| Ok(wordcraft_doc::para::Run { len: run.len, props: convert(run.props)? })).collect::<Result<_, DocxError>>()?;
    paragraph.props = convert(props)?;
    paragraph.mark = convert(mark)?;
    paragraph.objects = objects.into_iter().map(|o| object_value_from_shared(o, graphics)).collect::<Result<_, _>>()?;
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

fn table_value_from_shared(source: craft_wordmodel::table::Table, graphics: &mut GraphicCache) -> Result<wordcraft_doc::table::Table, DocxError> {
    Ok(wordcraft_doc::table::Table {
        props: convert(source.props)?,
        grid: source.grid,
        rows: source.rows.into_iter().map(|r| row_value_from_shared(r, graphics)).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
        // The calculation-data mark is read from the content control and store, not from the table.
        calc: None,
    })
}

fn row_value_from_shared(source: craft_wordmodel::table::Row, graphics: &mut GraphicCache) -> Result<wordcraft_doc::table::Row, DocxError> {
    Ok(wordcraft_doc::table::Row {
        props: convert(source.props)?,
        cells: source.cells.into_iter().map(|c| cell_value_from_shared(c, graphics)).collect::<Result<_, _>>()?,
        controls: convert(source.controls)?,
    })
}

fn cell_value_from_shared(source: craft_wordmodel::table::Cell, graphics: &mut GraphicCache) -> Result<wordcraft_doc::table::Cell, DocxError> {
    Ok(wordcraft_doc::table::Cell {
        props: convert(source.props)?,
        blocks: blocks_from_shared(source.blocks, graphics)?,
        controls: convert(source.controls)?,
    })
}

fn part_value_from_shared(source: craft_wordmodel::Part, graphics: &mut GraphicCache) -> Result<wordcraft_doc::Part, DocxError> {
    Ok(wordcraft_doc::Part { kind: convert(source.kind)?, blocks: blocks_from_shared(source.blocks, graphics)?, body: convert(source.body)? })
}

fn glossary_value_from_shared(
    source: craft_wordmodel::BuildingBlock,
    graphics: &mut GraphicCache,
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
        blocks: blocks_from_shared(source.blocks, graphics)?,
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

/// Most custom XML parts looked at for a calculation store that isn't under its own name.
const MAX_STORE_CANDIDATES: usize = 256;
/// Most bytes of custom XML parts read looking for a renamed store.
const MAX_SCANNED_BYTES: usize = 32 * 1024 * 1024;

/// Take one of WordCraft's stores out of the kept parts: under the name `part` it was written
/// with, or (Word may rename the custom XML parts it keeps) any custom XML part holding one, told
/// by its `kind` marker and read with `read`. The part found is removed with what came with it
/// ([`forget_kept_part`]), so a save never writes a stale copy beside the new one; so is a part
/// under our name that doesn't hold a sound store. Anyone else's part is never touched. `None`
/// when there is no sound store.
fn take_store<T>(
    passthrough: &mut std::collections::BTreeMap<String, Arc<Vec<u8>>>,
    part: &str,
    kind: &str,
    read: fn(&[u8]) -> Option<T>,
) -> Option<T> {
    if let Some(key) = passthrough.keys().find(|k| k.eq_ignore_ascii_case(part)).cloned() {
        let store = passthrough.get(&key).and_then(|bytes| read(bytes));
        forget_kept_part(passthrough, &key);
        if store.is_some() {
            return store;
        }
    }
    let renamed = passthrough
        .iter()
        .filter(|(k, _)| k.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("customxml/")))
        .take(MAX_STORE_CANDIDATES)
        // At most `MAX_SCANNED_BYTES` of them, in all.
        .scan(0usize, |seen, (k, bytes)| {
            *seen = seen.saturating_add(bytes.len());
            (*seen <= MAX_SCANNED_BYTES).then_some((k, bytes))
        })
        // Cheap test first: a store names its kind near the start.
        .filter(|(_, bytes)| {
            let head = bytes.get(..bytes.len().min(512)).unwrap_or(bytes);
            head.windows(kind.len()).any(|w| w == kind.as_bytes())
        })
        .find_map(|(k, bytes)| read(bytes).map(|store| (k.clone(), store)));
    let (key, store) = renamed?;
    forget_kept_part(passthrough, &key);
    Some(store)
}

/// Drop every kept custom XML data store whose properties part names data store id `item_id`
/// (a copy of a store about to be written, which Word saved under its own names), with its
/// properties and relationships: a package never has the same id twice. Only the custom XML
/// parts are looked at, at most [`MAX_STORE_CANDIDATES`] of them.
fn forget_kept_stores_with_id(passthrough: &mut std::collections::BTreeMap<String, Arc<Vec<u8>>>, item_id: &str) {
    let needle = format!("itemID=\"{item_id}\"");
    let props: Vec<String> = passthrough
        .iter()
        .filter(|(k, _)| k.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("customxml/")))
        .take(MAX_STORE_CANDIDATES)
        .filter(|(_, b)| b.len() <= 64 * 1024 && b.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle.as_bytes())))
        .map(|(k, _)| k.clone())
        .collect();
    if props.is_empty() {
        return;
    }
    const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";
    let manifest = passthrough.get(EMBEDDED_PARTS).map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
    let items: Vec<String> = manifest
        .lines()
        .filter_map(|l| match l.split('\t').collect::<Vec<_>>().as_slice() {
            ["R", from, _, _, target, "I"] if props.iter().any(|p| p == target) => Some((*from).to_string()),
            _ => None,
        })
        .collect();
    for item in items.iter().chain(&props) {
        forget_kept_part(passthrough, item);
    }
}

/// Drop a kept custom XML part that is read into the model (one of WordCraft's stores), with what
/// came with it: its properties parts (`customXmlProps` relationships), their manifest lines and
/// the relationship to it. A save then carries properties only for the stores it writes, never a
/// stale one (or a second with the same data store id) beside them.
fn forget_kept_part(passthrough: &mut std::collections::BTreeMap<String, Arc<Vec<u8>>>, part: &str) {
    const CUSTOM_XML_ROOTS: &str = "wordcraft:customXml.roots";
    const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";
    passthrough.remove(part);
    let manifest = passthrough.get(EMBEDDED_PARTS).map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
    // `R\tpart\tid\ttype\ttarget\tI|E`: the part's internal properties parts.
    let props: Vec<String> = manifest
        .lines()
        .filter_map(|l| match l.split('\t').collect::<Vec<_>>().as_slice() {
            ["R", from, _, kind, target, "I"] if *from == part && kind.ends_with("/customXmlProps") => Some((*target).to_string()),
            _ => None,
        })
        .collect();
    for p in &props {
        passthrough.remove(p);
    }
    let gone = |name: &str| name == part || props.iter().any(|p| p == name);
    let kept: String = manifest
        .lines()
        .filter(|l| match l.split('\t').collect::<Vec<_>>().as_slice() {
            ["P", name, _] => !gone(name),
            ["R", from, ..] => !gone(from),
            _ => true,
        })
        .flat_map(|l| [l, "\n"])
        .collect();
    set_or_remove(passthrough, EMBEDDED_PARTS, kept);
    if let Some(roots) = passthrough.get(CUSTOM_XML_ROOTS).map(|b| String::from_utf8_lossy(b).to_string()) {
        let kept: String = roots.lines().filter(|l| l.rsplit('\t').next() != Some(part)).flat_map(|l| [l, "\n"]).collect();
        set_or_remove(passthrough, CUSTOM_XML_ROOTS, kept);
    }
}

/// Keep `text` under `key`, or nothing when it is empty.
fn set_or_remove(passthrough: &mut std::collections::BTreeMap<String, Arc<Vec<u8>>>, key: &str, text: String) {
    if text.is_empty() {
        passthrough.remove(key);
    } else {
        passthrough.insert(key.into(), Arc::new(text.into_bytes()));
    }
}

/// Converts the shared reader's model without copying the full document or its embedded media.
pub fn from_shared_model(mut source: craft_wordmodel::Document) -> Result<wordcraft_doc::Document, DocxError> {
    let fit_state = take_store(&mut source.passthrough, crate::fit::PART, crate::fit::KIND, crate::fit::read_from_bytes);
    let calc_store = take_store(&mut source.passthrough, crate::calc_store::PART, crate::calc_store::KIND, crate::calc_store::read_from_bytes);

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

    let mut graphics = GraphicCache::new();
    let mut document = wordcraft_doc::Document::new();
    document.body = blocks_from_shared(body, &mut graphics)?;
    document.last_section = convert(last_section)?;
    document.parts = parts.into_iter().map(|(id, part)| Ok((id, part_value_from_shared(part, &mut graphics)?))).collect::<Result<_, DocxError>>()?;
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
    document.glossary = glossary.into_iter().map(|g| glossary_value_from_shared(g, &mut graphics)).collect::<Result<_, _>>()?;
    document.missing_pictures = missing_pictures_from_shared(missing_pictures)?;
    document.fit_state = fit_state;
    document.calc_store = calc_store;
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
        target.list.push(convert_ref(item)?);
    }
    Ok(target)
}

/// Have the writer save `xml` as custom XML part `part` of the main document, a proper custom
/// XML data store as Word keeps them: its bytes, its properties part (`ds:datastoreItem` with the
/// store's id `item_id` and its namespace `ns` as schema reference) related from it, their
/// manifest entries (with content types) and the main document's relationship, each added once.
fn keep_custom_xml_part(document: &mut craft_wordmodel::Document, part: &str, ns: &str, item_id: &str, xml: Vec<u8>) {
    const CUSTOM_XML_ROOTS: &str = "wordcraft:customXml.roots";
    const EMBEDDED_PARTS: &str = "wordcraft:embedded.parts";
    const PROPS_TYPE: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
    const PROPS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";
    const CUSTOM_XML_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";

    // `customxml/wordcraftcalc.xml` → `customxml/wordcraftcalcProps.xml`.
    let props_part = format!("{}Props.xml", part.strip_suffix(".xml").unwrap_or(part));
    let props = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?><ds:datastoreItem ds:itemID="{item_id}" xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs><ds:schemaRef ds:uri="{ns}"/></ds:schemaRefs></ds:datastoreItem>"#
    );
    forget_kept_stores_with_id(&mut document.passthrough, item_id);
    document.passthrough.insert(part.into(), Arc::new(xml));
    document.passthrough.insert(props_part.clone(), Arc::new(props.into_bytes()));
    // One relationship from the main document, however the one read with it spelled its type.
    let related = document
        .passthrough
        .get(CUSTOM_XML_ROOTS)
        .is_some_and(|b| String::from_utf8_lossy(b).lines().any(|l| l.starts_with("D\t") && l.rsplit('\t').next() == Some(part)));
    // The store's relationships are ours alone: any read with it (Word may renumber them) give way
    // to the one below, so it never has two properties parts.
    let own_rels = format!("R\t{part}\t");
    let mut add_lines = |key: &str, lines: &[String]| {
        let kept = document.passthrough.get(key).map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
        let mut text: String = kept.lines().filter(|l| !l.starts_with(&own_rels)).flat_map(|l| [l, "\n"]).collect();
        for line in lines {
            if !text.lines().any(|l| l == line) {
                text.push_str(line);
                text.push('\n');
            }
        }
        document.passthrough.insert(key.into(), Arc::new(text.into_bytes()));
    };
    add_lines(
        EMBEDDED_PARTS,
        &[
            format!("P\t{part}\tapplication/xml"),
            format!("P\t{props_part}\t{PROPS_TYPE}"),
            format!("R\t{part}\trIdWordCraftProps\t{PROPS_REL}\t{props_part}\tI"),
        ],
    );
    if !related {
        add_lines(CUSTOM_XML_ROOTS, &[format!("D\t{CUSTOM_XML_REL}\t{part}")]);
    }
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

    // WordCraft's own stores travel as custom XML parts.
    if let Some(fit) = &source.fit_state {
        keep_custom_xml_part(&mut document, crate::fit::PART, crate::fit::NS, crate::fit::ITEM_ID, crate::fit::write(fit)?);
    }
    if let Some(store) = source.calc_store.as_ref().filter(|s| !s.is_empty()) {
        keep_custom_xml_part(
            &mut document,
            crate::calc_store::PART,
            crate::calc_store::NS,
            crate::calc_store::ITEM_ID,
            crate::calc_store::write(store)?,
        );
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
}

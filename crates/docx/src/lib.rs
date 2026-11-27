//! WordCraft DOCX reader and writer (ECMA-376 / ISO/IEC 29500 WordprocessingML).
//!
//! Delegates file-format parsing and serialization to `craft-docx`, adapting
//! between the shared `craft-wordmodel` and WordCraft's application document model.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fit;
mod model_bridge;
pub mod units;

/// Maximum nesting depth of DrawingML `wpg:wgp` / `wpg:grpSp` groups inside a top-level group:
/// bounded well below the XML parser's 256-element recursion cap while supporting deep diagram/CAD
/// group hierarchies.
const MAX_DML_GROUP_DEPTH: usize = 64;

const ZIP_LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: 10_000, max_total_bytes: 256 << 20 };

fn normalize_deep_groups_el(el: &mut craft_xml::El, group_depth: usize) -> bool {
    let mut changed = false;
    let old_children = std::mem::take(&mut el.children);
    let mut new_children = Vec::with_capacity(old_children.len());
    for node in old_children {
        match node {
            craft_xml::Node::El(mut child) => {
                let is_grp = child.name == "wpg:wgp" || child.name == "wpg:grpSp";
                if is_grp {
                    let next_depth = group_depth + 1;
                    if next_depth > MAX_DML_GROUP_DEPTH + 1 {
                        changed = true;
                        continue;
                    }
                    if normalize_deep_groups_el(&mut child, next_depth) {
                        changed = true;
                    }
                    if next_depth > 8 {
                        changed = true;
                        for grand in child.children {
                            if let craft_xml::Node::El(ref g) = grand
                                && (g.name == "wpg:cNvGrpSpPr" || g.name == "wpg:grpSpPr")
                            {
                                continue;
                            }
                            new_children.push(grand);
                        }
                    } else {
                        new_children.push(craft_xml::Node::El(child));
                    }
                } else {
                    if normalize_deep_groups_el(&mut child, group_depth) {
                        changed = true;
                    }
                    new_children.push(craft_xml::Node::El(child));
                }
            }
            other => new_children.push(other),
        }
    }
    el.children = new_children;
    changed
}

fn normalize_deep_groups_xml(data: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(data).ok()?;
    if !s.contains("wpg:wgp") && !s.contains("wpg:grpSp") {
        return None;
    }
    let mut root = craft_xml::parse(data).ok()?;
    if !normalize_deep_groups_el(&mut root, 0) {
        return None;
    }
    let mut out = String::with_capacity(data.len());
    out.push_str(craft_xml::DECL_STANDALONE);
    root.write_xml(&mut out);
    Some(out.into_bytes())
}

fn preprocess_deep_groups(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut archive = craft_zip::Archive::open(bytes, ZIP_LIMITS).ok()?;
    let mut any_changed = false;
    let mut entries = Vec::with_capacity(archive.len());
    for idx in 0..archive.len() {
        let name = archive.entry_name(idx).ok()??;
        if archive.entry_is_dir(idx).ok()?? {
            continue;
        }
        let stored = archive.is_stored(&name).unwrap_or(false);
        let mut data = archive.read(&name, craft_docx::MAX_XML_FILE as u64).ok()??;
        if name.starts_with("word/")
            && name.ends_with(".xml")
            && let Some(rewritten) = normalize_deep_groups_xml(&data)
        {
            data = rewritten;
            any_changed = true;
        }
        let comp = if stored { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        entries.push((name, data, comp));
    }
    if !any_changed {
        return None;
    }
    let mut out = craft_zip::Writer::new();
    for (name, data, comp) in entries {
        out.add(&name, &data, comp).ok()?;
    }
    out.finish().ok()
}

pub use craft_docx::{
    DEFAULT_SPIN_COUNT, DocxError, EmbeddedFont, Flavor, MAX_PASSWORD_CHARS, MAX_SOURCES, MAX_XML_FILE, PASSWORD_REQUIRED, PasswordCheck,
    WRONG_PASSWORD, XmlKind, check_password, check_protection_password, encrypt, encrypt_standard_for_tests, encrypt_with_spin_count, package_title,
    protection_hash, sniff_xml,
};

pub fn decrypt(bytes: &[u8], password: Option<&str>) -> Result<Vec<u8>, DocxError> {
    wordcraft_doc::cfb_guard::check(bytes).map_err(DocxError::Xml)?;
    craft_docx::decrypt(bytes, password)
}

pub fn is_encrypted(bytes: &[u8]) -> bool {
    wordcraft_doc::cfb_guard::check(bytes).is_ok() && craft_docx::is_encrypted(bytes)
}

pub use model_bridge::{from_shared_model, to_shared_model};

/// An alternative format chunk's content, as a [`ChunkImporter`] gets it.
pub struct AltChunk<'a> {
    pub content_type: &'a str,
    pub part: &'a str,
    pub bytes: &'a [u8],
}

/// Turns a chunk into a document; `None` leaves the chunk out.
pub type ChunkImporter<'a> = &'a dyn Fn(&AltChunk) -> Option<wordcraft_doc::Document>;

/// How [`read_with`] reads a package.
#[derive(Clone, Copy, Default)]
pub struct ReadOptions<'a> {
    pub password: Option<&'a str>,
    pub chunks: Option<ChunkImporter<'a>>,
}

/// Read a package into WordCraft's document model.
pub fn read(bytes: &[u8]) -> Result<wordcraft_doc::Document, DocxError> {
    read_with(bytes, &Default::default())
}

/// Read a package with a password.
pub fn read_with_password(bytes: &[u8], password: Option<&str>) -> Result<wordcraft_doc::Document, DocxError> {
    read_with(bytes, &ReadOptions { password, ..Default::default() })
}

/// Read a package with options.
pub fn read_with(bytes: &[u8], opts: &ReadOptions) -> Result<wordcraft_doc::Document, DocxError> {
    let bridge_importer = |shared_chunk: &craft_docx::AltChunk| -> Option<craft_wordmodel::Document> {
        let importer = opts.chunks?;
        let chunk = AltChunk { content_type: shared_chunk.content_type, part: shared_chunk.part, bytes: shared_chunk.bytes };
        let doc = importer(&chunk)?;
        to_shared_model(&doc).ok()
    };

    let shared_opts =
        craft_docx::ReadOptions { password: opts.password, chunks: opts.chunks.as_ref().map(|_| &bridge_importer as craft_docx::ChunkImporter) };

    let rewritten = preprocess_deep_groups(bytes);
    let bytes = rewritten.as_deref().unwrap_or(bytes);
    let shared = craft_docx::read_with(bytes, &shared_opts)?;
    from_shared_model(shared)
}

/// Read a Word XML file (`.xml`).
pub fn read_xml(bytes: &[u8]) -> Result<wordcraft_doc::Document, DocxError> {
    let shared = craft_docx::read_xml(bytes)?;
    from_shared_model(shared)
}

/// Write a document as `.docx`.
pub fn write(doc: &wordcraft_doc::Document) -> Result<Vec<u8>, DocxError> {
    let shared = to_shared_model(doc)?;
    craft_docx::write(&shared)
}

/// Write a document with a specific package flavor.
pub fn write_as(doc: &wordcraft_doc::Document, flavor: Flavor) -> Result<Vec<u8>, DocxError> {
    let shared = to_shared_model(doc)?;
    craft_docx::write_as(&shared, flavor)
}

/// Write a document as a Flat OPC XML file.
pub fn write_flat(doc: &wordcraft_doc::Document) -> Result<Vec<u8>, DocxError> {
    let shared = to_shared_model(doc)?;
    craft_docx::write_flat(&shared)
}

/// Write a document as Word 2003 XML.
pub fn write_wordml2003(doc: &wordcraft_doc::Document) -> Result<Vec<u8>, DocxError> {
    let shared = to_shared_model(doc)?;
    craft_docx::write_wordml2003(&shared)
}

/// Draw a chart.
pub fn chart_items(spec: &wordcraft_doc::chart::ChartSpec, theme: &[wordcraft_doc::Rgb], w: f32, h: f32) -> Vec<wordcraft_doc::graphic::GraphicItem> {
    let (Ok(shared_spec), Ok(shared_theme)) = (
        model_bridge::convert_ref::<_, craft_wordmodel::chart::ChartSpec>(spec),
        model_bridge::convert_ref::<_, Vec<craft_wordmodel::props::Rgb>>(theme),
    ) else {
        return Vec::new();
    };
    let shared_items = craft_docx::chart_items(&shared_spec, &shared_theme, w, h);
    model_bridge::convert(shared_items).unwrap_or_default()
}

/// Write a chart part.
pub fn chart_xml(spec: &wordcraft_doc::chart::ChartSpec) -> Vec<u8> {
    let Ok(shared_spec) = model_bridge::convert_ref(spec) else {
        return Vec::new();
    };
    craft_docx::chart_xml(&shared_spec).into_bytes()
}

/// Draw SmartArt.
pub fn smart_art_items(
    spec: &wordcraft_doc::smart_art::SmartArtSpec,
    theme: &[wordcraft_doc::Rgb],
    w: f32,
    h: f32,
) -> Vec<wordcraft_doc::graphic::GraphicItem> {
    let (Ok(shared_spec), Ok(shared_theme)) = (
        model_bridge::convert_ref::<_, craft_wordmodel::smart_art::SmartArtSpec>(spec),
        model_bridge::convert_ref::<_, Vec<craft_wordmodel::props::Rgb>>(theme),
    ) else {
        return Vec::new();
    };
    let shared_items = craft_docx::smart_art_items(&shared_spec, &shared_theme, w, h);
    model_bridge::convert(shared_items).unwrap_or_default()
}

/// Read bibliography sources from a `b:Sources` file.
pub fn read_sources(bytes: &[u8]) -> Result<Sources, DocxError> {
    let shared = craft_docx::read_sources(bytes)?;
    Ok(Sources { sources: model_bridge::convert(shared.sources).map_err(|e| DocxError::Xml(e.to_string()))?, style: shared.style })
}

/// The font files embedded in the document's font table.
pub fn embedded_fonts(doc: &wordcraft_doc::Document) -> Vec<EmbeddedFont> {
    let mut shared = craft_wordmodel::Document::new();
    shared.passthrough = doc.passthrough.clone();
    craft_docx::embedded_fonts(&shared)
}

/// A `b:Sources` document read into the document model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    pub sources: Vec<wordcraft_doc::Source>,
    pub style: Option<String>,
}

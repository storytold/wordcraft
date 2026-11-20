//! WordCraft DOCX reader and writer (ECMA-376 / ISO/IEC 29500 WordprocessingML).
//!
//! Delegates file-format parsing and serialization to `craft-docx`, adapting
//! between the shared `craft-wordmodel` and WordCraft's application document model.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fit;
mod model_bridge;

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

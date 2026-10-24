//! WordCraft SVG pictures (#1910): reads an SVG file safely into a parsed tree (usvg), gives its
//! natural size, and rasterises it (resvg) at any resolution, so the canvas can draw it at device
//! resolution and the writers can store a PNG fallback where a format can't hold SVG.
//!
//! SVG is untrusted input. The reader never touches the file system or the network: `<image>`
//! links to files or URLs are ignored and only `data:` pictures are drawn; compressed SVG (SVGZ)
//! is refused, so a gzip bomb can't expand; the file size, element nesting and raster size are
//! capped, and entities that could expand past a bound are refused. Text in an SVG draws with the
//! fonts documents use (`wordcraft-fonts`, with Word's substitutions), loaded into usvg's otherwise
//! empty font database on demand; system fonts are never scanned for it.
//! Every failure is an [`Error`], never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod cost;
mod fonts;
mod refs;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};

pub use usvg;

/// Largest SVG file read (bytes).
pub const MAX_SVG_BYTES: usize = 32 << 20;
/// Deepest element nesting accepted (usvg allows 1024, which recursion on a small thread stack
/// can't always afford).
pub const MAX_DEPTH: usize = 256;
/// Most pixels one raster may have (8 megapixels: a 4K screen).
pub const MAX_RASTER_PIXELS: u64 = 8 * 1024 * 1024;
/// Longest side of a PNG fallback (pixels).
pub const MAX_FALLBACK_SIDE: u32 = 2048;
/// Most memory resvg may take to draw one raster (bytes, an upper bound worked out from the tree,
/// [`cost`]); past it the picture is drawn smaller and scaled up, or not at all.
pub const MAX_DRAW_BYTES: u64 = 512 << 20;

/// Why an SVG could not be read or drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not an SVG file (or a compressed one).
    NotSvg,
    /// Larger than [`MAX_SVG_BYTES`].
    TooLarge,
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// The XML or SVG could not be parsed.
    Parse(String),
    /// A raster could not be made (empty or oversized).
    Raster,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotSvg => f.write_str("not an SVG picture"),
            Error::TooLarge => f.write_str("SVG picture is too large"),
            Error::TooDeep => f.write_str("SVG picture is nested too deeply"),
            Error::Parse(e) => write!(f, "can't read the SVG picture: {e}"),
            Error::Raster => f.write_str("can't draw the SVG picture"),
        }
    }
}

impl std::error::Error for Error {}

/// A parsed SVG picture.
pub struct Svg {
    tree: usvg::Tree,
}

/// A raster of an SVG: premultiplied RGBA8 rows, `width` × `height`.
#[derive(Debug, Clone)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// How far into a file the `svg` root element is looked for (past the XML declaration, comments
/// and a document type declaration).
const SNIFF_BYTES: usize = 64 << 10;
/// Most nodes the XML parser builds (usvg's own limit is a million elements).
const MAX_XML_NODES: u32 = 4_000_000;
/// SVG pictures inside SVG pictures (`data:` links) are drawn this many levels deep.
const MAX_NESTED: u32 = 2;
/// Natural sizes are scaled down to this many points on the longer side (22 inches).
const MAX_NATURAL_PT: f32 = 1584.0;

/// Whether `bytes` look like an (uncompressed) SVG file: XML text whose first element is `svg`.
pub fn is_svg(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    // Cheap first: a bitmap (PNG, JPEG…) doesn't start with `<`.
    if b.iter().find(|c| !c.is_ascii_whitespace()) != Some(&b'<') {
        return false;
    }
    let head = String::from_utf8_lossy(b.get(..b.len().min(SNIFF_BYTES)).unwrap_or(b));
    let mut t = head.trim_start();
    // Skip the prolog: the XML declaration, processing instructions, comments and a DOCTYPE.
    loop {
        let skip = if t.starts_with("<?") {
            t.find("?>").map(|i| i + 2)
        } else if t.starts_with("<!--") {
            t.find("-->").map(|i| i + 3)
        } else if t.get(..9).is_some_and(|d| d.eq_ignore_ascii_case("<!DOCTYPE")) {
            // An internal subset (a `[` before the first `>`) ends at `]>`. Searches stay within
            // the declaration, so many declarations scan in linear time.
            t.find('>').and_then(|close| match t.get(..close).and_then(|d| d.find('[')) {
                Some(open) => t.get(open..).and_then(|r| r.find("]>")).map(|i| open + i + 2),
                None => Some(close + 1),
            })
        } else {
            break;
        };
        match skip.and_then(|i| t.get(i..)) {
            Some(rest) => t = rest.trim_start(),
            None => return false,
        }
    }
    // The root element's name: `svg`, or `prefix:svg`.
    let name: &str = t.strip_prefix('<').map_or("", |r| r.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or(""));
    let local = name.rsplit_once(':').map_or(name, |(_, l)| l);
    local == "svg"
}

/// Parse an SVG file safely (see the crate docs).
pub fn parse(bytes: &[u8]) -> Result<Svg, Error> {
    on_own_stack(|| {
        let budget = Budget::default();
        parse_nested(bytes, 0, 0, &budget)
    })
}

/// The stack a picture is read on (native targets). usvg builds a picture, and the style sheet
/// matcher matches it, by recursion along the picture's structure; the checks bound how deep that
/// goes, and a large stack of its own is a second guard behind them. The memory is only reserved:
/// pages are used as deep as the recursion actually reaches.
#[cfg(not(target_arch = "wasm32"))]
const READ_STACK: usize = 256 << 20;

/// Runs `f` on a thread with a [`READ_STACK`] stack, so a picture that recurses deeper than the
/// checks foresee still can't overflow the caller's stack (which aborts the process). A panic
/// in it becomes an error.
#[cfg(not(target_arch = "wasm32"))]
fn on_own_stack<T: Send>(f: impl FnOnce() -> Result<T, Error> + Send) -> Result<T, Error> {
    std::thread::scope(|s| {
        let reader = std::thread::Builder::new()
            .name("svg reader".to_string())
            .stack_size(READ_STACK)
            .spawn_scoped(s, f)
            .map_err(|e| Error::Parse(e.to_string()))?;
        reader.join().unwrap_or_else(|_| refused("the SVG reader stopped"))
    })
}

/// On the web there are no threads: the checks are the guard.
#[cfg(target_arch = "wasm32")]
fn on_own_stack<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    f()
}

/// What a picture and the SVG pictures nested in it may hold between them (#1910 review 3).
#[derive(Default)]
struct Budget {
    /// Text bytes in text elements so far.
    text: AtomicU64,
    /// Text bytes drawn so far, `<use>` copies counted.
    drawn: AtomicU64,
    /// Elements built so far, copies counted.
    elements: AtomicU64,
    /// Bytes of nested pictures read so far (usvg reads a nested picture once per copy of it).
    nested: AtomicU64,
    /// Bytes of raster image data read so far (usvg decodes a `data:` image once per copy of
    /// it, and keeps each).
    images: AtomicU64,
    /// The deepest usvg is to recurse for any picture read so far (nested ones included).
    deepest: AtomicU64,
    /// Why a nested picture went past the caps: the whole picture is refused.
    over: std::sync::OnceLock<&'static str>,
}

/// [`parse`] for a picture `level` SVG pictures deep, sharing `budget` with the whole picture,
/// read `base` levels deep in usvg's recursion (inside the pictures holding it).
fn parse_nested(bytes: &[u8], level: u32, base: u64, budget: &Budget) -> Result<Svg, Error> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err(Error::TooLarge);
    }
    if !is_svg(bytes) {
        return Err(Error::NotSvg);
    }
    if level > 0 {
        let len = bytes.len() as u64;
        if budget.nested.fetch_add(len, Ordering::Relaxed).saturating_add(len) > MAX_SVG_BYTES as u64 {
            return budget.refuse(NESTED_TOO_LARGE);
        }
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let text = std::str::from_utf8(b).map_err(|e| Error::Parse(e.to_string()))?;
    check_nesting(text)?;
    let xml_opts = usvg::roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: MAX_XML_NODES, ..Default::default() };
    let xml = usvg::roxmltree::Document::parse_with_options(text, xml_opts).map_err(|e| Error::Parse(e.to_string()))?;
    // Element nesting, measured without recursion (parents come before their children).
    let mut depth: Vec<u16> = Vec::new();
    for n in xml.descendants() {
        let d = n.parent().and_then(|p| depth.get(p.id().get_usize()).copied()).map_or(0, |d| d.saturating_add(1));
        if d as usize > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        let i = n.id().get_usize();
        if depth.len() <= i {
            depth.resize(i + 1, 0);
        }
        if let Some(slot) = depth.get_mut(i) {
            *slot = d;
        }
    }
    // How deep usvg will be when it reads a picture nested in this one: at most this one's depth
    // (known once checked). Pictures side by side don't add up (review 10 M-15).
    let depth_cell = AtomicU64::new(base);
    let depth = &depth_cell;
    let opts = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime: &str, data: std::sync::Arc<Vec<u8>>, _: &usvg::Options| match mime {
                // Past the cap on image data, the whole picture is refused (review 12 C-17).
                "image/jpg" | "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                    if budget.images.fetch_add(data.len() as u64, Ordering::Relaxed).saturating_add(data.len() as u64) > MAX_IMAGE_DATA =>
                {
                    let _ = budget.over.set(IMAGES_TOO_LARGE);
                    None
                }
                "image/jpg" | "image/jpeg" => Some(usvg::ImageKind::JPEG(data)),
                "image/png" => Some(usvg::ImageKind::PNG(data)),
                "image/gif" => Some(usvg::ImageKind::GIF(data)),
                "image/webp" => Some(usvg::ImageKind::WEBP(data)),
                // Our own reader, so a nested picture gets the same checks, and nesting is bounded.
                "image/svg+xml" if level < MAX_NESTED => {
                    parse_nested(&data, level + 1, depth.load(Ordering::Relaxed), budget).ok().map(|s| usvg::ImageKind::SVG(s.tree))
                }
                _ => None,
            }),
            // Files and URLs are never read.
            resolve_string: Box::new(|_: &str, _: &usvg::Options| None),
        },
        font_resolver: fonts::resolver(),
        ..usvg::Options::default()
    };
    depth.store(check_copies(&xml, base, budget)?, Ordering::Relaxed);
    let tree = usvg::Tree::from_xmltree(&xml, &opts).map_err(|e| Error::Parse(e.to_string()))?;
    if let Some(why) = budget.over.get() {
        return refused(why);
    }
    Ok(Svg { tree })
}

/// Pushes the groups drawn for `n` besides its children: clip paths and masks (with theirs),
/// filter images, patterns, nested pictures, text outlines.
fn push_subroots<'a>(n: &'a usvg::Node, stack: &mut Vec<&'a usvg::Group>) {
    match n {
        usvg::Node::Group(g) => {
            let mut clip = g.clip_path();
            while let Some(c) = clip {
                stack.push(c.root());
                clip = c.clip_path();
            }
            let mut mask = g.mask();
            while let Some(m) = mask {
                stack.push(m.root());
                mask = m.mask();
            }
            for f in g.filters() {
                for p in f.primitives() {
                    if let usvg::filter::Kind::Image(img) = p.kind() {
                        stack.push(img.root());
                    }
                }
            }
        }
        usvg::Node::Path(p) => {
            for paint in [p.fill().map(|f| f.paint()), p.stroke().map(|s| s.paint())].into_iter().flatten() {
                if let usvg::Paint::Pattern(pat) = paint {
                    stack.push(pat.root());
                }
            }
        }
        usvg::Node::Image(img) => {
            if let usvg::ImageKind::SVG(tree) = img.kind() {
                stack.push(tree.root());
            }
        }
        usvg::Node::Text(t) => stack.push(t.flattened()),
    }
}

/// Most entity declarations a file may have.
const MAX_ENTITIES: usize = 4096;
/// Most character data (bytes, entities expanded) inside text elements: past it usvg would shape
/// and lay out megabytes of text on the thread that opens the picture.
const MAX_TEXT_BYTES: u64 = 1 << 20;
/// Most text a picture may draw once `<use>` copies (and copies of copies) are counted.
const MAX_TEXT_DRAWN: u64 = 16 << 20;
/// Most text entity references may expand to (bytes): the longest entity value (with the
/// entities it references expanded) times the number of `&` in the file.
const MAX_ENTITY_EXPANSION: u64 = 16 << 20;

/// What the pre-parse scan has seen of the file's entities.
struct Entities<'a> {
    /// Expanded length of each declared entity, by name.
    sizes: std::collections::HashMap<&'a [u8], u64>,
    /// The longest expanded value.
    longest: u64,
    /// Declarations seen.
    count: usize,
    /// Number of `&` in the file, counted the first time a declaration needs it.
    refs: Option<u64>,
}

const TEXT_TOO_LONG: &str = "SVG text this long is not supported";
const NESTED_TOO_LARGE: &str = "SVG with this many nested pictures is not supported";
const IMAGES_TOO_LARGE: &str = "SVG with this much picture data is not supported";
/// Most raster image data a picture (and the pictures nested in it) may hold, copies counted.
const MAX_IMAGE_DATA: u64 = 256 << 20;

impl Budget {
    /// Refuses this picture, and with it the pictures it is nested in.
    fn refuse<T>(&self, why: &'static str) -> Result<T, Error> {
        let _ = self.over.set(why);
        refused(why)
    }
}

fn refused<T>(why: &str) -> Result<T, Error> {
    Err(Error::Parse(why.to_string()))
}

/// Refuses, before the XML parser (which recurses on nesting) sees it: markup nested deeper than
/// [`MAX_DEPTH`]; entities that hold markup (which could nest elements past the scan), that could
/// expand past [`MAX_ENTITY_EXPANSION`], or more than [`MAX_ENTITIES`] of them. One pass over the
/// tags, linear in the file.
fn check_nesting(text: &str) -> Result<(), Error> {
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut ents = Entities { sizes: std::collections::HashMap::new(), longest: 0, count: 0, refs: None };
    let mut depth = 0usize;
    let find = |from: usize, pat: &[u8]| b.get(from..).and_then(|rest| rest.windows(pat.len()).position(|w| w == pat)).map(|p| from + p);
    while let Some(lt) = b.get(i..).and_then(|rest| rest.iter().position(|c| *c == b'<')).map(|p| i + p) {
        let rest = b.get(lt..).unwrap_or_default();
        i = if rest.starts_with(b"<!--") {
            find(lt + 4, b"-->").map_or(b.len(), |e| e + 3)
        } else if rest.starts_with(b"<![CDATA[") {
            find(lt + 9, b"]]>").map_or(b.len(), |e| e + 3)
        } else if rest.starts_with(b"<?") {
            find(lt + 2, b"?>").map_or(b.len(), |e| e + 2)
        } else if rest.starts_with(b"<!") {
            // A declaration; a DOCTYPE's internal subset (a `[` before its first `>`) ends at
            // `]>`. The `[` is looked for within the declaration only, so a run of declarations
            // scans in linear time.
            let close = find(lt, b">").unwrap_or(b.len());
            let open = b.get(lt..close).and_then(|d| d.iter().position(|c| *c == b'[')).map(|p| lt + p);
            let end = match open {
                Some(open) => find(open, b"]>").map_or(b.len(), |e| e + 2),
                None => close.saturating_add(1),
            };
            let decl = b.get(lt..end.min(b.len())).unwrap_or_default();
            if decl.windows(8).any(|w| w == b"<!ENTITY") {
                check_entities(decl, b, &mut ents)?;
            }
            end
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            find(lt, b">").map_or(b.len(), |e| e + 1)
        } else {
            // A start tag: its end is the first `>` outside a quoted attribute value.
            let mut j = lt + 1;
            let mut quote = None;
            while let Some(&c) = b.get(j) {
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    None => {}
                }
                j += 1;
            }
            if b.get(j.saturating_sub(1)) != Some(&b'/') {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(Error::TooDeep);
                }
            }
            j.saturating_add(1)
        };
    }
    Ok(())
}

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
/// Most work `<tref>` links may cost: usvg finds each one's target by walking the file's nodes, so
/// the links resolved (copies counted) times the nodes in the file.
const MAX_TREF_WORK: u64 = 1 << 26;
const ENDLESS_COPIES: &str = "SVG whose copies of itself never end is not supported";
const SELECTORS_TOO_COMPLEX: &str = "SVG style sheet with selectors this complex is not supported";
const REFERENCE_LOOP: &str = "SVG whose clip paths, masks, patterns, filters or markers refer to each other in a loop is not supported";
const REFERENCES_TOO_DEEP: &str = "SVG whose clip paths, masks, patterns, filters or markers refer to each other this deeply is not supported";
const TOO_MANY_REFERENCES: &str = "SVG with this many clip paths, masks, patterns, filters or markers is not supported";
const TOO_MANY_ELEMENTS: &str = "SVG that draws this many elements is not supported";
fn refusal(r: refs::Refusal) -> &'static str {
    match r {
        refs::Refusal::Loop => REFERENCE_LOOP,
        refs::Refusal::TooBig => TOO_MANY_REFERENCES,
        refs::Refusal::Selectors => SELECTORS_TOO_COMPLEX,
        refs::Refusal::Deep => REFERENCES_TOO_DEEP,
    }
}

/// Most elements a picture may build, counting a path or point list by its size, copies made by `<use>`, markers, clip paths, masks, patterns
/// and filters counted.
const MAX_ELEMENTS: u64 = 4_000_000;
const TREFS_TOO_MANY: &str = "SVG with this many text references is not supported";

/// Refuses, before usvg builds the picture from the parsed XML: more than [`MAX_TEXT_BYTES`] of
/// text in text elements, and more than [`MAX_TEXT_DRAWN`] once usvg's copies are counted
/// ([`copies`]), counting what `budget` holds already (the pictures this one is nested in, and
/// those nested before it); `<use>` links that copy without end; and `<tref>` links that would
/// cost more than [`MAX_TREF_WORK`] to find.
/// Returns how deep usvg recurses to build it, `base` levels down.
fn check_copies(xml: &usvg::roxmltree::Document, base: u64, budget: &Budget) -> Result<u64, Error> {
    // Before anything matches a selector (usvg's matcher recurses along combinators).
    let per_depth = match refs::check_css(xml) {
        Ok(w) => w,
        Err(r) => return budget.refuse(refusal(r)),
    };
    let c = match copies_matching(xml, &per_depth) {
        Ok(c) => c,
        Err(why) => return budget.refuse(why),
    };
    if budget.text.fetch_add(c.written, Ordering::Relaxed).saturating_add(c.written) > MAX_TEXT_BYTES {
        return budget.refuse(TEXT_TOO_LONG);
    }
    if c.endless {
        return budget.refuse(ENDLESS_COPIES);
    }
    if budget.drawn.fetch_add(c.bytes, Ordering::Relaxed).saturating_add(c.bytes) > MAX_TEXT_DRAWN {
        return budget.refuse(TEXT_TOO_LONG);
    }
    if budget.elements.fetch_add(c.elements, Ordering::Relaxed).saturating_add(c.elements) > MAX_ELEMENTS {
        return budget.refuse(TOO_MANY_ELEMENTS);
    }
    let nodes = xml.descendants().count() as u64;
    if c.trefs.saturating_mul(nodes) > MAX_TREF_WORK {
        return budget.refuse(TREFS_TOO_MANY);
    }
    // Before usvg decodes them all (review 12 C-17; the same cap is kept as it reads them).
    if budget.images.load(Ordering::Relaxed).saturating_add(c.image_data) > MAX_IMAGE_DATA.saturating_mul(4) {
        return budget.refuse(IMAGES_TOO_LARGE);
    }
    // usvg matches every selector against every element it builds, `<use>` copies included.
    if c.matched > refs::MAX_MATCH_WORK {
        return budget.refuse(SELECTORS_TOO_COMPLEX);
    }
    // On the web the reader has the browser's stack (about 1 MB); on desktop, [`READ_STACK`].
    let depth = base.saturating_add(c.depth);
    budget.deepest.fetch_max(depth, Ordering::Relaxed);
    if cfg!(target_arch = "wasm32") && depth > WEB_MAX_BUILD_DEPTH {
        return budget.refuse(BUILT_TOO_DEEP);
    }
    Ok(depth)
}

/// How deep usvg may recurse building a picture on the web (levels: elements inside elements,
/// `<use>` copies, what clip paths, masks, patterns, filters and markers hold, nested pictures).
/// The browser's stack, about 1 MB, holds a few hundred (review 9 I-6). On desktop the reader
/// has `READ_STACK` and the other checks bound the depth.
const WEB_MAX_BUILD_DEPTH: u64 = 200;
const BUILT_TOO_DEEP: &str = "SVG nested this deeply is not supported here";

/// What usvg builds of a picture's text, `<use>` copies (and copies of copies) included, and the
/// copies markers, clip paths, masks, patterns and filters may make of it.
#[derive(Clone, Copy, Default, Debug)]
struct Copies {
    /// Text as written: the text in each outermost text element, once (only on the total).
    written: u64,
    /// Text elements built.
    texts: u64,
    /// Elements built.
    elements: u64,
    /// Bytes of text in them, `<tref>` text included.
    bytes: u64,
    /// `<tref>` links resolved.
    trefs: u64,
    /// The copies never end (usvg would give up deep down, after building a great deal).
    endless: bool,
    /// Places a clip path, mask, filter or pattern may be drawn: a few per element (the paint it
    /// inherits, and each `url(` it or a style sheet names).
    slots: u64,
    /// Places a start or end marker may be drawn: one per shape, when the file has markers.
    ends: u64,
    /// Places a middle marker may be drawn: one per vertex of each shape, when the file has
    /// markers.
    mids: u64,
    /// Something built names a `url(`.
    url: bool,
    /// The work of matching the style sheets against every element usvg builds (only on the
    /// total).
    matched: u64,
    /// How deep usvg recurses to build it (only on the total).
    depth: u64,
    /// Bytes of `data:` pictures usvg decodes, copies counted (only on the total).
    image_data: u64,
}

impl Copies {
    const ENDLESS: Copies = Copies {
        written: 0,
        texts: u64::MAX,
        elements: u64::MAX,
        bytes: u64::MAX,
        trefs: u64::MAX,
        endless: true,
        slots: u64::MAX,
        ends: u64::MAX,
        mids: u64::MAX,
        url: true,
        matched: u64::MAX,
        depth: u64::MAX,
        image_data: u64::MAX,
    };

    /// Places any marker may be drawn.
    fn marks(&self) -> u64 {
        self.ends.saturating_mul(2).saturating_add(self.mids)
    }

    fn add(self, o: Copies) -> Copies {
        Copies {
            written: self.written.saturating_add(o.written),
            texts: self.texts.saturating_add(o.texts),
            elements: self.elements.saturating_add(o.elements),
            bytes: self.bytes.saturating_add(o.bytes),
            trefs: self.trefs.saturating_add(o.trefs),
            endless: self.endless || o.endless,
            slots: self.slots.saturating_add(o.slots),
            ends: self.ends.saturating_add(o.ends),
            mids: self.mids.saturating_add(o.mids),
            url: self.url || o.url,
            matched: self.matched.saturating_add(o.matched),
            depth: self.depth.max(o.depth),
            image_data: self.image_data.saturating_add(o.image_data),
        }
    }
}

/// How usvg treats an XML node when it builds its tree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Not built, nor anything inside it: text and comments, elements outside the SVG namespace,
    /// `<style>`.
    Skip,
    /// A text element: its text (and `<tref>` text) is built, its children are not walked again.
    Text,
    /// A `<use>`: builds a copy of the element it links to.
    Use,
    /// Any other SVG element, built with its children. (usvg also skips SVG elements it doesn't
    /// know; they are counted here, which can only overcount.)
    Group,
}

fn kind(n: usvg::roxmltree::Node) -> Kind {
    if !n.is_element() || !matches!(n.tag_name().namespace(), None | Some(SVG_NS)) {
        return Kind::Skip;
    }
    match n.tag_name().name() {
        "style" => Kind::Skip,
        "text" => Kind::Text,
        "use" => Kind::Use,
        _ => Kind::Group,
    }
}

/// The element a `<use>` or `<tref>` links to, read as usvg reads it (with the same calls):
/// `xlink:href`, else the first attribute whose local name is `href` (roxmltree looks a name
/// without a namespace up in any namespace), holding `#id` with spaces around it allowed
/// (svgtypes' IRI), naming the first element with that `id`.
fn link<'a, 'i>(
    n: usvg::roxmltree::Node<'a, 'i>,
    ids: &std::collections::HashMap<&str, usvg::roxmltree::Node<'a, 'i>>,
) -> Option<usvg::roxmltree::Node<'a, 'i>> {
    let value = n.attribute((XLINK_NS, "href")).or_else(|| n.attribute("href"))?;
    let id = svgtypes::IRI::from_str(value).ok()?.0;
    ids.get(id).copied()
}

/// What usvg builds of the text in `xml`, following its rules for `<use>` (usvg 0.47,
/// `svgtree/parse.rs`): a `<use>` builds a copy of the element it links to unless it links to
/// itself, to a `<use>` that links back to it, to an element outside the SVG namespace, or to an
/// element holding a `<use>` that links to that element or to this `<use>`. Copies hold copies,
/// so the text adds up along every path; work shared by many copies is done once (each node is
/// worked out once, with an explicit stack: linear, no recursion), and copies that lead back to
/// themselves are endless. Where usvg's rules are not mirrored (a `<use>` without the SVG
/// namespace that links back to its copy), this counts more, never less. The text drawn by
/// markers, clip paths, masks, patterns and filter images (usvg's converter builds them per
/// use) is bounded from above rather than counted, along the references between them
/// ([`refs`]); references that lead back to themselves are refused (usvg overflows its stack).
#[cfg(test)]
fn copies(xml: &usvg::roxmltree::Document) -> Result<Copies, &'static str> {
    copies_matching(xml, &[])
}

/// [`copies`], with the work of matching the style sheets against an element at each depth
/// ([`refs::check_css`]).
fn copies_matching<'a, 'i>(xml: &'a usvg::roxmltree::Document<'i>, per_depth: &[u64]) -> Result<Copies, &'static str> {
    type Node<'a, 'i> = usvg::roxmltree::Node<'a, 'i>;
    let all: Vec<Node> = xml.descendants().collect();
    let len = all.iter().map(|n| n.id().get_usize() + 1).max().unwrap_or(0);
    let at = |n: Node| n.id().get_usize();
    // usvg draws a marker's content at every marker place of the shapes that name it, and builds
    // a clip path, mask, filter or pattern again for each element that names it (unless it can
    // share one, which isn't counted on). They refer to each other, which usvg follows without
    // limit: a loop is refused.
    let graph = refs::graph(xml).map_err(refusal)?;
    let mut ids = std::collections::HashMap::new();
    for n in &all {
        if let Some(id) = n.attribute("id") {
            ids.entry(id).or_insert(*n);
        }
    }
    // Each node's last descendant (for "is inside"), the text inside it, and how deep it goes.
    let mut last = vec![0usize; len];
    let mut text = vec![0u64; len];
    let mut sub_depth = vec![0u64; len];
    for n in all.iter().rev() {
        let i = at(*n);
        let l = n.last_child().and_then(|c| last.get(at(c)).copied()).unwrap_or(i);
        let own = if n.is_text() { n.text().map_or(0, |t| t.len() as u64) } else { 0 };
        let t = n.children().fold(own, |a, c| a.saturating_add(text.get(at(c)).copied().unwrap_or(0)));
        let d = n.children().map(|c| sub_depth.get(at(c)).copied().unwrap_or(0)).max().unwrap_or(0).saturating_add(1);
        if let (Some(ls), Some(ts), Some(ds)) = (last.get_mut(i), text.get_mut(i), sub_depth.get_mut(i)) {
            *ls = l;
            *ts = t;
            *ds = d;
        }
    }
    // `url(`s: each names a clip path, mask, filter, paint or marker. Per element, and (for text
    // elements, whose spans are drawn with them) per subtree.
    let svg_element = |n: Node| n.is_element() && matches!(n.tag_name().namespace(), None | Some(SVG_NS));
    let mut urls = vec![0u64; len];
    let mut sub_slots = vec![0u64; len];
    let mut sub_url = vec![false; len];
    // A `url(` is a place something is built if it names an element. (Gradients too: usvg keeps
    // one per id, but copies its stops for every shape it paints in bounding-box units; review 9
    // C-7.)
    let built_ids: std::collections::HashSet<&str> = all.iter().filter(|n| n.is_element()).flat_map(|n| refs::ids(*n)).collect();
    // The `url(`s each element names itself (attributes, `style`, matching style sheet rules).
    for n in all.iter().filter(|n| svg_element(**n)) {
        let mut k = 0u64;
        graph.urls_of(*n, |_, ids| k = k.saturating_add(u64::from(ids.iter().any(|id| built_ids.contains(id)))));
        if let Some(u) = urls.get_mut(at(*n)) {
            *u = k;
        }
    }
    // `inherit` copies the parent's clip path, mask or filter: then any element may name as many
    // as the most any element names.
    let inherited_urls = if graph.inherit { urls.iter().copied().max().unwrap_or(0) } else { 0 };
    let own_slots = |u: u64| 2u64.saturating_add(u).saturating_add(inherited_urls);
    for n in all.iter().rev() {
        let i = at(*n);
        let u = urls.get(i).copied().unwrap_or(0);
        let (slots, url) = n.children().fold((if svg_element(*n) { own_slots(u) } else { 0 }, u > 0), |(a, b), c| {
            (a.saturating_add(sub_slots.get(at(c)).copied().unwrap_or(0)), b || sub_url.get(at(c)).copied().unwrap_or(false))
        });
        if let (Some(ss), Some(su)) = (sub_slots.get_mut(i), sub_url.get_mut(i)) {
            *ss = slots;
            *su = url;
        }
    }
    // Whether an ancestor names a `url(` (its content inherits it), and whether a text element
    // holds the node; parents come before their children.
    let mut above_url = vec![false; len];
    let mut in_text = vec![false; len];
    for n in &all {
        if let Some(p) = n.parent() {
            let (pu, pt) = (above_url.get(at(p)).copied().unwrap_or(false), in_text.get(at(p)).copied().unwrap_or(false));
            let p_url = urls.get(at(p)).is_some_and(|u| *u > 0);
            if let (Some(a), Some(t)) = (above_url.get_mut(at(*n)), in_text.get_mut(at(*n))) {
                *a = pu || p_url;
                *t = pt || kind(p) == Kind::Text;
            }
        }
    }
    let has_markers = all.iter().any(|n| svg_element(*n) && n.tag_name().name() == "marker");
    // Marker places on a shape (review 9 M-11): usvg draws a start and an end marker once per
    // shape, and a middle marker at every vertex. A path or point list has at most one vertex
    // per number or command in its data (an arc's seven numbers make at most four curves).
    let marks = |n: Node| -> (u64, u64) {
        if !has_markers {
            return (0, 0);
        }
        let vertices = match n.tag_name().name() {
            "path" => n.attribute("d").map_or(0, data_items),
            "polyline" | "polygon" => n.attribute("points").map_or(0, data_items),
            "line" | "rect" | "circle" | "ellipse" => 16,
            _ => return (0, 0),
        };
        (1, vertices.saturating_add(2))
    };
    // What building an element costs, in elements: one, and for a path or point list, its
    // segments or points (usvg parses the data again for every copy).
    let weight = |n: Node| -> u64 {
        let data = match n.tag_name().name() {
            "path" => n.attribute("d"),
            "polyline" | "polygon" => n.attribute("points"),
            _ => None,
        };
        1u64.saturating_add(data.map_or(0, |d| d.len() as u64 / 8))
    };
    let inside = |a: Node, d: Node| at(a) < at(d) && last.get(at(a)).is_some_and(|l| at(d) <= *l);
    // usvg skips a `<use>` whose target holds a `<use>` (in the SVG namespace) linking to that
    // target (LOOPS on the target) or to the `<use>` itself (LOOPS_BACK on the `<use>`).
    const LOOPS: u8 = 1;
    const LOOPS_BACK: u8 = 2;
    let mut flags = vec![0u8; len];
    for d in &all {
        if !(d.is_element() && d.tag_name().namespace() == Some(SVG_NS) && d.tag_name().name() == "use") {
            continue;
        }
        let Some(x) = link(*d, &ids) else { continue };
        if inside(x, *d)
            && let Some(f) = flags.get_mut(at(x))
        {
            *f |= LOOPS;
        }
        if kind(x) == Kind::Use
            && link(x, &ids).is_some_and(|y| inside(y, *d))
            && let Some(f) = flags.get_mut(at(x))
        {
            *f |= LOOPS_BACK;
        }
    }
    let flag = |n: Node, f: u8| flags.get(at(n)).is_some_and(|v| v & f != 0);
    // What a `<use>` copies.
    let copied = |u: Node<'a, 'i>| -> Option<Node<'a, 'i>> {
        let t = link(u, &ids)?;
        let skipped = t == u
            || !svg_element(t)
            || flag(u, LOOPS_BACK)
            || flag(t, LOOPS)
            // A `<use>` of a `<use>` of this one: the inner one links to its own copy's origin.
            || (kind(t) == Kind::Use && link(t, &ids) == Some(u));
        (!skipped).then_some(t)
    };
    // `<tref>` text and links inside each text element.
    let mut trefs: std::collections::HashMap<usize, Copies> = std::collections::HashMap::new();
    for r in &all {
        if !(svg_element(*r) && r.tag_name().name() == "tref" && (r.has_attribute((XLINK_NS, "href")) || r.has_attribute("href"))) {
            continue;
        }
        let bytes = link(*r, &ids).filter(|t| svg_element(*t)).and_then(|t| text.get(at(t)).copied()).unwrap_or(0);
        for a in r.ancestors().skip(1).filter(|a| kind(*a) == Kind::Text) {
            let e = trefs.entry(at(a)).or_default();
            *e = e.add(Copies { bytes, trefs: 1, ..Copies::default() });
        }
    }
    // What each node builds where it stands, worked out after what it builds from.
    let own = |n: Node| match kind(n) {
        Kind::Text => {
            let t = Copies {
                texts: 1,
                elements: 1,
                bytes: text.get(at(n)).copied().unwrap_or(0),
                slots: sub_slots.get(at(n)).copied().unwrap_or(0),
                url: sub_url.get(at(n)).copied().unwrap_or(false),
                ..Copies::default()
            };
            t.add(trefs.get(&at(n)).copied().unwrap_or_default())
        }
        Kind::Group | Kind::Use if n != xml.root() => {
            let u = urls.get(at(n)).copied().unwrap_or(0);
            let (ends, mids) = marks(n);
            Copies { elements: weight(n), slots: own_slots(u), ends, mids, url: u > 0, ..Copies::default() }
        }
        _ => Copies::default(),
    };
    let from = |n: Node<'a, 'i>| -> Vec<Node<'a, 'i>> {
        match kind(n) {
            _ if n == xml.root() => n.children().filter(|c| kind(*c) != Kind::Skip).collect(),
            // A filter image draws the element it links to (usvg's `feImage`).
            Kind::Group if n.tag_name().name() == "feImage" => {
                n.children().filter(|c| kind(*c) != Kind::Skip).chain(link(n, &ids).filter(|t| *t != n && svg_element(*t))).collect()
            }
            Kind::Group => n.children().filter(|c| kind(*c) != Kind::Skip).collect(),
            Kind::Use => copied(n).into_iter().collect(),
            Kind::Text | Kind::Skip => Vec::new(),
        }
    };
    // 0: not reached, 1: being worked out (on the walk's path), 2: worked out.
    let mut state = vec![0u8; len];
    let mut built = vec![Copies::default(); len];
    // How deep usvg recurses to build each node where it stands.
    let mut deep = vec![0u64; len];
    let mut finished: Vec<Node> = Vec::new();
    let mut stack = vec![xml.root()];
    while let Some(&n) = stack.last() {
        let i = at(n);
        match state.get(i).copied() {
            Some(0) => {
                if let Some(s) = state.get_mut(i) {
                    *s = 1;
                }
                stack.extend(from(n).into_iter().filter(|d| state.get(at(*d)) == Some(&0)));
            }
            Some(1) => {
                // What it builds from is worked out now, or still on the path: a copy of itself.
                let c = from(n).into_iter().fold(own(n), |a, d| {
                    a.add(if state.get(at(d)) == Some(&2) { built.get(at(d)).copied().unwrap_or_default() } else { Copies::ENDLESS })
                });
                let below = match kind(n) {
                    Kind::Text => sub_depth.get(i).copied().unwrap_or(0),
                    _ => from(n).into_iter().map(|d| deep.get(at(d)).copied().unwrap_or(0)).max().unwrap_or(0),
                };
                if let (Some(b), Some(s), Some(dp)) = (built.get_mut(i), state.get_mut(i), deep.get_mut(i)) {
                    *b = c;
                    *s = 2;
                    *dp = below.saturating_add(1);
                }
                finished.push(n);
                stack.pop();
            }
            _ => {
                stack.pop();
            }
        }
    }
    let mut total = built.get(at(xml.root())).copied().unwrap_or_default();
    total.written = all
        .iter()
        .filter(|n| kind(**n) == Kind::Text && !in_text.get(at(**n)).copied().unwrap_or(true))
        .fold(0u64, |a, n| a.saturating_add(text.get(at(*n)).copied().unwrap_or(0)));
    if total.endless {
        return Ok(total);
    }
    // How often each node is built in the picture: copies (and copies of copies) counted.
    let mut made = vec![0u64; len];
    if let Some(m) = made.get_mut(at(xml.root())) {
        *m = 1;
    }
    for &n in finished.iter().rev() {
        let k = made.get(at(n)).copied().unwrap_or(0);
        for d in from(n) {
            if let Some(m) = made.get_mut(at(d)) {
                *m = m.saturating_add(k);
            }
        }
    }
    // A text element's spans are built with it.
    for n in &all {
        if in_text.get(at(*n)) == Some(&true)
            && let Some(k) = n.parent().and_then(|p| made.get(at(p)).copied())
            && let Some(m) = made.get_mut(at(*n))
        {
            *m = (*m).max(k);
        }
    }
    // usvg decodes a `data:` picture's data for every copy of its element: the data, copies
    // counted (base64 is four characters for three bytes).
    for n in all.iter().filter(|n| svg_element(**n) && matches!(n.tag_name().name(), "image" | "feImage")) {
        let len = n.attributes().filter(|a| a.name() == "href").map(|a| a.value().len() as u64).max().unwrap_or(0);
        total.image_data = total.image_data.saturating_add((len / 4 * 3).saturating_mul(made.get(at(*n)).copied().unwrap_or(0)));
    }
    // Every element usvg builds is matched against the style sheets once (copies included).
    if !per_depth.is_empty() {
        let mut depth = vec![0usize; len];
        for n in &all {
            let d = n.parent().and_then(|p| depth.get(at(p)).copied()).map_or(0, |d| d + 1);
            if let Some(slot) = depth.get_mut(at(*n)) {
                *slot = d;
            }
            if svg_element(*n) {
                let work = per_depth.get(d).copied().unwrap_or(u64::MAX);
                total.matched = total.matched.saturating_add(made.get(at(*n)).copied().unwrap_or(0).saturating_mul(work));
            }
        }
    }
    // How often the picture itself builds each special: for each element that names it, once per
    // copy of the element; for paint (or anything that may pass down), once per element built
    // inside it too; for a marker, once per marker place inside it.
    let mut named = vec![0u64; graph.nodes.len()];
    let mut specials: Vec<u32> = Vec::new();
    for n in all.iter().filter(|n| svg_element(**n)) {
        let k = made.get(at(*n)).copied().unwrap_or(0);
        if k == 0 {
            continue;
        }
        let b = built.get(at(*n)).copied().unwrap_or_default();
        // What passes down reaches every element built inside it (a text's spans included).
        let inside = b.elements.max(last.get(at(*n)).map_or(1, |l| (l.saturating_sub(at(*n)) as u64).saturating_add(1)));
        graph.urls_of(*n, |name, ids| {
            specials.clear();
            specials.extend(ids.iter().flat_map(|id| graph.named_by(id)));
            specials.sort_unstable();
            specials.dedup();
            for &s in &specials {
                let places = if graph.marker.get(s as usize) == Some(&true) {
                    match name {
                        "marker-start" | "marker-end" => b.ends,
                        "marker-mid" => b.mids,
                        _ => b.marks(),
                    }
                } else if graph.passes_down(name) {
                    inside
                } else {
                    1
                };
                if let Some(m) = named.get_mut(s as usize) {
                    *m = m.saturating_add(k.saturating_mul(places));
                }
            }
        });
    }
    // Along the references (what each one holds, inherits or names), a special is built once
    // for every place any referrer may name it, times how often that referrer is built. The
    // references are walked as the ways usvg may reach each special: a loop through markers is
    // unrolled, each of its markers built once at most along any path.
    let st = &graph.states;
    let special = |x: usize| st.of.get(x).map_or(usize::MAX, |s| *s as usize);
    let held = |x: usize| graph.nodes.get(special(x)).and_then(|id| built.get(*id)).copied().unwrap_or_default();
    let is_marker = |x: usize| graph.marker.get(special(x)) == Some(&true);
    let mut times = vec![0u64; st.of.len()];
    for (e, k) in st.entry.iter().zip(&named) {
        if let Some(m) = times.get_mut(*e as usize) {
            *m = m.saturating_add(*k);
        }
    }
    for &x in &st.order {
        let x = x as usize;
        let (b, k) = (held(x), times.get(x).copied().unwrap_or(0));
        for &t in st.edges.get(x).into_iter().flatten() {
            let places = if is_marker(t as usize) { b.marks() } else { b.slots };
            if let Some(m) = times.get_mut(t as usize) {
                *m = m.saturating_add(places.saturating_mul(k));
            }
        }
    }
    for (i, k) in times.iter().enumerate() {
        let b = held(i);
        total.bytes = total.bytes.saturating_add(b.bytes.saturating_mul(*k));
        total.texts = total.texts.saturating_add(b.texts.saturating_mul(*k));
        total.elements = total.elements.saturating_add(b.elements.saturating_mul(*k));
    }
    // How deep usvg recurses: the picture's own elements and `<use>` copies, and along the
    // deepest chain of specials, what each holds.
    let mut chain = vec![0u64; st.of.len()];
    for &x in &st.order {
        let x = x as usize;
        let own = graph.nodes.get(special(x)).and_then(|id| deep.get(*id)).copied().unwrap_or(0);
        let here = chain.get(x).copied().unwrap_or(0).max(own);
        if let Some(c) = chain.get_mut(x) {
            *c = here;
        }
        for &t in st.edges.get(x).into_iter().flatten() {
            let t_own = graph.nodes.get(special(t as usize)).and_then(|id| deep.get(*id)).copied().unwrap_or(0);
            if let Some(c) = chain.get_mut(t as usize) {
                *c = (*c).max(here.saturating_add(t_own));
            }
        }
    }
    total.depth = deep.get(at(xml.root())).copied().unwrap_or(0).saturating_add(chain.iter().copied().max().unwrap_or(0));
    Ok(total)
}

/// An upper bound on the vertices path or point data makes: one per number or command letter.
fn data_items(d: &str) -> u64 {
    let (mut n, mut prev_digit) = (0u64, false);
    for c in d.bytes() {
        let digit = c.is_ascii_digit();
        if (digit || c == b'.') && !prev_digit || c.is_ascii_alphabetic() {
            n += 1;
        }
        prev_digit = digit;
    }
    n
}

/// The length of character data once its entity references are expanded.
fn char_data_size(data: &[u8], ents: &Entities) -> u64 {
    let mut size = data.len() as u64;
    if ents.sizes.is_empty() {
        return size;
    }
    let mut k = 0usize;
    while let Some(amp) = data.get(k..).and_then(|r| r.iter().position(|c| *c == b'&')).map(|p| k + p) {
        // The name runs to the `;` (or the next `&` in a malformed reference).
        let semi = data.get(amp + 1..).and_then(|r| r.iter().position(|c| *c == b';' || *c == b'&')).map_or(data.len(), |p| amp + 1 + p);
        if let Some(n) = data.get(amp + 1..semi).and_then(|name| ents.sizes.get(name)) {
            size = size.saturating_add(*n);
        }
        k = if data.get(semi) == Some(&b';') { semi + 1 } else { semi };
    }
    size
}

/// Records the entity declarations in `decl` (a part of `file`). Refuses a value holding markup,
/// more than [`MAX_ENTITIES`] declarations, and values that could expand past
/// [`MAX_ENTITY_EXPANSION`] given the references in the file. The XML parser only limits
/// references nested in entities; any number at the top level each expand to a full copy.
fn check_entities<'a>(decl: &'a [u8], file: &[u8], ents: &mut Entities<'a>) -> Result<(), Error> {
    let mut i = 0usize;
    while let Some(at) = decl.get(i..).and_then(|r| r.windows(8).position(|w| w == b"<!ENTITY")).map(|p| i + p + 8) {
        ents.count += 1;
        if ents.count > MAX_ENTITIES {
            return refused("this many entities are not supported");
        }
        let rest = decl.get(at..).unwrap_or_default();
        // `<!ENTITY [%] name "value">` (an external entity has no quoted value of its own here).
        let name_start = rest.iter().position(|c| !c.is_ascii_whitespace() && *c != b'%').unwrap_or(rest.len());
        let name_len = rest.get(name_start..).unwrap_or_default().iter().position(|c| c.is_ascii_whitespace()).unwrap_or(0);
        let name = rest.get(name_start..name_start + name_len).unwrap_or_default();
        let after = name_start + name_len;
        let Some(q) = rest.get(after..).and_then(|r| r.iter().position(|c| *c == b'"' || *c == b'\'' || *c == b'>')).map(|p| after + p) else {
            break;
        };
        let quote = rest.get(q).copied().unwrap_or(b'>');
        if quote == b'>' {
            i = at + q + 1;
            continue;
        }
        let vlen = rest.get(q + 1..).and_then(|r| r.iter().position(|c| *c == quote)).unwrap_or(0);
        let value = rest.get(q + 1..q + 1 + vlen).unwrap_or_default();
        if value.contains(&b'<') {
            return refused("entities holding markup are not supported");
        }
        // The value's expanded length: its text plus the expansions of the entities it names.
        let size = char_data_size(value, ents);
        ents.sizes.entry(name).or_insert(size);
        ents.longest = ents.longest.max(size);
        let refs = *ents.refs.get_or_insert_with(|| file.iter().filter(|c| **c == b'&').count() as u64);
        if ents.longest.saturating_mul(refs) > MAX_ENTITY_EXPANSION {
            return refused("entities expanding this far are not supported");
        }
        i = at + q + 1 + vlen;
    }
    Ok(())
}

/// The natural size of an SVG picture in points (its width and height, or its `viewBox`, as CSS
/// pixels at 96 per inch), or None when it isn't a readable SVG. Very large pictures are scaled
/// down to 22 inches on the longer side, keeping the shape.
pub fn natural_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let (w, h) = parse(bytes).ok()?.size_px();
    let (w, h) = (w * 0.75, h * 0.75);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let k = (MAX_NATURAL_PT / w.max(h)).min(1.0);
    Some((w * k, h * k))
}

/// A PNG of the SVG at twice its natural size (capped at [`MAX_FALLBACK_SIDE`] on the longer
/// side), for formats that can't hold SVG.
pub fn fallback_png(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let svg = parse(bytes)?;
    let (w, h) = svg.size_px();
    let k = (2.0f32).min(MAX_FALLBACK_SIDE as f32 / w.max(h));
    let (pw, ph) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let pm = svg.pixmap(pw, ph, [pw as f32 / w, 0.0, 0.0, ph as f32 / h, 0.0, 0.0]).ok_or(Error::Raster)?;
    pm.encode_png().map_err(|_| Error::Raster)
}

impl Svg {
    /// The parsed tree (for drawing it as vectors, e.g. into a PDF).
    pub fn tree(&self) -> &usvg::Tree {
        &self.tree
    }

    /// The picture's size in CSS pixels (user units).
    pub fn size_px(&self) -> (f32, f32) {
        let s = self.tree.size();
        (s.width(), s.height())
    }

    /// A `width` × `height` raster of the picture drawn with `transform` (`[sx, ky, kx, sy, tx, ty]`
    /// from SVG user units to raster pixels). None when the raster would be empty or larger than
    /// [`MAX_RASTER_PIXELS`].
    pub fn render(&self, width: u32, height: u32, transform: [f32; 6]) -> Option<Raster> {
        let pm = self.pixmap(width, height, transform)?;
        let (width, height) = (pm.width(), pm.height());
        Some(Raster { width, height, rgba: pm.take() })
    }

    /// An upper bound on the memory resvg takes to draw a `width` × `height` raster with
    /// `transform` (as for [`Svg::render`]), the raster itself included. Past
    /// [`MAX_DRAW_BYTES`], [`Svg::render`] draws smaller and scales up.
    pub fn draw_bytes(&self, width: u32, height: u32, transform: [f32; 6]) -> u64 {
        let [sx, ky, kx, sy, tx, ty] = transform;
        let ts = resvg::tiny_skia::Transform::from_row(sx, ky, kx, sy, tx, ty);
        cost::draw_bytes(self.tree.root(), ts, width, height).saturating_add(u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4))
    }

    fn pixmap(&self, width: u32, height: u32, transform: [f32; 6]) -> Option<resvg::tiny_skia::Pixmap> {
        if width == 0 || height == 0 || (width as u64) * (height as u64) > MAX_RASTER_PIXELS || !transform.iter().all(|v| v.is_finite()) {
            return None;
        }
        let [sx, ky, kx, sy, tx, ty] = transform;
        let ts = resvg::tiny_skia::Transform::from_row(sx, ky, kx, sy, tx, ty);
        // What resvg will hold to draw it (review 10): past the budget, draw it smaller and scale
        // it up; if even a few pixels are past it, not at all (the caller draws a placeholder).
        if cost::draw_bytes(self.tree.root(), ts, width, height).saturating_add(u64::from(width) * u64::from(height) * 4) <= MAX_DRAW_BYTES {
            let mut pm = resvg::tiny_skia::Pixmap::new(width, height)?;
            resvg::render(&self.tree, ts, &mut pm.as_mut());
            return Some(pm);
        }
        // The size drawn, and the estimate for exactly that size (review 11 C-13: a side already
        // one pixel wide stays one pixel, so the scale on it doesn't shrink).
        let mut k = 1.0f32;
        let (w, h) = loop {
            k *= 0.5;
            let (w, h) = (((width as f32 * k) as u32).max(1), ((height as f32 * k) as u32).max(1));
            let (kx_, ky_) = (w as f32 / width as f32, h as f32 / height as f32);
            let need = cost::draw_bytes(self.tree.root(), ts.post_scale(kx_, ky_), w, h).saturating_add(u64::from(w) * u64::from(h) * 4);
            if need <= MAX_DRAW_BYTES {
                break (w, h);
            }
            if w == 1 && h == 1 {
                return None;
            }
        };
        let (kx_, ky_) = (w as f32 / width as f32, h as f32 / height as f32);
        let mut small = resvg::tiny_skia::Pixmap::new(w, h)?;
        resvg::render(&self.tree, ts.post_scale(kx_, ky_), &mut small.as_mut());
        let mut pm = resvg::tiny_skia::Pixmap::new(width, height)?;
        let paint = resvg::tiny_skia::PixmapPaint { quality: resvg::tiny_skia::FilterQuality::Bilinear, ..Default::default() };
        pm.draw_pixmap(0, 0, small.as_ref(), &paint, resvg::tiny_skia::Transform::from_scale(1.0 / kx_, 1.0 / ky_), None);
        Some(pm)
    }

    /// Whether krilla-svg can draw it into a PDF as vectors within the memory budget, at `scale`
    /// PDF units per SVG user unit where it is placed (the norm of the transform there).
    ///
    /// krilla-svg draws everything as vectors but a group with filters, which it rasterises with
    /// resvg, and the raster images it holds, which it decodes. Rather than follow krilla's own
    /// scale (review 11 C-14), this bounds it from above: krilla scales a filter group's layer by
    /// how its bounds map onto the page, at most the norm of every transform above it (nested
    /// pictures' placements included) times the layer's longer side over its shorter (a
    /// rotated thin layer), twice over, and at most 5,000 pixels a side. resvg's work at that
    /// scale ([`cost`]) and krilla's copies must fit the budget. Every raster image must have a
    /// readable header and a decoded size within the budget (review 11 C-15).
    pub fn pdf_vectors_fit(&self, scale: f32) -> bool {
        const FILTER_SCALE: f64 = 2.0;
        const MAX_SIDE: f64 = 5000.0;
        if !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        // An upper bound on a transform's stretch (its Frobenius norm).
        let norm = |t: resvg::tiny_skia::Transform| {
            let n = f64::from(t.sx).hypot(f64::from(t.ky)).hypot(f64::from(t.kx).hypot(f64::from(t.sy)));
            if n.is_finite() { n } else { f64::INFINITY }
        };
        let mut stack: Vec<(&usvg::Group, f64)> = vec![(self.tree.root(), f64::from(scale))];
        let mut seen = 0u64;
        while let Some((g, n)) = stack.pop() {
            seen += 1;
            if seen > 1 << 22 || !n.is_finite() {
                return false;
            }
            let inside = n * norm(g.transform());
            if !g.filters().is_empty()
                && let Some(lb) = g.layer_bounding_box().transform(g.transform())
            {
                let (lw, lh) = (f64::from(lb.width()), f64::from(lb.height()));
                // krilla maps the group's own (untransformed) layer by the transforms above and
                // then by the group's: its per-axis ratio is at most the norm above times the own
                // layer's longer side over its shorter (review 12 C-16). The mapped layer's own
                // proportions are counted too, which only adds.
                let own = g.layer_bounding_box();
                let (ow, oh) = (f64::from(own.width()), f64::from(own.height()));
                let aspect = ((ow + oh) / ow.min(oh)).max((lw + lh) / lw.min(lh));
                let rs = (FILTER_SCALE * n * aspect).min(MAX_SIDE / lw).min(MAX_SIDE / lh);
                let (w, h) = ((lw * rs).ceil(), (lh * rs).ceil());
                if !(rs.is_finite() && w.is_finite() && h.is_finite()) {
                    return false;
                }
                if w >= 1.0 && h >= 1.0 {
                    let (w, h) = (w as u32, h as u32);
                    let ts = resvg::tiny_skia::Transform::from_scale(rs as f32, rs as f32).pre_translate(-lb.x(), -lb.y());
                    // Its raster, the copies krilla makes of it, and resvg's work.
                    let raster = u64::from(w) * u64::from(h) * 4;
                    if cost::node_bytes(g, ts, w, h).saturating_add(raster.saturating_mul(3)) > MAX_DRAW_BYTES {
                        return false;
                    }
                }
            }
            let mut clip = g.clip_path();
            while let Some(c) = clip {
                stack.push((c.root(), inside * norm(c.transform())));
                clip = c.clip_path();
            }
            let mut mask = g.mask();
            while let Some(m) = mask {
                stack.push((m.root(), inside));
                mask = m.mask();
            }
            for f in g.filters() {
                for p in f.primitives() {
                    if let usvg::filter::Kind::Image(img) = p.kind() {
                        stack.push((img.root(), inside));
                    }
                }
            }
            for c in g.children() {
                match c {
                    usvg::Node::Group(cg) => stack.push((cg, inside)),
                    usvg::Node::Text(t) => stack.push((t.flattened(), inside)),
                    usvg::Node::Path(p) => {
                        for paint in [p.fill().map(|f| f.paint()), p.stroke().map(|s| s.paint())].into_iter().flatten() {
                            if let usvg::Paint::Pattern(pat) = paint {
                                stack.push((pat.root(), inside * norm(pat.transform())));
                            }
                        }
                    }
                    usvg::Node::Image(img) => match img.kind() {
                        usvg::ImageKind::SVG(tree) => {
                            // Placed at the image's size: at most that much larger again.
                            let (iw, ih) = (f64::from(img.size().width()), f64::from(img.size().height()));
                            let (tw, th) = (f64::from(tree.size().width()), f64::from(tree.size().height()));
                            let k = (iw / tw).max(ih / th).max(1.0) * std::f64::consts::SQRT_2;
                            stack.push((tree.root(), inside * k));
                        }
                        kind => {
                            let data = match kind {
                                usvg::ImageKind::JPEG(d) | usvg::ImageKind::PNG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d,
                                usvg::ImageKind::SVG(_) => continue,
                            };
                            let Ok(size) = imagesize::blob_size(data) else { return false };
                            if size.width == 0 || size.height == 0 {
                                return false;
                            }
                            // krilla embeds a JPEG as it is (review 12 M-18); any other it decodes,
                            // then converts and compresses a copy.
                            let bytes = (size.width as u64).saturating_mul(size.height as u64).saturating_mul(4);
                            if !matches!(kind, usvg::ImageKind::JPEG(_)) && bytes.saturating_mul(3) > MAX_DRAW_BYTES {
                                return false;
                            }
                        }
                    },
                }
            }
        }
        true
    }

    /// The raster images the picture draws, nested pictures' and those in clip paths, masks,
    /// patterns and filters included, each once: their kind as usvg read it (`"jpeg"`, `"png"`,
    /// `"gif"`, `"webp"`: what krilla decodes them as) and their data.
    pub fn raster_images(&self) -> Vec<(&'static str, std::sync::Arc<Vec<u8>>)> {
        let mut out: Vec<(&'static str, std::sync::Arc<Vec<u8>>)> = Vec::new();
        let mut stack: Vec<&usvg::Group> = vec![self.tree.root()];
        let mut seen = 0u64;
        while let Some(g) = stack.pop() {
            seen += 1;
            if seen > 1 << 22 {
                break;
            }
            for n in g.children() {
                if let usvg::Node::Group(cg) = n {
                    stack.push(cg);
                }
                push_subroots(n, &mut stack);
                if let usvg::Node::Image(img) = n {
                    let (kind, d) = match img.kind() {
                        usvg::ImageKind::JPEG(d) => ("jpeg", d),
                        usvg::ImageKind::PNG(d) => ("png", d),
                        usvg::ImageKind::GIF(d) => ("gif", d),
                        usvg::ImageKind::WEBP(d) => ("webp", d),
                        usvg::ImageKind::SVG(_) => continue,
                    };
                    if !out.iter().any(|(_, o)| std::sync::Arc::ptr_eq(o, d)) {
                        out.push((kind, d.clone()));
                    }
                }
            }
        }
        out
    }
}

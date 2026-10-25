//! Bounded, isolated first-page previews for File › New's personal templates.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use egui::{ColorImage, Context, TextureHandle};

const MAX_CACHE_ENTRIES: usize = 64;
const MAX_TEMPLATE_BYTES: u64 = 16 << 20;
const MAX_RASTER_SIDE: f32 = 512.0;
const TILE_WIDTH: f32 = 150.0;
const TILE_HEIGHT: f32 = 194.0;

fn cache_id() -> egui::Id {
    egui::Id::new("backstage_personal_template_previews")
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PreviewKey {
    path: String,
    revision: FileRevision,
    dpi_bits: u32,
    region: String,
    font_generation: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum FileRevision {
    Current { length: u64, modified_ns: Option<u128>, listed_modified: Option<String> },
    Unavailable,
}

#[derive(Clone)]
enum CachedPreview {
    Ready { texture: TextureHandle, aspect: f32 },
    Failed,
}

#[derive(Clone, Default)]
struct PreviewCache {
    entries: HashMap<PreviewKey, CachedPreview>,
    lru: VecDeque<PreviewKey>,
    last_attempt_frame: Option<u64>,
    last_gallery_frame: Option<u64>,
    #[cfg(test)]
    attempts: usize,
    #[cfg(test)]
    attempted_paths: Vec<String>,
    #[cfg(test)]
    rendered_pixel_hashes: Vec<u64>,
}

#[derive(Clone)]
pub(super) struct Preview {
    pub texture: TextureHandle,
    pub aspect: f32,
}

impl PreviewCache {
    fn touch(&mut self, key: &PreviewKey) {
        self.lru.retain(|entry| entry != key);
        self.lru.push_back(key.clone());
    }

    fn clear_entries(&mut self) {
        self.entries.clear();
        self.lru.clear();
    }
}

/// Invalidate stale successes and failures when File › New is visited after another page.
pub(super) fn gallery_visit(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr();
    let id = cache_id();
    let mut cache = ctx.data(|data| data.get_temp::<PreviewCache>(id)).unwrap_or_default();
    if cache.last_gallery_frame.is_some_and(|last| frame > last.saturating_add(1)) {
        cache.clear_entries();
    }
    cache.last_gallery_frame = Some(frame);
    ctx.data_mut(|data| data.insert_temp(id, cache));
}

/// Explicit Refresh invalidates successful previews and cached failures alike.
pub(super) fn invalidate(ctx: &Context) {
    let id = cache_id();
    let mut cache = ctx.data(|data| data.get_temp::<PreviewCache>(id)).unwrap_or_default();
    cache.clear_entries();
    ctx.data_mut(|data| data.insert_temp(id, cache));
}

/// Get a cached preview or render at most one visible template during this egui frame.
pub(super) fn get(
    ctx: &Context,
    path: &str,
    listed_modified: Option<&str>,
    region: &wordcraft_geom::Region,
    ppp: f32,
    visible: bool,
) -> Option<Preview> {
    if !visible {
        return None;
    }
    let id = cache_id();
    let mut cache = ctx.data(|data| data.get_temp::<PreviewCache>(id)).unwrap_or_default();
    let unavailable_key = base_key(path, FileRevision::Unavailable, region, ppp);
    if let Some(CachedPreview::Failed) = cache.entries.get(&unavailable_key) {
        cache.touch(&unavailable_key);
        ctx.data_mut(|data| data.insert_temp(id, cache));
        return None;
    }
    let key = preview_key(path, listed_modified, region, ppp)?;
    if let Some(entry) = cache.entries.get(&key).cloned() {
        cache.touch(&key);
        ctx.data_mut(|data| data.insert_temp(id, cache));
        return match entry {
            CachedPreview::Ready { texture, aspect } => Some(Preview { texture, aspect }),
            CachedPreview::Failed => None,
        };
    }

    let frame = ctx.cumulative_frame_nr();
    if cache.last_attempt_frame == Some(frame) {
        ctx.data_mut(|data| data.insert_temp(id, cache));
        ctx.request_repaint_after(Duration::from_millis(16));
        return None;
    }
    cache.last_attempt_frame = Some(frame);
    #[cfg(test)]
    {
        cache.attempts = cache.attempts.saturating_add(1);
        cache.attempted_paths.push(path.to_string());
    }

    let rendered = if matches!(key.revision, FileRevision::Unavailable) {
        Err("template is unavailable for preview".to_string())
    } else {
        render(path, region, ppp)
    };
    #[cfg(test)]
    if let Ok(raster) = &rendered {
        cache.rendered_pixel_hashes.push(image_hash(&raster.image));
    }
    let result = rendered.map(|raster| {
        let hash = key_hash(&key);
        let texture = ctx.load_texture(format!("personal-template-preview-{hash:016x}"), raster.image, egui::TextureOptions::LINEAR);
        CachedPreview::Ready { texture, aspect: raster.aspect }
    });
    let cached = result.unwrap_or(CachedPreview::Failed);
    cache.entries.insert(key.clone(), cached.clone());
    cache.touch(&key);
    while cache.entries.len() > MAX_CACHE_ENTRIES {
        if let Some(oldest) = cache.lru.pop_front() {
            cache.entries.remove(&oldest);
        } else {
            break;
        }
    }
    ctx.data_mut(|data| data.insert_temp(id, cache));
    match cached {
        CachedPreview::Ready { texture, aspect } => Some(Preview { texture, aspect }),
        CachedPreview::Failed => None,
    }
}

fn key_hash(key: &PreviewKey) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[cfg(not(target_arch = "wasm32"))]
fn preview_key(path: &str, listed_modified: Option<&str>, region: &wordcraft_geom::Region, ppp: f32) -> Option<PreviewKey> {
    use std::time::UNIX_EPOCH;

    let revision = if wordcraft_engine::file_access::check_read(path).is_err() {
        FileRevision::Unavailable
    } else if let Ok(metadata) = std::fs::metadata(path)
        && metadata.is_file()
    {
        let modified_ns = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|time| time.as_nanos());
        FileRevision::Current { length: metadata.len(), modified_ns, listed_modified: listed_modified.map(str::to_string) }
    } else {
        FileRevision::Unavailable
    };
    Some(base_key(path, revision, region, ppp))
}

#[cfg(target_arch = "wasm32")]
fn preview_key(_path: &str, _listed_modified: Option<&str>, _region: &wordcraft_geom::Region, _ppp: f32) -> Option<PreviewKey> {
    None
}

fn base_key(path: &str, revision: FileRevision, region: &wordcraft_geom::Region, ppp: f32) -> PreviewKey {
    PreviewKey {
        path: path.to_string(),
        revision,
        dpi_bits: ppp.to_bits(),
        region: format!("{:?}", region),
        font_generation: wordcraft_fonts::FontDb::global().generation(),
    }
}

struct Raster {
    image: ColorImage,
    aspect: f32,
}

#[cfg(not(target_arch = "wasm32"))]
fn render(path: &str, region: &wordcraft_geom::Region, ppp: f32) -> Result<Raster, String> {
    let bytes = wordcraft_engine::io::read_file_capped(path, MAX_TEMPLATE_BYTES, "preview template")?;
    let (doc, _) = wordcraft_engine::io::open_bytes_in(path, &bytes, None, region)?;
    render_document(doc, region, ppp)
}

#[cfg(not(target_arch = "wasm32"))]
fn render_document(doc: wordcraft_doc::Document, region: &wordcraft_geom::Region, ppp: f32) -> Result<Raster, String> {
    let mut session = wordcraft_engine::Session::new(doc);
    session.system_region = region.clone();
    let layout = session.export_layout();
    let page = layout.pages.first().ok_or_else(|| "template has no first page".to_string())?;
    if !page.w.is_finite() || !page.h.is_finite() || page.w <= 0.0 || page.h <= 0.0 {
        return Err("template first page has invalid dimensions".into());
    }
    let aspect = page.w / page.h;
    if !aspect.is_finite() || aspect <= 0.0 || !ppp.is_finite() || ppp <= 0.0 {
        return Err("template first page cannot be rendered".into());
    }
    let display_scale = (TILE_WIDTH / page.w).min(TILE_HEIGHT / page.h) * ppp;
    let raster_scale = (MAX_RASTER_SIDE - 1.0) / page.w.max(page.h);
    let scale = display_scale.min(raster_scale);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("template first page cannot be rendered".into());
    }
    let image = wordcraft_render::render_page(&session.doc, page, scale, &crate::canvas::screen_render_options());
    if image.width == 0 || image.height == 0 || image.width > MAX_RASTER_SIDE as u32 || image.height > MAX_RASTER_SIDE as u32 {
        return Err("template preview exceeded the raster limit".into());
    }
    let pixels = image.to_straight();
    Ok(Raster { image: ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &pixels), aspect })
}

#[cfg(target_arch = "wasm32")]
fn render(_path: &str, _region: &wordcraft_geom::Region, _ppp: f32) -> Result<Raster, String> {
    Err("template previews require a local file".into())
}

#[cfg(test)]
pub(super) fn stats(ctx: &Context) -> (usize, usize, Vec<String>) {
    let id = cache_id();
    let cache = ctx.data(|data| data.get_temp::<PreviewCache>(id)).unwrap_or_default();
    (cache.entries.len(), cache.attempts, cache.attempted_paths)
}

#[cfg(test)]
pub(super) fn rendered_pixel_hashes(ctx: &Context) -> Vec<u64> {
    let id = cache_id();
    let cache = ctx.data(|data| data.get_temp::<PreviewCache>(id)).unwrap_or_default();
    cache.rendered_pixel_hashes
}

#[cfg(test)]
fn image_hash(image: &ColorImage) -> u64 {
    image.pixels.iter().flat_map(|pixel| pixel.to_array()).fold(0xcbf29ce484222325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
pub(super) fn render_for_test(path: &str, region: &wordcraft_geom::Region, ppp: f32) -> Result<(ColorImage, f32), String> {
    render(path, region, ppp).map(|raster| (raster.image, raster.aspect))
}

#[cfg(test)]
pub(super) fn render_document_for_test(doc: wordcraft_doc::Document, region: &wordcraft_geom::Region, ppp: f32) -> Result<(ColorImage, f32), String> {
    render_document(doc, region, ppp).map(|raster| (raster.image, raster.aspect))
}

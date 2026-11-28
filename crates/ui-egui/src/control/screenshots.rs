use serde_json::{Value, json};

/// A page rendered by the engine, as base64 PNG in the reply (the same shape as the headless
/// MCP server's `ui.render`).
pub(super) fn render_png(session: &wordcraft_engine::Session, page: u64, scale: f32) -> Result<Value, String> {
    let page = usize::try_from(page.max(1)).unwrap_or(usize::MAX).saturating_sub(1);
    let l = session.export_layout();
    let pg = l.pages.get(page).ok_or_else(|| format!("no page {}", page.saturating_add(1)))?;
    let img = wordcraft_render::render_page(&session.doc, pg, scale.clamp(0.1, 8.0), &Default::default());
    let png = img.to_png();
    Ok(json!({"png": wordcraft_engine::cmd::insert::base64_encode(&png), "width": img.width, "height": img.height, "pages": l.pages.len()}))
}

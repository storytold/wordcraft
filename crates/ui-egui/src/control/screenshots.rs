use super::geometry::CaptureGeometry;
use super::{CaptureSlot, Sender, Value, WordApp, json};

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

/// The bounded admission policy for `ui.screenshot` captures. This is the screenshot queue;
/// the engine's 64-entry UI-intent queue is unrelated. The per-image and aggregate limits are
/// working-set limits, not just raw RGBA: encoding a PNG and base64-encoding it for a reply can
/// briefly need several times the raw pixel buffer.
pub(crate) const SCREENSHOT_MAX_REQUESTS: usize = 4;

pub(crate) const SCREENSHOT_MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;

pub(crate) const SCREENSHOT_MAX_AGGREGATE_BYTES: usize = 256 * 1024 * 1024;

pub(crate) const SCREENSHOT_MAX_RETRIES: u32 = 2;

/// An incompressible PNG of `raw` RGBA bytes. Deflate's stored-block worst case adds five bytes
/// per 64 KiB block; zlib/PNG framing and the encoder's small internal buffers get 64 KiB more.
/// Checked, because every byte count here comes from a hostile window size.
pub(super) fn png_upper_bound(raw: usize) -> Option<usize> {
    let blocks = raw.div_ceil(65_535);
    raw.checked_add(blocks.checked_mul(5)?)?.checked_add(64 * 1024)
}

/// The length of the base64 text for `bytes`, rounded up, as `base64_encode` produces it.
pub(super) fn base64_upper_bound(bytes: usize) -> Option<usize> {
    bytes.div_ceil(3).checked_mul(4)
}

/// Peak memory of one screenshot, in bytes: the completed `ColorImage`, the one straight-alpha
/// RGBA copy handed to the PNG encoder, the PNG buffer, and (for a reply without a path) the
/// base64 string. All of these can be live at once, so admission reserves their sum rather than
/// only the raw pixels or a PNG-only estimate. Checked throughout: a hostile surface size must
/// result in an error, never a wrapped budget.
pub(super) fn working_set(raw: usize, inline_reply: bool) -> Option<usize> {
    let png = png_upper_bound(raw)?;
    let mut peak = raw.checked_add(raw)?.checked_add(png)?;
    if inline_reply {
        peak = peak.checked_add(base64_upper_bound(png)?)?;
    }
    Some(peak)
}

/// One admitted capture. The request owns its token, path, deadline, immutable document identity,
/// geometry and reservation; the shared slot owns the image and its weak callback.
pub(crate) struct Shot {
    pub(crate) token: u64,
    pub(crate) path: Option<String>,
    pub(crate) reply: Sender<Value>,
    /// One monotonic deadline for the whole request, including retries.
    pub(crate) deadline: f64,
    pub(crate) ready_at: f64,
    pub(crate) frames: u32,
    pub(crate) issued: bool,
    pub(crate) retries: u32,
    /// The document the request was admitted for. It never changes; a document switch cancels.
    pub(crate) document: u64,
    /// The viewport of the window the request was admitted for. It never changes; bringing
    /// another window forward cancels.
    pub(crate) viewport: egui::ViewportId,
    pub(crate) geometry: CaptureGeometry,
    pub(crate) epoch: u64,
    pub(crate) expected: [usize; 2],
    /// Admission reservation, including the encoding working set.
    pub(crate) bytes: usize,
    pub(crate) slot: Option<CaptureSlot>,
    pub(crate) callback: Option<egui::ScreenshotCallback>,
}

/// All outstanding `ui.screenshot` requests.
#[derive(Default)]
pub(crate) struct Shots {
    pub(crate) token: u64,
    pub(crate) requests: Vec<Shot>,
}

impl Shots {
    /// Whether any screenshot request is outstanding.
    pub(crate) fn in_flight(&self) -> bool {
        !self.requests.is_empty()
    }

    /// Sums, saturating, the admission bytes reserved by outstanding screenshot requests.
    fn reserved(&self) -> usize {
        self.requests.iter().fold(0usize, |sum, shot| sum.saturating_add(shot.bytes))
    }
}

/// Builds a screenshot error reply carrying the request token.
fn error_reply(token: u64, message: impl std::fmt::Display) -> Value {
    json!({"ok": false, "error": message.to_string(), "request": token})
}

/// Sends one screenshot error reply, ignoring a disconnected receiver.
fn reject_shot(reply: Sender<Value>, token: u64, message: impl std::fmt::Display) {
    let _ = reply.send(error_reply(token, message));
}

/// Advances the screenshot token, wrapping and skipping zero so every request has a non-zero
/// token.
fn next_token(app: &mut WordApp) -> u64 {
    let mut token = app.shots.token.wrapping_add(1);
    if token == 0 {
        token = 1;
    }
    app.shots.token = token;
    token
}

/// Admit one screenshot request against the count and byte budgets **before** a capture is
/// issued. A rejected request gets one error reply, with its request token, and never evicts an
/// earlier request.
pub(crate) fn admit_screenshot(app: &mut WordApp, ctx: &egui::Context, path: Option<String>, reply: Sender<Value>) {
    let token = next_token(app);
    // An earlier request in this batch may have brought another window forward.
    app.refresh_capture_geometry(ctx);
    let Some(geometry) = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g }) else {
        reject_shot(reply, token, "screenshot failed: the window geometry is not known yet");
        return;
    };
    if let Some(reason) = geometry.unavailable_reason() {
        reject_shot(reply, token, format!("screenshot failed: {reason}"));
        return;
    }
    let Some(expected) = geometry.expected_size() else {
        reject_shot(reply, token, "screenshot failed: the window has no capturable size");
        return;
    };
    if let Some(reason) = geometry.oversize_reason() {
        reject_shot(reply, token, reason);
        return;
    }
    let inline_reply = path.is_none();
    let Some(bytes) = geometry.reserved_bytes(inline_reply) else {
        reject_shot(reply, token, "screenshot failed: the window size overflows the screenshot budget");
        return;
    };
    if bytes > SCREENSHOT_MAX_IMAGE_BYTES {
        reject_shot(
            reply,
            token,
            format!(
                "screenshot budget exceeded: {bytes} bytes reserved for one {}×{} image, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit",
                expected[0], expected[1]
            ),
        );
        return;
    }
    if app.shots.requests.len() >= SCREENSHOT_MAX_REQUESTS {
        reject_shot(
            reply,
            token,
            format!("screenshot budget exceeded: {} requests are already outstanding (limit {SCREENSHOT_MAX_REQUESTS})", app.shots.requests.len()),
        );
        return;
    }
    let reserved = app.shots.reserved().saturating_add(bytes);
    if reserved > SCREENSHOT_MAX_AGGREGATE_BYTES {
        reject_shot(
            reply,
            token,
            format!("screenshot budget exceeded: {reserved} bytes reserved, over the {SCREENSHOT_MAX_AGGREGATE_BYTES}-byte aggregate limit"),
        );
        return;
    }
    let (slot, callback) = CaptureSlot::new(ctx, ctx.viewport_id(), SCREENSHOT_MAX_IMAGE_BYTES);
    let now = crate::now_ms();
    app.shots.requests.push(Shot {
        token,
        path,
        reply,
        deadline: now + 8000.0,
        ready_at: now + 120.0,
        frames: 0,
        issued: false,
        retries: 0,
        document: geometry.document(),
        viewport: geometry.viewport,
        geometry,
        epoch: app.capture_epoch,
        expected,
        bytes,
        slot: Some(slot),
        callback: Some(callback),
    });
    // The capture command is issued from `ui`; while hidden, logic keeps asking for a painted
    // frame until it gets there.
    ctx.request_repaint();
}

/// Issued only from `ui`, after the layout has settled. Counts one frame per host frame on the
/// first pass, revalidates geometry and the byte reservation, and sends one callback command.
/// `current_pass_index() == 0` is meaningful here because this function is already in a UI pass.
pub(crate) fn issue_screenshots(app: &mut WordApp, ctx: &egui::Context) {
    let now = crate::now_ms();
    let first_pass = ctx.current_pass_index() == 0;
    let mut failures: Vec<(usize, String)> = Vec::new();
    for i in 0..app.shots.requests.len() {
        let (issued, ready_at, frames) = {
            let shot = &app.shots.requests[i];
            (shot.issued, shot.ready_at, shot.frames)
        };
        // Each window issues its own captures, from a pass that drew its layout.
        if issued || app.shots.requests[i].viewport != ctx.viewport_id() {
            continue;
        }
        let frames = if first_pass { frames.saturating_add(1) } else { frames };
        app.shots.requests[i].frames = frames;
        if now < ready_at || frames < 3 {
            continue;
        }
        let Some(current) = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g }) else {
            failures.push((i, "screenshot failed: the window geometry is not known yet".into()));
            continue;
        };
        if let Some(reason) = current.unavailable_reason() {
            failures.push((i, format!("screenshot failed: {reason} while the capture was queued")));
            continue;
        }
        if let Some(reason) = current.moved_reason(app.shots.requests[i].viewport, app.shots.requests[i].document) {
            failures.push((i, reason.into()));
            continue;
        }
        let Some(expected) = current.expected_size() else {
            failures.push((i, "screenshot failed: the window size overflows the screenshot budget".into()));
            continue;
        };
        if let Some(reason) = current.oversize_reason() {
            failures.push((i, reason));
            continue;
        }
        let inline_reply = app.shots.requests[i].path.is_none();
        let Some(bytes) = current.reserved_bytes(inline_reply) else {
            failures.push((i, "screenshot failed: the window size overflows the screenshot budget".into()));
            continue;
        };
        if bytes > SCREENSHOT_MAX_IMAGE_BYTES {
            failures.push((
                i,
                format!(
                    "screenshot budget exceeded: {bytes} bytes reserved for one image, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit"
                ),
            ));
            continue;
        }
        let others = app.shots.requests.iter().enumerate().filter(|(j, _)| *j != i).fold(0usize, |sum, (_, shot)| sum.saturating_add(shot.bytes));
        if others.saturating_add(bytes) > SCREENSHOT_MAX_AGGREGATE_BYTES {
            failures.push((i, format!("screenshot budget exceeded: outstanding requests would reserve over {SCREENSHOT_MAX_AGGREGATE_BYTES} bytes")));
            continue;
        }
        let epoch = app.capture_epoch;
        let shot = &mut app.shots.requests[i];
        shot.geometry = current;
        shot.expected = expected;
        shot.bytes = bytes;
        shot.epoch = epoch;
        if let Some(callback) = shot.callback.take() {
            ctx.send_viewport_cmd_to(shot.viewport, egui::ViewportCommand::Screenshot(callback));
            shot.issued = true;
        }
    }
    for (i, message) in failures.into_iter().rev() {
        let shot = app.shots.requests.remove(i);
        if let Some(slot) = shot.slot {
            slot.cancel();
        }
        reject_shot(shot.reply, shot.token, message);
    }
    if app.shots.in_flight() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// A completed image is either stale because the surface changed (safe to retry once within the
/// deadline) or invalid for this request (report it at once; retrying cannot help).
pub(super) enum ImageCheck {
    Stale(String),
    Invalid(String),
}

/// The callback's physical size is the host surface texture's extent, rounded independently of
/// the expected `round(logical points × pixels_per_point)`. One physical pixel on each axis is
/// therefore the same surface seen through a different rounding boundary, not a resize.
pub(super) fn dimensions_are_a_rounded_variant(expected: [usize; 2], actual: [usize; 2]) -> bool {
    // Both axes are derived from the same logical viewport and the same pixels-per-point; each
    // may round at a boundary one pixel away, but a larger change on either axis is a resize.
    expected[0].abs_diff(actual[0]) <= 1 && expected[1].abs_diff(actual[1]) <= 1
}

/// Rejects an empty, overflowing, underfilled or oversized image. A size one pixel away from the
/// expected surface is accepted as a rounding variant; a larger difference is stale.
pub(super) fn validate_image(image: &egui::ColorImage, expected: [usize; 2]) -> Result<(), ImageCheck> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return Err(ImageCheck::Invalid("the screenshot has no pixels".to_string()));
    }
    // A screenshot is issued for one exact logical viewport. Exact equality would reject a valid
    // fractional-DPI capture whose callback rounds by one pixel; a larger difference, or a
    // different aspect, is a different surface.
    if !dimensions_are_a_rounded_variant(expected, [width, height]) {
        return Err(ImageCheck::Stale(format!(
            "screenshot is {width}×{height} pixels, expected {}×{}: the window changed size while it was captured",
            expected[0], expected[1]
        )));
    }
    let pixels = width.checked_mul(height).ok_or_else(|| ImageCheck::Invalid("screenshot dimensions overflow".to_string()))?;
    if image.pixels.len() != pixels {
        return Err(ImageCheck::Invalid(format!("screenshot has {} pixels for its {width}×{height} dimensions", image.pixels.len())));
    }
    // The largest image allowed by the one-pixel rounding allowance. The reservation was made for
    // the expected size; a sibling one pixel wider or taller is still the same surface.
    let largest = expected[0]
        .checked_add(1)
        .and_then(|w| expected[1].checked_add(1).and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| ImageCheck::Invalid("screenshot dimensions overflow".to_string()))?;
    if pixels > largest {
        return Err(ImageCheck::Invalid(format!("screenshot has {pixels} pixels, over the {largest} allowed by its reservation")));
    }
    Ok(())
}

/// What one pass over an outstanding screenshot request decides.
enum ShotAction {
    Keep,
    Remove(Value),
    Restart,
}

/// Classify one pending request: deadline first, then admission state, then the completed slot.
fn take_screenshot_action(app: &mut WordApp, index: usize, now: f64, current: Option<CaptureGeometry>, epoch: u64) -> ShotAction {
    let completed = {
        let shot = &mut app.shots.requests[index];
        if now >= shot.deadline {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, "no frame was presented before the screenshot timeout"));
        }
        // Before the issue check: a window that closed never draws to issue its capture.
        if let Some(reason) = current.and_then(|geometry| geometry.moved_reason(shot.viewport, shot.document)) {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, reason));
        }
        if !shot.issued {
            return ShotAction::Keep;
        }
        if let Some(reason) = current.and_then(CaptureGeometry::unavailable_reason) {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, format!("screenshot failed: {reason} while the capture was pending")));
        }
        shot.slot.as_ref().and_then(|slot| slot.take())
    };
    match completed {
        None => ShotAction::Keep,
        Some(Err(error)) => ShotAction::Remove(error_reply(app.shots.requests[index].token, error)),
        Some(Ok(image)) => completed_screenshot_action(app, index, image, current, epoch),
    }
}

/// Judge one taken image: its geometry and epoch identity first, then the image's own validity.
fn completed_screenshot_action(
    app: &mut WordApp,
    index: usize,
    image: std::sync::Arc<egui::ColorImage>,
    current: Option<CaptureGeometry>,
    epoch: u64,
) -> ShotAction {
    let shot = &app.shots.requests[index];
    let stale = current != Some(shot.geometry) || epoch != shot.epoch;
    if stale {
        if shot.retries < SCREENSHOT_MAX_RETRIES {
            return ShotAction::Restart;
        }
        return ShotAction::Remove(error_reply(shot.token, "screenshot geometry changed while it was captured; no image was written"));
    }
    match validate_image(&image, shot.expected) {
        Err(ImageCheck::Stale(_)) if shot.retries < SCREENSHOT_MAX_RETRIES => ShotAction::Restart,
        Err(ImageCheck::Stale(message) | ImageCheck::Invalid(message)) => ShotAction::Remove(error_reply(shot.token, message)),
        Ok(()) => ShotAction::Remove(save_screenshot(image.as_ref(), shot.path.as_deref(), shot.token)),
    }
}

/// Drop one request at `index` with its reply, cancelling any slot still held.
fn remove_shot(app: &mut WordApp, index: usize, value: Value) {
    let shot = app.shots.requests.remove(index);
    if let Some(slot) = shot.slot {
        slot.cancel();
    }
    let _ = shot.reply.send(value);
}

/// Drop one request at `index` and send an error reply for it.
fn reject_removed_shot(app: &mut WordApp, index: usize, message: impl std::fmt::Display) {
    let shot = app.shots.requests.remove(index);
    if let Some(slot) = shot.slot {
        slot.cancel();
    }
    reject_shot(shot.reply, shot.token, message);
}

/// Reissue the request at `index` against the current geometry. Returns whether it was reissued:
/// a request removed for a refusal keeps `index` for the request that slides into its place.
fn restart_shot(app: &mut WordApp, ctx: &egui::Context, index: usize, now: f64, current: Option<CaptureGeometry>, epoch: u64) -> bool {
    let Some(current) = current else {
        reject_removed_shot(app, index, "screenshot failed: the window geometry is not known");
        return false;
    };
    let Some(expected) = current.expected_size() else {
        reject_removed_shot(app, index, "screenshot failed: the window size overflows the screenshot budget");
        return false;
    };
    if let Some(reason) = current.oversize_reason() {
        reject_removed_shot(app, index, reason);
        return false;
    }
    let inline_reply = app.shots.requests[index].path.is_none();
    let Some(bytes) = current.reserved_bytes(inline_reply) else {
        reject_removed_shot(app, index, "screenshot failed: the window size overflows the screenshot budget");
        return false;
    };
    let others = app.shots.requests.iter().enumerate().filter(|(j, _)| *j != index).fold(0usize, |sum, (_, shot)| sum.saturating_add(shot.bytes));
    if bytes > SCREENSHOT_MAX_IMAGE_BYTES || others.saturating_add(bytes) > SCREENSHOT_MAX_AGGREGATE_BYTES {
        reject_removed_shot(app, index, "screenshot budget exceeded while retrying after a resize");
        return false;
    }
    let (slot, callback) = CaptureSlot::new(ctx, ctx.viewport_id(), SCREENSHOT_MAX_IMAGE_BYTES);
    let shot = &mut app.shots.requests[index];
    shot.slot = Some(slot);
    shot.callback = Some(callback);
    shot.issued = false;
    shot.ready_at = now + 120.0;
    shot.frames = 0;
    shot.retries = shot.retries.saturating_add(1);
    shot.geometry = current;
    shot.expected = expected;
    shot.bytes = bytes;
    shot.epoch = epoch;
    true
}

/// Called from `logic`: take each completed slot once, check the deadline **before** writing,
/// validate request/document/geometry identity, and only then encode and reply. A late, stale or
/// oversized image is dropped without a file; a document switch cancels instead of rebinding.
pub(crate) fn drain_screenshots(app: &mut WordApp, ctx: &egui::Context) {
    let now = crate::now_ms();
    let current = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g });
    let epoch = app.capture_epoch;
    let mut i = 0;
    while i < app.shots.requests.len() {
        match take_screenshot_action(app, i, now, current, epoch) {
            ShotAction::Keep => i += 1,
            ShotAction::Remove(value) => remove_shot(app, i, value),
            ShotAction::Restart => {
                if restart_shot(app, ctx, i, now, current, epoch) {
                    i += 1;
                }
            }
        }
    }
    if app.shots.in_flight() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// The screenshot as a straight-alpha PNG, using one RGBA copy rather than the extra full-image
/// copy [`wordcraft_render::Rendered::to_png`] makes for its premultiplied input. The callback's
/// `ColorImage` is premultiplied; PNG stores straight alpha.
fn screenshot_png(image: &egui::ColorImage) -> Option<Vec<u8>> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = width.checked_mul(height)?;
    if image.pixels.len() != pixels {
        return None;
    }
    let mut rgba = Vec::with_capacity(pixels.checked_mul(4)?);
    for color in &image.pixels {
        let [r, g, b, a] = color.to_array();
        if a == 0 || a == 255 {
            rgba.extend_from_slice(&[r, g, b, a]);
        } else {
            let straight = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            rgba.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    let img = image::RgbaImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, rgba)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// The window screenshot: written to `path` when one was given, else base64 PNG in the reply
/// (never a fixed name in the shared temporary folder). Every reply carries the request token so
/// a client can correlate it with the admission, timeout and error replies.
pub fn save_screenshot(image: &egui::ColorImage, path: Option<&str>, token: u64) -> Value {
    let [w, h] = image.size;
    let Some(raw) = w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4)) else {
        return error_reply(token, "screenshot dimensions overflow the byte count");
    };
    let inline_reply = path.is_none();
    let Some(peak) = working_set(raw, inline_reply) else {
        return error_reply(token, "screenshot working set overflows the byte count");
    };
    if peak > SCREENSHOT_MAX_IMAGE_BYTES {
        return error_reply(
            token,
            format!("screenshot needs {peak} bytes of working set, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit"),
        );
    }
    let Some(png) = screenshot_png(image) else {
        return error_reply(token, "screenshot PNG encoding failed");
    };
    if png.len() > SCREENSHOT_MAX_IMAGE_BYTES {
        return error_reply(token, format!("screenshot PNG uses {} bytes, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit", png.len()));
    }
    let Some(path) = path else {
        let encoded = wordcraft_engine::cmd::insert::base64_encode(&png);
        if encoded.len() > SCREENSHOT_MAX_IMAGE_BYTES {
            return error_reply(
                token,
                format!("screenshot base64 reply uses {} bytes, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit", encoded.len()),
            );
        }
        return json!({"ok": true, "request": token, "result": {"png": encoded, "width": w, "height": h}});
    };
    #[cfg(target_arch = "wasm32")]
    let _ = (&png, path);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return error_reply(token, format!("{path}: a screenshot path must end in .png"));
        }
        if let Err(e) = wordcraft_engine::io::write_file(path, &png) {
            return error_reply(token, e);
        }
    }
    json!({"ok": true, "request": token, "result": {"path": path, "width": w, "height": h}})
}

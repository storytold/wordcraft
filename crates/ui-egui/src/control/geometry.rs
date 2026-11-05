use super::screenshots::working_set;

/// The surface reported by one `RawInput`, before it is tied to a document. Captured in the host
/// input hook because `InputState::viewport_rect` and the last painted screen rect are stale on a
/// hidden logic tick; RawInput is the host's answer for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfaceGeometry {
    viewport: egui::Vec2,
    /// Native pixels per point times egui's zoom, so a zoom change is its own epoch.
    pixels_per_point: f32,
    max_texture_side: usize,
    minimized: bool,
    occluded: bool,
}

impl SurfaceGeometry {
    /// Derives this frame's capturable surface from raw viewport data, rejecting a missing, non-finite
    /// or zero-sized visible surface and recording minimized or occluded state.
    pub(crate) fn from_raw(raw: &egui::RawInput, ctx: &egui::Context) -> Option<Self> {
        Self::from_viewport(raw.viewports.get(&raw.viewport_id)?, raw.screen_rect, raw.max_texture_side, ctx)
    }

    /// [`Self::from_raw`] for one viewport's info, content rectangle and texture limit.
    pub(crate) fn from_viewport(
        viewport: &egui::ViewportInfo,
        screen_rect: Option<egui::Rect>,
        max_texture_side: Option<usize>,
        ctx: &egui::Context,
    ) -> Option<Self> {
        let minimized = viewport.minimized == Some(true);
        let occluded = viewport.occluded == Some(true);
        // A minimized/occluded surface may report no rectangle at all. Its stale size is harmless
        // because such a surface is never capturable; a visible surface without a rectangle is not
        // fresh enough to use.
        let rect = match viewport.inner_rect.or(screen_rect) {
            Some(rect) => rect,
            None if minimized || occluded => ctx.content_rect(),
            None => return None,
        };
        let zoom = ctx.zoom_factor();
        let pixels_per_point = viewport.native_pixels_per_point.map_or_else(|| ctx.pixels_per_point(), |native| native * zoom);
        let max_texture_side = max_texture_side.filter(|side| *side > 0).unwrap_or_else(|| ctx.input(|i| i.max_texture_side));
        if !rect.width().is_finite() || !rect.height().is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        Some(Self { viewport: rect.size(), pixels_per_point, max_texture_side, minimized, occluded })
    }

    /// Pairs the surface geometry with the window's viewport and the document id a capture
    /// belongs to.
    pub(crate) fn with_target(self, viewport: egui::ViewportId, document: u64) -> CaptureGeometry {
        CaptureGeometry { surface: self, viewport, document }
    }
}

/// The viewport geometry and document identity a capture is admitted, issued and completed
/// against. `WordApp` keeps a monotonic epoch: any change, including a change back, makes an
/// older image stale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CaptureGeometry {
    pub(super) surface: SurfaceGeometry,
    /// The active window's viewport, which the capture command goes to.
    pub(super) viewport: egui::ViewportId,
    pub(super) document: u64,
}

impl CaptureGeometry {
    /// Returns the document id the capture was admitted for.
    pub(crate) fn document(self) -> u64 {
        self.document
    }

    /// Why a capture admitted for `viewport` and `document` can't go on against this geometry:
    /// another window was brought forward, or the window shows another document.
    pub(super) fn moved_reason(self, viewport: egui::ViewportId, document: u64) -> Option<&'static str> {
        if self.viewport != viewport {
            Some("screenshot failed: another window was brought forward while the capture was queued")
        } else if self.document != document {
            Some("screenshot failed: the document changed while the capture was queued")
        } else {
            None
        }
    }

    /// Explains why the surface cannot be captured, when it is minimized or fully occluded, and
    /// returns `None` when it can.
    pub(crate) fn unavailable_reason(self) -> Option<&'static str> {
        if self.surface.minimized {
            Some("the window is minimized")
        } else if self.surface.occluded {
            Some("the window is fully occluded")
        } else {
            None
        }
    }

    /// The screenshot's physical size, rounded to the nearest pixel.
    ///
    /// The renderer's maximum texture side is deliberately not applied here. It is a texture
    /// limit, not the size of the surface the callback reports; capping one axis would distort
    /// the expected aspect and reject the real image. [`Self::oversize_reason`] rejects a window
    /// that cannot have a surface texture at all, before admission.
    pub(crate) fn expected_size(self) -> Option<[usize; 2]> {
        if self.unavailable_reason().is_some() {
            return None;
        }
        let axis = |points: f32| -> Option<usize> {
            let px = (f64::from(points) * f64::from(self.surface.pixels_per_point)).round();
            if !px.is_finite() || px <= 0.0 || px > usize::MAX as f64 {
                return None;
            }
            Some(px as usize)
        };
        Some([axis(self.surface.viewport.x)?, axis(self.surface.viewport.y)?])
    }

    /// Why this surface is too large to capture at all: a window larger than the renderer's
    /// maximum texture side has no surface texture to copy. Reject it explicitly rather than
    /// silently capping the expected size and failing exact comparisons later.
    pub(crate) fn oversize_reason(self) -> Option<String> {
        let [width, height] = self.expected_size()?;
        let max = self.surface.max_texture_side;
        (max > 0 && (width > max || height > max))
            .then(|| format!("screenshot failed: the window is {width}×{height} pixels, over the renderer's {max}-pixel texture limit"))
    }

    /// Checked `width * height * 4` bytes for the expected image.
    pub(crate) fn expected_bytes(self) -> Option<usize> {
        let [width, height] = self.expected_size()?;
        width.checked_mul(height)?.checked_mul(4)
    }

    /// The admission reservation for the complete capture working set (image plus encoding).
    /// `inline_reply` includes the base64 string a reply without a path produces.
    pub(crate) fn reserved_bytes(self, inline_reply: bool) -> Option<usize> {
        working_set(self.expected_bytes()?, inline_reply)
    }
}

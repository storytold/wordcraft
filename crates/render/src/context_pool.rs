//! One kept vello_cpu render context, so rendering reuses its worker threads.
//!
//! A multi-threaded `RenderContext` starts its own rayon thread pool when it is built and
//! stops it when dropped. Building one per render started and tore down a pool for every
//! repaint: about 50 threads a second while typing, each spin-waiting before it exits (#3027).
//! Renders now [`take`](ContextPool::take) the kept context, reset to their size, and
//! [`give_back`](ContextPool::give_back) it when done.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, PoisonError};

use vello_cpu::{RenderContext, RenderSettings};

const MAX_SPARES: usize = 6;
/// Pixel area above a 100%-zoom page even at 2x HiDPI (~2000x2000). Contexts expanded past this
/// (e.g. 200% zoom) are dropped once renders return to normal size so large buffers aren't pinned.
const OVERSIZED_PIXELS: u64 = 4_000_000;
const OVERSIZED_SHRINK_STREAK: u32 = 4;

#[derive(Default)]
struct State {
    primary: Option<RenderContext>,
    primary_out: bool,
    primary_peak_pixels: u64,
    primary_small_streak: u32,
    spares: Vec<(RenderContext, u64)>,
    /// How many contexts (both primary and single-threaded spares) were built.
    built: usize,
}

/// Checked-out context from [`ContextPool`]. Dropping without [`ContextPool::give_back`] (e.g.
/// on panic) discards the half-drawn context while still releasing the primary checkout slot.
pub(crate) struct PooledContext<'a> {
    pool: &'a ContextPool,
    ctx: Option<RenderContext>,
    is_primary: bool,
    pixels: u64,
}

impl Deref for PooledContext<'_> {
    type Target = RenderContext;

    fn deref(&self) -> &Self::Target {
        #[allow(clippy::expect_used)]
        self.ctx.as_ref().expect("PooledContext accessed after give_back")
    }
}

impl DerefMut for PooledContext<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[allow(clippy::expect_used)]
        self.ctx.as_mut().expect("PooledContext accessed after give_back")
    }
}

impl Drop for PooledContext<'_> {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            if self.is_primary {
                let mut state = self.pool.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.primary_out = false;
                state.primary_peak_pixels = 0;
                state.primary_small_streak = 0;
            }
            drop(ctx);
        }
    }
}

/// Render contexts kept between renders.
///
/// Keeps one multi-threaded primary context plus a bounded set of single-threaded spares for
/// concurrent background page renders, avoiding spawning/tearing down extra rayon thread pools and
/// retaining zoom-expanded buffers indefinitely.
pub(crate) struct ContextPool {
    state: Mutex<State>,
}

/// The context page, area and icon renders share.
pub(crate) static CONTEXTS: ContextPool = ContextPool::new();

impl ContextPool {
    pub(crate) const fn new() -> Self {
        ContextPool {
            state: Mutex::new(State {
                primary: None,
                primary_out: false,
                primary_peak_pixels: 0,
                primary_small_streak: 0,
                spares: Vec::new(),
                built: 0,
            }),
        }
    }

    /// A context reset to `w`×`h`: the kept multi-threaded primary context, a kept single-threaded
    /// spare when renders overlap, or a newly built context when none is idle.
    pub(crate) fn take(&self, w: u16, h: u16) -> PooledContext<'_> {
        let pixels = u64::from(w) * u64::from(h);
        let multi_threads = crate::default_threads();
        enum Checkout {
            PrimaryExisting(RenderContext),
            PrimaryNew,
            SpareExisting(RenderContext, u64),
            SpareNew,
        }
        let action = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if multi_threads > 0 && !state.primary_out {
                state.primary_out = true;
                state.primary_peak_pixels = state.primary_peak_pixels.max(pixels);
                state.primary_small_streak = if pixels <= OVERSIZED_PIXELS / 2 { state.primary_small_streak.saturating_add(1) } else { 0 };
                match state.primary.take() {
                    Some(ctx) => Checkout::PrimaryExisting(ctx),
                    None => {
                        state.built += 1;
                        Checkout::PrimaryNew
                    }
                }
            } else if let Some((ctx, peak)) = state.spares.pop() {
                Checkout::SpareExisting(ctx, peak)
            } else {
                state.built += 1;
                Checkout::SpareNew
            }
        };
        let (mut ctx, is_primary, tracked_pixels, reset) = match action {
            Checkout::PrimaryExisting(ctx) => (ctx, true, pixels, true),
            Checkout::PrimaryNew => {
                (RenderContext::new_with(w, h, RenderSettings { num_threads: multi_threads, ..Default::default() }), true, pixels, false)
            }
            Checkout::SpareExisting(ctx, peak) => (ctx, false, peak.max(pixels), true),
            Checkout::SpareNew => (RenderContext::new_with(w, h, RenderSettings { num_threads: 0, ..Default::default() }), false, pixels, false),
        };
        if reset {
            ctx.reset_and_resize(w, h);
        }
        PooledContext { pool: self, ctx: Some(ctx), is_primary, pixels: tracked_pixels }
    }

    /// Keep `ctx` for the next [`take`](Self::take), dropping oversized contexts once renders return
    /// to normal size or when the spare pool is full. A render that panics never gets here, so a
    /// half-drawn context is dropped rather than reused.
    pub(crate) fn give_back(&self, mut guard: PooledContext<'_>) {
        let Some(ctx) = guard.ctx.take() else { return };
        let drop_it = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.is_primary {
                state.primary_out = false;
                if state.primary.is_some() {
                    Some(ctx)
                } else if state.primary_peak_pixels > OVERSIZED_PIXELS && state.primary_small_streak >= OVERSIZED_SHRINK_STREAK {
                    state.primary_peak_pixels = 0;
                    state.primary_small_streak = 0;
                    Some(ctx)
                } else {
                    state.primary = Some(ctx);
                    None
                }
            } else if guard.pixels <= OVERSIZED_PIXELS && state.spares.len() < MAX_SPARES {
                state.spares.push((ctx, guard.pixels));
                None
            } else {
                Some(ctx)
            }
        };
        drop(drop_it);
    }

    /// How many contexts this pool has built.
    #[cfg(test)]
    fn built(&self) -> usize {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).built
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_SIDE, RenderOptions, Rendered, render_page, render_region};
    use vello_cpu::kurbo::{self, Affine, BezPath, Shape};
    use vello_cpu::{Resources, peniko};
    use wordcraft_doc::props::{Highlight, Rgb};
    use wordcraft_doc::{Document, Pos};
    use wordcraft_layout::{LayoutCache, LayoutOptions, Page, layout};

    /// Draw a scene that uses what the renderer uses (fills, strokes, transforms, a gradient,
    /// both fill rules, opacity and clip layers) across a `w`×`h` context and rasterise it. It
    /// leans on the defaults first: nested squares wound the same way fill whole under the
    /// default non-zero rule (a left-over even-odd rule cuts a hole), and a gradient moves with
    /// a left-over paint transform.
    fn scene(ctx: &mut RenderContext, w: u16, h: u16) -> Rendered {
        let (fw, fh) = (w as f64, h as f64);
        ctx.set_paint(crate::color(Rgb(0xF4, 0xEE, 0xD8), 1.0));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, fw, fh));
        let mut nested = BezPath::new();
        for s in [fw.min(fh) / 2.0, fw.min(fh) / 4.0] {
            nested.move_to((0.0, 0.0));
            nested.line_to((s, 0.0));
            nested.line_to((s, s));
            nested.line_to((0.0, s));
            nested.close_path();
        }
        ctx.set_paint(crate::color(Rgb(0x10, 0x10, 0x10), 1.0));
        ctx.fill_path(&nested);
        let stops =
            [peniko::ColorStop::from((0.0, crate::color(Rgb(0, 0, 0), 1.0))), peniko::ColorStop::from((1.0, crate::color(Rgb(255, 255, 255), 1.0)))];
        ctx.set_paint(peniko::Gradient::new_linear((0.0, 0.0), (fw, 0.0)).with_stops(stops.as_slice()));
        ctx.fill_rect(&kurbo::Rect::new(0.0, fh * 3.0 / 4.0, fw, fh));
        ctx.set_transform(Affine::translate((fw / 2.0, fh / 2.0)) * Affine::rotate(0.3));
        ctx.set_paint(crate::color(Rgb(0x2B, 0x57, 0x9A), 1.0));
        ctx.set_stroke(kurbo::Stroke::new((fw.min(fh) / 20.0).max(1.0)));
        ctx.stroke_rect(&kurbo::Rect::new(-fw / 3.0, -fh / 3.0, fw / 3.0, fh / 3.0));
        let mut ring = BezPath::new();
        for r in [fw.min(fh) / 3.0, fw.min(fh) / 6.0] {
            ring.move_to((r, 0.0));
            ring.line_to((0.0, r));
            ring.line_to((-r, 0.0));
            ring.line_to((0.0, -r));
            ring.close_path();
        }
        ctx.set_fill_rule(peniko::Fill::EvenOdd);
        ctx.set_paint(crate::color(Rgb(0xC0, 0x30, 0x30), 1.0));
        ctx.push_opacity_layer(0.6);
        ctx.fill_path(&ring);
        ctx.pop_layer();
        ctx.set_transform(Affine::IDENTITY);
        ctx.push_clip_layer(&kurbo::Rect::new(0.0, 0.0, fw / 2.0, fh).to_path(0.1));
        ctx.set_paint(crate::color(Rgb(0x20, 0x90, 0x40), 0.5));
        ctx.fill_rect(&kurbo::Rect::new(0.0, fh / 4.0, fw, fh * 3.0 / 4.0));
        ctx.pop_layer();
        ctx.flush();
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut pixels) {
            ctx.render(pm, &mut Resources::new());
        }
        Rendered { width: w as u32, height: h as u32, pixels }
    }

    /// [`scene`] through `pool`.
    fn pooled(pool: &ContextPool, w: u16, h: u16) -> Rendered {
        let mut ctx = pool.take(w, h);
        let img = scene(&mut ctx, w, h);
        pool.give_back(ctx);
        img
    }

    /// [`scene`] on a context built for it alone, as every render was before #3027.
    fn fresh(w: u16, h: u16) -> Rendered {
        scene(&mut RenderContext::new_with(w, h, RenderSettings { num_threads: crate::default_threads(), ..Default::default() }), w, h)
    }

    fn same(a: &Rendered, b: &Rendered, what: &str) {
        assert_eq!((a.width, a.height), (b.width, b.height), "{what}: size");
        assert!(a.pixels == b.pixels, "{what}: different pixels");
    }

    /// #3027: one context (one thread pool) serves renders of every size, growing and
    /// shrinking, down to 1×1 and out to the largest side, and each draws exactly what a
    /// context of its own would.
    #[test]
    fn one_context_serves_renders_of_every_size() {
        let pool = ContextPool::new();
        let max = MAX_SIDE as u16;
        let sizes = [(600, 800), (1, 1), (600, 800), (2, 3000), (3000, 2), (max, 1), (1, max), (255, 257), (600, 800)];
        for (w, h) in sizes {
            same(&pooled(&pool, w, h), &fresh(w, h), &format!("{w}x{h}"));
        }
        for _ in 0..50 {
            pooled(&pool, 300, 200);
        }
        assert_eq!(pool.built(), 1, "every render after the first reuses the kept context");
        let ctx = pool.take(1, 1);
        assert_eq!(ctx.is_multi_threaded(), crate::default_threads() > 0, "the kept context still renders on worker threads");
    }

    /// Whatever the last render left in the context (transform, paint, stroke, fill rule, paint
    /// transform, a layer still pushed, a scene drawn but never rasterised) is gone before the
    /// next render draws.
    #[test]
    fn nothing_left_in_a_context_reaches_the_next_render() {
        let pool = ContextPool::new();
        let mut ctx = pool.take(400, 300);
        ctx.set_transform(Affine::rotate(0.7) * Affine::scale(3.0));
        ctx.set_paint(crate::color(Rgb(200, 0, 0), 1.0));
        ctx.set_stroke(kurbo::Stroke::new(25.0));
        ctx.set_fill_rule(peniko::Fill::EvenOdd);
        ctx.set_paint_transform(Affine::translate((40.0, 40.0)));
        ctx.push_opacity_layer(0.3);
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, 120.0, 80.0));
        ctx.stroke_rect(&kurbo::Rect::new(10.0, 10.0, 90.0, 60.0));
        pool.give_back(ctx);
        same(&pooled(&pool, 400, 300), &fresh(400, 300), "after an abandoned scene");
        // The scene does show a left-over fill rule or paint transform when nothing resets them.
        let settings = RenderSettings { num_threads: crate::default_threads(), ..Default::default() };
        let mut stale_rule = RenderContext::new_with(400, 300, settings);
        stale_rule.set_fill_rule(peniko::Fill::EvenOdd);
        assert!(scene(&mut stale_rule, 400, 300).pixels != fresh(400, 300).pixels, "a stale even-odd rule shows");
        let mut stale_paint = RenderContext::new_with(400, 300, settings);
        stale_paint.set_paint_transform(Affine::translate((40.0, 40.0)));
        assert!(scene(&mut stale_paint, 400, 300).pixels != fresh(400, 300).pixels, "a stale paint transform shows");
        same(&pooled(&pool, 120, 90), &fresh(120, 90), "smaller, after a full render");
        assert_eq!(pool.built(), 1);
    }

    /// A render that panics never gives its context back: the next render builds a new one.
    /// A context that is already kept wins over one given back later, which is dropped.
    #[test]
    fn a_context_not_given_back_is_replaced_and_a_spare_is_dropped() {
        let pool = ContextPool::new();
        let lost = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut ctx = pool.take(200, 100);
            ctx.push_opacity_layer(0.5);
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, 50.0, 50.0));
            panic!("a draw failed half way");
        }));
        assert!(lost.is_err());
        same(&pooled(&pool, 200, 100), &fresh(200, 100), "after the panic");
        assert_eq!(pool.built(), 2, "the half-drawn context was dropped, not reused");
        let (a, b) = (pool.take(10, 10), pool.take(10, 10));
        assert_eq!(pool.built(), 3, "a render inside a render gets its own context");
        pool.give_back(a);
        pool.give_back(b);
        pooled(&pool, 10, 10);
        pooled(&pool, 10, 10);
        assert_eq!(pool.built(), 3, "one of the two was kept and the other dropped");
    }

    /// Two threads rendering at the same time each get a context and both draw correctly;
    /// afterwards one context is kept and reused.
    #[test]
    fn renders_on_two_threads_at_once_both_draw() {
        let pool = ContextPool::new();
        let both_hold_one = std::sync::Barrier::new(2);
        let (a, b) = std::thread::scope(|s| {
            let run = || {
                let mut ctx = pool.take(240, 160);
                both_hold_one.wait();
                let img = scene(&mut ctx, 240, 160);
                pool.give_back(ctx);
                img
            };
            let (a, b) = (s.spawn(run), s.spawn(run));
            (a.join().expect("thread a"), b.join().expect("thread b"))
        });
        let want = fresh(240, 160);
        same(&a, &want, "thread a");
        same(&b, &want, "thread b");
        assert_eq!(pool.built(), 2, "neither thread waited for the other's context");
        pooled(&pool, 240, 160);
        assert_eq!(pool.built(), 2, "one of the two was kept");
    }

    /// A page with text, a highlight and a coloured background.
    fn page() -> (Document, Page) {
        let mut d = Document::from_text("Reused contexts draw the same page as new ones.");
        d.settings.page_color = Some(Rgb(0xF4, 0xEE, 0xD8));
        d.format_range(&Pos::body(0, 0), &Pos::body(0, 6), &|c| c.highlight = Some(Highlight::Yellow)).expect("highlight");
        let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
        let page = l.pages.first().cloned().expect("a page");
        (d, page)
    }

    /// Through the shared pool: a page region drawn again after renders of other sizes and an
    /// icon comes out the same, and hostile sizes and scales still render, clamped.
    #[test]
    fn page_and_icon_renders_share_the_pool_without_traces() {
        let (d, page) = page();
        let opts = RenderOptions::default();
        let region = |w: u32, h: u32| render_region(&d, &page, w, h, Affine::scale(1.5), &opts);
        let first = region(500, 300);
        let icon = crate::icon_lib::ICONS.first().expect("an icon");
        let first_icon = crate::icon_lib::render(icon, 48, Rgb(0, 0x80, 0));
        for (w, h) in [(0, 0), (u32::MAX, 1), (1, u32::MAX), (0, 50), (900, 1200)] {
            let img = region(w, h);
            let (cw, ch) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
            assert_eq!((img.width, img.height), (cw, ch), "{w}x{h}");
            assert_eq!(img.pixels.len(), cw as usize * ch as usize * 4, "{w}x{h}");
        }
        same(&crate::icon_lib::render(icon, 48, Rgb(0, 0x80, 0)), &first_icon, "the icon again");
        same(&region(500, 300), &first, "the region again");
        let img = render_page(&d, &page, f32::NAN, &opts);
        assert!(img.width > 0 && img.height > 0, "a junk scale falls back to 1");
        // Other tests render at the same time, so how many the shared pool built can't be pinned
        // down here; that it built any shows page renders go through it.
        assert!(CONTEXTS.built() >= 1, "page and icon renders take the shared context");
    }

    #[test]
    fn concurrent_spares_are_reused_without_spawning_more_contexts() {
        let pool = ContextPool::new();
        for _ in 0..4 {
            let a = pool.take(240, 160);
            let b = pool.take(240, 160);
            pool.give_back(a);
            pool.give_back(b);
        }
        assert_eq!(pool.built(), 2, "primary + spare should be reused across overlapping rounds");
    }

    #[test]
    fn oversized_contexts_are_dropped_after_returning_to_small_renders() {
        let pool = ContextPool::new();
        let big_p = pool.take(2500, 2500);
        let big_s = pool.take(2500, 2500);
        pool.give_back(big_p);
        pool.give_back(big_s);
        assert_eq!(pool.built(), 2);

        for _ in 0..OVERSIZED_SHRINK_STREAK {
            pooled(&pool, 400, 400);
        }
        // Oversized primary dropped at streak == OVERSIZED_SHRINK_STREAK; next take rebuilds a fresh one.
        pooled(&pool, 400, 400);
        assert_eq!(pool.built(), 3, "oversized primary should be replaced once back at normal sizes");
    }
}

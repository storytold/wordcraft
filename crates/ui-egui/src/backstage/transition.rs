//! File page transitions live in the window's temporary UI state, never in saved preferences.

const DURATION: f64 = 0.18;

#[derive(Clone)]
struct Entry {
    page: String,
    start: f64,
}

/// Each native window owns its own page-entry timer.
fn id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique(("backstage_page_entry", ctx.viewport_id()))
}

/// Clear on the editor frame so reopening File animates even when its page has not changed.
pub(crate) fn reset(ctx: &egui::Context) {
    let id = id(ctx);
    ctx.data_mut(|d| d.remove::<Entry>(id));
}

/// Fade and lift the incoming page without drawing the outgoing page's controls a second time.
/// Navigation changes immediately, including when another page is picked before this settles.
pub(super) fn progress(ctx: &egui::Context, page: &str, now: f64, animate: bool) -> f32 {
    let id = id(ctx);
    let entry = ctx.data_mut(|d| {
        let mut entry = d.get_temp::<Entry>(id).filter(|e| e.page == page).unwrap_or_else(|| Entry { page: page.into(), start: now });
        if !animate {
            // Switching effects off finishes this transition; switching back on must not replay it.
            entry.start = now - DURATION;
        }
        d.insert_temp(id, entry.clone());
        entry
    });
    let t = ((now - entry.start) / DURATION).clamp(0.0, 1.0) as f32;
    if animate && t < 1.0 {
        ctx.request_repaint();
    }
    if animate { 1.0 - (1.0 - t).powi(3) } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Page changes and reopening start a fresh entry; selecting another page never waits.
    #[test]
    fn page_changes_animate_and_reopening_restarts_the_entry() {
        let ctx = egui::Context::default();
        assert_eq!(progress(&ctx, "home", 0.0, true), 0.0);
        let middle = progress(&ctx, "home", 0.09, true);
        assert!(middle > 0.0 && middle < 1.0);
        assert_eq!(progress(&ctx, "home", 0.2, true), 1.0);
        assert_eq!(progress(&ctx, "open", 0.21, true), 0.0);
        // A second selection takes effect immediately, even before Open has settled.
        assert_eq!(progress(&ctx, "new", 0.22, true), 0.0);
        assert_eq!(progress(&ctx, "new", 0.5, true), 1.0);
        reset(&ctx);
        assert_eq!(progress(&ctx, "new", 0.51, true), 0.0);
    }

    /// Turning effects off settles the page and reenabling them does not replay old motion.
    #[test]
    fn turning_effects_off_finishes_the_current_and_future_transitions() {
        let ctx = egui::Context::default();
        progress(&ctx, "home", 0.0, true);
        assert_eq!(progress(&ctx, "home", 0.04, false), 1.0);
        assert_eq!(progress(&ctx, "home", 0.05, true), 1.0, "reenabling does not replay the interrupted entry");
        assert_eq!(progress(&ctx, "open", 0.06, false), 1.0);
        assert_eq!(progress(&ctx, "open", 0.07, true), 1.0);
    }

    /// A page painted in another viewport cannot restart this window's entry.
    #[test]
    fn windows_keep_their_own_page_transition() {
        let ctx = egui::Context::default();
        for (viewport, page, time, want) in [(1, "home", 0.0, 0.0), (2, "open", 0.1, 0.0), (1, "home", 0.3, 1.0), (2, "open", 0.4, 1.0)] {
            let viewport_id = egui::ViewportId::from_hash_of(viewport);
            let mut input = egui::RawInput { viewport_id, ..Default::default() };
            input.viewports.insert(viewport_id, egui::ViewportInfo::default());
            ctx.run_ui(input, |_| assert_eq!(progress(&ctx, page, time, true), want)).drop_without_applying_deltas();
        }
    }
}

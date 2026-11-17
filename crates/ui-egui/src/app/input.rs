#[cfg(not(target_arch = "wasm32"))]
use super::i18n;
use super::{WordApp, json};

impl WordApp {
    /// Read a Control-click as the right-click on macOS, then offer at most one synthetic gesture
    /// step to the host. eframe carries an unconsumed RawInput into the next pass and into hidden
    /// logic ticks, so offering it again would duplicate input. The step stays queued until a
    /// final (not discarded) UI pass acknowledges it, and a press/release pair stays intact.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Capture the surface from the raw input itself. `InputState::viewport_rect` and the
        // last painted screen rect are stale in a hidden `run_logic` tick; RawInput is the host's
        // current answer, including minimized/occluded and a Wayland `inner_rect: None`.
        if raw.viewport_id == egui::ViewportId::ROOT
            && let Some(geometry) = crate::control::SurfaceGeometry::from_raw(raw, ctx)
        {
            // A hidden resize and change back still invalidate an older capture, even if
            // neither intermediate geometry reaches a drawing or logic pass.
            if self.surface_geometry != Some(geometry) {
                self.capture_epoch = self.capture_epoch.wrapping_add(1);
            }
            self.surface_geometry = Some(geometry);
        }
        if cfg!(target_os = "macos") {
            control_click_as_secondary(&mut raw.events, &mut self.control_click);
        }
        self.apply_raw_input_scroll(raw);
        if self.synthetic_pending || self.synthetic.is_empty() || self.fonts_frames < 2 || !self.control_input_is_roots(ctx) {
            self.record_scaled_scroll_events(raw);
            return;
        }
        let Some((events, n)) = self.peek_synthetic() else {
            self.record_scaled_scroll_events(raw);
            return;
        };
        if events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))) {
            // Control input belongs to this window's remembered widget even while another app
            // is active. Give only this input frame egui focus, without requesting OS activation.
            raw.focused = true;
            if let (Some(ctx), Some((id, _))) = (&self.ctx, self.canvas.widget_id) {
                // Canvas keys run before painting updates last frame's native focus flag.
                self.canvas.focused = ctx.memory(|m| m.has_focus(id));
            }
        }
        raw.events.extend(events);
        self.synthetic_pending = true;
        self.synthetic_injected = n;
        self.record_scaled_scroll_events(raw);
    }

    /// Scales point-wheel events in the incoming [`egui::RawInput`] by the touchpad / wheel speed multiplier.
    /// When eframe or web runners replay an unconsumed [`egui::RawInput`] across hidden logic ticks,
    /// already-scaled events in the pending prefix are skipped so the multiplier is applied exactly once per event (#6146).
    fn apply_raw_input_scroll(&mut self, raw: &mut egui::RawInput) {
        let previously_scaled = self.raw_scroll_events_scaled;
        let is_replayed = previously_scaled > 0
            && raw.events.len() >= previously_scaled
            && raw.events.get(..previously_scaled) == Some(&self.last_scaled_scroll_events[..]);
        let start_idx = if is_replayed { previously_scaled } else { 0 };
        if let Some(new_events) = raw.events.get_mut(start_idx..) {
            self.ui.touchpad_scroll_speed.apply_events_scroll(new_events, self.ui.scroll_speed);
        }
    }

    /// Records the current state of scaled events in the raw input batch so replay across hidden ticks
    /// does not compound multipliers.
    fn record_scaled_scroll_events(&mut self, raw: &egui::RawInput) {
        self.raw_scroll_events_scaled = raw.events.len();
        self.last_scaled_scroll_events = raw.events.clone();
    }

    /// Clears recorded scroll replay tracking when the UI pass consumes the frame's input.
    pub(crate) fn clear_raw_scroll_replay(&mut self) {
        self.raw_scroll_events_scaled = 0;
        self.last_scaled_scroll_events.clear();
    }

    /// Whether this pass's input came from the control channel (`ui.click`, `ui.key`, `ui.drag`,
    /// `ui.text`…, which MCP's `click`, `key` and `type` tools send). Decisions that are only the
    /// user's ignore it: letting agent messages through again, and answering an agent's question
    /// (#959 review). A real click in the same frame as a control step is ignored too, which errs
    /// the safe way.
    pub(crate) fn input_from_control(&self, ctx: &egui::Context) -> bool {
        let viewport = ctx.viewport_id();
        self.synthetic_pending || ctx.with_plugin(|p: &mut crate::windows::ViewportInput| p.injected == Some(viewport)).unwrap_or(false)
    }

    /// The control channel's next input frame, taken off the queue.
    pub(crate) fn next_synthetic(&mut self) -> Option<Vec<egui::Event>> {
        let (events, n) = self.peek_synthetic()?;
        self.synthetic.drain(..n);
        Some(events)
    }

    /// The control channel's next input frame: one pointer event (after a move to it), or keys
    /// and text up to the next key release; with how many queued events it takes.
    fn peek_synthetic(&self) -> Option<(Vec<egui::Event>, usize)> {
        if self.synthetic.is_empty() {
            return None;
        }
        let n = match self.synthetic.first() {
            Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        let mut events = Vec::new();
        if let Some(egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. }) = self.synthetic.first() {
            events.push(egui::Event::PointerMoved(*p));
        }
        let n = n.min(self.synthetic.len());
        events.extend(self.synthetic.iter().take(n).cloned());
        Some((events, n))
    }

    /// Raw input read in the known UI phase, first pass only: dropped files and the SmartArt Text
    /// Pane's Tab/Shift+Tab, before the font warm-up return. `logic` also runs hidden, where its
    /// input events are from the previous shown frame.
    pub(super) fn handle_ui_input_events(&mut self, ctx: &egui::Context) {
        // eframe does not preserve dropped files for a later frame, so a drop in the first two
        // passes would otherwise disappear.
        #[cfg(not(target_arch = "wasm32"))]
        if ctx.current_pass_index() == 0 {
            let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
            // A drop behind a modal dialog does nothing: it would insert a picture or open a
            // document under it.
            if !dropped.is_empty() && !self.modal_open() {
                self.open_dropped_files(dropped);
                let lang = i18n::Lang::from_pref(&self.ui.language);
                self.install_ui_fonts(ctx, lang);
            }
        }
        // SmartArt Text Pane (#2701): Tab demotes and Shift+Tab promote the focused item, as
        // Word's pane does. This is an input action, so it runs in the first UI pass only; hidden
        // `run_logic` ticks still see the previous shown frame's keys.
        if ctx.current_pass_index() == 0
            && self.session.view.smart_art_pane
            && self.smart_art_row_focused
            && self.smart_art_item.is_some()
            && !self.modal_open()
        {
            let shift_tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            let tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            if shift_tab || tab {
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                let _ = self.run(if shift_tab { "smartArt.promote" } else { "smartArt.demote" }, json!({"index": self.smart_art_item.unwrap_or(0)}));
            }
        }
    }

    /// A click on the window's chrome (ribbon tabs and empty ribbon space, galleries, check
    /// boxes, the status bar, panes) hands keyboard focus back to the document, as in Word, so the
    /// caret keeps blinking and typing continues where it was (#1093). egui drops the page's focus
    /// on any click elsewhere; only a click that left nothing focused counts, so a text field
    /// (font size, zoom, command search) keeps the focus it took, and clicks on a dialog, menu or
    /// popup, or while one is open, leave focus alone. Clicks on the page are the canvas's own.
    pub(super) fn refocus_after_chrome_click(&mut self, ctx: &egui::Context) {
        if self.ui.backstage || self.dialog.is_some() || self.colors.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.file_browser.open.is_some() {
            return;
        }
        let Some(pos) = ctx.input(|i| if i.pointer.any_click() { i.pointer.interact_pos() } else { None }) else { return };
        // The ribbon dropped over the page in Focus (#1678) is chrome, though it lies over the page.
        let on_focus_ribbon = self.focus_ribbon && ctx.layer_id_at(pos).is_some_and(|l| l.id == egui::Id::unique("focus_ribbon"));
        // The page handles its own clicks (comment balloons there take the keyboard themselves).
        if !on_focus_ribbon && (self.canvas.balloon.is_some() || self.canvas.viewport.is_some_and(|v| v.contains(pos))) {
            return;
        }
        let on_chrome = on_focus_ribbon || ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background);
        if on_chrome && ctx.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ctx) {
            self.canvas.want_focus = true;
            // The page has already been drawn, so it only sees `want_focus` next frame, and its own
            // `focused` flag (which gates the keys) isn't recomputed until then either. Claim both
            // now, or the click swallows the first keystroke.
            self.canvas.focused = true;
            // Only a page this pass drew. The File page's back arrow closes the File page in the
            // pass that drew it instead of the page, and focus on a widget missing from the pass
            // names a node AccessKit doesn't have: with accessibility on, AccessKit ends the program
            // over that. `want_focus` hands the keyboard over next frame instead.
            if let Some((id, pass)) = self.canvas.widget_id
                && pass == ctx.cumulative_pass_nr()
            {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            // The canvas reads keys before it takes focus back (#1107), so it counts as focused
            // now: a keystroke in the very next frame reaches the page instead of being dropped.
            self.canvas.focused = true;
            ctx.request_repaint();
        }
    }
}

/// macOS reads a Control-click as the right-click (winit reports the primary button with Control
/// held), so menus open on it as on a two-finger click. The release follows its press even if
/// Control was let go in between.
pub(crate) fn control_click_as_secondary(events: &mut [egui::Event], held: &mut bool) {
    for e in events {
        if let egui::Event::PointerButton { button: button @ egui::PointerButton::Primary, pressed, modifiers, .. } = e {
            if *pressed {
                *held = modifiers.ctrl && !modifiers.mac_cmd;
            }
            if *held {
                *button = egui::PointerButton::Secondary;
            }
            if !*pressed {
                *held = false;
            }
        }
    }
}

//! RibbonFade: switching ribbon tabs cross-fades their content instead of swapping it in one
//! frame, and the selected tab's underline slides to the new tab.
//!
//! - **Outgoing content** fades from where it was to 0 opacity in the first part of the fade
//!   ([`FadeConfig::exit_share`]), along [`FadeConfig::exit_easing`].
//! - **Incoming content** fades from 0 to 100% over the whole fade ([`FadeConfig::ms`], 200 ms by
//!   default: Fluent's "normal" duration), along [`FadeConfig::easing`]. It takes clicks and keys
//!   from its first frame, so a fast user never waits for the animation.
//! - **The underline** under the selected tab glides from the old tab to the new one in the same
//!   time, along the incoming curve.
//!
//! The curves are cubic Béziers, as in CSS; [`EASINGS`] names Fluent 2's motion curves. By default
//! the exit is shorter than the entrance and the entrance decelerates, as Fluent's motion guidance
//! asks: the old tab fades out evenly in 80 ms while the new one arrives along `decelerateMin`, so
//! the two cross at about 40% each some 50 ms in. No frame shows an empty ribbon, and the double
//! image is faint and brief. Our own choice of timing, from looking at rendered frames: nothing was
//! measured from Word.
//!
//! **How the outgoing tab is drawn.** egui is immediate mode, and a tab's controls can only be laid
//! out by running its code, which would also make them clickable. So [`show`] keeps the shapes the
//! ribbon painted on the previous pass and, after a switch, paints those again, fading, under the
//! new tab. They are only pixels: no clicks, no keyboard focus and no accessibility nodes, so a
//! screen reader and the keyboard only ever meet the new tab. Nothing moves or resizes: both tabs
//! are painted at their own place in the ribbon, whose height is fixed.
//!
//! **When it doesn't animate:** File › Options › Advanced › Animation (`ui.ribbonAnimation`) off,
//! animation effects off (`ui.animations`, which follows the system's reduce-motion setting under
//! "Use system setting"), egui's `animation_time` at 0 (the headless test harness, so scripted
//! screenshots never catch a tab mid-fade), and `WORDCRAFT_RIBBON_FADE_MS=0`. The variable also
//! sets the duration, for looking at the motion slowed down. The first tab shown, a ribbon that
//! wasn't drawn on the previous pass (collapsed, behind File) and a ribbon that moved or resized
//! since then come in without a fade.
//!
//! **Reuse.** [`Easing`], [`FadeConfig`], [`Crossfade`] and [`Glide`] know nothing about egui or
//! WordCraft: they turn a key and a clock into opacities and positions, for any front end. [`show`]
//! and [`indicator`] are the egui side; `apps/wordcraft/examples/ribbon_fade.rs` is a standalone
//! ribbon (Home, Insert, Layout, View) built on them.

use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

use egui::epaint::ClippedShape;
use egui::{Id, Rect, Ui};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The default fade (milliseconds): Fluent's "normal" duration.
pub const DEFAULT_MS: u32 = 200;
/// The longest fade the setting or `WORDCRAFT_RIBBON_FADE_MS` can ask for (milliseconds).
pub const MAX_MS: u32 = 2000;
/// The default share of the fade the outgoing tab takes to disappear.
pub const DEFAULT_EXIT_SHARE: f32 = 0.4;
/// The most shapes a kept tab may hold; a tab with more comes in without a fade (memory bound).
const MAX_SHAPES: usize = 50_000;

/// A timing curve: a CSS-style cubic Bézier from (0, 0) to (1, 1) through two control points.
/// The control points' x stays within 0–1 so time only moves forwards; y may overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Easing {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl Easing {
    /// Constant speed.
    pub const LINEAR: Easing = Easing::new(0.0, 0.0, 1.0, 1.0);
    /// Fluent `curveEasyEase`: gentle at both ends.
    pub const EASY_EASE: Easing = Easing::new(0.33, 0.0, 0.67, 1.0);
    /// Fluent `curveEasyEaseMax`.
    pub const EASY_EASE_MAX: Easing = Easing::new(0.8, 0.0, 0.2, 1.0);
    /// Fluent `curveDecelerateMax`: fast in, slow out, for things arriving.
    pub const DECELERATE_MAX: Easing = Easing::new(0.1, 0.9, 0.2, 1.0);
    /// Fluent `curveDecelerateMid`.
    pub const DECELERATE_MID: Easing = Easing::new(0.0, 0.0, 0.0, 1.0);
    /// Fluent `curveDecelerateMin`.
    pub const DECELERATE_MIN: Easing = Easing::new(0.33, 0.0, 0.1, 1.0);
    /// Fluent `curveAccelerateMax`: slow in, fast out, for things leaving.
    pub const ACCELERATE_MAX: Easing = Easing::new(0.9, 0.1, 1.0, 0.2);
    /// Fluent `curveAccelerateMid`.
    pub const ACCELERATE_MID: Easing = Easing::new(1.0, 0.0, 1.0, 1.0);
    /// Fluent `curveAccelerateMin`.
    pub const ACCELERATE_MIN: Easing = Easing::new(0.8, 0.0, 0.78, 1.0);

    /// A curve through the control points (x1, y1) and (x2, y2). Out-of-range or non-finite
    /// values are tamed: x clamped to 0–1, y to −1–2, NaN read as the linear curve's.
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        const fn tame(v: f32, lo: f32, hi: f32, nan: f32) -> f32 {
            if v.is_nan() { nan } else { v.clamp(lo, hi) }
        }
        Easing { x1: tame(x1, 0.0, 1.0, 0.0), y1: tame(y1, -1.0, 2.0, 0.0), x2: tame(x2, 0.0, 1.0, 1.0), y2: tame(y2, -1.0, 2.0, 1.0) }
    }

    /// The curve's value at time `t` (0–1; clamped). 0 at the start, 1 at the end.
    pub fn apply(self, t: f32) -> f32 {
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let Easing { x1, y1, x2, y2 } = Easing::new(self.x1, self.y1, self.x2, self.y2);
        if x1 == y1 && x2 == y2 {
            return t;
        }
        // One coordinate of the Bézier at parameter s, and its derivative.
        let at = |a: f32, b: f32, s: f32| {
            let m = 1.0 - s;
            3.0 * m * m * s * a + 3.0 * m * s * s * b + s * s * s
        };
        let slope = |a: f32, b: f32, s: f32| {
            let m = 1.0 - s;
            3.0 * m * m * a + 6.0 * m * s * (b - a) + 3.0 * s * s * (1.0 - b)
        };
        // The parameter whose x is t: a few Newton steps, then bisection if they didn't settle.
        // x(s) never decreases (its control points' x are within 0–1), so bisection always works.
        let mut s = t;
        for _ in 0..8 {
            let err = at(x1, x2, s) - t;
            if err.abs() < 1e-5 {
                return at(y1, y2, s);
            }
            let d = slope(x1, x2, s);
            if d.abs() < 1e-6 {
                break;
            }
            s = (s - err / d).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        s = t;
        for _ in 0..32 {
            let x = at(x1, x2, s);
            if (x - t).abs() < 1e-5 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
        at(y1, y2, s)
    }

    /// The curve's name in [`EASINGS`], if it has one.
    pub fn name(self) -> Option<&'static str> {
        EASINGS.iter().find(|(_, e)| *e == self).map(|(n, _)| *n)
    }

    /// A curve from its name in [`EASINGS`] (any case, `-` and `_` ignored), or from four numbers
    /// `[x1, y1, x2, y2]`. `None` for anything else.
    pub fn parse(v: &serde_json::Value) -> Option<Easing> {
        match v {
            serde_json::Value::String(s) => {
                let key = |n: &str| n.chars().filter(|c| *c != '-' && *c != '_').flat_map(char::to_lowercase).collect::<String>();
                let wanted = key(s);
                EASINGS.iter().find(|(n, _)| key(n) == wanted).map(|(_, e)| *e)
            }
            serde_json::Value::Array(a) if a.len() == 4 => {
                let n: Vec<f32> = a.iter().filter_map(serde_json::Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite()).collect();
                match n.as_slice() {
                    [x1, y1, x2, y2] => Some(Easing::new(*x1, *y1, *x2, *y2)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The curve as a name when it has one, otherwise `[x1, y1, x2, y2]`.
    pub fn to_json(self) -> serde_json::Value {
        match self.name() {
            Some(n) => serde_json::Value::from(n),
            // Four decimals: what a person would type, without f32's noise.
            None => {
                let round = |v: f32| (f64::from(v) * 1e4).round() / 1e4;
                serde_json::json!([round(self.x1), round(self.y1), round(self.x2), round(self.y2)])
            }
        }
    }
}

/// The named curves: Fluent 2's motion curves, and linear.
pub const EASINGS: &[(&str, Easing)] = &[
    ("linear", Easing::LINEAR),
    ("easyEase", Easing::EASY_EASE),
    ("easyEaseMax", Easing::EASY_EASE_MAX),
    ("decelerateMax", Easing::DECELERATE_MAX),
    ("decelerateMid", Easing::DECELERATE_MID),
    ("decelerateMin", Easing::DECELERATE_MIN),
    ("accelerateMax", Easing::ACCELERATE_MAX),
    ("accelerateMid", Easing::ACCELERATE_MID),
    ("accelerateMin", Easing::ACCELERATE_MIN),
];

impl Serialize for Easing {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

/// How tab switches animate. Kept between runs (`ui.json`), so every field is read leniently and
/// tamed by [`FadeConfig::ms`] and friends before use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FadeConfig {
    /// Animate tab switches at all.
    pub enabled: bool,
    /// The fade's length in milliseconds (0–[`MAX_MS`]); 0 switches at once.
    #[serde(rename = "ms", deserialize_with = "lenient_ms")]
    pub duration_ms: u32,
    /// The incoming tab's curve (and the underline's).
    #[serde(deserialize_with = "lenient_easing_in")]
    pub easing: Easing,
    /// The outgoing tab's curve.
    #[serde(deserialize_with = "lenient_easing_out")]
    pub exit_easing: Easing,
    /// The share of the fade the outgoing tab takes to disappear (0.1–1).
    #[serde(deserialize_with = "lenient_share")]
    pub exit_share: f32,
    /// The underline under the selected tab glides to the new tab.
    pub slide_indicator: bool,
}

impl Default for FadeConfig {
    fn default() -> Self {
        FadeConfig {
            enabled: true,
            duration_ms: DEFAULT_MS,
            easing: Easing::DECELERATE_MIN,
            exit_easing: Easing::LINEAR,
            exit_share: DEFAULT_EXIT_SHARE,
            slide_indicator: true,
        }
    }
}

impl FadeConfig {
    /// The fade's length in seconds, 0 when it is off.
    pub fn secs(&self) -> f64 {
        if !self.enabled { 0.0 } else { f64::from(self.duration_ms.min(MAX_MS)) / 1000.0 }
    }

    /// [`FadeConfig::exit_share`], tamed to 0.1–1.
    pub fn exit_share(&self) -> f32 {
        if self.exit_share.is_finite() { self.exit_share.clamp(0.1, 1.0) } else { DEFAULT_EXIT_SHARE }
    }

    /// This configuration with the duration from `WORDCRAFT_RIBBON_FADE_MS`, when set.
    pub fn with_env(mut self) -> Self {
        if let Some(ms) = env_ms() {
            self.duration_ms = ms;
        }
        self
    }
}

/// `WORDCRAFT_RIBBON_FADE_MS`, read once: a duration in milliseconds (clamped to [`MAX_MS`]).
fn env_ms() -> Option<u32> {
    static MS: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *MS.get_or_init(|| std::env::var("WORDCRAFT_RIBBON_FADE_MS").ok().and_then(|v| v.trim().parse::<f64>().ok()).and_then(ms_from))
}

/// A number of milliseconds as the setting holds it: finite, 0–[`MAX_MS`].
fn ms_from(v: f64) -> Option<u32> {
    v.is_finite().then(|| v.clamp(0.0, f64::from(MAX_MS)).round() as u32)
}

fn lenient_ms<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_f64().and_then(ms_from).unwrap_or(DEFAULT_MS))
}

fn lenient_easing_in<'de, D: Deserializer<'de>>(d: D) -> Result<Easing, D::Error> {
    Ok(Easing::parse(&serde_json::Value::deserialize(d)?).unwrap_or(FadeConfig::default().easing))
}

fn lenient_easing_out<'de, D: Deserializer<'de>>(d: D) -> Result<Easing, D::Error> {
    Ok(Easing::parse(&serde_json::Value::deserialize(d)?).unwrap_or(FadeConfig::default().exit_easing))
}

fn lenient_share<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let v = serde_json::Value::deserialize(d)?.as_f64().map(|f| f as f32).filter(|f| f.is_finite());
    Ok(v.map_or(DEFAULT_EXIT_SHARE, |f| f.clamp(0.1, 1.0)))
}

/// How strongly to paint each tab this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opacities {
    /// The tab that was showing: 1 when the switch starts, 0 when it is gone.
    pub outgoing: f32,
    /// The tab now showing: 0 when the switch starts, 1 at rest.
    pub incoming: f32,
    /// A fade is running: draw again next frame.
    pub running: bool,
    /// The key changed this frame and a fade started.
    pub switched: bool,
}

impl Opacities {
    /// At rest: only the current tab, fully drawn.
    pub const REST: Opacities = Opacities { outgoing: 0.0, incoming: 1.0, running: false, switched: false };
}

/// The cross-fade's state between frames: which key is showing and when it took over. Time-based,
/// so it looks the same at any frame rate.
#[derive(Clone, Debug, Default)]
pub struct Crossfade {
    key: Option<u64>,
    started: Option<f64>,
}

impl Crossfade {
    /// The opacities for showing `key` at time `now` (seconds, any monotonic clock). A new key
    /// starts a fade, except the first one ever shown.
    pub fn frame(&mut self, key: u64, now: f64, config: &FadeConfig) -> Opacities {
        let secs = config.secs();
        let switched = self.key.is_some_and(|k| k != key) && secs > 0.0 && now.is_finite();
        self.key = Some(key);
        if switched {
            self.started = Some(now);
        }
        let Some(started) = self.started.filter(|_| secs > 0.0) else {
            self.started = None;
            return Opacities::REST;
        };
        let t = (now - started) / secs;
        // A clock that went back, or a fade that is over: at rest.
        if !t.is_finite() || !(0.0..1.0).contains(&t) {
            self.started = None;
            return Opacities { switched, ..Opacities::REST };
        }
        let t = t as f32;
        let exit = (t / config.exit_share()).min(1.0);
        Opacities {
            outgoing: (1.0 - config.exit_easing.apply(exit)).clamp(0.0, 1.0),
            incoming: config.easing.apply(t).clamp(0.0, 1.0),
            running: true,
            switched,
        }
    }

    /// A fade is running.
    pub fn is_running(&self) -> bool {
        self.started.is_some()
    }

    /// Forget the fade: the next key comes in at once.
    pub fn reset(&mut self) {
        self.started = None;
    }
}

/// A rectangle gliding to its new place when its key changes: the selected tab's underline.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glide {
    key: Option<u64>,
    /// Where it was drawn last, and on which pass.
    drawn: Option<(Rect, u64)>,
    /// Where the glide started, and when.
    from: Option<(Rect, f64)>,
}

impl Glide {
    /// Where to draw the rectangle for `key`, whose place at rest is `target`, at time `now` on
    /// pass `pass` (a counter that goes up by one per drawn pass). It glides only when the key
    /// changed and the old one was drawn on this pass or the one before.
    pub fn frame(&mut self, key: u64, target: Rect, now: f64, pass: u64, config: &FadeConfig) -> (Rect, bool) {
        let secs = if config.slide_indicator { config.secs() } else { 0.0 };
        let recent = self.drawn.filter(|(_, p)| p.saturating_add(1) >= pass).map(|(r, _)| r);
        if self.key.is_some_and(|k| k != key) {
            self.from = recent.filter(|_| secs > 0.0 && now.is_finite()).map(|r| (r, now));
        } else if recent.is_none() {
            self.from = None;
        }
        self.key = Some(key);
        let mut running = false;
        let rect = match self.from {
            Some((from, started)) if secs > 0.0 => {
                let t = (now - started) / secs;
                if t.is_finite() && (0.0..1.0).contains(&t) {
                    running = true;
                    let k = config.easing.apply(t as f32);
                    Rect::from_min_max(from.min.lerp(target.min, k), from.max.lerp(target.max, k))
                } else {
                    self.from = None;
                    target
                }
            }
            _ => {
                self.from = None;
                target
            }
        };
        self.drawn = Some((rect, pass));
        (rect, running)
    }
}

/// What the ribbon painted on one pass.
#[derive(Clone, Debug)]
struct Snapshot {
    shapes: Arc<Vec<ClippedShape>>,
    pass: u64,
    rect: Rect,
}

/// [`show`]'s memory between passes.
#[derive(Clone, Debug, Default)]
struct State {
    fade: Crossfade,
    /// The shapes painted on the latest pass.
    latest: Option<Snapshot>,
    /// The tab being faded out.
    outgoing: Option<Snapshot>,
}

/// The configuration actually in force for `ui`: off when egui's animations are off.
fn in_force(ui: &Ui, config: &FadeConfig) -> FadeConfig {
    let mut config = config.clone().with_env();
    if ui.style().animation_time <= 0.0 {
        config.enabled = false;
    }
    config
}

/// Draw ribbon content keyed by `key` (the tab's name) with `add_contents`, cross-fading from the
/// previous key's content when it changes. `id_salt` tells several faders apart. `ui` should be a
/// region of fixed size (the ribbon's panel): the old content is painted where it was.
pub fn show<R>(ui: &mut Ui, id_salt: impl Hash + Debug, key: impl Hash + Debug, config: &FadeConfig, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let config = in_force(ui, config);
    let ctx = ui.ctx().clone();
    let id = Id::unique(("ribbon_fade", id_salt));
    let key = Id::unique(key).value();
    let now = ui.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let layer = ui.layer_id();
    let area = ui.max_rect();
    let mut state = ctx.data_mut(|d| d.get_temp::<State>(id)).unwrap_or_default();
    let ops = state.fade.frame(key, now, &config);
    if ops.switched {
        // Only what was painted on the pass just before, at the same place, can fade out.
        let same_place = |r: Rect| (r.min - area.min).length() < 0.5 && (r.max - area.max).length() < 0.5;
        state.outgoing = state.latest.take().filter(|s| s.pass.saturating_add(1) == pass && same_place(s.rect));
    }
    if !ops.running {
        state.outgoing = None;
    }
    let start = ctx.graphics(|g| g.get(layer).map_or(0, |l| l.next_idx().0));
    // The outgoing tab, under the incoming one.
    if let Some(out) = &state.outgoing
        && ops.outgoing > 0.0
    {
        let mut painter = ui.painter().clone();
        painter.set_opacity(ops.outgoing);
        let clip = ui.clip_rect();
        for cs in out.shapes.iter() {
            painter.set_clip_rect(cs.clip_rect.intersect(clip));
            painter.add(cs.shape.clone());
        }
    }
    let before = ui.opacity();
    ui.multiply_opacity(ops.incoming);
    let inner = add_contents(ui);
    ui.set_opacity(before);
    // Keep this pass's shapes (the fading tab included) for a switch on the next pass.
    state.latest = if config.secs() > 0.0 {
        ctx.graphics(|g| {
            let list = g.get(layer)?;
            let n = list.all_entries().len().saturating_sub(start);
            (n <= MAX_SHAPES).then(|| list.all_entries().skip(start).cloned().collect::<Vec<_>>())
        })
        .map(|shapes| Snapshot { shapes: Arc::new(shapes), pass, rect: area })
    } else {
        None
    };
    if ops.running {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    inner
}

/// Where to draw the selected tab's underline: `target` at rest, gliding there from the old tab's
/// underline after a switch. `key` is the selected tab.
pub fn indicator(ui: &Ui, id_salt: impl Hash + Debug, key: impl Hash + Debug, target: Rect, config: &FadeConfig) -> Rect {
    let config = in_force(ui, config);
    let ctx = ui.ctx();
    let id = Id::unique(("ribbon_fade_indicator", id_salt));
    let now = ui.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let (rect, running) = ctx.data_mut(|d| d.get_temp_mut_or_default::<Glide>(id).frame(Id::unique(key).value(), target, now, pass, &config));
    if running {
        ctx.request_repaint();
    }
    rect
}

/// A fade is running in the fader `id_salt` (for tests and tools).
pub fn is_fading(ctx: &egui::Context, id_salt: impl Hash + Debug) -> bool {
    ctx.data(|d| d.get_temp::<State>(Id::unique(("ribbon_fade", id_salt)))).is_some_and(|s| s.fade.is_running())
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use serde_json::json;

    use super::*;

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn named_curves_start_at_zero_end_at_one_and_never_go_back() {
        for (name, e) in EASINGS {
            assert_eq!(e.apply(0.0), 0.0, "{name}");
            assert_eq!(e.apply(1.0), 1.0, "{name}");
            let mut last = 0.0;
            for i in 0..=100 {
                let v = e.apply(i as f32 / 100.0);
                assert!(v >= last - 1e-4, "{name} goes back at {i}: {v} < {last}");
                last = v;
            }
        }
    }

    #[test]
    fn curves_have_their_shape() {
        assert!((Easing::LINEAR.apply(0.3) - 0.3).abs() < 1e-6);
        // Decelerating curves are past halfway at half time; accelerating ones aren't.
        assert!(Easing::DECELERATE_MAX.apply(0.5) > 0.9);
        assert!(Easing::DECELERATE_MID.apply(0.5) > 0.8);
        assert!(Easing::ACCELERATE_MID.apply(0.5) < 0.2);
        assert!((Easing::EASY_EASE.apply(0.5) - 0.5).abs() < 0.01, "symmetric");
        // CSS `ease-out` (0, 0, 0.58, 1) at 50%: about 0.69.
        assert!((Easing::new(0.0, 0.0, 0.58, 1.0).apply(0.5) - 0.69).abs() < 0.02);
    }

    #[test]
    fn hostile_curves_and_times_never_panic_or_escape() {
        let junk = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30, -0.0, 0.5];
        for &a in &junk {
            for &b in &junk {
                let e = Easing::new(a, b, b, a);
                assert!(e.x1.is_finite() && e.y1.is_finite() && e.x2.is_finite() && e.y2.is_finite());
                for &t in &junk {
                    let v = e.apply(t);
                    assert!(v.is_finite() && (-1.5..=2.5).contains(&v), "{e:?} at {t}: {v}");
                }
                // Hand-built curves skip `new`'s taming; `apply` tames them itself.
                let raw = Easing { x1: a, y1: b, x2: b, y2: a };
                assert!(raw.apply(0.5).is_finite(), "{raw:?}");
            }
        }
    }

    #[test]
    fn curves_parse_from_names_and_numbers() {
        assert_eq!(Easing::parse(&json!("decelerateMax")), Some(Easing::DECELERATE_MAX));
        assert_eq!(Easing::parse(&json!("decelerate-max")), Some(Easing::DECELERATE_MAX), "any spelling");
        assert_eq!(Easing::parse(&json!("EASY_EASE")), Some(Easing::EASY_EASE));
        assert_eq!(Easing::parse(&json!([0.25, 0.1, 0.25, 1.0])), Some(Easing::new(0.25, 0.1, 0.25, 1.0)));
        assert_eq!(Easing::parse(&json!([2, 0, -3, 1])), Some(Easing::new(1.0, 0.0, 0.0, 1.0)), "x clamped");
        for junk in [json!("bouncy"), json!([1, 2, 3]), json!([0, "a", 1, 1]), json!(null), json!(3), json!({})] {
            assert_eq!(Easing::parse(&junk), None, "{junk}");
        }
        assert_eq!(Easing::EASY_EASE.to_json(), json!("easyEase"));
        assert_eq!(Easing::parse(&Easing::new(0.2, 0.0, 0.3, 1.0).to_json()), Some(Easing::new(0.2, 0.0, 0.3, 1.0)));
    }

    #[test]
    fn settings_read_back_leniently() {
        let c = FadeConfig { duration_ms: 150, easing: Easing::LINEAR, exit_easing: Easing::new(0.1, 0.2, 0.3, 0.4), ..Default::default() };
        let back: FadeConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let junk: FadeConfig = serde_json::from_value(json!({"ms": -5, "easing": "wobble", "exitEasing": [1], "exitShare": "x"})).unwrap();
        assert_eq!(junk.duration_ms, 0, "a negative duration is no fade");
        assert_eq!(junk.easing, FadeConfig::default().easing);
        assert_eq!(junk.exit_easing, FadeConfig::default().exit_easing);
        assert_eq!(junk.exit_share, DEFAULT_EXIT_SHARE);
        let huge: FadeConfig = serde_json::from_value(json!({"ms": 1e300, "exitShare": 50})).unwrap();
        assert_eq!((huge.duration_ms, huge.exit_share), (MAX_MS, 1.0));
        assert_eq!(serde_json::from_value::<FadeConfig>(json!({})).unwrap(), FadeConfig::default());
    }

    /// Runs a fade frame by frame at 60 fps from a switch at 0 s.
    fn run(config: &FadeConfig) -> Vec<Opacities> {
        let mut f = Crossfade::default();
        assert_eq!(f.frame(1, -DT, config), Opacities::REST, "the first tab comes in at once");
        (0..40).map(|i| f.frame(2, i as f64 * DT, config)).collect()
    }

    #[test]
    fn a_switch_cross_fades_in_the_configured_time() {
        let config = FadeConfig::default();
        let frames = run(&config);
        assert!(frames[0].switched && frames[0].running);
        assert_eq!((frames[0].outgoing, frames[0].incoming), (1.0, 0.0), "starts on the old tab");
        // 200 ms at 60 fps: 12 frames, then at rest.
        let end = frames.iter().position(|o| !o.running).unwrap();
        assert_eq!(end, 12);
        assert!(frames[end..].iter().all(|o| *o == Opacities::REST));
        // The old tab only leaves, the new one only arrives; the old one is gone in 80 ms.
        for w in frames.windows(2) {
            assert!(w[1].outgoing <= w[0].outgoing && w[1].incoming >= w[0].incoming, "{w:?}");
        }
        assert_eq!(frames[5].outgoing, 0.0);
        // They cross about 50 ms in, neither strong: a faint double image, never an empty ribbon.
        let cross = frames[3];
        assert!((0.25..0.5).contains(&cross.outgoing) && (0.25..0.5).contains(&cross.incoming), "{cross:?}");
        assert!(frames.iter().all(|o| o.outgoing + o.incoming > 0.7), "no deep dip: {frames:?}");
    }

    #[test]
    fn duration_and_curves_are_configurable() {
        let slow = FadeConfig { duration_ms: 500, easing: Easing::LINEAR, exit_easing: Easing::LINEAR, exit_share: 1.0, ..Default::default() };
        let frames = run(&slow);
        assert_eq!(frames.iter().position(|o| !o.running), Some(30));
        let mid = frames[15];
        assert!((mid.incoming - 0.5).abs() < 0.01 && (mid.outgoing - 0.5).abs() < 0.01, "linear: halfway at half time {mid:?}");
        for off in [FadeConfig { enabled: false, ..Default::default() }, FadeConfig { duration_ms: 0, ..Default::default() }] {
            assert!(run(&off).iter().all(|o| *o == Opacities::REST), "{off:?}");
        }
    }

    #[test]
    fn the_same_tab_or_a_clock_going_back_doesnt_fade() {
        let config = FadeConfig::default();
        let mut f = Crossfade::default();
        for i in 0..5 {
            assert_eq!(f.frame(7, i as f64 * DT, &config), Opacities::REST);
        }
        assert!(f.frame(8, 1.0, &config).running);
        assert!(!f.frame(8, 0.5, &config).running, "a clock that went back ends the fade");
        assert!(f.frame(9, f64::NAN, &config) == Opacities::REST, "a broken clock doesn't fade");
        assert!(f.frame(10, 2.0, &config).running);
        f.reset();
        assert!(!f.is_running() && !f.frame(10, 2.01, &config).running);
    }

    #[test]
    fn another_switch_mid_fade_starts_over_from_the_new_tab() {
        let config = FadeConfig::default();
        let mut f = Crossfade::default();
        f.frame(1, 0.0, &config);
        f.frame(2, 1.0, &config);
        let o = f.frame(3, 1.05, &config);
        assert!(o.switched && o.outgoing == 1.0 && o.incoming == 0.0);
        assert!(f.frame(3, 1.24, &config).running && !f.frame(3, 1.26, &config).running, "a full fade from the second switch");
    }

    #[test]
    fn the_underline_glides_to_the_new_tab() {
        let config = FadeConfig::default();
        let a = Rect::from_min_size(pos2(10.0, 27.0), vec2(40.0, 3.0));
        let b = Rect::from_min_size(pos2(110.0, 27.0), vec2(60.0, 3.0));
        let mut g = Glide::default();
        assert_eq!(g.frame(1, a, 0.0, 1, &config), (a, false), "the first tab: in place");
        assert_eq!(g.frame(1, a, DT, 2, &config), (a, false));
        let (r, running) = g.frame(2, b, 2.0 * DT, 3, &config);
        assert!(running && r == a, "starts at the old tab");
        let (r, running) = g.frame(2, b, 2.0 * DT + 0.1, 4, &config);
        assert!(running && r.min.x > a.min.x && r.min.x < b.min.x && r.width() > 40.0 && r.width() < 60.0, "{r:?}");
        assert_eq!(g.frame(2, b, 2.0 * DT + 0.25, 5, &config), (b, false), "and settles");
        // Not drawn for a while (the tab was in the » menu): no glide.
        assert_eq!(g.frame(1, a, 5.0, 20, &config), (a, false));
        // Turned off: in place at once.
        let off = FadeConfig { slide_indicator: false, ..Default::default() };
        assert_eq!(g.frame(2, b, 5.1, 21, &off), (b, false));
    }

    /// The egui side: a ribbon whose two tabs paint a different number of rectangles.
    #[test]
    fn show_paints_the_old_tab_under_the_new_one_while_it_fades() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let tab = Rc::new(RefCell::new(1u8));
        let seen = Rc::new(RefCell::new(Vec::<(usize, f32, bool)>::new()));
        let (t, s) = (tab.clone(), seen.clone());
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 120.0)).with_step_dt(DT as f32).build_ui(move |ui| {
            ui.style_mut().animation_time = 0.1;
            let key = *t.borrow();
            let config = FadeConfig::default();
            let layer = ui.layer_id();
            let start = ui.ctx().graphics(|g| g.get(layer).map_or(0, |l| l.next_idx().0));
            let opacity = show(ui, "test", key, &config, |ui| {
                for i in 0..usize::from(key) {
                    ui.painter().rect_filled(Rect::from_min_size(pos2(10.0 + 30.0 * i as f32, 10.0), vec2(20.0, 20.0)), 0.0, egui::Color32::RED);
                }
                ui.opacity()
            });
            let shapes = ui.ctx().graphics(|g| g.get(layer).map_or(0, |l| l.all_entries().len().saturating_sub(start)));
            s.borrow_mut().push((shapes, opacity, is_fading(ui.ctx(), "test")));
        });
        h.run_steps(3);
        assert_eq!(seen.borrow().last().copied(), Some((1, 1.0, false)), "at rest: one tab");
        *tab.borrow_mut() = 3;
        h.step();
        let (shapes, opacity, fading) = seen.borrow().last().copied().unwrap();
        assert!(fading, "the switch starts a fade");
        assert_eq!(shapes, 1 + 3, "the old tab's rectangle under the new tab's three");
        assert!(opacity < 0.5, "the new tab starts faint: {opacity}");
        // kittest's clock moves 1/60 s a step: 200 ms later it's over.
        h.run_steps(14);
        assert_eq!(seen.borrow().last().copied(), Some((3, 1.0, false)), "at rest again: only the new tab");
    }

    #[test]
    fn show_switches_at_once_without_animations() {
        let tab = std::rc::Rc::new(std::cell::Cell::new(1u8));
        let t = tab.clone();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(200.0, 80.0)).build_ui(move |ui| {
            ui.style_mut().animation_time = 0.0;
            show(ui, "off", t.get(), &FadeConfig::default(), |ui| ui.label("x"));
        });
        h.run_steps(2);
        tab.set(2);
        h.step();
        assert!(!is_fading(&h.ctx, "off"));
    }
}

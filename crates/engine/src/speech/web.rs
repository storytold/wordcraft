//! Read Aloud in the browser, through the Web Speech API (`speechSynthesis`).
//!
//! The browser speaks one sentence at a time, as the desktop speech programs do: each sentence is
//! an utterance in its language, at the reading speed, in the chosen voice when it suits the
//! sentence. Its end event moves on to the next sentence; every change to the player (pause, skip,
//! stop, a new speed) cancels the utterance and speaks the current sentence again if still playing.
//! The generation and the sentence index an utterance was started for keep a late event of a
//! cancelled one from moving the player. An utterance cancelled from outside the player (another
//! script on the page) is spoken again, one the browser can't speak in its language or voice is
//! spoken in the default voice, as on the desktop ([`super::web_retry`]), and one the browser never
//! reports the end of stops reading after a while ([`super::web_sentence_limit_ms`]).

use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex, Weak};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{SpeechSynthesis, SpeechSynthesisUtterance, SpeechSynthesisVoice};

use super::{
    Shared, State, Voice, VoiceKind, WebAttempt, WebRetry, ask_browser_voices_again, lock, lose_speech, may_cancel, plain_tag, voice_for, web_retry,
    web_sentence_limit_ms,
};

type Handler = Closure<dyn FnMut(web_sys::Event)>;
type Timer = Closure<dyn FnMut()>;
type Player = (Mutex<Shared>, Condvar);

/// The utterance being spoken, its event handler, the player it speaks for, and the timer that
/// gives up on it.
struct Current {
    utterance: SpeechSynthesisUtterance,
    handler: Handler,
    player: Weak<Player>,
    watchdog: Option<(i32, Timer)>,
}

/// The browser's voices, as listed and as the browser's own objects (same order), and when they
/// were asked for.
struct Voices {
    listed: Arc<[Voice]>,
    browser: Vec<SpeechSynthesisVoice>,
    asked: f64,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    /// A sentence waiting to be spoken once the browser has handled a cancel: its timer, and the
    /// player it is for.
    static DEFERRED: RefCell<Option<(i32, Timer, Weak<Player>)>> = const { RefCell::new(None) };
    /// Closures replaced while they may still be running (a handler that ends a sentence starts
    /// the next one): kept until later replacements push them out.
    static RETIRED_HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
    static RETIRED_TIMERS: RefCell<Vec<Timer>> = const { RefCell::new(Vec::new()) };
    static VOICES: RefCell<Option<Voices>> = const { RefCell::new(None) };
    /// When the voices were first asked for (ms), and the `voiceschanged` listener.
    static FIRST_ASKED: RefCell<Option<f64>> = const { RefCell::new(None) };
    static VOICES_CHANGED: RefCell<Option<Closure<dyn FnMut()>>> = const { RefCell::new(None) };
}

/// How many replaced closures are kept alive.
const RETIRED_KEPT: usize = 4;

/// What Read Aloud says when the browser has no speech synthesis.
const UNAVAILABLE: &str = "Read Aloud isn't available in this browser";

/// The browser's speech synthesis. `None` when the browser has none: some (Tor Browser,
/// LibreWolf, Firefox with speech turned off) leave `window.speechSynthesis` undefined, and
/// calling into it would throw.
fn synth() -> Option<SpeechSynthesis> {
    web_sys::window()?.speech_synthesis().ok().filter(|s| !s.is_undefined() && !s.is_null())
}

fn retire_handler(h: Handler) {
    RETIRED_HANDLERS.with(|r| {
        let mut r = r.borrow_mut();
        r.push(h);
        let extra = r.len().saturating_sub(RETIRED_KEPT);
        r.drain(..extra);
    });
}

fn retire_timer(id: i32, t: Timer) {
    if let Some(w) = web_sys::window() {
        w.clear_timeout_with_handle(id);
    }
    RETIRED_TIMERS.with(|r| {
        let mut r = r.borrow_mut();
        r.push(t);
        let extra = r.len().saturating_sub(RETIRED_KEPT);
        r.drain(..extra);
    });
}

/// Start `f` after `ms` milliseconds; the timer's id and closure, which must live until then.
fn after(ms: f64, f: impl FnMut() + 'static) -> Option<(i32, Timer)> {
    let t: Timer = Closure::new(f);
    let ms = if ms.is_finite() { ms.clamp(0.0, f64::from(i32::MAX)) as i32 } else { i32::MAX };
    let id = web_sys::window()?.set_timeout_with_callback_and_timeout_and_arguments_0(t.as_ref().unchecked_ref(), ms).ok()?;
    Some((id, t))
}

/// Stop the sentence being spoken (and one waiting to be spoken) and detach its handlers. Returns
/// whether the browser was told to cancel: only when it had something to cancel, since Chrome and
/// Safari can drop an utterance spoken right after a cancel.
fn cancel() -> bool {
    if let Some((id, t, _)) = DEFERRED.with(|d| d.borrow_mut().take()) {
        retire_timer(id, t);
    }
    if let Some(c) = CURRENT.with(|c| c.borrow_mut().take()) {
        c.utterance.set_onend(None);
        c.utterance.set_onerror(None);
        retire_handler(c.handler);
        if let Some((id, t)) = c.watchdog {
            retire_timer(id, t);
        }
    }
    match synth() {
        Some(s) if s.speaking() || s.pending() => {
            s.cancel();
            true
        }
        _ => false,
    }
}

/// The owner of the browser's speech: the player whose sentence is being spoken, or whose next
/// sentence waits to be spoken after a cancel.
fn owner() -> Option<Weak<Player>> {
    let current = CURRENT.with(|c| c.borrow().as_ref().map(|c| c.player.clone()));
    let deferred = DEFERRED.with(|d| d.borrow().as_ref().map(|d| d.2.clone()));
    current.or(deferred)
}

/// [`cancel`], when the sentence being spoken is `player`'s: another player going away (the
/// session a template preview builds and drops) leaves the reading alone.
pub(super) fn cancel_for(player: &Arc<Player>) {
    if may_cancel(owner().as_ref(), player, false) {
        cancel();
    }
}

/// The browser's voices as Read Aloud lists them (the id is the voice URI), and how long ago they
/// were first asked for (ms). The list is kept until the browser says it changed
/// (`voiceschanged`); an empty one is asked for again after a second.
pub(super) fn voices() -> (Arc<[Voice]>, f64) {
    let now = js_sys::Date::now();
    let first = FIRST_ASKED.with(|f| *f.borrow_mut().get_or_insert(now));
    let fresh =
        VOICES.with(|v| v.borrow().as_ref().filter(|v| !ask_browser_voices_again(v.listed.is_empty(), now - v.asked)).map(|v| Arc::clone(&v.listed)));
    if let Some(listed) = fresh {
        return (listed, now - first);
    }
    let Some(synth) = synth() else { return (Arc::new([]), now - first) };
    listen_for_voices(&synth);
    let browser: Vec<SpeechSynthesisVoice> = synth.get_voices().iter().filter_map(|v| v.dyn_into::<SpeechSynthesisVoice>().ok()).collect();
    let listed: Arc<[Voice]> =
        browser.iter().map(|v| Voice { id: v.voice_uri(), name: v.name(), lang: v.lang(), kind: VoiceKind::Web }).collect::<Vec<_>>().into();
    VOICES.with(|v| *v.borrow_mut() = Some(Voices { listed: Arc::clone(&listed), browser, asked: now }));
    (listed, now - first)
}

/// Forget the voice list when the browser's changes (online voices arrive late).
fn listen_for_voices(synth: &SpeechSynthesis) {
    VOICES_CHANGED.with(|l| {
        let mut l = l.borrow_mut();
        if l.is_none() {
            let f: Closure<dyn FnMut()> = Closure::new(|| VOICES.with(|v| *v.borrow_mut() = None));
            synth.set_onvoiceschanged(Some(f.as_ref().unchecked_ref()));
            *l = Some(f);
        }
    });
}

/// The voice `id` from the list, with the browser's own object for it.
fn voice(id: &str) -> Option<(Voice, SpeechSynthesisVoice)> {
    let _ = voices();
    VOICES.with(|v| {
        let v = v.borrow();
        let v = v.as_ref()?;
        let at = v.listed.iter().position(|l| l.id == id)?;
        Some((v.listed.get(at)?.clone(), v.browser.get(at)?.clone()))
    })
}

/// Stop with `error`.
fn fail(shared: &Arc<Player>, error: String) {
    let mut s = lock(&shared.0);
    s.state = State::Stopped;
    s.error = Some(error);
}

/// After a change: cancel what is being spoken, and speak the current sentence if playing.
pub(super) fn sync(shared: &Arc<Player>) {
    speak(shared, WebAttempt::default());
}

/// Cancel what is being spoken and speak the current sentence as `attempt` says, if `shared` is
/// playing; a player that isn't only stops its own sentence. After a cancel the sentence is
/// spoken from a timer, once the browser has handled the cancel.
fn speak(shared: &Arc<Player>, attempt: WebAttempt) {
    let playing = lock(&shared.0).state == State::Playing;
    let owner = owner();
    if !may_cancel(owner.as_ref(), shared, playing) {
        return;
    }
    lose_speech(owner, shared);
    if !cancel() {
        return speak_now(shared, attempt);
    }
    let player = Arc::downgrade(shared);
    let later = after(0.0, move || {
        if let Some(p) = player.upgrade() {
            if let Some((id, t, _)) = DEFERRED.with(|d| d.borrow_mut().take()) {
                retire_timer(id, t);
            }
            speak_now(&p, attempt);
        }
    });
    match later {
        Some((id, t)) => DEFERRED.with(|d| *d.borrow_mut() = Some((id, t, Arc::downgrade(shared)))),
        None => speak_now(shared, attempt),
    }
}

/// Speak the current sentence as `attempt` says, if playing.
fn speak_now(shared: &Arc<Player>, attempt: WebAttempt) {
    let (text, lang, voice_id, rate, generation, index) = {
        let s = lock(&shared.0);
        if s.state != State::Playing {
            return;
        }
        let Some(u) = s.queue.get(s.index) else { return };
        (u.text.clone(), u.lang.clone(), s.voice.clone(), s.rate, s.generation, s.index)
    };
    let Some(synth) = synth() else { return fail(shared, UNAVAILABLE.into()) };
    let Ok(u) = SpeechSynthesisUtterance::new_with_text(&text) else { return fail(shared, UNAVAILABLE.into()) };
    // Whether the utterance was given a language or a voice the browser might not have.
    let mut tailored = false;
    if !attempt.plain {
        if let Some(tag) = lang.as_deref().and_then(plain_tag) {
            u.set_lang(tag);
            tailored = true;
        }
        // The chosen voice when it suits the sentence; else the browser picks one for the language.
        if let Some((v, bv)) = voice_id.as_deref().and_then(voice)
            && voice_for(Some(&v), lang.as_deref(), &text).is_some()
        {
            u.set_voice(Some(&bv));
            tailored = true;
        }
    }
    u.set_rate(rate);
    let player = Arc::downgrade(shared);
    let handler: Handler = Closure::new(move |e: web_sys::Event| {
        if let Some(p) = player.upgrade() {
            ended(&p, generation, index, attempt, tailored, &e);
        }
    });
    u.set_onend(Some(handler.as_ref().unchecked_ref()));
    u.set_onerror(Some(handler.as_ref().unchecked_ref()));
    // A browser that never reports the end stops reading after a while, instead of hanging.
    let player = Arc::downgrade(shared);
    let watchdog = after(web_sentence_limit_ms(text.chars().count(), rate), move || {
        if let Some(p) = player.upgrade() {
            gave_up(&p, generation, index);
        }
    });
    // Recorded before `speak`: a browser may report an error at once, from inside `speak`, and
    // the next sentence must find this one to replace.
    CURRENT.with(|c| *c.borrow_mut() = Some(Current { utterance: u.clone(), handler, player: Arc::downgrade(shared), watchdog }));
    synth.speak(&u);
}

/// The browser never reported the end of sentence `index` of `generation`.
fn gave_up(shared: &Arc<Player>, generation: u64, index: usize) {
    {
        let mut s = lock(&shared.0);
        if s.generation != generation || s.index != index || s.state != State::Playing {
            return;
        }
        s.state = State::Stopped;
        s.error = Some("The browser stopped reading aloud without finishing the sentence".into());
    }
    cancel();
}

/// The utterance for sentence `index` of `generation`, spoken as `attempt` says, ended (`end`)
/// or failed (`error`).
fn ended(shared: &Arc<Player>, generation: u64, index: usize, attempt: WebAttempt, tailored: bool, e: &web_sys::Event) {
    let next = {
        let mut s = lock(&shared.0);
        if s.generation != generation || s.index != index || s.state != State::Playing {
            return;
        }
        if e.type_() == "error" {
            let code = js_sys::Reflect::get(e, &"error".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
            match web_retry(&code, attempt, tailored) {
                WebRetry::Again(again) => again,
                WebRetry::Pause => {
                    s.state = State::Paused;
                    return;
                }
                WebRetry::Stop => {
                    s.state = State::Stopped;
                    s.error = Some(format!("The browser couldn't read aloud ({code})"));
                    return;
                }
            }
        } else {
            s.index += 1;
            if s.index >= s.queue.len() {
                s.state = State::Stopped;
                return;
            }
            WebAttempt::default()
        }
    };
    speak(shared, next);
}

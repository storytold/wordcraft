//! Protocol 2: the push channel. Every running plugin's standard output is read all the time;
//! what a plugin sends on its own (notifications, and requests to the *anytime* methods) lands
//! here, for the app to take in its main loop ([`take`]) and answer ([`crate::Plugin::respond`]).
//! Answers to the app's own requests still go to the request that waits for them.
// The browser build runs no plugins: nothing pushes there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use serde_json::Value as Json;

/// Longest line (64 MiB, protocol 2.0.1: a 16 MiB update in base64 must fit); a longer
/// notification is dropped and counts as one failure.
pub const MAX_PUSH_LINE: usize = 64 << 20;
/// More messages than this per second, for [`FLOOD_SECONDS`] seconds in a row, and the plugin
/// is stopped and marked failed.
pub const FLOOD_RATE: u32 = 1_000;
pub const FLOOD_SECONDS: u32 = 5;
/// Messages kept for the app; older ones go first when a host never takes them.
const MAX_QUEUED: usize = 20_000;

/// What a plugin pushed.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A JSON-RPC notification (no `id`) or request (with the plugin's own `id`).
    Message(Json),
    /// The process exited (or its output broke): `why`.
    Exited(String),
    /// A message longer than [`MAX_PUSH_LINE`] was dropped.
    Overlong,
    /// The plugin flooded the app and was cut off.
    Flood,
}

/// One pushed message and the plugin it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Pushed {
    pub plugin: String,
    pub incoming: Incoming,
}

fn queue() -> &'static Mutex<VecDeque<Pushed>> {
    static Q: OnceLock<Mutex<VecDeque<Pushed>>> = OnceLock::new();
    Q.get_or_init(Default::default)
}

pub(crate) fn push(plugin: &str, incoming: Incoming) {
    let mut q = crate::lock(queue());
    if q.len() >= MAX_QUEUED {
        q.pop_front();
        log::warn!("plugin {plugin}: the app isn't taking pushed messages; the oldest was dropped");
    }
    q.push_back(Pushed { plugin: plugin.to_string(), incoming });
}

/// Everything pushed since the last call, oldest first.
pub fn take() -> Vec<Pushed> {
    crate::lock(queue()).drain(..).collect()
}

/// Takes only `plugin`'s messages (tests, and hosts that serve one plugin at a time).
pub fn take_for(plugin: &str) -> Vec<Pushed> {
    let mut q = crate::lock(queue());
    let (mine, rest): (VecDeque<Pushed>, VecDeque<Pushed>) = q.drain(..).partition(|p| p.plugin == plugin);
    *q = rest;
    mine.into_iter().collect()
}

/// Whether anything is waiting.
pub fn pending() -> bool {
    !crate::lock(queue()).is_empty()
}

/// Counts messages per second and says when a plugin floods.
#[derive(Debug, Default)]
pub(crate) struct FloodMeter {
    second: u64,
    count: u32,
    over: u32,
}

impl FloodMeter {
    /// Records a message at `now_secs`; true once the plugin has flooded for long enough.
    pub(crate) fn hit(&mut self, now_secs: u64) -> bool {
        if now_secs != self.second {
            if now_secs == self.second + 1 && self.count > FLOOD_RATE {
                self.over += 1;
            } else {
                self.over = 0;
            }
            self.second = now_secs;
            self.count = 0;
        }
        self.count += 1;
        // The current second counts once it is over the rate.
        self.over + u32::from(self.count > FLOOD_RATE) >= FLOOD_SECONDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flooding_is_measured_over_five_seconds() {
        let mut m = FloodMeter::default();
        for s in 0..4 {
            for _ in 0..=FLOOD_RATE {
                assert!(!m.hit(s));
            }
        }
        for _ in 0..FLOOD_RATE {
            assert!(!m.hit(4));
        }
        assert!(m.hit(4));
        // A quiet second resets it.
        let mut m = FloodMeter::default();
        for s in [0, 1, 2, 3, 5, 6] {
            for _ in 0..=FLOOD_RATE {
                assert!(!m.hit(s));
            }
        }
    }
}

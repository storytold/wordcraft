//! One message, one line; the prefix comes from the role (never from `from`).

use std::collections::HashSet;

use wordcraft_chat::rules::{addresses_everyone, one_line};
use wordcraft_chat::{Message, Role};

/// Marks lines older than this `listen`: context only, never an order.
pub const HISTORY: &str = "(history) ";
const HISTORY_LINES: usize = 10;

pub fn format_line(m: &Message, me: &str, marker: bool) -> String {
    let text = one_line(&m.text);
    match m.role {
        Role::Owner => {
            let mine = marker && (m.mentions.iter().any(|x| x == me) || addresses_everyone(&m.mentions));
            format!("OWNER #{}{}: {text}", m.seq, if mine { " [@you]" } else { "" })
        }
        Role::Agent => format!("AGENT {} #{}: {text}", one_line(&m.from), m.seq),
        Role::System => format!("SYSTEM #{}: {text}", m.seq),
    }
}

fn key(m: &Message) -> (u64, String, String) {
    (m.ts_ms, m.from.clone(), m.text.clone())
}

fn mine(m: &Message, me: &str) -> bool {
    m.role == Role::Agent && m.from == me
}

/// Where a `listen` is: the last seq seen, what counts as history, what was printed.
pub struct Cursor {
    me: String,
    after: u64,
    start_ms: u64,
    start_seq: u64,
    seen: HashSet<(u64, String, String)>,
}

impl Cursor {
    /// From the history (`chat.poll {after: 0, wait_s: 0}`): the last 10 messages not by `me`,
    /// marked as history and never `[@you]`. `now_ms` is this client's clock.
    pub fn start(me: &str, history: &[Message], now_ms: u64) -> (Cursor, Vec<String>) {
        let mut old: Vec<&Message> = history.iter().collect();
        old.sort_by_key(|m| m.seq);
        let after = old.iter().map(|m| m.seq).max().unwrap_or(0);
        let theirs: Vec<&&Message> = old.iter().filter(|m| !mine(m, me)).collect();
        let lines =
            theirs.iter().skip(theirs.len().saturating_sub(HISTORY_LINES)).map(|m| format!("{HISTORY}{}", format_line(m, me, false))).collect();
        let seen = old.iter().map(|m| key(m)).collect();
        (Cursor { me: me.to_string(), after, start_ms: now_ms, start_seq: after, seen }, lines)
    }

    pub fn after(&self) -> u64 {
        self.after
    }

    /// New messages: one line each; duplicates and `me`'s own skipped; older ones (a document
    /// opened later replays its log) are history.
    pub fn take(&mut self, msgs: &[Message]) -> Vec<String> {
        let mut out = Vec::new();
        for m in msgs {
            self.after = self.after.max(m.seq);
            if !self.seen.insert(key(m)) || mine(m, &self.me) {
                continue;
            }
            if m.ts_ms < self.start_ms || m.seq <= self.start_seq {
                out.push(format!("{HISTORY}{}", format_line(m, &self.me, false)));
            } else {
                out.push(format_line(m, &self.me, true));
            }
        }
        out
    }
}

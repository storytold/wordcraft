//! When an IndexedDB write counts as stored (#1362): not when its request succeeds, but when
//! the transaction fires `complete`. `abort` and `error` mean the browser rolled it back. Also
//! the queue that applies writes one at a time, in call order.

use std::collections::VecDeque;

/// Something the browser reported about one write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    RequestSuccess,
    RequestError,
    TxComplete,
    TxAbort,
    TxError,
}

/// Where a write stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pending,
    Stored,
    Failed(String),
}

/// Follows the events of one write.
#[derive(Debug, Default)]
pub struct WriteGate {
    request_failed: bool,
    verdict: Option<Verdict>,
}

impl WriteGate {
    /// Feed an event; the verdict so far. The first final verdict sticks.
    pub fn on(&mut self, event: Event) -> Verdict {
        if let Some(v) = &self.verdict {
            return v.clone();
        }
        let failed = |why: &str| Verdict::Failed(why.to_string());
        let v = match event {
            Event::RequestSuccess => Verdict::Pending,
            Event::RequestError => {
                self.request_failed = true;
                failed("IndexedDB request failed")
            }
            Event::TxComplete if self.request_failed => failed("IndexedDB request failed"),
            Event::TxComplete => Verdict::Stored,
            Event::TxAbort => failed("IndexedDB transaction aborted (storage full or blocked?)"),
            Event::TxError => failed("IndexedDB transaction failed"),
        };
        if v != Verdict::Pending {
            self.verdict = Some(v.clone());
        }
        v
    }
}

/// Writes waiting for their turn; one runs at a time, oldest first.
#[derive(Debug)]
pub struct WriteQueue<T> {
    waiting: VecDeque<T>,
    running: bool,
}

impl<T> Default for WriteQueue<T> {
    fn default() -> Self {
        Self { waiting: VecDeque::new(), running: false }
    }
}

impl<T> WriteQueue<T> {
    /// Add a write.
    pub fn push(&mut self, op: T) {
        self.waiting.push_back(op);
    }

    /// The next write to run, if none is running; marks it running.
    pub fn start_next(&mut self) -> Option<T> {
        if self.running {
            return None;
        }
        let op = self.waiting.pop_front()?;
        self.running = true;
        Some(op)
    }

    /// The running write is over.
    pub fn finish(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[Event]) -> Verdict {
        let mut g = WriteGate::default();
        let mut v = Verdict::Pending;
        for e in events {
            v = g.on(*e);
        }
        v
    }

    #[test]
    fn request_success_alone_is_not_stored() {
        assert_eq!(run(&[Event::RequestSuccess]), Verdict::Pending);
    }

    #[test]
    fn request_success_then_complete_is_stored() {
        assert_eq!(run(&[Event::RequestSuccess, Event::TxComplete]), Verdict::Stored);
    }

    #[test]
    fn request_success_then_abort_fails() {
        assert!(matches!(run(&[Event::RequestSuccess, Event::TxAbort]), Verdict::Failed(_)));
    }

    #[test]
    fn tx_error_fails() {
        assert!(matches!(run(&[Event::RequestSuccess, Event::TxError]), Verdict::Failed(_)));
    }

    #[test]
    fn request_error_fails_at_once() {
        assert!(matches!(run(&[Event::RequestError]), Verdict::Failed(_)));
    }

    #[test]
    fn request_error_then_complete_still_fails() {
        assert!(matches!(run(&[Event::RequestError, Event::TxComplete]), Verdict::Failed(_)));
    }

    #[test]
    fn first_final_verdict_sticks() {
        assert!(matches!(run(&[Event::RequestSuccess, Event::TxAbort, Event::TxComplete]), Verdict::Failed(_)));
    }

    #[test]
    fn queue_runs_one_at_a_time_in_call_order() {
        let mut q = WriteQueue::default();
        q.push("store a");
        q.push("forget a");
        assert_eq!(q.start_next(), Some("store a"));
        assert_eq!(q.start_next(), None, "the first write is still running");
        q.finish();
        assert_eq!(q.start_next(), Some("forget a"));
        q.finish();
        assert_eq!(q.start_next(), None);
    }
}

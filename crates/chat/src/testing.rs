//! Deterministic stand-ins for tests (feature `testing`, never in a release build): a clock set
//! by hand, "random" bytes from a counter, and a port that only remembers whether it is open.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use crate::{Env, Port};

pub struct TestEnv {
    now: AtomicU64,
    counter: AtomicU64,
    fail_random: AtomicBool,
}

impl TestEnv {
    pub fn at(now_ms: u64) -> Arc<TestEnv> {
        Arc::new(TestEnv { now: AtomicU64::new(now_ms), counter: AtomicU64::new(1), fail_random: AtomicBool::new(false) })
    }
    pub fn set(&self, now_ms: u64) {
        self.now.store(now_ms, SeqCst);
    }
    pub fn advance(&self, ms: u64) {
        let _ = self.now.fetch_update(SeqCst, SeqCst, |t| Some(t.saturating_add(ms)));
    }
    pub fn fail_random(&self, fail: bool) {
        self.fail_random.store(fail, SeqCst);
    }
}

impl Env for TestEnv {
    fn now_ms(&self) -> u64 {
        self.now.load(SeqCst)
    }
    fn random(&self, buf: &mut [u8]) -> Result<(), String> {
        if self.fail_random.load(SeqCst) {
            return Err("no random source (test)".into());
        }
        for chunk in buf.chunks_mut(8) {
            // splitmix64 of a counter: different bytes on every call.
            let mut z = self.counter.fetch_add(1, SeqCst).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            for (b, x) in chunk.iter_mut().zip(z.to_le_bytes()) {
                *b = x;
            }
        }
        Ok(())
    }
}

pub struct FakePort {
    addr: Mutex<Option<String>>,
    flag: bool,
}

impl FakePort {
    /// Closed until `open` (a window started without `--control`); opens on `127.0.0.1:7981`.
    pub fn closed() -> Arc<FakePort> {
        Arc::new(FakePort { addr: Mutex::new(None), flag: false })
    }
    /// Already open with `--control` at `addr`; `close` leaves it open.
    pub fn flag(addr: &str) -> Arc<FakePort> {
        Arc::new(FakePort { addr: Mutex::new(Some(addr.to_string())), flag: true })
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.addr.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Port for FakePort {
    fn open(&self) -> Result<String, String> {
        Ok(self.lock().get_or_insert_with(|| "127.0.0.1:7981".to_string()).clone())
    }
    fn close(&self) {
        if !self.flag {
            *self.lock() = None;
        }
    }
    fn address(&self) -> Option<String> {
        self.lock().clone()
    }
}

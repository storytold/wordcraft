//! A background thread that checks paragraphs one at a time, so typing never waits on the
//! network. Requests are paced ([`Pacing`]): LanguageTool's hosted service allows about 20
//! requests a minute for free use, and asks clients to back off when they send more.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use wordcraft_proof::Issue;

use crate::{LtError, Request, Settings};

/// A paragraph to check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// The paragraph's proofing text (the answer is stored for exactly this text).
    pub text: String,
    /// `auto` or a language code.
    pub language: String,
}

/// A finished check.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub text: String,
    /// The language it was checked in (the job's).
    pub language: String,
    pub result: Result<Vec<Issue>, LtError>,
}

/// How fast requests go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacing {
    /// Least time between two requests.
    pub gap: Duration,
    /// Pause after the server says "too many requests".
    pub backoff: Duration,
    /// How long one request may take.
    pub timeout: Duration,
}

impl Pacing {
    /// For `settings`' server: the hosted service gets about 19 requests a minute; a self-hosted
    /// server is only kept from being flooded.
    pub fn for_settings(settings: &Settings) -> Pacing {
        let gap = if settings.is_hosted() && !settings.has_account() { Duration::from_millis(3200) } else { Duration::from_millis(250) };
        Pacing { gap, backoff: Duration::from_secs(60), timeout: Duration::from_secs(20) }
    }
}

/// Sends a request and returns the answer's body ([`crate::http::post`] in the app; a fake in
/// tests).
pub type Transport = Box<dyn FnMut(&Request) -> Result<String, LtError> + Send>;

/// The checking thread. Dropping it lets the thread finish its current request and end.
pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Worker {
    /// Start checking with `settings` through `transport`; `wake` is called after each result
    /// (the app asks for a repaint). The browser build has no threads to run it on.
    pub fn spawn(settings: Settings, pacing: Pacing, transport: Transport, wake: Box<dyn Fn() + Send>) -> Result<Worker, LtError> {
        if cfg!(target_arch = "wasm32") {
            return Err(LtError::Unsupported(crate::http::UNSUPPORTED_REASON));
        }
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<Done>();
        std::thread::Builder::new()
            .name("languagetool".into())
            .spawn(move || run(&settings, pacing, transport, &job_rx, &done_tx, wake.as_ref()))
            .map_err(|e| LtError::Network(format!("couldn't start the checker: {e}")))?;
        Ok(Worker { jobs, done })
    }

    /// The desktop app's worker: real HTTP.
    pub fn spawn_http(settings: Settings, wake: Box<dyn Fn() + Send>) -> Result<Worker, LtError> {
        let pacing = Pacing::for_settings(&settings);
        let timeout = pacing.timeout;
        Worker::spawn(settings, pacing, Box::new(move |req: &Request| crate::http::post(req, timeout)), wake)
    }

    /// Queue `job`. False when the thread has ended.
    pub fn send(&self, job: Job) -> bool {
        self.jobs.send(job).is_ok()
    }

    /// A finished check, if one is ready. `Err(())` when the thread has ended.
    #[allow(clippy::result_unit_err)]
    pub fn try_recv(&self) -> Result<Option<Done>, ()> {
        match self.done.try_recv() {
            Ok(d) => Ok(Some(d)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }

    /// Wait up to `timeout` for a finished check (tests, CLI).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Done> {
        self.done.recv_timeout(timeout).ok()
    }
}

fn run(settings: &Settings, pacing: Pacing, mut transport: Transport, jobs: &Receiver<Job>, done: &Sender<Done>, wake: &dyn Fn()) {
    let mut next_at: Option<Instant> = None;
    while let Ok(job) = jobs.recv() {
        if let Some(at) = next_at {
            let now = Instant::now();
            if at > now {
                std::thread::sleep(at - now);
            }
        }
        let result = crate::check_with(settings, &mut |req| transport(req), &job.text, &job.language);
        let pause = if result == Err(LtError::RateLimited) { pacing.backoff } else { pacing.gap };
        next_at = Instant::now().checked_add(pause);
        if let Err(e) = &result {
            // Never the text, the key or the request body: only what went wrong.
            log::warn!("LanguageTool check failed: {e}");
        }
        if done.send(Done { text: job.text, language: job.language, result }).is_err() {
            break;
        }
        wake();
    }
}

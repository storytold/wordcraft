//! One Windows driver submission per process: a cancelled driver cannot retain another
//! job's provider lock. The child acknowledges its Cancel token before it exits.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};

#[cfg(windows)]
const FLAG: &str = "--wordcraft-print-worker";
const PREFIX: &str = "WORDCRAFT_PRINT:";
const MAX_MESSAGE: u64 = 64 * 1024;
const MAX_PDF: u64 = 512 * 1024 * 1024;
const CANCELLED: &str = "the print job was cancelled before anything was sent to the printer";

#[derive(Serialize, Deserialize)]
enum Duplex {
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Serialize, Deserialize)]
struct Request {
    printer: Option<String>,
    copies: u32,
    collate: bool,
    duplex: Option<Duplex>,
    grayscale: bool,
    shrink: bool,
    title: String,
    options: Vec<(String, String)>,
}

impl Request {
    /// Preserve all spooler options in a platform-independent wire representation.
    fn from_job(job: &craft_print::Job) -> Self {
        Self {
            printer: job.printer.clone(),
            copies: job.copies,
            collate: job.collate,
            duplex: job.duplex.map(|d| match d {
                craft_print::Duplex::Off => Duplex::Off,
                craft_print::Duplex::LongEdge => Duplex::LongEdge,
                craft_print::Duplex::ShortEdge => Duplex::ShortEdge,
            }),
            grayscale: job.grayscale,
            shrink: job.scaling == craft_print::Scaling::ShrinkToFit,
            title: job.title.clone(),
            options: job.options.clone(),
        }
    }

    /// Restore the job without substituting printer defaults or changing copy semantics.
    fn job(self) -> craft_print::Job {
        craft_print::Job {
            printer: self.printer,
            copies: self.copies,
            collate: self.collate,
            duplex: self.duplex.map(|d| match d {
                Duplex::Off => craft_print::Duplex::Off,
                Duplex::LongEdge => craft_print::Duplex::LongEdge,
                Duplex::ShortEdge => craft_print::Duplex::ShortEdge,
            }),
            grayscale: self.grayscale,
            scaling: if self.shrink { craft_print::Scaling::ShrinkToFit } else { craft_print::Scaling::Exact },
            title: self.title,
            options: self.options,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Event {
    Ready,
    Cancelled,
    Sending,
    Done(Result<String, String>),
}

enum State {
    Starting,
    Waking(Arc<Mutex<Child>>),
    Running(ChildStdin),
    Sending(ChildStdin),
    Cancelled,
    Finished,
}

#[derive(Clone)]
pub(crate) struct Submission(Arc<(Mutex<State>, Condvar)>);

impl Submission {
    /// Create one submission whose driver cannot run until the parent grants startup.
    pub(crate) fn new() -> Self {
        Self(Arc::new((Mutex::new(State::Starting), Condvar::new())))
    }

    /// Recover the lifecycle state after poisoning without invoking another driver call.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Only an acknowledgement of the child's token can cancel a running helper. A timeout
    /// is a refusal, never permission to kill a process that may already be sending.
    pub(crate) fn cancel(&self) -> bool {
        let mut state = self.state();
        match &mut *state {
            State::Starting => {
                *state = State::Cancelled;
                return true;
            }
            State::Waking(child) => {
                let _ = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kill();
                *state = State::Cancelled;
                return true;
            }
            State::Cancelled => return true,
            State::Finished | State::Sending(_) => return false,
            State::Running(input) => {
                if input.write_all(b"cancel\n").and_then(|_| input.flush()).is_err() {
                    return false;
                }
            }
        }
        let (state, _) = self
            .0
            .1
            .wait_timeout_while(state, Duration::from_secs(1), |s| matches!(s, State::Running(_)))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        matches!(*state, State::Cancelled)
    }

    #[cfg(windows)]
    /// Submit through this executable's private helper mode and await its final receipt.
    pub(crate) fn submit(&self, pdf: &[u8], job: &craft_print::Job) -> Result<String, String> {
        self.submit_with(pdf, job, |directory| {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg(FLAG).arg(directory);
            Ok(command)
        })
    }

    /// Publish a terminal state on every result, waking a cancellation caller awaiting an ack.
    fn submit_with(&self, pdf: &[u8], job: &craft_print::Job, command: impl FnOnce(&Path) -> std::io::Result<Command>) -> Result<String, String> {
        let result = self.run(pdf, job, command);
        let mut state = self.state();
        if !matches!(*state, State::Cancelled) {
            *state = State::Finished;
        }
        self.0.1.notify_all();
        if matches!(*state, State::Cancelled) { Err(CANCELLED.into()) } else { result }
    }

    /// Own the private inputs and child until receipt, acknowledged cancellation or failure.
    fn run(&self, pdf: &[u8], job: &craft_print::Job, command: impl FnOnce(&Path) -> std::io::Result<Command>) -> Result<String, String> {
        if matches!(*self.state(), State::Cancelled) {
            return Err(CANCELLED.into());
        }
        if pdf.len() as u64 > MAX_PDF {
            return Err("the print PDF exceeds the 512 MiB helper limit".into());
        }
        let request = serde_json::to_vec(&Request::from_job(job)).map_err(error)?;
        if request.len() as u64 > MAX_MESSAGE {
            return Err("the print settings exceed the helper message limit".into());
        }
        let directory = tempfile::tempdir().map_err(error)?;
        std::fs::write(directory.path().join("job.json"), request).map_err(error)?;
        std::fs::write(directory.path().join("input.pdf"), pdf).map_err(error)?;
        let child =
            command(directory.path()).map_err(error)?.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(error)?;
        let child = Arc::new(Mutex::new(child));
        let outcome = (|| {
            let (input, output) = {
                let mut process = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                (process.stdin.take().ok_or("print helper has no input pipe")?, process.stdout.take().ok_or("print helper has no output pipe")?)
            };
            {
                let mut state = self.state();
                if matches!(*state, State::Cancelled) {
                    return Err(CANCELLED.into());
                }
                *state = State::Waking(child.clone());
            }
            let mut output = BufReader::new(output);
            loop {
                let message = line(&mut output)?.ok_or("the print helper stopped before it was ready")?;
                let Some((_, json)) = message.split_once(PREFIX) else { continue };
                if !matches!(serde_json::from_str::<Event>(json).map_err(error)?, Event::Ready) {
                    return Err("the print helper did not acknowledge startup".into());
                }
                break;
            }
            {
                let mut state = self.state();
                if matches!(*state, State::Cancelled) {
                    return Err(CANCELLED.into());
                }
                *state = State::Running(input);
                if let State::Running(input) = &mut *state {
                    input.write_all(b"start\n").and_then(|_| input.flush()).map_err(error)?;
                }
            }
            while let Some(line) = line(&mut output)? {
                let Some((_, json)) = line.split_once(PREFIX) else { continue };
                match serde_json::from_str::<Event>(json).map_err(error)? {
                    Event::Ready => return Err("the print helper repeated its startup acknowledgement".into()),
                    Event::Cancelled => {
                        *self.state() = State::Cancelled;
                        self.0.1.notify_all();
                        return Err(CANCELLED.into());
                    }
                    Event::Sending => {
                        let mut state = self.state();
                        let previous = std::mem::replace(&mut *state, State::Finished);
                        *state = match previous {
                            State::Running(input) | State::Sending(input) => State::Sending(input),
                            other => other,
                        };
                        self.0.1.notify_all();
                    }
                    Event::Done(result) => return result,
                }
            }
            Err("the print helper stopped without a receipt".into())
        })();
        // A cancelled child has acknowledged that it cannot send. Other outcomes are terminal
        // receipts or a broken protocol/pipe; reap the process and its private input files.
        if outcome.is_err() {
            let _ = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kill();
        }
        let status = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).wait().map_err(error)?;
        if outcome.is_ok() && !status.success() {
            return Err("the print helper failed after its receipt".into());
        }
        outcome
    }
}

/// Keep helper transport failures distinct from a printer's own error message.
fn error(error: impl std::fmt::Display) -> String {
    format!("print helper: {error}")
}

/// Read at most one byte beyond the cap so oversized files fail without unbounded allocation.
fn bounded(mut input: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    input.by_ref().take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > limit { Err("print helper input exceeds its limit".into()) } else { Ok(bytes) }
}

/// Read one bounded UTF-8 protocol record, distinguishing EOF from an oversized message.
fn line(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    input.take(MAX_MESSAGE + 1).read_until(b'\n', &mut bytes).map_err(error)?;
    if bytes.len() as u64 > MAX_MESSAGE {
        return Err("print helper message exceeds its limit".into());
    }
    if bytes.is_empty() { Ok(None) } else { String::from_utf8(bytes).map(|s| Some(s.trim_end().to_owned())).map_err(error) }
}

/// Serialize complete events under stdout's lock so the driver and listener cannot interleave.
fn emit(event: &Event) -> Result<(), String> {
    let mut output = std::io::stdout().lock();
    write!(output, "{PREFIX}").map_err(error)?;
    serde_json::to_writer(&mut output, event).map_err(error)?;
    writeln!(output).and_then(|_| output.flush()).map_err(error)
}

/// Remove both plaintext inputs even when one fails; already-removed inputs count as success.
fn remove_inputs(directory: &Path) -> Result<(), String> {
    let mut first_error = None;
    for name in ["job.json", "input.pdf"] {
        if let Err(failure) = std::fs::remove_file(directory.join(name))
            && failure.kind() != std::io::ErrorKind::NotFound
        {
            first_error.get_or_insert_with(|| error(format!("could not remove {name}: {failure}")));
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Load and erase the private inputs before driver work, while an independent listener can cancel.
fn helper(
    directory: &Path,
    cancel_request: impl Fn(&craft_print::Cancel) -> bool + Send + 'static,
    submit: impl FnOnce(&[u8], &craft_print::Job, &craft_print::Cancel) -> Result<String, String>,
) -> Result<(), String> {
    let mut input = BufReader::new(std::io::stdin());
    emit(&Event::Ready)?;
    if line(&mut input)?.as_deref() != Some("start") {
        return Err("print helper requires a start acknowledgement".into());
    }
    let cancel = craft_print::Cancel::new();
    let listener = cancel.clone();
    std::thread::Builder::new()
        .name("print-cancel".into())
        .spawn(move || {
            loop {
                match line(&mut input) {
                    Ok(Some(command)) if command == "cancel" => {
                        if cancel_request(&listener) {
                            let _ = emit(&Event::Cancelled);
                            std::process::exit(0);
                        }
                        if emit(&Event::Sending).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            // If the parent disappears, never leave a waiting driver process behind. Sending
            // jobs finish normally so the OS gets their pages even if the UI has gone away.
            if cancel_request(&listener) {
                std::process::exit(0);
            }
        })
        .map_err(error)?;
    let request: Request =
        serde_json::from_slice(&bounded(std::fs::File::open(directory.join("job.json")).map_err(error)?, MAX_MESSAGE)?).map_err(error)?;
    let pdf = bounded(std::fs::File::open(directory.join("input.pdf")).map_err(error)?, MAX_PDF)?;
    remove_inputs(directory)?;
    let result = submit(&pdf, &request.job(), &cancel);
    emit(&Event::Done(result))
}

#[cfg(windows)]
/// Handle only the private first-argument flag, before logging or GUI initialization.
pub(crate) fn helper_mode() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return None;
    }
    let result = args.next().ok_or_else(|| "print helper directory missing".to_owned()).and_then(|dir| {
        helper(Path::new(&dir), craft_print::Cancel::cancel, |pdf, job, cancel| {
            craft_print::submit_cancellable(pdf, job, &mut |_, _| {}, cancel).map_err(error)
        })
    });
    if let Err(message) = &result {
        let _ = emit(&Event::Done(Err(message.clone())));
    }
    Some(i32::from(result.is_err()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real test child exercises production pipes, token handling and process cleanup.
    #[test]
    #[ignore = "launched by the process protocol tests"]
    fn helper_process() {
        let Some(directory) = std::env::var_os("WORDCRAFT_TEST_PRINT_DIR") else { return };
        if std::env::var("WORDCRAFT_TEST_PRINT_MODE").as_deref() == Ok("waking") {
            std::fs::write(Path::new(&directory).join("waking"), b"starting").unwrap();
            loop {
                std::thread::park();
            }
        }
        let result = helper(
            Path::new(&directory),
            |token| {
                if std::env::var("WORDCRAFT_TEST_PRINT_MODE").as_deref() == Ok("sending") { false } else { token.cancel() }
            },
            |pdf, job, _token| {
                assert_eq!(pdf, b"synthetic PDF");
                assert_eq!(job.printer.as_deref(), Some("test queue"));
                match std::env::var("WORDCRAFT_TEST_PRINT_MODE").as_deref() {
                    Ok("held") => {
                        let _provider = Mutex::new(());
                        let _held = _provider.lock().unwrap();
                        std::fs::write(Path::new(&directory).join("held"), b"driver entered").unwrap();
                        loop {
                            std::thread::park();
                        }
                    }
                    Ok("crash") => std::process::exit(7),
                    Ok("sending") => {
                        std::fs::write(Path::new(&directory).join("sending"), b"sending").unwrap();
                        while !Path::new(&directory).join("release").exists() {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Ok("spooled synthetic job".into())
                    }
                    _ => Ok("spooled synthetic job".into()),
                }
            },
        );
        std::process::exit(i32::from(result.is_err()));
    }

    /// Launch just the ignored helper entry, without invoking printer hardware.
    fn command(directory: &Path, mode: &str) -> std::io::Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        let name = module_path!().split_once("::").map_or(module_path!(), |(_, name)| name);
        command.args(["--ignored", "--exact", &format!("{name}::helper_process"), "--nocapture"]);
        command.env("WORDCRAFT_TEST_PRINT_DIR", directory).env("WORDCRAFT_TEST_PRINT_MODE", mode);
        Ok(command)
    }

    /// Give the child a literal printer identity to verify transport.
    fn job() -> craft_print::Job {
        craft_print::Job { printer: Some("test queue".into()), ..Default::default() }
    }

    #[test]
    /// Cancelling before submission cannot launch a driver process.
    fn cancelled_start_never_launches_a_helper() {
        let submission = Submission::new();
        assert!(submission.cancel());
        let result = submission.submit_with(b"synthetic PDF", &job(), |_| Err(std::io::Error::other("must not launch")));
        assert_eq!(result, Err(CANCELLED.into()));
    }

    #[test]
    /// A cancellation during creation withholds the start grant and removes private inputs.
    fn cancellation_during_startup_never_starts_the_driver() {
        let submission = Submission::new();
        let worker = submission.clone();
        let (started, starting) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            worker.submit_with(b"synthetic PDF", &job(), |directory| {
                started.send(directory.to_path_buf()).unwrap();
                wait.recv().unwrap();
                command(directory, "held")
            })
        });
        let directory = starting.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(submission.cancel());
        release.send(()).unwrap();
        assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
        assert!(!directory.exists(), "startup cancellation reaps the child and removes its inputs");
    }

    #[test]
    /// A hung startup has no permission to send and can be killed without a token ack.
    fn an_unresponsive_starting_process_can_be_stopped_without_an_acknowledgement() {
        let submission = Submission::new();
        let worker = submission.clone();
        let (path_tx, path_rx) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            worker.submit_with(b"synthetic PDF", &job(), |dir| {
                path_tx.send(dir.to_path_buf()).unwrap();
                command(dir, "waking")
            })
        });
        let directory = path_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !directory.join("waking").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(directory.join("waking").exists());
        assert!(submission.cancel(), "a process without a start grant cannot send anything");
        assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
        assert!(!directory.exists());
    }

    #[test]
    /// Job transport preserves every explicit setting, including Unicode and duplex direction.
    fn job_options_survive_the_process_boundary() {
        let expected = craft_print::Job {
            printer: Some("Office λ".into()),
            copies: 3,
            collate: false,
            duplex: Some(craft_print::Duplex::ShortEdge),
            grayscale: true,
            scaling: craft_print::Scaling::ShrinkToFit,
            title: "A document".into(),
            options: vec![("media".into(), "iso_a4_210x297mm".into())],
        };
        let bytes = serde_json::to_vec(&Request::from_job(&expected)).unwrap();
        let received: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(received.job(), expected);
    }

    #[test]
    /// Retry succeeds immediately after cancellation, before joining the old worker.
    fn cancelled_driver_does_not_block_the_next_process() {
        let submission = Submission::new();
        let worker = submission.clone();
        let (path_tx, path_rx) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            worker.submit_with(b"synthetic PDF", &job(), |dir| {
                path_tx.send(dir.to_path_buf()).unwrap();
                command(dir, "held")
            })
        });
        let directory = path_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !directory.join("held").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(directory.join("held").exists(), "the child is held inside its driver");
        assert!(!directory.join("input.pdf").exists(), "the stalled driver retains no plaintext PDF");
        assert!(!directory.join("job.json").exists(), "loaded settings were erased before driver work");
        assert!(submission.cancel(), "a stalled driver still acknowledges cancellation");
        let next = Submission::new();
        assert_eq!(next.submit_with(b"synthetic PDF", &job(), |dir| command(dir, "ready")), Ok("spooled synthetic job".into()));
        assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
        assert!(!directory.exists(), "the reaped helper's PDF and settings were removed");
        assert!(!next.cancel(), "a completed job keeps its receipt");
    }

    #[test]
    /// A refused cancellation keeps the sending process alive until its receipt.
    fn cancellation_refusal_preserves_the_sending_job() {
        let submission = Submission::new();
        let worker = submission.clone();
        let (path_tx, path_rx) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            worker.submit_with(b"synthetic PDF", &job(), |dir| {
                path_tx.send(dir.to_path_buf()).unwrap();
                command(dir, "sending")
            })
        });
        let directory = path_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !directory.join("sending").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(directory.join("sending").exists());
        assert!(!submission.cancel(), "a sending job refuses cancellation");
        assert!(matches!(*submission.state(), State::Sending(_)));
        assert!(!running.is_finished(), "the child is retained until it finishes sending");
        std::fs::write(directory.join("release"), b"continue").unwrap();
        assert_eq!(running.join().unwrap(), Ok("spooled synthetic job".into()));
        assert!(!directory.exists());
    }

    #[test]
    /// Closing the parent's pipe stops a waiting driver and permits parent-side cleanup.
    fn parent_disconnect_stops_a_waiting_driver() {
        let submission = Submission::new();
        let worker = submission.clone();
        let (path_tx, path_rx) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            worker.submit_with(b"synthetic PDF", &job(), |dir| {
                path_tx.send(dir.to_path_buf()).unwrap();
                command(dir, "held")
            })
        });
        let directory = path_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !directory.join("held").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(directory.join("held").exists());
        *submission.state() = State::Finished; // Drops the parent's pipe as on process exit.
        assert_eq!(running.join().unwrap(), Err("the print helper stopped without a receipt".into()));
        assert!(!directory.exists());
    }

    #[test]
    /// Helper failure removes its input directory; input limits reject oversize data.
    fn helper_crashes_are_errors_and_inputs_are_bounded() {
        let submission = Submission::new();
        let mut inputs = None;
        assert_eq!(
            submission.submit_with(b"synthetic PDF", &job(), |dir| {
                inputs = Some(dir.to_path_buf());
                command(dir, "crash")
            }),
            Err("the print helper stopped without a receipt".into())
        );
        assert!(!inputs.unwrap().exists(), "a helper crash removes the PDF and settings");
        assert_eq!(bounded(&b"abcd"[..], 3), Err("print helper input exceeds its limit".into()));
        assert_eq!(bounded(&b"abc"[..], 3), Ok(b"abc".to_vec()));
    }

    /// Cleanup is repeatable and reports real failures while still attempting both inputs.
    #[test]
    fn input_cleanup_tolerates_missing_files_and_reports_failures() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("job.json"), b"settings").unwrap();
        std::fs::write(directory.path().join("input.pdf"), b"private PDF").unwrap();
        assert_eq!(remove_inputs(directory.path()), Ok(()));
        assert_eq!(remove_inputs(directory.path()), Ok(()));
        assert!(!directory.path().join("input.pdf").exists());
        std::fs::create_dir(directory.path().join("job.json")).unwrap();
        std::fs::write(directory.path().join("input.pdf"), b"private PDF").unwrap();
        let error = remove_inputs(directory.path()).unwrap_err();
        assert!(error.contains("could not remove job.json"), "{error}");
        assert!(!directory.path().join("input.pdf").exists(), "a settings cleanup failure does not retain the PDF");
    }
}

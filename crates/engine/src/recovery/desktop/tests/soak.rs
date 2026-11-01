//! Opt-in process replay for the recovery worker. Uses real timers and retains its profiles.

use super::{Document, Duration, Instant, Path, PathBuf, Session, StoryRef, fs, json, new_id, read_snapshot, settle};
use sha2::Digest;
use std::process::{Child, Command, Stdio};

const CHILD_TEST: &str = "recovery::desktop::tests::soak::recovery_soak_child";
const CYCLE_BUDGET: Duration = Duration::from_secs(50);

/// Retain one evidence record, failing the check if serialization or writing fails.
fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// Read a complete evidence record; missing or malformed evidence must fail the check.
fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// SHA-256 is evidence; exact byte comparisons below also guard the originals.
fn sha256(path: &Path) -> String {
    sha2::Sha256::digest(fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A handle from our own spawn, checked against Linux's executable identity before killing it.
/// The guard also settles an owned child when an assertion fails. No name/port process lookup.
struct OwnedChild {
    child: Child,
    exe: PathBuf,
}

impl OwnedChild {
    /// Start one role in this test executable with its own log and explicit parent identity.
    fn spawn(dir: &Path, role: &str) -> Self {
        let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
        let log = fs::File::create(dir.join(format!("{role}.log"))).unwrap();
        let child = Command::new(&exe)
            .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
            .env("WORDCRAFT_RECOVERY_SOAK_CHILD", role)
            .env("WORDCRAFT_RECOVERY_SOAK_CYCLE", dir)
            .env("WORDCRAFT_RECOVERY_SOAK_PARENT", std::process::id().to_string())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        Self { child, exe }
    }

    /// Verify a live spawned child's executable and parent, and retain its PID and start time.
    fn identity(&mut self) -> serde_json::Value {
        assert!(self.child.try_wait().unwrap().is_none(), "child exited; inspect its retained log");
        let proc = PathBuf::from(format!("/proc/{}", self.child.id()));
        assert_eq!(fs::read_link(proc.join("exe")).unwrap(), self.exe);
        let stat = fs::read_to_string(proc.join("stat")).unwrap();
        // After the final ')' come state, ppid, ... starttime (field 22).
        let fields: Vec<_> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
        assert_eq!(fields[1].parse::<u32>().unwrap(), std::process::id());
        json!({"pid": self.child.id(), "executable": self.exe, "startTicks": fields[19]})
    }

    /// Require the owned child to exit successfully before the caller's deadline.
    fn wait(&mut self, deadline: Instant) {
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "child failed; inspect its retained log: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "child deadline; inspect its retained log");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Verify this spawned child, send SIGKILL through its handle and wait for signal 9.
    fn crash(&mut self) -> serde_json::Value {
        use std::os::unix::process::ExitStatusExt;
        let identity = self.identity();
        self.child.kill().unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(9));
        identity
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() && fs::read_link(format!("/proc/{}/exe", self.child.id())).is_ok_and(|p| p == self.exe) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Advances the actual host hook, without changing the recovery interval or forcing a due slot.
fn until(s: &mut Session, deadline: Instant, ready: impl Fn(&mut Session) -> bool) {
    loop {
        s.recovery_tick();
        if ready(s) {
            return;
        }
        assert!(Instant::now() < deadline, "snapshot deadline: {:?}", s.recovery_error());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Append a phase-specific witness to every open document through the normal edit commands.
fn edit_all(s: &mut Session, phase: usize) {
    let ids: Vec<_> = s.documents().iter().map(|d| d.id).collect();
    for (index, id) in ids.into_iter().enumerate() {
        s.with_document(id, |s| {
            s.run("caret.docEnd", &json!({})).unwrap();
            s.run("text.insert", &json!({"text": format!(" Saved edit {phase}, document {index}."), "raw": true})).unwrap();
        })
        .unwrap();
    }
}

/// Compare completed snapshots with the whole live models and retain expected models and hashes.
fn capture(s: &mut Session, dir: &Path, phase: usize) -> Vec<serde_json::Value> {
    let mut copies = Vec::new();
    for d in s.recovery_docs() {
        let id = d.slot.id.as_ref().unwrap();
        let record = dir.join("profile").join(id);
        let path = super::super::Store::files(&record).unwrap().first().unwrap().clone();
        let snapshot = read_snapshot(&path).unwrap();
        assert_eq!(&snapshot.document, d.doc, "the completed snapshot must match the entire live model");
        let expected = dir.join(format!("expected-{id}-{phase}.json"));
        fs::write(&expected, serde_json::to_vec(d.doc).unwrap()).unwrap();
        copies.push(json!({"id": id, "path": path, "expected": expected, "sha256": sha256(&path)}));
    }
    copies.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    copies
}

/// Use real timers for two documents' successful snapshots and failed successors, then await crash.
fn writer(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(38);
    let mut s = Session::new(Document::new());
    s.run("file.open", &json!({"path": dir.join("original-a.docx")})).unwrap();
    s.run("file.open", &json!({"path": dir.join("original-b.json")})).unwrap();
    assert_eq!(s.documents().len(), 2);
    s.enable_recovery(dir.join("profile"), std::sync::Arc::new(|| {})).unwrap();
    settle(&mut s);
    let started = Instant::now();
    edit_all(&mut s, 1);
    until(&mut s, deadline, |s| s.recovery_docs().all(|d| d.slot.id.is_some() && d.slot.revision == Some(d.rev)) && !s.recovery_busy());
    let first_ms = started.elapsed().as_millis();
    let first = capture(&mut s, dir, 1);
    let started = Instant::now();
    edit_all(&mut s, 2);
    until(&mut s, deadline, |s| s.recovery_docs().all(|d| d.slot.revision == Some(d.rev)) && !s.recovery_busy());
    let second_ms = started.elapsed().as_millis();
    let second = capture(&mut s, dir, 2);
    // Force a final rename failure in each synthetic record. Retain the obstruction as evidence.
    for copy in &second {
        let path = Path::new(copy["path"].as_str().unwrap());
        let next = read_snapshot(path).unwrap().generation + 1;
        fs::create_dir(path.parent().unwrap().join(format!("snapshot-{next:020}.json"))).unwrap();
    }
    let started = Instant::now();
    edit_all(&mut s, 3);
    until(&mut s, deadline, |s| s.recovery_docs().all(|d| d.slot.revision.is_none()) && !s.recovery_busy());
    assert!(s.recovery_error().is_some());
    for copy in first.iter().chain(&second) {
        assert_eq!(sha256(Path::new(copy["path"].as_str().unwrap())), copy["sha256"]);
    }
    write_json(
        &dir.join("ready.json"),
        &json!({"pid": std::process::id(), "first": first, "second": second,
        "snapshotLatencyMs": [first_ms, second_ms], "failedWriteLatencyMs": started.elapsed().as_millis(),
        "failedWriteError": s.recovery_error(), "dirtyDocuments": 2}),
    );
    loop {
        // No graceful finish/drop: the parent terminates this exact child with SIGKILL.
        // A forcibly terminated parent cannot run its guard, so the writer also expires itself.
        assert!(Instant::now() < deadline, "writer exceeded its bounded crash wait");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Restore the latest valid and fallback models beside unsaved work, then save independent copies.
fn restore(dir: &Path) {
    let expected = read_json(&dir.join("restore-plan.json"));
    let mut s = Session::new(Document::from_text("Work already open at restart."));
    s.run("text.insert", &json!({"text": "Kept. ", "raw": true})).unwrap();
    let live = s.doc.clone();
    let live_id = s.document_id();
    let start = Instant::now();
    s.enable_recovery(dir.join("profile"), std::sync::Arc::new(|| {})).unwrap();
    settle(&mut s);
    assert!(s.recovery_error().is_some(), "corrupt newest generation must be reported");
    assert_eq!(s.recovery_documents().len(), 2);
    for copy in expected.as_array().unwrap() {
        s.run("file.recover", &json!({"restore": copy["id"]})).unwrap();
        settle(&mut s);
        let document: Document = serde_json::from_slice(&fs::read(copy["expected"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(s.doc, document, "restore the whole latest valid saved model, never the failed edit");
        assert!(s.dirty && s.path.is_none());
        assert_eq!(s.recovery_slot.id.as_deref(), copy["id"].as_str());
        // Save from an independent session so the recovered dirty documents keep their records.
        let mut saved = Session::new(s.doc.clone());
        for ext in ["json", "docx"] {
            let path = dir.join(format!("restored-{}.{ext}", copy["id"].as_str().unwrap()));
            saved.run("file.save", &json!({"path": path})).unwrap();
        }
    }
    assert_eq!(s.documents().len(), 3);
    assert_eq!(s.with_document(live_id, |s| s.doc.clone()).unwrap(), live);
    write_json(&dir.join("restored.json"), &json!({"passed": true, "latencyMs": start.elapsed().as_millis()}));
    // No finish_recovery: leave every profile and generation available for inspection.
}

/// Reopen both saved formats in a fresh process and compare text and the native model.
fn observe(dir: &Path) {
    let expected = read_json(&dir.join("restore-plan.json"));
    for copy in expected.as_array().unwrap() {
        let document: Document = serde_json::from_slice(&fs::read(copy["expected"].as_str().unwrap()).unwrap()).unwrap();
        for ext in ["json", "docx"] {
            let path = dir.join(format!("restored-{}.{ext}", copy["id"].as_str().unwrap()));
            let mut s = Session::new(Document::new());
            s.run("file.open", &json!({"path": path})).unwrap();
            assert_eq!(s.doc.plain_text(StoryRef::Body), document.plain_text(StoryRef::Body));
            // Save legitimately updates core timestamps/revision; snapshot equality was checked
            // before saving. Compare every other native model field after a fresh process open.
            if ext == "json" {
                let mut actual = s.doc.clone();
                actual.core.last_modified_by = document.core.last_modified_by.clone();
                actual.core.created = document.core.created.clone();
                actual.core.modified = document.core.modified.clone();
                actual.core.revision = document.core.revision;
                assert_eq!(actual, document);
            }
        }
    }
    write_json(&dir.join("observed.json"), &json!({"passed": true, "freshProcess": true}));
}

/// Only the parent below supplies the role, fresh cycle directory and verified parent identity.
#[test]
#[ignore = "subprocess entry point for the opt-in Linux recovery soak"]
fn recovery_soak_child() {
    let Ok(role) = std::env::var("WORDCRAFT_RECOVERY_SOAK_CHILD") else { return };
    let dir = PathBuf::from(std::env::var_os("WORDCRAFT_RECOVERY_SOAK_CYCLE").unwrap());
    assert!(dir.is_absolute() && dir.canonicalize().unwrap() == dir);
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let parent = status.lines().find_map(|l| l.strip_prefix("PPid:")).unwrap().trim();
    assert_eq!(parent, std::env::var("WORDCRAFT_RECOVERY_SOAK_PARENT").unwrap());
    match role.as_str() {
        "writer" => writer(&dir),
        "restore" => restore(&dir),
        "observe" => observe(&dir),
        _ => panic!("unknown recovery soak role"),
    }
}

/// Sustained, explicitly bounded process replay. CI only compiles it; profiles are never removed.
#[test]
#[ignore = "opt-in Linux process soak; see docs/recovery.md"]
fn sustained_crash_restart() {
    let seconds: u64 = std::env::var("WORDCRAFT_RECOVERY_SOAK_SECONDS").expect("set an explicit duration").parse().unwrap();
    let limit: usize = std::env::var("WORDCRAFT_RECOVERY_SOAK_ITERATIONS").unwrap_or_else(|_| "32".into()).parse().unwrap();
    assert!((50..=3600).contains(&seconds) && (1..=100).contains(&limit));
    let root = PathBuf::from(std::env::var_os("WORDCRAFT_RECOVERY_SOAK_DIR").expect("set a fresh absolute output directory"));
    assert!(root.is_absolute() && root.parent().unwrap().is_dir());
    fs::create_dir(&root).expect("output must be new; retained evidence is never overwritten");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut iterations = Vec::new();
    // The validated minimum admits the first cycle. Comparing its deadline after startup
    // would reject exactly 50 seconds before doing any work.
    while iterations.len() < limit && (iterations.is_empty() || deadline.saturating_duration_since(Instant::now()) >= CYCLE_BUDGET) {
        let dir = root.join(format!("cycle-{}-{}", iterations.len() + 1, new_id().unwrap()));
        fs::create_dir(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut originals = Vec::new();
        for (name, text) in [("original-a.docx", "Original synthetic DOCX."), ("original-b.json", "Original synthetic native document.")] {
            let path = dir.join(name);
            let mut document = Document::from_text(text);
            document.core.title = format!("Synthetic recovery {name}");
            document.core.subject = "Crash and restart retention".into();
            document.core.creator = "Synthetic author".into();
            document.core.keywords = "recovery, soak".into();
            document.core.description = "Original metadata must survive native save and reopen.".into();
            document.core.category = "Reliability verification".into();
            crate::io::save_path(&path, &document).unwrap();
            originals.push((path.clone(), fs::read(&path).unwrap(), sha256(&path)));
        }
        let cycle_start = Instant::now();
        let mut writer = OwnedChild::spawn(&dir, "writer");
        let writer_deadline = Instant::now() + Duration::from_secs(39);
        let ready_path = dir.join("ready.json");
        let ready = loop {
            writer.identity();
            if let Ok(bytes) = fs::read(&ready_path)
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            {
                break value;
            }
            assert!(Instant::now() < writer_deadline, "writer deadline; retained log: {}", dir.display());
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(ready["pid"].as_u64(), Some(u64::from(writer.child.id())));
        let identity = writer.crash();
        let first = ready["first"].as_array().unwrap();
        let second = ready["second"].as_array().unwrap();
        let newest = Path::new(second[1]["path"].as_str().unwrap());
        fs::copy(newest, dir.join("newest-before-corruption.json")).unwrap();
        fs::write(newest, b"{truncated synthetic newest generation").unwrap();
        // One record proves latest valid save after failed writes; the other proves fallback.
        write_json(&dir.join("restore-plan.json"), &json!([second[0], first[1]]));
        let mut restore = OwnedChild::spawn(&dir, "restore");
        restore.wait(deadline.min(Instant::now() + Duration::from_secs(5)));
        let mut observe = OwnedChild::spawn(&dir, "observe");
        observe.wait(deadline.min(Instant::now() + Duration::from_secs(5)));
        for (path, bytes, digest) in &originals {
            assert_eq!(fs::read(path).unwrap(), *bytes, "original bytes changed");
            assert_eq!(sha256(path), *digest, "original SHA-256 changed");
        }
        iterations.push(json!({"directory": dir, "durationMs": cycle_start.elapsed().as_millis(),
            "crashedProcess": identity, "writer": ready, "restore": read_json(&dir.join("restored.json")),
            "observe": read_json(&dir.join("observed.json")),
            "originalSha256": originals.iter().map(|(p, _, h)| json!({"path": p, "sha256": h})).collect::<Vec<_>>()}));
        write_json(&root.join("progress.json"), &json!({"iterations": iterations}));
    }
    assert!(!iterations.is_empty(), "no complete iteration fitted the duration");
    let report = json!({"passed": true, "requestedSeconds": seconds, "maxIterations": limit,
        "elapsedMs": started.elapsed().as_millis(), "completedIterations": iterations.len(), "iterations": iterations,
        "stoppingCondition": if iterations.len() == limit { "iteration limit" } else { "remaining duration cannot admit a complete 50-second cycle" },
        "profilesRetained": true, "platform": "Linux child processes and engine recovery host hook",
        "nativeDesktopOrWordTested": false, "powerLossTested": false});
    write_json(&root.join("report.json"), &report);
    println!("{}", serde_json::to_string(&report).unwrap());
}

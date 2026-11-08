#![allow(clippy::unwrap_used)]
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wordcraft-launcher-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn absent_configuration_is_disabled_and_relative_paths_are_refused() {
    assert!(configured_from("TEST", None, None).unwrap().is_none());
    assert!(configured_from("TEST", Some(PathBuf::from("relative-launcher")), None).unwrap_err().contains("absolute"));
    let folder = Temp::new();
    let executable = folder.0.join("wordcraft");
    assert!(configured_from("TEST", None, Some(executable)).unwrap().is_none());
}

#[test]
fn a_preconnected_agent_does_not_start_any_launcher() {
    let hub = AgentHub::default();
    let _ = hub.listen(1, None, std::time::Duration::ZERO, || true).unwrap();
    let mut launcher = Launcher::default();
    launcher.start("WORDCRAFT_TEST_NO_CONFIGURATION", &hub).unwrap();
    assert!(launcher.child.is_none());
}

#[cfg(unix)]
#[test]
fn launcher_paths_are_not_shell_commands_and_receive_no_document_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let folder = Temp::new();
    let program = folder.0.join("agent 'launcher'");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s' \"$#\" > argc\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(configured_from("TEST", Some(program.clone()), None).unwrap(), Some(program.clone()));
    let mut launcher = Launcher::default();
    launcher.spawn(&program).unwrap();
    let pid = launcher.child.as_ref().unwrap().id();
    launcher.start("WORDCRAFT_TEST_NO_CONFIGURATION", &AgentHub::default()).unwrap();
    assert_eq!(launcher.child.as_ref().unwrap().id(), pid, "an already-running launcher is never duplicated");
    let marker = folder.0.join("argc");
    for _ in 0..200 {
        if marker.is_file() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "0", "no prompt, selection, key or token travels through argv");
    let child = launcher.child.as_mut().unwrap();
    let _ = child.kill();
    child.wait().unwrap();
}

#![cfg(unix)]

use std::process::{Command, Output};

const SCRIPT: &str = include_str!("../../packaging/macos/package.sh");

fn section(start: &str, end: &str) -> &'static str {
    let (_, rest) = SCRIPT.split_once(start).unwrap();
    let (part, _) = rest.split_once(end).unwrap();
    &SCRIPT[SCRIPT.len() - rest.len() - start.len()..SCRIPT.len() - rest.len() + part.len()]
}

fn bash(path: &std::ffi::OsStr) -> Command {
    let mut cmd = Command::new("bash");
    cmd.env_clear().env("PATH", path);
    cmd
}

#[test]
fn bash_is_resolved_from_the_supplied_executable_path() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"))
        .join("macos-test-bash-path");
    std::fs::create_dir_all(&root).unwrap();
    let mut fixture = None;
    for n in 0..1000 {
        let dir = root.join(format!("wordcraft-bash-path-{}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                fixture = Some(dir);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => panic!("create test directory: {e}"),
        }
    }
    let dir = fixture.unwrap();
    let executable = dir.join("bash");
    std::fs::write(&executable, "#!/bin/sh\nprintf 'synthetic-bash-from-path\\n'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = bash(dir.as_os_str()).args(["-c", "exit 73"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "synthetic-bash-from-path\n");
}

fn run(tool: &str, failures: u8, dmg: bool) -> Output {
    let setup = r#"
export PATH=/usr/bin:/bin
APP='Synthetic App.app'; DMG='Synthetic image.dmg'; WORK='Synthetic work'; NOTARIZE=1
attempts=0
warn() { printf '%s\n' "$*" >&2; }
sleep() { printf 'PAUSE:%s\n' "$1"; }
notarize() { :; }
spctl() { printf 'ASSESS'; printf '|%s' "$@"; printf '\n'; }
xcrun() {
  if [[ "$1" = --find && "$2" = stapler ]]; then
    [[ "$TOOL" = native ]]; return
  fi
  [[ "$TOOL" = native && "$#" = 3 && "$1" = stapler && "$3" = "$FILE" ]] || return 71
  printf 'NATIVE:%s\n' "$2"
  if [[ "$2" = staple ]]; then
    attempts=$((attempts + 1))
    [[ "$attempts" -gt "$FAILURES" ]] || return 1
  else
    [[ "$2" = validate ]] || return 72
  fi
}
if [[ "$TOOL" != none ]]; then
  rcodesign() {
    [[ "$TOOL" = rcodesign && "$#" = 2 && "$1" = staple && "$2" = "$FILE" ]] || return 73
    printf 'RCODESIGN:staple\n'
    attempts=$((attempts + 1))
    [[ "$attempts" -gt "$FAILURES" ]]
  }
fi
"#;
    let helper = section("stapler_retry() {", "\nnotarize() {");
    let caller = if dmg {
        section("if [[ \"$NOTARIZE\" = 1 ]]; then\n  notarize \"$DMG\"", "\n# ---- CLI")
    } else {
        section("if [[ \"$NOTARIZE\" = 1 ]]; then\n  ditto -c -k", "\n# ---- DMG")
    };
    // The archive command precedes notarization; only its external operation is mocked.
    let setup = format!("{setup}\nditto() {{ :; }}");
    let path = std::env::var_os("PATH").unwrap_or_default();
    bash(&path)
        .env("TOOL", tool)
        .env("FAILURES", failures.to_string())
        .env("FILE", if dmg { "Synthetic image.dmg" } else { "Synthetic App.app" })
        .args(["-euc", &format!("{setup}\n{helper}\n{caller}")])
        .output()
        .unwrap()
}

#[test]
fn native_stapler_is_preferred_and_validates_both_containers() {
    for dmg in [false, true] {
        let out = run("native", 0, dmg);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("NATIVE:staple\nNATIVE:validate\n"));
        assert!(!text.contains("RCODESIGN"));
        assert!(text.contains("ASSESS|--assess|--type|"));
    }
}

#[test]
fn rcodesign_fallback_staples_without_a_validate_step() {
    for dmg in [false, true] {
        let out = run("rcodesign", 0, dmg);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("RCODESIGN:staple\n"));
        assert!(!text.contains("NATIVE") && !text.contains("validate"));
        assert!(text.contains("ASSESS|--assess|--type|"));
    }
}

#[test]
fn missing_tools_warn_and_still_run_gatekeeper_assessment() {
    for dmg in [false, true] {
        let out = run("none", 0, dmg);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stderr).contains("stapler/rcodesign not found: ticket not stapled; Gatekeeper will fetch it online"));
        assert!(String::from_utf8_lossy(&out.stdout).contains("ASSESS|--assess|--type|"));
    }
}

#[test]
fn both_tools_retry_four_times_with_the_same_backoff() {
    for tool in ["native", "rcodesign"] {
        let out = run(tool, 3, false);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        for delay in [20, 40, 60] {
            assert!(text.contains(&format!("PAUSE:{delay}\n")));
        }
        assert_eq!(text.matches(if tool == "native" { "NATIVE:staple" } else { "RCODESIGN:staple" }).count(), 4);
    }
}

#[test]
fn persistent_stapling_errors_fail_instead_of_becoming_missing_tool_warnings() {
    for tool in ["native", "rcodesign"] {
        let out = run(tool, 4, false);
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("ASSESS"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("failed 4 times"));
    }
}

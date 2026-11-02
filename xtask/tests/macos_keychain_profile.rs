#![cfg(unix)]

use std::process::{Command, Output};

const SCRIPT: &str = include_str!("../../packaging/macos/package.sh");

fn section(start: &str, end: &str) -> &'static str {
    let (_, rest) = SCRIPT.split_once(start).unwrap();
    let (part, _) = rest.split_once(end).unwrap();
    &SCRIPT[SCRIPT.len() - rest.len() - start.len()..SCRIPT.len() - rest.len() + part.len()]
}

fn run(identity: &str, profile: bool, legacy: bool, accepted: bool, call: bool) -> Output {
    let setup = r#"
export PATH=/usr/bin:/bin
warn() { printf '%s\n' "$*" >&2; }
xcrun() {
  [[ "$1" = notarytool ]] || return 71
  local verb="$2"; shift 2
  if [[ "$verb" = submit ]]; then
    [[ "$1" = 'Synthetic app.zip' ]] || return 72
  else
    [[ "$verb" = log && "$1" = synthetic-submission ]] || return 73
  fi
  shift
  if [[ "$PROFILE" = 1 ]]; then
    [[ "$1" = --keychain-profile && "$2" = 'Synthetic profile' ]] || return 74
    shift 2
  else
    [[ "$1" = --apple-id && "$2" = fixture && "$3" = --password && "$4" = fixture && "$5" = --team-id && "$6" = fixture ]] || return 75
    shift 6
  fi
  if [[ "$verb" = submit ]]; then
    [[ "$#" = 5 && "$1" = --wait && "$2" = --timeout && "$3" = 1h && "$4" = --output-format && "$5" = json ]] || return 76
    printf 'submit-auth-ok\n' >&2
    printf '{"id":"synthetic-submission","status":"%s"}\n' "$RESULT"
  else
    [[ "$#" = 0 ]] || return 77
    printf 'log-auth-ok\n' >&2
    printf 'synthetic rejection log\n'
  fi
}
plutil() {
  local input; input="$(cat)"
  [[ "$input" = *synthetic-submission* ]] || return 1
  case "$2" in
    id) printf 'synthetic-submission\n' ;;
    status) printf '%s\n' "$RESULT" ;;
    *) return 1 ;;
  esac
}
"#;
    let decision = section("NOTARIZE=0\n", "\necho \"==> WordCraft");
    let function = section("notarize() {", "\n# ---- WordCraft.app");
    let tail = if call { "[[ $NOTARIZE = 1 ]]; notarize 'Synthetic app.zip'" } else { "[[ $NOTARIZE = 0 ]]" };
    let mut cmd = Command::new("/bin/bash");
    cmd.env_clear().env("IDENTITY", identity).env("PROFILE", if profile { "1" } else { "0" });
    cmd.env("RESULT", if accepted { "Accepted" } else { "Invalid" });
    if profile {
        cmd.env("APPLE_KEYCHAIN_PROFILE", "Synthetic profile");
    }
    if legacy {
        for key in ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"] {
            cmd.env(key, "fixture");
        }
    }
    cmd.args(["-euc", &format!("{setup}\n{decision}\n{function}\n{tail}")]).output().unwrap()
}

#[test]
fn keychain_profile_enables_notarization_without_other_apple_variables() {
    let out = run("Developer ID Application: Example", true, false, true, true);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("submit-auth-ok"));
}

#[test]
fn keychain_profile_takes_precedence_over_legacy_variables() {
    let out = run("Developer ID Application: Example", true, true, true, true);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("submit-auth-ok"));
}

#[test]
fn legacy_authentication_remains_supported() {
    let out = run("Developer ID Application: Example", false, true, true, true);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn rejected_submissions_fetch_logs_with_the_same_authentication() {
    for profile in [false, true] {
        let out = run("Developer ID Application: Example", profile, !profile, false, true);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("log-auth-ok"));
        assert!(String::from_utf8_lossy(&out.stdout).contains("synthetic rejection log"));
    }
}

#[test]
fn profile_does_not_enable_notarization_for_ad_hoc_signing() {
    assert!(run("-", true, false, true, false).status.success());
}

#[test]
fn incomplete_authentication_keeps_notarization_disabled() {
    assert!(run("Developer ID Application: Example", false, false, true, false).status.success());
}

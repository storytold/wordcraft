//! glibc malloc tuning for native binaries (adapted from Storytold's `photocraft` #8743).
//!
//! With glibc's defaults, transient large buffers (page render targets, autofit tables, decoded
//! images) raise glibc's dynamic `mmap` and `trim` thresholds (up to 32 MiB on 64-bit), so later
//! heap allocations stay in glibc arenas instead of returning freed pages to the OS. Fixing
//! `MALLOC_TRIM_THRESHOLD_` at 128 KiB (`131072`) disables dynamic threshold growth so freed
//! pages above 128 KiB are returned cleanly while preserving multi-arena allocation throughput.
//!
//! Because glibc reads its malloc environment variables once at process start and mutating the
//! process environment in Rust requires `unsafe` (forbidden workspace-wide) and happens after glibc
//! initialization, [`apply`] re-executes the current executable via `exec` when no malloc tuning
//! variable is already set (keeping PID, argv, `argv[0]`, and environment). Setting any `MALLOC_*`
//! variable or `glibc.malloc.*` tunable opts out.

use std::ffi::OsString;

/// The glibc environment variable [`apply`] sets.
pub const TRIM_THRESHOLD_VAR: &str = "MALLOC_TRIM_THRESHOLD_";

/// Fixed trim threshold (128 KiB, glibc's initial default) to disable dynamic threshold escalation.
pub const TRIM_THRESHOLD: &str = "131072";

/// glibc malloc variables that indicate callers or wrappers already configured malloc tuning.
const MALLOC_VARS: [&str; 5] = [TRIM_THRESHOLD_VAR, "MALLOC_MMAP_THRESHOLD_", "MALLOC_TOP_PAD_", "MALLOC_MMAP_MAX_", "MALLOC_ARENA_MAX"];

/// Whether startup should re-execute with [`TRIM_THRESHOLD_VAR`] set, given an environment reader:
/// true only when no malloc variable or `glibc.malloc.*` tunable is present.
pub fn needs_restart(var: impl Fn(&str) -> Option<OsString>) -> bool {
    if MALLOC_VARS.iter().any(|v| var(v).is_some()) {
        return false;
    }
    !var("GLIBC_TUNABLES").is_some_and(|t| t.to_string_lossy().contains("glibc.malloc."))
}

/// Re-execute this binary with [`TRIM_THRESHOLD_VAR`] set when [`needs_restart`] is true.
///
/// Must be called early in `main()` before worker threads start or logs/crash hooks initialize.
/// Returns immediately if tuning variables are already present or if `exec` fails.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn apply() {
    use std::io::Write;
    use std::os::unix::process::CommandExt;

    if !needs_restart(|k| std::env::var_os(k)) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut args = std::env::args_os();
    let mut cmd = std::process::Command::new(exe);
    if let Some(argv0) = args.next() {
        cmd.arg0(argv0);
    }
    // `exec` replaces the current process in place and only returns on error.
    let err = cmd.args(args).env(TRIM_THRESHOLD_VAR, TRIM_THRESHOLD).exec();
    let _ = writeln!(std::io::stderr(), "wordcraft: could not restart with {TRIM_THRESHOLD_VAR} set ({err})");
}

/// No-op on platforms without Linux glibc.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn apply() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn restarts_only_without_malloc_tuning() {
        assert!(needs_restart(env(&[])));
        assert!(needs_restart(env(&[("HOME", "/home/a"), ("GLIBC_TUNABLES", "glibc.cpu.hwcaps=-AVX2")])));
        assert!(!needs_restart(env(&[(TRIM_THRESHOLD_VAR, TRIM_THRESHOLD)])), "re-executed process stops");
        assert!(!needs_restart(env(&[(TRIM_THRESHOLD_VAR, "")])), "any explicit value opts out");
        assert!(!needs_restart(env(&[("MALLOC_MMAP_THRESHOLD_", "1048576")])));
        assert!(!needs_restart(env(&[("MALLOC_ARENA_MAX", "2")])));
        assert!(!needs_restart(env(&[("GLIBC_TUNABLES", "glibc.malloc.trim_threshold=0")])));
    }
}

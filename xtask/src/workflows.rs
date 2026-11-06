//! `cargo xtask workflows`: every external GitHub Actions `uses:` ref must be immutable.
//!
//! GitHub resolves a `uses:` ref when a run starts. A branch or tag can move, so a workflow
//! that calls one runs code that is not in this repository's history. The one deliberate
//! exception is `storytold/craft-common-actions` at `@main`: the org tracks it on purpose so
//! CI fixes reach every craft app without a PR in each repository
//! (storytold/craft-common-actions#6). The allowlist below makes that exception explicit;
//! anything else must name a full 40-character commit SHA. See WordCraft #1358.

use std::fs;
use std::path::Path;

const WORKFLOW_DIR: &str = ".github/workflows";

/// Mutable refs that are deliberate. Keep this list short and give every entry a reason.
const ALLOWED: &[Allowed] = &[Allowed {
    repo: "storytold/craft-common-actions",
    reference: "main",
    reason: "shared craft CI, intentionally tracked at @main (craft-common-actions#6; WordCraft #1358)",
}];

struct Allowed {
    repo: &'static str,
    reference: &'static str,
    reason: &'static str,
}

impl Allowed {
    fn matches(&self, target: &str, reference: &str) -> bool {
        self.reference == reference && repo_of(target) == Some(self.repo)
    }
}

pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join(WORKFLOW_DIR);
    let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e == "yml" || e == "yaml") {
            files.push(path);
        }
    }
    files.sort();

    let mut checked = 0usize;
    let mut exceptions = Vec::new();
    let mut violations = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.strip_prefix(root).unwrap_or(path).display();
        for (index, line) in text.lines().enumerate() {
            let Some(uses) = uses_value(line) else { continue };
            checked += 1;
            if uses.starts_with("./") {
                continue;
            }
            let Some((target, reference)) = uses.rsplit_once('@') else {
                violations.push(format!("{name}:{}: `uses: {uses}` names no `@ref`", index + 1));
                continue;
            };
            if is_immutable_reference(reference) {
                continue;
            }
            if let Some(allowed) = ALLOWED.iter().find(|a| a.matches(target, reference)) {
                exceptions.push(format!("{name}:{}: {target}@{reference} ({})", index + 1, allowed.reason));
                continue;
            }
            violations.push(format!(
                "{name}:{}: `uses: {uses}` is mutable; pin it to a full 40-character commit SHA (or an immutable Docker digest), or add an allowlist entry with a reason",
                index + 1
            ));
        }
    }

    println!("workflows: checked {checked} `uses:` ref(s) in {} file(s)", files.len());
    if !exceptions.is_empty() {
        println!("workflows: {} allowlisted mutable ref(s):", exceptions.len());
        for exception in &exceptions {
            println!("  - {exception}");
        }
    }
    if violations.is_empty() {
        println!("OK: no unapproved mutable refs.");
        Ok(())
    } else {
        Err(format!("{} mutable `uses:` ref(s):\n  {}", violations.len(), violations.join("\n  ")))
    }
}

/// `owner/repo` from `owner/repo/path`.
fn repo_of(target: &str) -> Option<&str> {
    let mut parts = target.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(&target[..owner.len() + 1 + repo.len()])
}

/// The value of a `uses:` line, without a YAML list marker, quotes or a trailing comment.
fn uses_value(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest.strip_prefix('-').map_or(rest, str::trim_start);
    let rest = rest.strip_prefix("uses:")?.trim();
    if rest.starts_with('#') {
        return None;
    }
    let token = rest.split_whitespace().next()?;
    let value = token.trim_matches(|c| c == '"' || c == '\'');
    if value.is_empty() { None } else { Some(value) }
}

fn is_commit_sha(reference: &str) -> bool {
    reference.len() == 40 && reference.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A full commit SHA or a Docker `sha256:` digest. Tags and branches are mutable.
fn is_immutable_reference(reference: &str) -> bool {
    if is_commit_sha(reference) {
        return true;
    }
    reference.strip_prefix("sha256:").is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn accepts_full_sha_refs() {
        let uses = format!("actions/checkout@{SHA}");
        assert_eq!(uses_value(&format!("    uses: {uses}")), Some(uses.as_str()));
        assert!(is_commit_sha(SHA));
        assert!(!is_commit_sha("v4"));
        assert!(!is_commit_sha("main"));
    }

    #[test]
    fn accepts_immutable_docker_digests() {
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(is_immutable_reference(&digest));
        assert!(!is_immutable_reference("sha256:latest"));
        assert!(!is_immutable_reference("sha256:abc"));
    }

    #[test]
    fn finds_the_owner_and_repo() {
        assert_eq!(repo_of("storytold/craft-common-actions/actions/foo"), Some("storytold/craft-common-actions"));
        assert_eq!(repo_of("actions/checkout"), Some("actions/checkout"));
        assert_eq!(repo_of("actions"), None);
        assert_eq!(repo_of("/actions"), None);
        assert_eq!(repo_of("actions/"), None);
    }

    #[test]
    fn allows_only_the_documented_exception() {
        let shared = "storytold/craft-common-actions/.github/workflows/release.yml@main";
        let (target, reference) = shared.rsplit_once('@').unwrap_or(("", ""));
        assert!(ALLOWED.iter().any(|a| a.matches(target, reference)));

        let other = "someone/else/.github/workflows/release.yml@main";
        let (target, reference) = other.rsplit_once('@').unwrap_or(("", ""));
        assert!(!ALLOWED.iter().any(|a| a.matches(target, reference)));
    }

    #[test]
    fn reads_quoted_values_and_comments() {
        assert_eq!(uses_value("    uses: \"actions/checkout@v4\" # v4"), Some("actions/checkout@v4"));
        assert_eq!(uses_value("    uses: 'actions/checkout@v4'"), Some("actions/checkout@v4"));
        assert_eq!(uses_value("    uses: actions/checkout@v4 # latest"), Some("actions/checkout@v4"));
        assert_eq!(uses_value("    uses: # none"), None);
        assert_eq!(uses_value("    # uses: actions/checkout@v4"), None);
    }

    #[test]
    fn reads_yaml_list_items() {
        assert_eq!(uses_value("      - uses: actions/checkout@v4"), Some("actions/checkout@v4"));
        assert_eq!(uses_value("      -   uses: actions/checkout@v4"), Some("actions/checkout@v4"));
        assert_eq!(uses_value("      - name: checkout"), None);
    }

    #[test]
    fn local_actions_need_no_ref() {
        assert_eq!(uses_value("    uses: ./actions/foo"), Some("./actions/foo"));
    }
}

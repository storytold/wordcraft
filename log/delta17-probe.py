#!/usr/bin/env python3
"""Round-17 independent probes: mutate the copied audited sources and run the real gate."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import sys

ROOT = Path('/tmp/opencode/delta17-review')
GEN = ROOT / 'crates/validate/observations/generate.py'
SRC = ROOT / 'crates/validate/src'
originals = {p.name: p.read_text() for p in SRC.glob('*.rs')}

PROBES = [
    ('history/closure-parameter-destructure', 'lib.rs', '''
fn unregistered_read(pkg: &Package) -> usize {
    (|Package { entries: items, .. }: &Package| items.len())(pkg)
}
'''),
    ('history/destructuring-assignment', 'lib.rs', '''
fn unregistered_read(pkg: Package) -> usize {
    let items;
    Package { entries: items, .. } = pkg;
    items.len()
}
'''),
    ('history/parenthesized-destructuring-assignment', 'lib.rs', '''
fn unregistered_read(pkg: Package) -> usize {
    let items;
    (Package { entries: items, .. }) = pkg;
    items.len()
}
'''),
    ('history/macro-produced-pattern', 'lib.rs', '''
macro_rules! package_pattern { ($items:ident) => { Package { entries: $items, .. } }; }
fn unregistered_read(pkg: &Package) -> usize {
    (|package_pattern!(items): &Package| items.len())(pkg)
}
'''),
    ('history/raw-field', 'lib.rs', '''
fn unregistered_read(pkg: &Package) -> usize { pkg.r#entries.len() }
'''),
    ('history/raw-registry-method', 'ownership.rs', None),
    ('history/module-callback', 'lib.rs', '''
const UNREGISTERED_READ: fn(&Package) -> usize = |pkg| pkg.entries.len();
'''),
    ('seam/type-alias-carrying-callback', 'lib.rs', '''
type PackageRead = fn(&Package) -> usize;
const UNREGISTERED_READ: PackageRead = |pkg| pkg.entries.len();
'''),
    ('seam/trait-default-body', 'lib.rs', '''
trait HiddenRead {
    fn read(pkg: &Package) -> usize { pkg.entries.len() }
}
'''),
    ('seam/trait-impl-body', 'lib.rs', '''
trait HiddenRead { fn read(pkg: &Package) -> usize; }
impl HiddenRead for Package {
    fn read(pkg: &Package) -> usize { pkg.entries.len() }
}
'''),
    ('seam/associated-const-callback', 'lib.rs', '''
impl Package {
    const UNREGISTERED_READ: fn(&Package) -> usize = |pkg| pkg.entries.len();
}
'''),
    ('seam/include-item', 'lib.rs', '''
include!("/tmp/opencode/delta17-review/hidden-consumer.rs");
'''),
    ('seam/cfg-gated-consumer', 'lib.rs', '''
#[cfg(all())]
fn unregistered_read(pkg: &Package) -> usize { pkg.entries.len() }
'''),
    ('seam/other-macro-path', 'lib.rs', '''
fn unregistered_read(pkg: &Package) -> usize {
    std::format!("{}", pkg.entries.len());
    pkg.entries.len()
}
'''),
    ('seam/supported-macro-arguments-audited', 'lib.rs', '''
fn unregistered_read(pkg: &Package) -> usize {
    format!("{}", pkg.entries.len());
    0
}
'''),
    ('escape/raw-iterator-absence', 'package.rs', '''
fn unregistered_absence(entries: &[Entry], name: &str) -> bool {
    !entries.iter().any(|entry| entry.name == name)
}
'''),
    ('escape/existing-canonical-iterator-absence', 'package.rs', None),
    ('escape/remove-entry-truncation-absence-guard', 'lib.rs', None),
    ('escape/helper-mediated-package-absence', 'lib.rs', '''
fn unregistered_absence(pkg: &Package, name: &str) -> bool {
    entry_by_name(pkg, name).is_none()
}
'''),
    ('escape/aliased-ufcs-observed-absence', 'lib.rs', '''
type HiddenDefinitions = Observed<HashSet<String>>;
fn unregistered_absence(value: &HiddenDefinitions, key: &String) -> bool {
    !HiddenDefinitions::prefix(value).contains(key)
}
'''),
    ('escape/self-ufcs-observed-absence', 'lib.rs', '''
impl Observed<HashSet<String>> {
    fn unregistered_absence(&self, key: &String) -> bool {
        !Self::prefix(self).contains(key)
    }
}
'''),
    ('escape/serde-content-types-consumer', 'package.rs', None),
    ('false-rejection/ordinary-named-field-binding', 'lib.rs', '''
fn configured_entry_limit(limits: &Limits) -> usize {
    let Limits { entries, .. } = limits;
    *entries
}
'''),
    ('false-rejection/qualified-format', 'lib.rs', '''
fn display_rule(rule: &str) -> String { std::format!("rule: {rule}") }
'''),
    ('false-rejection/inline-attribute', 'lib.rs', '''
#[inline]
fn configured_entry_limit(limits: &Limits) -> usize { limits.entries }
'''),
    ('false-rejection/trait-default-nonconsumer', 'lib.rs', '''
trait EntryLimit {
    fn configured_entry_limit(&self) -> usize { 20_000 }
}
'''),
]


def mutate(label, name, addition):
    text = originals[name]
    if label == 'history/raw-registry-method':
        return text.replace('impl PackageRegistryContext {', 'impl PackageRegistryContext { fn unregistered_registry(&self) -> usize { self.r#contexts("x").len() }', 1)
    if label == 'escape/remove-entry-truncation-absence-guard':
        old = 'if !pkg.entries_truncated && package::canonical_entry(pkg, target).is_none() {'
        new = 'if package::canonical_entry(pkg, target).is_none() {'
        changed = text.replace(old, new, 1)
        if changed == text:
            raise RuntimeError('truncation-guard probe did not mutate source')
        return changed
    if label == 'escape/existing-canonical-iterator-absence':
        old = '''    let n = name.trim_start_matches('/');
    entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)).map(|e| e.name.clone())
'''
        new = '''    let n = name.trim_start_matches('/');
    if !entries.iter().any(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)) {
        return None;
    }
    entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)).map(|e| e.name.clone())
'''
        changed = text.replace(old, new, 1)
        if changed == text:
            raise RuntimeError('canonical probe did not mutate source')
        return changed
    if label == 'escape/serde-content-types-consumer':
        text = text.replace('use std::collections::{BTreeSet, HashMap, HashSet};', 'use std::collections::{BTreeSet, HashMap, HashSet};\nuse serde::Serialize;', 1)
        text = text.replace('#[derive(Default)]\npub(crate) struct ContentTypes {', '#[derive(Default, Serialize)]\npub(crate) struct ContentTypes {', 1)
        return text + '''
fn unregistered_registry_serde<S: serde::Serializer>(
    registry: &ContentTypes,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    registry.serialize(serializer)
}
'''
    return text + addition


def main():
    selected = set(sys.argv[1:])
    results = []
    for label, name, addition in PROBES:
        if selected and label not in selected:
            continue
        path = SRC / name
        path.write_text(mutate(label, name, addition))
        try:
            run = subprocess.run(
                ['python3', str(GEN), '--check'], cwd=ROOT,
                env={**os.environ, 'PATH': str(Path.home() / '.cargo/bin') + os.pathsep + os.environ['PATH']},
                text=True, capture_output=True, timeout=120,
            )
        finally:
            path.write_text(originals[name])
        output = (run.stdout + run.stderr).strip()
        result = {
            'label': label,
            'file': name,
            'construct': mutate(label, name, addition)[len(originals[name]):] if addition is not None else mutate(label, name, addition),
            'exit': run.returncode,
            'output': output,
            'register_sha256': hashlib.sha256((ROOT / 'crates/validate/observation-contracts.md').read_bytes()).hexdigest(),
        }
        results.append(result)
        print(f'===== {label} =====\nexit={run.returncode}\n{output}\n')
    (ROOT / 'probe-results.json').write_text(json.dumps(results, indent=2) + '\n')

if __name__ == '__main__':
    main()

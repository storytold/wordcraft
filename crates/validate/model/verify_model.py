#!/usr/bin/env python3
"""Verify the checked-in content-model tables have not drifted.

Two checks, both stdlib-only and offline:

1. Every file listed in README.md's SHA-256 table (the extractor, the
   generator, the four JSON tables, and ``../src/model.rs``) still hashes
   to the recorded value. Any hand edit to a table, script, or generated
   file breaks its hash.
2. Each JSON table is internally consistent: the element/type counts in
   its ``meta.counts`` equal the actual table sizes, and its meta records
   the v2 namespace-qualified keying.

The schema -> JSON direction cannot be re-run without the ECMA-376
archives; README.md records their URLs and hashes, and the regeneration
commands there reproduce these files byte-identically (the output does
not depend on the schema directory's path or name).

Usage: python3 verify_model.py   (run from this directory)
Exit code 0 means every check passed; 1 means drift was found.
"""

import hashlib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
README = os.path.join(HERE, "README.md")
TABLES = [
    "content_model_v2.json",
    "content_model_v2_transitional.json",
    "content_model_dml_v2.json",
    "content_model_dml_v2_transitional.json",
]


def sha256_of(path):
    """Return the SHA-256 hex digest of a file's bytes."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def readme_hashes():
    """Parse README.md's SHA-256 table into a {file name: digest} dict."""
    out = {}
    row = re.compile(r"^\| `([^`]+)` \| `([0-9a-f]{64})` \|$")
    with open(README, encoding="utf-8") as f:
        for line in f:
            m = row.match(line.strip())
            if m:
                out[m.group(1)] = m.group(2)
    return out


def resolve(name):
    """Resolve a README-listed file name against this directory."""
    if name.startswith("../"):
        return os.path.normpath(os.path.join(HERE, name))
    return os.path.join(HERE, name)


def check_hashes(failures):
    """Compare every README-listed file against its recorded SHA-256."""
    listed = readme_hashes()
    if not listed:
        failures.append("README.md: no SHA-256 table rows parsed")
        return
    for name, want in sorted(listed.items()):
        path = resolve(name)
        if not os.path.exists(path):
            failures.append(f"{name}: listed in README.md but missing")
            continue
        got = sha256_of(path)
        if got != want:
            failures.append(f"{name}: sha256 {got} != README {want}")


def check_table(name, failures):
    """Check one JSON table's meta counts against its actual content."""
    path = os.path.join(HERE, name)
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    meta = data.get("meta", {})
    if meta.get("type_keying") != "namespace-qualified (v2)":
        failures.append(f"{name}: meta.type_keying is not the v2 qualified keying")
    counts = meta.get("counts", {})
    if counts.get("element_names") != len(data.get("elements", {})):
        failures.append(f"{name}: meta counts {counts.get('element_names')} elements, "
                        f"table holds {len(data.get('elements', {}))}")
    if counts.get("complex_types") != len(data.get("types", {})):
        failures.append(f"{name}: meta counts {counts.get('complex_types')} types, "
                        f"table holds {len(data.get('types', {}))}")


def main():
    """Run all drift checks; print a verdict and return the exit code."""
    failures = []
    check_hashes(failures)
    for name in TABLES:
        check_table(name, failures)
    if failures:
        for f in failures:
            print(f"DRIFT: {f}")
        print(f"verify_model: FAIL ({len(failures)} problem(s))")
        return 1
    print("verify_model: OK (hashes match README.md; tables internally consistent)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Prepare complete, revision-specific offline inputs for flatpak-cargo-generator.

flatpak-cargo-generator reads [[package]] but omits [[patch.unused]]. Cargo still
loads those patch manifests before resolving the locked dependency graph. Add
them to the generator's in-memory input without changing the project's lockfile.
The upstream generator also keys git replacements by URL alone. Keep each locked
revision separate when several dependencies use different commits of one repo.
The upstream generator supplies the pinned sources and normalizes git packages.
"""
import argparse
import asyncio
import json
from pathlib import Path
import runpy
from urllib.parse import parse_qs, urlparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cargo_lock", type=Path)
    parser.add_argument("--generator", type=Path, default=Path("flatpak-cargo-generator.py"))
    parser.add_argument("-o", "--output", type=Path, required=True)
    args = parser.parse_args()
    generator = runpy.run_path(str(args.generator), run_name="flatpak_cargo_generator")
    lock = generator["load_toml"](str(args.cargo_lock))
    packages = lock["package"]
    keys = {(p["name"], p["version"], p.get("source")) for p in packages}
    for patch in lock.get("patch", {}).get("unused", []):
        key = (patch["name"], patch["version"], patch.get("source"))
        if key not in keys:
            packages.append(patch)
            keys.add(key)
    sources = asyncio.run(generator["generate_sources"](lock))
    config_source = next(
        s for s in sources
        if s.get("dest") == generator["CARGO_HOME"] and s.get("dest-filename") == "config"
    )
    config = generator["tomlkit"].parse(config_source["contents"]).unwrap()
    replacements = config["source"]
    for name in list(replacements):
        if "git" in replacements[name]:
            del replacements[name]
    for package in packages:
        source = package.get("source", "")
        if not source.startswith("git+"):
            continue
        replacement = {
            "git": generator["canonical_url"](source).geturl(),
            "replace-with": generator["VENDORED_SOURCES"],
        }
        query = parse_qs(urlparse(source).query)
        for reference in ("rev", "tag", "branch"):
            if reference in query:
                value, = query[reference]
                replacement[reference] = value
        replacements[source] = replacement
    config_source["contents"] = generator["tomlkit"].dumps(config)
    args.output.write_text(json.dumps(sources, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()

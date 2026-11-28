#!/usr/bin/env python3
"""Regress model verification under Git autocrlf checkout and Windows text output.

Run from any directory with ``python3 crates/validate/model/test_model_checkout.py``.
Temporary copies stay under the worktree's ignored log directory; no Windows host is required.
"""

import contextlib
import importlib.util
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def load_module(name):
    """Load a model tool without executing its command-line entry point."""
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CheckoutTests(unittest.TestCase):
    """Keep checkout transformations distinct from model-content changes."""

    def test_autocrlf_checkout_verifies(self):
        """Simulate checkout EOL bytes using Git's attributes, then run the real verifier."""
        verifier = load_module("verify_model")
        paths = [HERE / name for name in verifier.readme_hashes()]
        paths += [HERE / "README.md", HERE / "verify_model.py"]
        (ROOT / "log").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "log", prefix="fredapps-checkout-") as tmp:
            checkout = pathlib.Path(tmp)
            git_env = {
                k: v for k, v in os.environ.items()
                if k not in {"GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"}
            }
            for path in paths:
                path = path.resolve()
                rel = path.relative_to(ROOT)
                attr = subprocess.check_output(
                    ["git", "check-attr", "eol", "--", str(rel)], cwd=ROOT, env=git_env, text=True
                ).strip().rsplit(": ", 1)[-1]
                data = path.read_bytes().replace(b"\r\n", b"\n")
                if attr != "lf":
                    data = data.replace(b"\n", b"\r\n")
                target = checkout / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            result = subprocess.run(
                [sys.executable, str(checkout / "crates/validate/model/verify_model.py")],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_lf_entry_point_preserves_table_bytes(self):
        """The LF writer serializes all four existing models without content or byte drift."""
        extractor = load_module("extract_model_lf")
        verifier = load_module("verify_model")
        (ROOT / "log").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "log", prefix="fredapps-output-") as tmp:
            for name in verifier.TABLES:
                with self.subTest(table=name):
                    source = HERE / name
                    output = pathlib.Path(tmp) / name
                    profile = "dml" if "dml" in name else "wml"
                    with mock.patch.object(extractor, "build", return_value=json.loads(source.read_text())), \
                         mock.patch.object(extractor.sys, "argv", ["extract_model_lf.py", "unused", str(output), "--profile", profile]), \
                         contextlib.redirect_stdout(io.StringIO()):
                        extractor.main()
                    self.assertEqual(output.read_bytes(), source.read_bytes())

    def test_extractor_specifies_lf(self):
        """Capture the extractor's open call: platform-default newlines are not deterministic."""
        extractor = load_module("extract_model_lf")
        calls = []

        def output_open(*args, **kwargs):
            """Capture text-output options without depending on schemas or the host OS."""
            calls.append(kwargs)
            return io.StringIO()

        with mock.patch.object(extractor, "build", return_value={"meta": {"counts": {}, "unresolved": []}}), \
             mock.patch.object(extractor, "open", side_effect=output_open, create=True), \
             mock.patch.object(extractor.sys, "argv", ["extract_model_lf.py", "unused", "unused.json"]), \
             contextlib.redirect_stdout(io.StringIO()):
            extractor.main()
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0].get("newline"), "\n", "extractor must explicitly request LF")


if __name__ == "__main__":
    unittest.main(verbosity=2)

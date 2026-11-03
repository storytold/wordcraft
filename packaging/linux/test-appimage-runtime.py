"""AppImage keyboard libraries and launcher contract, without packaging or launching a release."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
LIBRARIES = {"libxkbcommon-x11.so.0": "libxkbcommon-x11-0", "libxcb-xkb.so.1": "libxcb-xkb1"}


class AppImageRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="wordcraft-appimage-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sysroot = self.root / "target sysroot"
        self.appdir = self.root / "Word Craft.AppDir"
        self.tools = self.root / "tools"
        self.tools.mkdir()
        compiler = self.tools / "gcc"
        compiler.write_text(
            '#!/usr/bin/env bash\nset -eu\ncase "$1" in\n'
            '  -dumpmachine) printf "%s\\n" "$TEST_MACHINE" ;;\n'
            '  -print-sysroot) printf "%s\\n" "$TEST_SYSROOT" ;;\n'
            '  -print-file-name=*) printf "%s/usr/lib/%s\\n" "$TEST_SYSROOT" "${1#*=}" ;;\n'
            'esac\n'
        )
        compiler.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"], TEST_SYSROOT=str(self.sysroot), TEST_MACHINE="x86_64-linux-gnu")
        self.env.pop("CROSS_COMPILE", None)
        for library, package in LIBRARIES.items():
            lib = self.sysroot / "usr/lib" / library
            lib.parent.mkdir(parents=True, exist_ok=True)
            lib.write_bytes(library.encode())
            notice = self.sysroot / "usr/share/doc" / package / "copyright"
            notice.parent.mkdir(parents=True)
            notice.write_text("Original test notice for " + package)

    def bundle(self, arch="x86_64"):
        return subprocess.run(
            ["bash", "-eu", "-c", 'HERE="$1"; ARCH="$2"; . "$HERE/appimage-runtime.sh"; bundle_x11_keyboard "$3"', "test", str(HERE), arch, str(self.appdir)],
            env=self.env, capture_output=True, text=True, check=False,
        )

    def test_target_libraries_and_notices_are_copied(self):
        for arch, machine in [("x86_64", "x86_64-linux-gnu"), ("aarch64", "aarch64-linux-gnu")]:
            with self.subTest(arch=arch):
                self.env["TEST_MACHINE"] = machine
                result = self.bundle(arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                for library, package in LIBRARIES.items():
                    self.assertEqual((self.appdir / "usr/lib" / library).read_bytes(), library.encode())
                    self.assertEqual((self.appdir / "usr/share/doc/wordcraft" / (package + "-copyright")).read_text(), "Original test notice for " + package)
                self.assertTrue(os.access(self.appdir / "AppRun", os.X_OK))

    def test_missing_library_notice_or_wrong_architecture_refuses_packaging(self):
        for missing in ["usr/lib/libxkbcommon-x11.so.0", "usr/share/doc/libxcb-xkb1/copyright"]:
            path = self.sysroot / missing
            original = path.read_bytes()
            path.unlink()
            result = self.bundle()
            self.assertNotEqual(result.returncode, 0, missing)
            self.assertIn("error:", result.stderr)
            path.write_bytes(original)
        self.assertNotEqual(self.bundle("aarch64").returncode, 0)

    def test_launcher_keeps_arguments_and_existing_library_path(self):
        result = self.bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = self.appdir / "usr/bin/wordcraft"
        binary.parent.mkdir(parents=True)
        binary.write_text('#!/usr/bin/env python3\nimport json, os, sys\nprint(json.dumps([sys.argv[1:], os.environ["LD_LIBRARY_PATH"]]))\n')
        binary.chmod(0o755)
        args = ["a document.docx", "--control", "7981", "Unicode é"]
        for prior in [None, "", "/existing/library path"]:
            env = dict(os.environ)
            env.pop("LD_LIBRARY_PATH", None)
            if prior is not None:
                env["LD_LIBRARY_PATH"] = prior
            run = subprocess.run([str(self.appdir / "AppRun"), *args], cwd=self.root, env=env, capture_output=True, text=True, check=True)
            expected = str(self.appdir / "usr/lib") + (os.pathsep + prior if prior else "")
            self.assertEqual(json.loads(run.stdout), [args, expected])


if __name__ == "__main__":
    unittest.main()

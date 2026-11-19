"""AppImage keyboard libraries and launcher contract, without packaging or launching a release."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
LIBRARIES = {
    "libxkbcommon.so.0": "libxkbcommon0",
    "libxkbcommon-x11.so.0": "libxkbcommon-x11-0",
    "libxcb-xkb.so.1": "libxcb-xkb1",
}


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

    def compile_probe(self):
        binary = self.appdir / "usr/bin/wordcraft"
        binary.parent.mkdir(parents=True)
        subprocess.run(
            ["gcc", "-x", "c", "-", "-o", str(binary), "-ldl", "-Wl,--disable-new-dtags,-rpath,$ORIGIN/wordcraft-x11-libraries"], input=r"""
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--appimage-needs-x11-fallback") == 0) {
        return getenv("TEST_NO_SYSTEM_KEYBOARD") == NULL;
    }
    char executable[4096];
    ssize_t length = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
    if (length < 0) return 2;
    executable[length] = 0;
    printf("%s\n%s\n%s\n", executable, getenv("LD_LIBRARY_PATH") ?: "(unset)",
           dlopen("libwordcraft-keyboard-probe.so", RTLD_NOW) ? "bundled" : "system");
    for (int arg = 1; arg < argc; arg++) puts(argv[arg]);
    return 0;
}
""", check=True, text=True,
        )
        return binary

    def bundle(self, arch="x86_64"):
        return subprocess.run(
            ["bash", "-eu", "-c", 'HERE="$1"; ARCH="$2"; . "$HERE/appimage-runtime.sh"; bundle_x11_keyboard "$3"', "test", str(HERE), arch, str(self.appdir)],
            env=self.env, capture_output=True, text=True, check=False,
        )

    def test_target_libraries_and_notices_are_copied(self):
        for arch, machine in [("x86_64", "x86_64-linux-gnu"), ("aarch64", "aarch64-linux-gnu")]:
            with self.subTest(arch=arch):
                self.env["TEST_MACHINE"] = machine
                self.appdir = self.root / f"AppDir {arch}"
                binary = self.compile_probe()
                result = self.bundle(arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                for library, package in LIBRARIES.items():
                    self.assertEqual((self.appdir / "usr/lib/wordcraft-x11/wordcraft-x11-libraries" / library).read_bytes(), library.encode())
                    self.assertEqual((self.appdir / "usr/share/doc/wordcraft" / (package + "-copyright")).read_text(), "Original test notice for " + package)
                self.assertTrue(os.access(self.appdir / "AppRun", os.X_OK))
                self.assertTrue(binary.samefile(self.appdir / "usr/lib/wordcraft-x11/wordcraft"))

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

    def test_launcher_uses_fallback_without_changing_the_library_environment(self):
        binary = self.compile_probe()
        result = self.bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        fallback = self.appdir / "usr/lib/wordcraft-x11"
        subprocess.run(
            ["gcc", "-shared", "-fPIC", "-x", "c", "-", "-o", str(fallback / "wordcraft-x11-libraries/libwordcraft-keyboard-probe.so")],
            input="int fixture(void) { return 0; }", check=True, text=True,
        )
        args = ["a document.docx", "--control", "7981", "Unicode é"]
        for display, missing, prior in [
            (None, True, None), (":123", False, ""), (":123", True, "/existing/library path"),
        ]:
            with self.subTest(display=display, missing=missing, prior=prior):
                env = dict(os.environ)
                for key, value in [("DISPLAY", display), ("TEST_NO_SYSTEM_KEYBOARD", "1" if missing else None), ("LD_LIBRARY_PATH", prior)]:
                    if value is None:
                        env.pop(key, None)
                    else:
                        env[key] = value
                run = subprocess.run([str(self.appdir / "AppRun"), *args], cwd=self.root, env=env, capture_output=True, text=True, check=True)
                selected = bool(display and missing)
                executable = fallback / "wordcraft" if selected else binary
                self.assertEqual(run.stdout.splitlines(), [str(executable), "(unset)" if prior is None else prior,
                                                           "bundled" if selected else "system", *args])


if __name__ == "__main__":
    unittest.main()

# Windows 7 x64 compatibility build (experimental, unsupported)

> **Status:** experimental, not runtime-verified · **Last updated:** 2026-10-27 · **Change:** new (ported from VectorCraft's Windows 7 build)

Windows 7 is not a supported platform. This build is experimental and community-maintained,
and the WordCraft maintainers have not verified it on Windows 7. It follows VectorCraft's Windows 7
build (storytold/vectorcraft `docs/windows7.md`), with the same toolchain, patch and checks.

This separate build targets **64-bit Windows 7 SP1**. Compared with the regular Windows build:

- It draws with **OpenGL** instead of wgpu (Direct3D 12), so `WGPU_BACKEND` and the Direct3D 12
  start-up probe don't apply. A vendor graphics driver providing **OpenGL 3.3 or newer** is
  required; Windows' built-in generic display driver is insufficient.
- The native **AccessKit** accessibility adapter is left out: screen readers don't see the UI.
- **Printing** lists printers through PowerShell's WMI (`Win32_Printer`, PowerShell 2.0 is part
  of Windows 7 SP1) and hands the print-ready PDF to the installed PDF viewer's Print / PrintTo
  verb. The regular build's craft-print draws jobs with `Windows.Data.Pdf`, a Windows Runtime API
  Windows 7 doesn't have, so craft-print isn't in this build. The viewer prints with its own
  settings: copies, two-sided and grayscale chosen in WordCraft don't reach it (the status bar
  says so). A PDF viewer that can print must be installed.
- Windows' **Animation effects** setting and **home region** aren't read (both are Windows Runtime
  APIs): File › Options › Accessibility › Animation effects decides, and new documents follow
  the display language's region (or `WORDCRAFT_REGION`).
- **Make WordCraft the default** opens Control Panel › Default Programs, since Windows 7 has no
  Settings app. The portable build registers no file types, so WordCraft is listed there only
  after it has been chosen once through Open With.

The `windows7` feature and the win7 Rust target must be selected together; compile-time guards
in `apps/wordcraft/src/main.rs` reject mixing this feature with the ordinary Windows target, and
with the `wgpu` and `accessibility` features. The normal desktop and web builds keep their
existing defaults.

## Build or download

In your GitHub fork, open **Actions → Windows 7 x64 compatibility → Run workflow** (it also runs
weekly and on pull requests changing the compatibility path). When it succeeds, download the
`wordcraft-windows7-x64-portable` artifact, extract the contained portable ZIP, then run
`wordcraft.exe` on Windows 7. The CLI is included. No MSI is provided: the existing installers
remain for modern Windows.

To build locally, use a **Windows 10/11 build computer**, with Rust/rustup and Visual Studio 2022
C++ build tools (x64 compiler and Windows SDK). Open an **x64 Native Tools Command Prompt for
VS 2022**, change to your clone's repository root (the directory containing `Cargo.toml`), and run:

```powershell
powershell -ExecutionPolicy Bypass -File packaging/windows/windows7.ps1
```

Add `-Test` to also run the app and CLI tests on the build computer.

Output: `dist/release/wordcraft-windows7-x64-portable.zip`. The script installs the pinned nightly
compiler and `rust-src`, rebuilds the standard library for `x86_64-win7-windows-msvc`, and links
the C runtime statically. It never installs Rust on the Windows 7 machine. The optional
`CRAFT_FONTS_DIR` setting works as in the normal build. Without it, install fonts for the
languages you need on the destination machine.

The Windows 7 target matters: modern Rust's ordinary `x86_64-pc-windows-msvc` target requires
Windows 10, including standard-library and random-number APIs. Selecting OpenGL alone or changing
an installer version check cannot fix that.

References: [Rust's Windows 7 target](https://doc.rust-lang.org/rustc/platform-support/win7-windows-msvc.html)
and [Windows baseline change](https://blog.rust-lang.org/2024/02/26/Windows-7/).

The packaging script checks the dependency tree (no wgpu, AccessKit adapter or craft-print), x64
PE headers, the subsystem version, and known incompatible imports (including
`GetSystemTimePreciseAsFileTime`, `combase.dll` and the Windows Runtime's
`api-ms-win-core-winrt-*`) before creating the ZIP. This is a regression check, not an exhaustive
audit of every Windows API.

The small vendored `windows-link` patch routes `CoTaskMemFree` to its documented `ole32.dll`
export on the Windows 7 target; the newer binding otherwise imports `combase.dll`, which is absent
on Windows 7. Only this script uses it: it passes
`--config "patch.crates-io.windows-link.path='vendor/windows-link'"` to Cargo, so every other
build, including the normal Windows one, keeps the checksum-verified crates.io crate. Cargo
records the patch in `Cargo.lock`, so the script resolves once, fails if anything other than
windows-link's source changed, builds with `--locked`, and then restores `Cargo.lock`.

## Runtime validation

None yet. A successful build on the Windows 2022 CI runner does **not** prove Windows 7 runtime
compatibility.

## Runtime acceptance checklist

Before declaring support, test on a Windows 7 SP1 x64 machine and record its GPU, driver version,
Windows updates and any missing-DLL/entry-point error:

- Start `wordcraft.exe`; verify a visible window and page, resizing, minimize/restore and exit.
- Type, format and spell-check text; save a `.docx`, close and reopen it.
- Start with `--sample`; export it to PDF and inspect the result.
- Exercise native Open/Save dialogs, clipboard text/images/rich text and drag-and-drop.
- Print to a printer (or Microsoft XPS Document Writer) with a PDF viewer installed.
- Change a preference in File › Options, exit and verify it survives restarting.
- Run `wordcraft-cli.exe --version` and `wordcraft-cli.exe convert` on a document from a Command
  Prompt in the extracted application folder.

This build is experimental until that checklist passes. New dependency versions can add newer
Windows imports, so retain Cargo.lock and repeat runtime QA after dependency updates.

## Revision history

- 2026-10-27: First version, ported from VectorCraft's Windows 7 build.

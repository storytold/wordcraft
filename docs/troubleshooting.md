# Troubleshooting WordCraft

- [Windows graphics problems](#windows-graphics-problems)
- [Wayland (Linux)](#wayland-linux)
- [Read Aloud (Linux)](#read-aloud-linux)
- [Logs](#logs)

For installation help, see [install.md](install.md). If none of this fixes your problem, open an
[issue](https://github.com/storytold/wordcraft/issues) and attach your log file (see [Logs](#logs)).

## Windows graphics problems

### The app doesn't open

The desktop app initializes only DirectX 12 by default.
Letting wgpu also create an OpenGL instance can crash some graphics drivers (AMD's
`atio6axx.dll`) before the window appears, so the app would flash in Task Manager and quit.
`WGPU_BACKEND` overrides the default for troubleshooting (for example `dx12` or `vulkan`). In
PowerShell, from the folder containing the executable:

```powershell
$env:WGPU_BACKEND = "vulkan"
& .\wordcraft.exe
Remove-Item Env:WGPU_BACKEND                     # restore the default for later launches
```

An explicit `gl` override can bring the driver crash back on affected systems. The macOS, Linux
and web backend defaults are unchanged.

The **32-bit** build uses OpenGL even when a DirectX 12 graphics card is present. On 32-bit
Windows, creating a DirectX 12 graphics pipeline fails immediately and the app exits before the
window appears. `WGPU_BACKEND=vulkan` (or `dx12`) still overrides that.

### The app quits right after its window appears ("Parent device is lost")

If the log shows "Parent device is lost":
WordCraft compiles DirectX 12 shaders with FXC, the compiler built into Windows, so it needs no
`dxcompiler.dll`/`dxil.dll` and ignores any copy another program put on `PATH` (#603).
`WGPU_DX12_COMPILER` (`fxc`, `dxc`, `auto`) overrides this; with `dxc`, a matching
`dxcompiler.dll` *and* `dxil.dll` must sit next to `wordcraft.exe`. `WGPU_BACKEND = "vulkan"`
also avoids DirectX 12 altogether.

### The app is slow on an older Windows PC

When DirectX 12 finds no graphics card with a
driver for it (for example Intel 4th-generation graphics on Windows 11), it only offers the built-in
software renderer, which draws every frame on the processor. WordCraft normally uses OpenGL instead,
without setting anything. If OpenGL has no usable adapter, it can use DirectX 12's WARP software
adapter. The log file records the choice (lines starting with
`graphics:`). `WGPU_BACKEND` still overrides it, e.g. `$env:WGPU_BACKEND = "gl"` or `"dx12"`.

## Wayland (Linux)

On a Wayland desktop the app runs through XWayland when it is available, because
winit (the windowing library) supports dragging files into the window only on X11. To use the
native Wayland backend instead (for example for sharper text with fractional scaling, at the cost
of file drag-and-drop), choose **File › Options › Advanced › Linux display server › Wayland**; it
applies the next time WordCraft starts, and **X11 (XWayland)** or **Automatic** switches back. The
`WORDCRAFT_WAYLAND` environment variable overrides the setting: `1` for native Wayland, `0` for
XWayland:

```sh
WORDCRAFT_WAYLAND=1 wordcraft
```

## Read Aloud (Linux)

**Read Aloud** (Review › Read Aloud) speaks through a speech program installed on the system: it
tries `espeak-ng`, then `espeak`, and when neither is installed it talks to Speech Dispatcher.
The `.rpm` package recommends `espeak-ng`; the `.deb` recommends `espeak-ng` or
`speech-dispatcher`, so the package manager installs one of the two by default. With the
AppImage or the tarball, install one yourself if neither is there (`sudo apt install espeak-ng`,
`sudo dnf install espeak-ng`, `sudo pacman -S espeak-ng`). The Flatpak doesn't speak yet: it has
no audio access and doesn't include `espeak-ng`, `espeak` or Speech Dispatcher.

## Logs

The desktop app writes its log records to standard error and to `logs/wordcraft.log` next to its
preferences: on Linux `$XDG_CONFIG_HOME/wordcraft/logs/` (by default `~/.config/wordcraft/logs/`),
on macOS `~/Library/Application Support/WordCraft/logs/`, on Windows `%APPDATA%\WordCraft\logs\`.
A start from a desktop menu or the Dock has no terminal, so this file is what to attach to a bug
report: unreadable parts of a .docx, pictures or fonts a PDF export had to leave out and panics the
command guard recovered from land there. Each launch moves the previous log to `wordcraft.1.log`
(and that one to `wordcraft.2.log`), so the log of a run that crashed survives the next start; a
log stops growing at 16 MiB. `--version` writes no file, and runs with `WORDCRAFT_NO_PREFS` (agents'
test runs) log to standard error only. Document text is never logged; a panic message leaves out
the text it quotes.

By default WordCraft's own crates log at `info` and everything else at `warn`. `RUST_LOG` replaces
that with env_logger-style directives, for example `RUST_LOG=debug`,
`RUST_LOG=warn,wordcraft_docx=trace` or `RUST_LOG=info,wgpu_core=warn`; a directive ending in `*`
covers every target starting with it (`wordcraft*=debug`). The logger is
`apps/wordcraft/src/logging.rs`.

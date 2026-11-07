# Installing WordCraft

**Download WordCraft** from GitHub: the [latest release](https://github.com/storytold/wordcraft/releases/latest) has every build listed below, and [all releases](https://github.com/storytold/wordcraft/releases) has earlier versions and their notes. `<ver>` in the file names is the version number.

WordCraft is alpha software. Keep backups of important documents. If something goes wrong after installing, see [troubleshooting.md](troubleshooting.md). To build from source instead, see [Quick start](../README.md#quick-start).

## Which file do I need?

| System | Easiest choice |
|---|---|
| Windows (most PCs) | `wordcraft-<ver>-windows-x64.msi` |
| Windows on ARM (Snapdragon) | `wordcraft-<ver>-windows-arm64.msi` |
| macOS (Apple silicon or Intel) | `wordcraft-<ver>-macos-universal.dmg` |
| Linux | `wordcraft-<ver>-linux-x86_64.AppImage` (or `aarch64` on ARM) |
| Browser | `wordcraft-web-<ver>.zip` |

## Verify your download

`SHA256SUMS.txt` lists a checksum for every file in the release. Download it next to the file you chose, then compare.

```sh
# Linux
sha256sum -c --ignore-missing SHA256SUMS.txt

# macOS: print the hash, then compare it with the line in SHA256SUMS.txt
shasum -a 256 wordcraft-<ver>-macos-universal.dmg
```

```powershell
# Windows (PowerShell): print the hash, then compare it with SHA256SUMS.txt
Get-FileHash .\wordcraft-<ver>-windows-x64.msi -Algorithm SHA256
```

## Windows

| Build | Installer | Portable |
|---|---|---|
| x64 (64-bit Intel/AMD) | `wordcraft-<ver>-windows-x64.msi` | `wordcraft-<ver>-windows-x64-portable.zip` |
| arm64 (Snapdragon and other ARM PCs) | `wordcraft-<ver>-windows-arm64.msi` | `wordcraft-<ver>-windows-arm64-portable.zip` |
| x86 (32-bit) | `wordcraft-<ver>-windows-x86.msi` | `wordcraft-<ver>-windows-x86-portable.zip` |

The installer is one dual-scope package. By default it installs for your account only,
into `%LOCALAPPDATA%\Programs\ArtCraft\WordCraft`, and needs no administrator rights; choosing
"Everyone on this computer" on the first page installs into `C:\Program Files\ArtCraft\WordCraft` (`Program Files (x86)` for the 32-bit build)
and asks for an administrator. Install only one copy: a copy for another scope is refused, so
uninstall first to change who WordCraft is installed for. Installers and executables are code-signed.

If the app doesn't open, flashes and quits, or is slow, see [Windows graphics problems](troubleshooting.md#windows-graphics-problems).

## macOS

| Build | File | Notes |
|---|---|---|
| App, universal (Apple silicon + Intel) | `wordcraft-<ver>-macos-universal.dmg` | Signed and notarized |
| Command-line tool, universal | `wordcraft-cli-<ver>-macos-universal.zip` | Signed and notarized |

## Linux

| Format | x86_64 | aarch64 (ARM64) | Notes |
|---|---|---|---|
| AppImage | `wordcraft-<ver>-linux-x86_64.AppImage` | `wordcraft-<ver>-linux-aarch64.AppImage` | Runs anywhere; updates itself with [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) (`.zsync` files) |
| Flatpak | `wordcraft-<ver>-linux-x86_64.flatpak` | `wordcraft-<ver>-linux-aarch64.flatpak` | Sandboxed; `flatpak install --user <file>` |
| Debian/Ubuntu | `wordcraft-<ver>-linux-x86_64.deb` | `wordcraft-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `wordcraft-<ver>-linux-x86_64.rpm` | `wordcraft-<ver>-linux-aarch64.rpm` | |
| Tarball | `wordcraft-<ver>-linux-x86_64.tar.gz` | `wordcraft-<ver>-linux-aarch64.tar.gz` | Unpack anywhere |

RISC-V (riscv64): `wordcraft-<ver>-linux-riscv64.tar.gz`, a tarball cross-compiled for glibc 2.39+
(Ubuntu 24.04, Debian 13 and newer).

Linux notes that affect behavior (see [troubleshooting.md](troubleshooting.md)):

- [Read Aloud](troubleshooting.md#read-aloud-linux) needs a speech program installed on the system.
- [Wayland](troubleshooting.md#wayland-linux) runs through XWayland by default; you can switch to native Wayland.

## FreeBSD

| Build | File |
|---|---|
| x86_64 | `wordcraft-<ver>-freebsd-x86_64.tar.gz` |

## Web (WebAssembly)

| Build | File | Notes |
|---|---|---|
| Static site | `wordcraft-web-<ver>.zip` | Runs in a modern browser; host it on any static server |

## Community packages

These packages are maintained by community members, not by the WordCraft team.

### Arch Linux (AUR)

Community members maintain three AUR packages; the project doesn't publish
to the AUR itself. Install one of them:

| Package | What it installs | Choose it when |
|---|---|---|
| [`wordcraft-bin`](https://aur.archlinux.org/packages/wordcraft-bin) | The release tarball above, prebuilt (x86_64, aarch64) | You want the latest release without compiling (quickest) |
| [`wordcraft`](https://aur.archlinux.org/packages/wordcraft) | The latest release, built from source with `cargo` | You prefer to build releases yourself |
| [`wordcraft-git`](https://aur.archlinux.org/packages/wordcraft-git) | The current `main` branch, built from source | You want unreleased changes and accept an occasional broken build |

For example `yay -S wordcraft-bin` or `paru -S wordcraft-bin`.

### Gentoo

The [::snakebyte overlay](https://github.com/switch87/snakebyte-overlay) packages the
Linux release as `app-office/wordcraft-bin` (prebuilt) and `app-office/wordcraft` (built from source):

```sh
eselect repository add snakebyte git https://github.com/switch87/snakebyte-overlay.git
emaint sync -r snakebyte
echo 'app-office/wordcraft-bin ~amd64' >> /etc/portage/package.accept_keywords/wordcraft
emerge --ask app-office/wordcraft-bin
```

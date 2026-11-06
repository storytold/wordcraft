---
title: Install WordCraft
description: Download WordCraft for Windows 11, macOS, Linux or FreeBSD from GitHub, or install it with ArtCraft Launcher.
order: 20
verified: 0.6.1
---

Download the file for your system from the [latest release](https://github.com/storytold/wordcraft/releases/latest) and install it. In the file names, `<version>` stands for the version number, for example `0.6.1`. The release also has a `SHA256SUMS.txt` file with a checksum for every download.

## Windows 11

1. Open the [latest release](https://github.com/storytold/wordcraft/releases/latest).
2. Download `wordcraft-<version>-windows-x64.msi`. It fits most PCs (Intel and AMD, 64-bit). On a Snapdragon or other Arm PC, download `wordcraft-<version>-windows-arm64.msi` instead.
3. Run the file and follow the installer.
4. Start **WordCraft** from the Start menu.

The installer puts WordCraft in Program Files and adds a Start menu shortcut. On its last page it offers **Launch WordCraft**. If you tick that box and cannot type in the window that opens, see [Typing does nothing after the installer](../troubleshooting/typing-fails-after-install.md).

The installer also adds WordCraft to **Open with** for Word, RTF, OpenDocument, Markdown and text files. It does not change which app opens them by default. To make WordCraft the default, see [Open Word files with WordCraft by default](../guides/open-and-save-documents.md#open-word-files-with-wordcraft-by-default).

To run WordCraft without installing, download `wordcraft-<version>-windows-x64-portable.zip` (or `windows-arm64-portable.zip`), unzip it and run `wordcraft.exe`.

The release also has 32-bit builds (`windows-x86.msi` and `windows-x86-portable.zip`). On Windows 11, use the x64 or arm64 file.

If WordCraft starts and closes at once, see [WordCraft does not open on Windows](../troubleshooting/does-not-open-on-windows.md). The installers and executables are code-signed.

## macOS

1. Download `wordcraft-<version>-macos-universal.dmg`. It is one build for Apple silicon and Intel Macs, signed and notarized.
2. Open it and move WordCraft to Applications.

The release also has `wordcraft-cli-<version>-macos-universal.zip`, a command-line tool. You do not need it to use WordCraft.

## Linux

Pick one file from the release. Replace `x86_64` with `aarch64` on Arm.

| File                                        | How to use it                                                     |
|---------------------------------------------|-------------------------------------------------------------------|
| `wordcraft-<version>-linux-x86_64.AppImage` | Runs anywhere. Make the file executable, then run it.             |
| `wordcraft-<version>-linux-x86_64.flatpak`  | `flatpak install --user wordcraft-<version>-linux-x86_64.flatpak` |
| `wordcraft-<version>-linux-x86_64.deb`      | For Debian and Ubuntu.                                            |
| `wordcraft-<version>-linux-x86_64.rpm`      | For Fedora, RHEL and openSUSE.                                    |
| `wordcraft-<version>-linux-x86_64.tar.gz`   | Unpack it anywhere.                                               |

For RISC-V the release has `wordcraft-<version>-linux-riscv64.deb`, `.rpm` and `.tar.gz`. They are built for glibc 2.39 or newer, such as Ubuntu 24.04 and Debian 13.

Community members maintain Arch Linux packages in the AUR: `wordcraft-bin` (prebuilt release), `wordcraft` (release built from source) and `wordcraft-git` (the current `main` branch). For example `yay -S wordcraft-bin`. The WordCraft project does not publish to the AUR itself.

## FreeBSD

Download `wordcraft-<version>-freebsd-x86_64.tar.gz`.

## In the browser

The release has `wordcraft-web-<version>.zip`, a static site that runs in a modern browser. You host it yourself on any static web server.

## Install with ArtCraft Launcher

ArtCraft Launcher lists WordCraft with the other Craft apps. See [ArtCraft Launcher](https://getartcraft.com/docs/craft-launcher) to install and update it from there.

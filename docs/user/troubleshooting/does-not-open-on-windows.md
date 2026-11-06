---
title: WordCraft does not open on Windows
description: Update to WordCraft 0.6.1 or newer, then try a different graphics backend if WordCraft still flashes and quits at start.
order: 20
verified: 0.6.1
---

Install the [latest release](https://github.com/storytold/wordcraft/releases/latest). WordCraft 0.6.1 fixed the most common cause, a crash with "Parent device is lost" in the log. If WordCraft still quits at start, start it with another graphics backend by setting `WGPU_BACKEND` in PowerShell.

## Symptoms

- WordCraft quits before its window appears.
- WordCraft quits right after its window appears, and the log says "Parent device is lost".

## Fix

1. Check the version you have: the downloaded file name contains it. If it is older than 0.6.1, install the [latest release](https://github.com/storytold/wordcraft/releases/latest) and start WordCraft again.
2. If it still quits, open PowerShell in the folder that contains `wordcraft.exe`.
3. Run these commands:

        $env:WGPU_BACKEND = "vulkan"
        & .\wordcraft.exe
        Remove-Item Env:WGPU_BACKEND

   The last line restores the default for later launches.

**Intel graphics (UHD, Iris):** use `gl` in place of `vulkan` in the first line. Intel's Vulkan driver (`igvk64.dll`) can crash WordCraft at start ([issue 170](https://github.com/storytold/wordcraft/issues/170)).

If this works, use the same steps each time you start WordCraft. On AMD graphics do not use `gl`: it can crash AMD's `atio6axx.dll` driver.

## Why this happens

WordCraft 0.5.0 compiled its DirectX 12 shaders with DXC but did not ship `dxcompiler.dll` and `dxil.dll`. On some PCs the graphics device was lost before the first frame and the log ended with `Parent device is lost` ([issue 603](https://github.com/storytold/wordcraft/issues/603)). Since 0.6.1, WordCraft uses FXC, the shader compiler built into Windows, so it needs no extra files.

By default WordCraft uses DirectX 12 and leaves OpenGL out, because creating OpenGL can crash AMD's `atio6axx.dll` before the window appears. The 32-bit (`windows-x86`) build is the exception: it always uses OpenGL, because DirectX 12 fails to start there.

## Slow on an older PC

If your PC is old and WordCraft feels slow, it may be using Microsoft's software renderer. When DirectX 12 finds no graphics card with a driver for it, WordCraft switches to OpenGL by itself. The log file records the choice on lines starting with `graphics:`. See [Get help and find the log file](get-help-and-find-the-log.md) for where the log is.

## Still stuck?

Ask in [Discord](https://discord.gg/artcraft) or open an issue in the [issue tracker](https://github.com/storytold/wordcraft/issues). Attach the log file.

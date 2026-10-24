---
title: Typing does nothing after the installer
description: If WordCraft shows no text or caret when launched from the Windows installer, close it and start it from the Start menu.
order: 10
verified: 0.6.1
---

Close WordCraft and start it again from the Start menu. Typing works there.

## Symptoms

- You installed WordCraft with the Windows `.msi` installer and ticked **Launch WordCraft** on its last page.
- The window opens, but typed text does not appear and there is no blinking caret.
- The portable zip works fine.

## Fix

1. Close the WordCraft window.
2. Open the Start menu and start **WordCraft**.
3. Click the page and type.

## Why this happens

In one report, typing failed only in the window opened by the installer's **Launch WordCraft** box and worked when WordCraft was started normally. The maintainers think the way the installer starts the app leaves it without keyboard focus. This is an open issue: [issue 533](https://github.com/storytold/wordcraft/issues/533).

## Still stuck?

If typing also fails when you start WordCraft from the Start menu, tell us in [Discord](https://discord.gg/artcraft) or on [issue 533](https://github.com/storytold/wordcraft/issues/533). Say whether the ribbon buttons react to clicks and which keyboard layout you use. See [Get help and find the log file](get-help-and-find-the-log.md).

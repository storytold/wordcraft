---
title: Get help and find the log file
description: What to include and where to post when WordCraft misbehaves, including where to find the log file.
order: 30
verified: 0.6.1
---

Post in [Discord](https://discord.gg/artcraft) or open an issue in the [issue tracker](https://github.com/storytold/wordcraft/issues). Attach the log file.

## What to include

- Your system (Windows 11, macOS, Linux distribution) and the WordCraft version, for example 0.6.1. Click the **File** tab, then **About**, to see the version.
- What you did, what you expected and what happened instead.
- The log file (see below). If the problem happens with one document, say what is special about it.

## Find the log file

WordCraft writes `wordcraft.log` in a `logs` folder next to its preferences:

| System  | Folder                                                                              |
|---------|-------------------------------------------------------------------------------------|
| Windows | `%APPDATA%\WordCraft\logs\`                                                         |
| macOS   | `~/Library/Application Support/WordCraft/logs/`                                     |
| Linux   | `$XDG_CONFIG_HOME/wordcraft/logs/`, which is `~/.config/wordcraft/logs/` by default |

Each launch moves the previous log to `wordcraft.1.log`, and that one to `wordcraft.2.log`. If WordCraft crashed, the log of the crashed run is the `wordcraft.1.log` after your next start. A log stops growing at 16 MiB. Document text is never logged.

The log also records unreadable parts of a .docx, pictures or fonts a PDF export had to leave out, and errors WordCraft recovered from.

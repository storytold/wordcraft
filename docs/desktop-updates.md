# Desktop updates

WordCraft checks [GitHub Releases](https://github.com/storytold/wordcraft/releases) for a newer version. The check is optional and privacy-preserving.

## What is sent

When you choose **Check for updates** (Help ribbon, File backstage, `file.checkForUpdates` — including through the MCP server — or `wordcraft-cli update --check`), WordCraft makes a single anonymous HTTPS `GET` to:

`https://api.github.com/repos/storytold/wordcraft/releases/latest`

No account, identifier, document content, or telemetry is sent. GitHub sees a normal API request from your IP, as any browser would.

## When checks run

- **Check for updates on startup** (File › Options › General) is **off** by default. On first launch the app asks once whether to turn it on.
- With the setting on, at most **one** check runs per 24 hours (failed checks count toward the interval too).
- Flatpak and AppImage builds use their own updaters; WordCraft still shows a notice when a newer version exists but does not download installers for those formats.

## One-click update

For Windows `.msi` and Linux `.deb` packages, **Update** downloads the matching release asset, verifies it against `SHA256SUMS.txt` on the release, then starts the platform installer (`msiexec` or `dpkg` via `pkexec`). If the document has unsaved changes, WordCraft asks Save, Don't Save or Cancel first. **Save** saves the document, then the update starts. **Don't Save** starts the update without saving (an edit made after that answer asks again). **Cancel** leaves the document as it is and doesn't update. WordCraft then quits cleanly so the installer can finish.

On Windows the MSI runs with `/norestart` and `MSIRESTARTMANAGERCONTROL=DisableShutdown`: the installer never restarts the computer and never closes WordCraft to replace in-use files, so an update started while you are editing cannot shut the editor down behind the save prompt. An install that still needs a restart (MSI return code `3010` or `1641`) counts as a success: WordCraft shows a restart instruction, waits for you to acknowledge it, then quits so the restart can finish the update.

On macOS, **Update** downloads the universal `.dmg`, verifies the checksum, and opens it in Finder. Drag WordCraft into Applications, then reopen it. WordCraft does not replace the installed bundle or quit automatically.

Portable zip installs, copies outside `/Applications` on macOS, and unknown or development builds open the release page in your browser instead of replacing files in place.

A manual **Check for updates** answers every outcome: a newer release shows the notice bar, "up to date" and network errors appear in the status bar. Network and installer work run off the UI thread.

On Linux, only `.deb` packages installed under system paths (verified with `dpkg-query`) use one-click install; RPM and unpackaged `/usr/bin` copies fall back to the release page. On Windows, a copy under Program Files is treated as an MSI install only when the installer's own `App Paths` registry entry names it, and is otherwise handed off to the release page rather than the wrong installer.

## Disabling checks

- Turn off **Check for updates on startup** in File › Options.
- Set `WORDCRAFT_NO_UPDATE_CHECK=1` (or any non-empty value other than `0`) for a single run, or
  compile with it set for packagers who ship updates another way. Automatic startup checks make no
  request at all; a manual **Check for updates** still works.

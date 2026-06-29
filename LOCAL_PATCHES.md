# Local patch update procedure

This installed build is a local-patch build. When the update check reports a
new upstream version, do not replace the app with the upstream installer
directly.

Merge the upstream tag into this source tree, keep the local patches below, and
then rebuild and replace the installed executable:

- Profile `b` login uses `codex app-server` and opens the returned auth URL in
  Chrome with its own browser profile directory.
- Profile `c` login uses `codex app-server` and opens the returned auth URL in
  Edge with its own browser profile directory.
- Do not replace those two paths with plain `codex login`; the upstream CLI
  opens the default browser and does not expose a supported browser-selection
  flag for this flow.
- Update checks prefer `D:\Codex\tools\PowerShell\7.6.3\pwsh.exe` before
  falling back to other PowerShell executables.
- Update dialogs warn that local patches must be merged before replacement.
- Windows real-Codex discovery validates cached and user-specified paths with
  `codex --version`, skips stale shims, and probes official OpenAI versioned
  runtime directories so Codex upgrades do not strand the app on a deleted
  runtime path.
- When mirroring auth/profile state to another Windows host, preserve that
  host's `%CODEX_HOME%\account_backup\windows\install_state.json`; only sync
  root/profile auth and metadata. Disposable `login_runtime`, `refresh_runtime`,
  `login_browser_profiles`, and stale `.switch.lock` may be removed because the
  app recreates them.

Acceptance target: the installed
`D:\Codex\apps\codex_switch\codex_switch.exe` must be rebuilt from this merged
source tree, not copied from a plain upstream release. For local portable Tauri
builds, copy the adjacent `WebView2Loader.dll` into the same install directory
as the executable; otherwise the desktop app can fail at startup before loading
the UI.

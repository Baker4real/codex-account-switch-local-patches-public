# Local patch update procedure

This installed build is a local-patch build. When the update check reports a
new upstream version, do not replace the app with the upstream installer
directly.

Merge the upstream tag into this source tree, keep the local patches below, and
then rebuild and replace the installed executable:

- Windows 定向浏览器登录不再按 profile `b` / `c` 槽位选浏览器。仅当目标
  profile 已有账号标签匹配 `FORCED_EDGE_LOGIN_ACCOUNT_LABEL` 时，登录使用
  `codex app-server`，并用用户正常 Edge 状态打开返回的 auth URL。
- 其他 profile 不强制 Chrome 或 Edge，可继续走普通 `codex login` /
  上游默认浏览器行为；不要恢复按槽位名路由浏览器的旧规则。
- 定向 Edge flow 不要再传 `--user-data-dir`、`--new-window`，也不要创建
  独立浏览器 profile。
- Because this flow reuses normal browser state, preserve the pre-write account
  identity check: if an existing profile has a known ChatGPT identity, a login
  completed by a different browser account must fail before replacing
  `auth.json`.
- Do not replace the forced-Edge account path with plain `codex login`; the
  upstream CLI opens the default browser and does not expose a supported
  browser-selection flag for this flow.
- Update checks prefer `D:\Codex\tools\PowerShell\7.6.3\pwsh.exe` before
  falling back to other PowerShell executables.
- Update dialogs warn that local patches must be merged before replacement.
- Windows real-Codex discovery validates cached and user-specified paths with
  `codex --version` and a Codex version line, skips stale shims, and probes
  official OpenAI versioned runtime directories so Codex upgrades do not strand
  the app on a deleted runtime path.
- When mirroring auth/profile state to another Windows host, preserve that
  host's `%CODEX_HOME%\account_backup\windows\install_state.json`; only sync
  root/profile auth and metadata. Disposable `login_runtime`, `refresh_runtime`,
  and stale `.switch.lock` may be removed because the app recreates them. Old
  `login_browser_profiles` directories from earlier local builds may also be
  removed; current builds no longer create them.

Acceptance target: the installed
`D:\Codex\apps\codex_switch\codex_switch.exe` must be rebuilt from this merged
source tree, not copied from a plain upstream release. For local portable Tauri
builds, copy the adjacent `WebView2Loader.dll` into the same install directory
as the executable; otherwise the desktop app can fail at startup before loading
the UI.

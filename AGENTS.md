# Repository AGENTS

## Platform Isolation

- Windows and macOS should be developed independently whenever practical.
- Do not introduce shared modules or shared runtime paths unless the behavior is a necessary cross-platform contract, disk format, or build/versioning concern.
- Do not extract or expand shared modules for platform-specific behavior unless the shared code is strictly necessary for identical cross-platform contracts, on-disk formats, or version/build plumbing.
- When syncing a feature from one platform to the other, prefer implementing or adapting it under `src-tauri/win/**` or `src-tauri/mac/**` first. Use `src-tauri/shared/**` only for unavoidable neutral contracts.
- For Windows-only changes, use Windows-only compile, check, test, and build commands; do not run validation paths that require compiling `src-tauri/mac/**` unless the task explicitly touches macOS.
- Windows Vite/Tauri build commands should be run with escalation by default. In this repository, sandboxed builds are known to fail with `spawn EPERM` while starting `esbuild` from the Vite/Tauri `beforeBuildCommand`.

## Local Windows Patch Discipline

- The installed Windows build may include local patches documented in `LOCAL_PATCHES.md`; when updating from upstream, merge the upstream tag into this source tree and preserve those patches before rebuilding.
- For future local Windows rebuilds, start from `LOCAL_PATCHES.md` section `已验证 Windows 编译路径（优先使用）`; use `npm run tauri:build:windows:portable` first for installed-app replacement instead of rediscovering GNU/CI paths.
- Windows directed-browser login is explicit metadata based, not slot-name or account-label based: a profile with `login_browser` set to `edge` uses `codex app-server` + the user's normal Edge state regardless of profile slot. Do not restore profile `b`/`c` browser routing, and do not reintroduce `--user-data-dir` or dedicated browser profile directories. Preserve the pre-write account identity check so a normal-browser login cannot silently overwrite a known profile with a different ChatGPT account.
- Profile write-back is identity-routed: `.current_profile` and `.active_profile` are hints only. Compare live `auth.json` identity with known profile identities, allow only the matching profile or a genuine empty placeholder, and refuse unmanaged drift instead of overwriting a stale slot.
- When replacing an installed Windows directory, copy both `codex_switch.exe` and the adjacent `WebView2Loader.dll` from the Tauri release output.
- After this project builds or edits a new Windows `codex_switch.exe`, also deploy that same exe plus its matching x64 `WebView2Loader.dll` to the remote Windows machine's desktop-shortcut target. Resolve the remote shortcut target first, replace the target installation files rather than copying to the desktop path literally, and verify hashes after replacement.

## Agent skills

### Issue tracker

Issues and PRDs are tracked as local markdown under `.scratch/`; external PRs are not a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

The repo uses the default five-label triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context domain docs layout. See `docs/agents/domain.md`.

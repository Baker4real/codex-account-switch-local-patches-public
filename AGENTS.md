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
- Keep profile `b` login on `codex app-server` + Chrome and profile `c` login on `codex app-server` + Edge. Do not collapse this back to plain `codex login`, which opens the default browser. Do not reintroduce `--user-data-dir` or dedicated browser profile directories for these browser launches.
- When replacing `D:\Codex\apps\codex_switch\`, copy both `codex_switch.exe` and the adjacent `WebView2Loader.dll` from the Tauri release output.

## Agent skills

### Issue tracker

Issues and PRDs are tracked as local markdown under `.scratch/`; external PRs are not a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

The repo uses the default five-label triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context domain docs layout. See `docs/agents/domain.md`.

# Implementation Notes

## Data locations

- Shared Codex state root: `CODEX_HOME` or `~/.codex`
- Profile backups: `~/.codex/account_backup/<profile>`
- Current profile pointer: `~/.codex/account_backup/.current_profile` (routing hint, not account identity)
- Active marker per profile: `~/.codex/account_backup/<profile>/.active_profile` (routing hint, not account identity)
- Auto snapshots: `~/.codex/account_backup/_autosave/<timestamp>/auth.json`
- macOS runtime files: `~/.codex/account_backup/macos/`
- macOS command shim: `~/.codex/bin/codex`
- Windows runtime files: `%CODEX_HOME%\account_backup\windows\`
- Windows command shim: `%CODEX_HOME%\bin\codex.cmd`

## Runtime shape

- macOS still keeps shell entrypoints under `macOS-backup/` as retained compatibility fallback
- Windows desktop UI and CLI both use Rust/Tauri:
  - Windows shell frontend under `src-tauri/win/front/`
  - shared frontend bridge code under `src-tauri/shared/front/`
    - controller/orchestration in `src-tauri/shared/front/actions.ts`
    - dashboard view-model shaping in `src-tauri/shared/front/dashboard-view-model.ts`
    - native invoke wrapper in `src-tauri/shared/front/tauri.ts`
- macOS desktop shell now has a separate frontend root under `src-tauri/mac/front/`
  - `src-tauri/mac/front/index.html` removes the Windows-style custom title bar
  - `src-tauri/mac/front/styles.css` carries macOS-specific shell styling overrides
  - `src-tauri/mac/front/lib/window-controls.ts` is intentionally a no-op because macOS uses native window controls
- native commands and CLI runtime stay under `src-tauri/`, but the source tree is now split by responsibility:
  - `src-tauri/win/front/` holds the Windows desktop shell
  - `src-tauri/mac/front/` holds the macOS desktop shell
  - `src-tauri/shared/front/` holds shared frontend bridge code and font assets
  - `src-tauri/win/runtime/` holds Windows-only bootstrap, install, process, refresh, and windowing code
  - `src-tauri/mac/runtime/` holds macOS-only bootstrap, CLI shim, install, process, switch, and windowing code
  - `src-tauri/shared/runtime/` holds platform-neutral profile, metadata, quota, path, config, CLI, error, and model logic
  - `src-tauri/shared/platform/` defines platform lifecycle hooks so window close, login, auth refresh, and app reopen can be routed without hard-coding those call sites to Windows modules
  - `src-tauri/shared/commands/` holds the Tauri command boundary used by both platforms
  - `src-tauri/src/` is now kept as the crate entry layer only, because Cargo expects `lib.rs` and `main.rs` there by default
- `src-tauri/shared/runtime/switch_core.rs` now owns the profile switch orchestration
- `src-tauri/mac/runtime/windowing.rs` now applies native macOS window decorations instead of reusing the Windows custom chrome assumptions
- `src-tauri/shared/runtime/cli.rs` now resolves to `windows::*` or `macos::*` through compile-time aliases instead of directly binding the CLI to Windows modules
- The desktop app does not use a separate local backend or HTTP server at runtime
- `windows/` remains only as a historical note directory while the Rust path is the primary runtime and regression target

## Installation behavior

- `macOS-backup/install.sh` is now a compatibility entrypoint with `auto`, `desktop`, and `legacy` modes
- In `auto` mode, `macOS-backup/install.sh` prefers the native desktop installer and falls back to the legacy shell installer if no native installer binary is available
- `macOS-backup/install-legacy.sh` keeps the original shell-based installation behavior
- `macOS-backup/install-desktop.sh` delegates to the native `codex_switch` installer and then wires `~/.codex/bin` into the shell
- Legacy macOS install creates `~/.codex/account_backup` if missing
- Legacy macOS install creates `~/.codex/account_backup/a` through `~/.codex/account_backup/d`
- Legacy macOS install fills any missing `a`-`d` `auth.json` files from `examples/account_backup/demo/auth.json.example`
- If `~/.codex/auth.json` exists during legacy macOS install, it is copied to `~/.codex/account_backup/a/auth.json`
- If a real root auth exists and no active profile is initialized yet, legacy macOS install sets `a` as the active profile
- `codex_switch.exe install` creates the same profile layout plus `%CODEX_HOME%\account_backup\windows\` and `%CODEX_HOME%\bin\`
- Windows install copies `codex_switch_cli.exe` into the runtime directory
- Windows install fills any missing `a`-`d` `auth.json` files from `examples/account_backup/demo/auth.json.example`
- If a real root `%CODEX_HOME%\auth.json` exists, Windows install overwrites `a/auth.json` with it
- If a real root auth exists and no active profile is initialized yet, Windows install sets `a` as the active profile
- Windows install records `real_codex_path` and `path_added_by_installer` in `install_state.json` after resolving a runnable real Codex CLI

## Desktop app first-run bootstrap

- On desktop app startup, if `account_backup` is missing, the app initializes it automatically
- Bootstrap creates `a` through `d`
- Bootstrap writes placeholder `auth.json` files from `examples/account_backup/demo/auth.json.example`
- If root `auth.json` exists, it is copied into `a/auth.json`
- If root `auth.json` exists, bootstrap marks `a` as the active profile
- Bootstrap also refreshes `%CODEX_HOME%\account_backup\windows\install_state.json`

## Preconditions for switching

- The target profile name must pass `validate_profile_name`
- The target profile directory must already exist, must pass `is_profile_dir`, and must not be a symlink or Windows reparse point
- The target profile directory must already contain `auth.json`
- Profile-managed entries such as `auth.json` and `profile.json` must be regular files when present

The switch script itself does not create profile folders or generate missing auth files.

## Switching algorithm

1. Validate that the target profile name and directory are valid profile state, not runtime folders, symlinks, or reparse points.
2. Validate that the target profile contains a regular-file `auth.json`.
3. If the Codex desktop app is running, terminate it before switching.
4. Resolve a current-profile marker candidate from `.current_profile` only when it is a valid profile name whose directory passes `is_profile_dir`; otherwise fall back to `.active_profile`.
5. Load the live root `auth.json` identity and choose the write-back target by account identity, not by marker alone:
   - if the marker profile has the same `account_id` / `email`, write back to that marker;
   - if another managed profile owns the live identity, write back to that actual owner and heal the stale marker;
   - if the marker is a genuine empty placeholder, seat the new account there;
   - if the live root identity is known but unmanaged, do not write it into any existing slot and clear active markers.
6. Write managed root account-state files from `~/.codex` back only into the resolved backup target.
7. Save a timestamped snapshot of root `auth.json`.
8. Overlay managed account-state files from the target profile into `~/.codex`.
9. Update `.current_profile` and `.active_profile`.
10. If the app was running before the switch, relaunch it.

## File sync strategy

- The native Rust profile backup and root overlay steps manage only `auth.json` and `profile.json`.
- The root copy step does not whole-tree copy a profile directory into `~/.codex`.
- Root write-back treats `.current_profile` and `.active_profile` as hints only. The actual destination is resolved from the live `auth.json` account identity so a stale marker cannot overwrite the wrong profile.
- Global or runtime assets such as `skills`, plugin caches, `AGENTS.md`, `rules`, `memories`, `vendor_imports`, sqlite state, and sessions/jsonl history are ignored even if they remain in an old profile directory.
- Managed entries are validated before copy; directories, symlinks, Windows reparse points, and other non-regular files fail with `PROFILE_STATE_INVALID` instead of replacing live root files.
- Files absent from the target profile are not automatically removed from the root state.
- `.active_profile` and `.DS_Store` are excluded from copy operations.
- Legacy macOS shell mode still belongs to `macOS-backup/`; the native Rust runtime is the regression target for profile switching.

## Login / refresh runtime 隔离

- Login / refresh runtime home 仍会播种上游共享运行态资产：`plugins`、`cache`、`sqlite`。
- Runtime prune 只移除可能干扰 login / refresh 行为的隔离状态：`AGENTS.md`、`rules`、`skills`、`vendor_imports`、`memories`。
- Runtime prune 不得删除 `plugins`、`.tmp`、`cache`、`sqlite` 或 `_autosave`；插件和 connector 行为依赖这些上游共享资产继续可用。
- `auth.json` autosave 只创建 timestamp 快照，并在 timestamp 冲突时选择唯一目录。它不执行保留策略；备份保留必须放在显式 dry-run / apply 清理工具里，不能藏进 login、refresh、switch 或 autosave 流程。

## macOS wrapper behavior

- In desktop mode, the macOS compatibility installer injects a PATH hook into `~/.zshrc` so `~/.codex/bin/codex` is resolved before the existing CLI
- In legacy mode, the macOS installer injects a `codex()` shell wrapper into `~/.zshrc`
- Legacy `codex switch ...` routes to `~/.codex/account_backup/codex-switch.sh`
- Legacy non-switch `codex` commands continue to use the user's existing `codex` CLI in `PATH`

## Native macOS runtime behavior

- The native macOS runtime stores its install state in `~/.codex/account_backup/macos/install_state.json`
- The native macOS runtime copies `codex_switch_cli` into `~/.codex/account_backup/macos/`
- The native macOS runtime writes a managed `~/.codex/bin/codex` shim that forwards into the runtime CLI
- Native macOS CLI forwarding skips the managed shim when resolving the real `codex` binary from `PATH`
- Native macOS app activation first probes `/Applications/Codex.app`, then `~/Applications/Codex.app`, and otherwise falls back to `open -a Codex`
- If Codex is already running on macOS, the runtime first tries AppleScript activation before doing a fresh open
- Native macOS app shutdown uses `pgrep -x Codex`, `pkill -TERM -x Codex`, and finally `pkill -KILL -x Codex`
- Native macOS desktop UI now loads from `src-tauri/mac/front/` instead of the Windows shell under `src-tauri/win/front/`
- Native macOS windows restore system decorations and native title bar behavior during startup
- Native macOS bundle packaging is isolated in `src-tauri/tauri.macos.conf.json`, which enables `app` + `dmg` outputs without changing the default Windows/base Tauri config
- `npm run tauri:build:macos-dmg` moves the generated `.dmg` beside the `.app` under `src-tauri/target/release/bundle/macos/` to keep macOS artifacts in one folder
- `macOS-backup/uninstall.sh` mirrors the same compatibility split and chooses native desktop teardown when the desktop runtime is installed

## Windows shim behavior

The Windows installer writes `%CODEX_HOME%\bin\codex.cmd` and ensures `%CODEX_HOME%\bin` is first in the user PATH.

- `codex switch ...` routes to `%CODEX_HOME%\account_backup\windows\codex_switch_cli.exe shim switch ...`
- Non-switch `codex` commands are forwarded to the previously resolved real Codex CLI path from `install_state.json` only while that path still passes `codex --version` and reports a Codex version line
- `%CODEX_HOME%\account_backup\windows` is reserved runtime state and excluded from profile listing / active-profile scans

## Windows desktop app actions

- `Switch` writes current root state back to the identity-resolved profile, snapshots `auth.json`, overlays the target profile into root state, updates active markers, and relaunches Codex if needed
- `Login` reuses the per-card login path for the current profile, so login output is checked against the target profile identity before any profile `auth.json` is replaced
- Local Windows patched login routes only the profile whose known account label matches `FORCED_EDGE_LOGIN_ACCOUNT_LABEL` through `codex app-server`, then opens the returned `account/login/start` auth URL in Edge.
- The patched browser-login flow no longer passes `--user-data-dir`, `--new-window`, or creates dedicated browser profile directories. Edge handles the URL with the user's normal browser state. Existing `login_browser_profiles` directories from older local builds are disposable cleanup leftovers.
- Because normal browser state can already be logged into the wrong ChatGPT account, per-card login checks the new OAuth identity against any known identity in the target profile before replacing `auth.json`. A mismatch fails before touching the target profile or live root auth.
- Browser path override `CODEX_SWITCH_EDGE_PATH` is still supported when common install paths do not find Edge.
- Other profiles keep the standard `codex login` path so upstream behavior remains the default outside the forced-Edge account label.
- `Open Codex` activates the Codex desktop app if already running, or launches it if not
- `Add Profiles` creates a new profile directory and writes template `auth.json` plus `profile.json`
- `Contact Us` opens the project GitHub repository

## Plan / quota 来源与可信度

- `QuotaSummary` 在 5 小时和周额度窗口之外携带 `provenance` 元数据，让前端能区分官方新鲜数据、fallback 数据和本地观测数据。
- 来源取值为 `chat_gpt_api`、`codex_app_server`、`session_jsonl`、`unknown`；可信度取值为 `official`、`fallback`、`local_observation`、`unknown`。
- ChatGPT Web API 路径把 quota 标记为 `official`；Codex app-server JSON-RPC 路径标记为 `fallback`；session JSONL 解析标记为 `local_observation`，并在可用时附带来源文件 mtime。
- `last_plan_check_ms` 的 6 小时窗口只控制后台 bulk plan refresh 是否再次查询 usage；它不是授权边界，不能因为 plan 缓存过期就强制 OAuth rotation。
- 账号页在 quota 数值旁显示来源 badge 和观测年龄。5 小时或周额度剩余百分比低于或等于 20% 时只进入警示状态；账号切换仍必须由用户显式触发。

## 授权缓存状态

- `RefreshTokenStatus` 从每个 profile 的 `auth.json` 派生，前端在账号卡片和当前账号面板显示健康状态、上次刷新时间和可观察到的 refresh token 到期时间。
- ChatGPT / Codex OAuth 的 `refresh_token` 通常是 opaque string；项目不能从 token 本体解析固定 TTL。只有 `auth.json` 显式出现 `tokens.refresh_token_expires_at_ms` 或 `tokens.refresh_token_expires_at` 时，UI 才显示 refresh token 到期时间。
- 官方 Codex CI/CD 文档把 `last_refresh` 超过约 8 天描述为 token bundle stale；本项目用这个规则提示“建议刷新授权缓存”。这不是 refresh token 的官方到期保证，也不能替代 401 后的 refresh-and-retry。
- `access_token` / `id_token` 的 JWT `exp` 可用于判断短期登录缓存是否需要刷新，但不能被当作 refresh token 到期时间展示。
- Plan / quota 刷新会先使用当前 `access_token` 请求官方 usage；只有 access token 已过期、临近过期或 usage 请求返回 401 时，才通过 `refresh_token` 换新 token。
- 前端把状态做成单行 chip，完整解释放在 `title` / `aria-label`；可见文案避免“点刷新 / 点登录”这类看起来像额外按钮的动作提示。

## Local Windows patch build

- `LOCAL_PATCHES.md` is the upgrade checklist for this machine's installed local-patch build
- When an upstream release exists, merge the upstream tag into this source tree and keep the local browser-login and update-request patches before rebuilding
- The local update checker prefers `D:\Codex\tools\PowerShell\7.6.3\pwsh.exe`, then `pwsh`, then `powershell`, because Windows PowerShell 5.x is blocked on the target host
- The verified local installed-app replacement path starts from `LOCAL_PATCHES.md`, runs `npm run build -- --mode windows` as the Windows frontend precheck, then uses `npm run tauri:build:windows:portable` as the main MSVC portable build command; the installed target is `D:\Codex\apps\codex_switch\codex_switch.exe`
- 本地 MSVC 便携构建命令需要 Rust target `x86_64-pc-windows-msvc`、Visual Studio Build Tools C++ 工具链和 Windows SDK，产物写入 `src-tauri/target/x86_64-pc-windows-msvc/release/codex_switch.exe`；`npm run tauri:build:portable` 只作为 GNU/MinGW 诊断路径，不是替换安装版的主构建路径
- `scripts/tauri-windows-msvc-build.mjs` 只修改构建子进程内的 `PATH`：它把 `D:\Codex\.cargo\bin` 放到前面，检查 `rustc -vV`，并在当前 compiler host 不是 `x86_64-pc-windows-msvc` 时中止
- 本地便携替换必须把 `codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll` 一起复制到 `D:\Codex\apps\codex_switch\`；target-specific MSVC 构建里，loader 可能位于 `webview2-com-sys` build output，而不是 release 可执行文件旁边

## Windows Codex CLI discovery

Windows real-Codex resolution rejects the managed shim, Windows Apps aliases, and any existing user/cached path that cannot run `codex --version` successfully with a Codex version line. Discovery includes:

1. `CODEX_EXE`
2. `CODEX_INSTALL_DIR\codex.exe`
3. versioned subdirectories under `CODEX_INSTALL_DIR`
4. `%LOCALAPPDATA%\OpenAI\Codex\bin\codex.exe`
5. versioned `%LOCALAPPDATA%\OpenAI\Codex\bin\*\codex.exe`
6. PATH and the existing npm/common install locations

If a cached `install_state.json` path becomes stale after a Codex upgrade, the resolver falls through to discovery and persists the first runnable replacement. `install_state.json` is machine-local: account/auth state syncs should copy root/profile auth and metadata, but preserve the target host's `%CODEX_HOME%\account_backup\windows\install_state.json`.

## Windows app discovery

Windows desktop app discovery first prefers the path recorded in `install_state.json`. If that is missing or invalid, it probes common install locations including:

1. `%LOCALAPPDATA%\Programs\Codex\Codex.exe`
2. `%LOCALAPPDATA%\Programs\OpenAI\Codex\Codex.exe`
3. `%LOCALAPPDATA%\Codex\Codex.exe`
4. `%LOCALAPPDATA%\OpenAI\Codex\Codex.exe`
5. `%ProgramFiles%\Codex\Codex.exe`
6. `%ProgramFiles%\OpenAI\Codex\Codex.exe`
7. directories under `%LOCALAPPDATA%\Programs` or `%ProgramFiles` whose names contain `codex`
8. Windows `App Paths\Codex.exe` registry entries

## Validation strategy

- Primary regression baseline is the Rust suite under `src-tauri/`
- Root command: `npm test`
- Equivalent direct command: `cargo test --manifest-path src-tauri/Cargo.toml`
- Windows GNU/MinGW host check: `npm run test:rust:windows-gnu`

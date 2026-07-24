# Repository AGENTS

## Platform Isolation

- This product is Windows x64 only. Keep the active runtime, frontend entry, assets, build, CI, and release paths Windows-specific.
- Keep `src-tauri/shared/**` limited to unavoidable neutral contracts such as on-disk formats, protocol models, and shared account/quota logic; Windows process, login, window, and install behavior belongs under `src-tauri/win/**`.
- Do not reintroduce retired non-Windows source trees, package scripts, release assets, or platform-specific build commands. Historical changelog entries are evidence only, not support promises.
- Use Windows-only compile, check, test, and build commands. Do not add validation paths for retired platforms.
- Windows Vite/Tauri build commands should be run with escalation by default. In this repository, sandboxed builds are known to fail with `spawn EPERM` while starting `esbuild` from the Vite/Tauri `beforeBuildCommand`.

## Local Windows Patch Discipline

- The installed Windows build may include local patches documented in `LOCAL_PATCHES.md`; when updating from upstream, merge the upstream tag into this source tree and preserve those patches before rebuilding.
- For future local Windows rebuilds, start from `LOCAL_PATCHES.md` section `已验证 Windows 编译路径（优先使用）`; use `npm run tauri:build:windows:portable` first for installed-app replacement instead of rediscovering GNU/CI paths.
- Windows directed-browser login is explicit metadata based, not slot-name or account-label based: a profile with `login_browser` set to `edge` uses `codex app-server` + the user's normal Edge state regardless of profile slot. Do not restore profile `b`/`c` browser routing, and do not reintroduce `--user-data-dir` or dedicated browser profile directories. Preserve the pre-write account identity check so a normal-browser login cannot silently overwrite a known profile with a different ChatGPT account.
- Profile write-back is identity-routed: `.current_profile` and `.active_profile` are hints only. Compare live `auth.json` identity with known profile identities, allow only the matching profile or a genuine empty placeholder, and refuse unmanaged drift instead of overwriting a stale slot.
- Windows profile switching must preserve the Store-package-scoped lifecycle and transactional rollback contract documented in `LOCAL_PATCHES.md`; never replace it with blanket image-name `taskkill` logic.
- When replacing an installed Windows directory, copy both `codex_switch.exe` and the adjacent `WebView2Loader.dll` from the Tauri release output.
- After this project builds or edits a new Windows `codex_switch.exe`, also deploy that same exe plus its matching x64 `WebView2Loader.dll` to the remote Windows machine's desktop-shortcut target. Resolve the remote shortcut target first, replace the target installation files rather than copying to the desktop path literally, and verify hashes after replacement.

## Agent skills

### Issue tracker

Issues are tracked in the private GitHub repo `Baker4real/codex-account-switch-local-patches` and implemented on `main`. See `docs/agents/issue-tracker.md`.

### Triage labels

The repo uses the default five-label triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context domain docs layout. See `docs/agents/domain.md`.

## Knowledge Scope Policy

使用 `neat-freak`、整理文档或准备提交前，先把新增知识分类：

- `project`：换设备仍成立，写入 `README.md`、`docs/` 或 `CONTEXT.md`。
- `device`：只对某台设备成立但另一台设备需要知道，写入 `docs/environments/<device-id>.md`。
- `local`：只对当前机器有用，不提交，写入 `.local/`、`*.local.md` 或 `.env.local`。
- `agent`：下次 AI 写代码必须遵守，写入 `AGENTS.md`。
- `evidence`：一次性验证、日志、截图或 run output，写入主机级 Codex evidence 目录或项目 `reports/`。

凡是涉及本机路径、桌面快捷方式、端口占用、代理、登录态、cookie、安装目录、工具链路径、系统服务状态或硬件差异的事实，默认不是 `project`。

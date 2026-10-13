---
type: report
title: "Changelog"
tags: [project-docs, report]
intent: "Changelog"
---

# Changelog

## Unreleased

- 仓库迁到公开的 `Baker4real/codex-account-switch-local-patches-public`：应用里打开仓库主页和 Releases 页的入口、更新检查返回的发布页链接、README 的 Releases 链接和 issue 跟踪都改指新仓库。旧私有仓库归档只读，保留 #1–#25 历史 issue。迁移改写了本地补丁提交，SHA 已变；旧 clone 需把 `private` 远端改到新仓库，再用 `git reset --keep private/main` 对齐。
- 远端 Windows 同步从每次构建的硬性完成条件改为按需操作（owner 2026-09-18 裁定）：同构建 loader 与本机安装校验完成即视为发布完成；远端需要更新时仍按既有部署合同（只读 inspect、停目标进程、exe+loader 成套替换、远端哈希复核）执行。
- 远端桌面部署脚本里仅剩的两处 `WScript.Shell` 快捷方式读取（inspect 探针与远端替换脚本解析 `.lnk` 的 `TargetPath` / `WorkingDirectory`）同样对齐到 `Shell.Application` 的 `NameSpace()/ParseName()/GetLink`（基于 PIDL、全程 Unicode）：`WScript.Shell` 读端也会按系统 ANSI 代码页转换 .lnk 路径，在非中文系统 locale 的远端主机上 CJK 快捷方式名（如「重启 Codex.lnk」）同样会读到空/损坏值。inspect 的 fail-soft 语义（.lnk 缺失或不可读时相关 JSON 字段为 null）、部署的 fail-closed 报错（"Desktop shortcut has no target path"）、两处 JSON 字段集与格式均保持不变。
- 修复「重启 Codex.lnk」快捷方式在非中文系统 locale（如 en-US Windows）上创建失败的问题：`WScript.Shell` 的快捷方式管线会按系统 ANSI 代码页转换 .lnk 路径，CJK 文件名被 best-fit 替换成非法的 `?` 后 `Save()` 抛 `FileNotFoundException`。创建与测试读回都改走 `Shell.Application` 的 `NameSpace()/ParseName()/GetLink`（基于 PIDL、全程 Unicode），对系统 locale 不再敏感；快捷方式文件名、目标、参数和图标保持不变。
- 远端桌面部署的 `EvidenceRoot` 恢复为必须显式提供：`-EvidenceRoot` 参数或 `CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT` 环境变量，两者都缺失时脚本明确报错；不再回退 `CODEX_HOME`，也不再用静默的本机默认证据根。
- 新增无窗口「重启 Codex」入口：`codex_switch_cli restart` 与 `install-shortcut` 创建的桌面「重启 Codex.lnk」。二者复用现有 Store 包生命周期锁；成功静默，失败弹出含原始错误码的 MessageBox。`uninstall` 会删除该固定名快捷方式。主窗口重启按钮保持原样。
- 移除使用统计页面、前端刷新、Tauri IPC、归因采集和本机 ledger/定价实现；现有 `openai_usage.sqlite`、归因 JSONL 与 evidence 目录保持原样，不自动删除。账号套餐、5 小时/周额度和 additional quota 不受影响。
- “重启 Codex”从控制区提升到仪表盘顶层标题栏，保留执行锁、busy 状态和失败提示，并为窄窗口提供整行按钮布局。
- 修复账号卡刷新在本机代理端口变化后可能返回 `/wham/usage` 网络错误的问题：ChatGPT API 和 Codex 子进程不再硬编码失效的 `127.0.0.1:10809`，会优先读取 `CODEX_SWITCH_PROXY_URL` 及标准代理环境变量，默认回落到 `127.0.0.1:10808`。
- 合并公开上游 1.6.1 的 quota 隔离修复：只会读取当前账号激活后写入的 session JSONL，且仅更晚的快照才能覆盖已持久化数据；每次 root 状态回写前都会先同步卡片，避免旧 root `profile.json` 覆盖已刷新的套餐、额度或重置卡。
- Windows Store 包中的 Codex 宿主已按合并后的 `ChatGPT.exe` 识别；仍严格要求 `OpenAI.Codex` 包身份，消费级 ChatGPT 不会被关闭或重启。
- 账号套餐现在持久化记录来源：官方 `/wham/usage` 的 live `plan_type` 不会再被滞后的 OAuth `id_token` 覆盖，正常刷新只在授权确有需要时轮换 token；较新的本地 session 快照未携带重置卡时，已确认的官方可用卡会被保留，切换写回也不会将其清零。
- 远端 Windows 桌面部署收紧为严格 OpenSSH/SCP：必须显式提供方向专用私钥和 `known_hosts`，固定非交互、严格主机密钥校验，不再自动接纳主机密钥或回退到密码/askpass 认证。
- Cockpit phase one 已落地 Windows GUI 的 nvm-windows/fnm/asdf CLI 候选发现、账号搜索与动态套餐筛选、显示数/总数提示，以及 additional/model-specific 配额显示偏好；默认隐藏 Spark 噪声行，筛选和偏好不写入账号凭据。
- **Breaking:** the supported product boundary is now Windows x64 only. Runtime sources,
  frontend entrypoints, build scripts, CI, and release assets are maintained for the Windows
  MSVC target; other platform packages and launch paths are no longer part of the product.
- Update checks remain informational: they query version metadata and open release notes only.
  A local-patch build must merge the upstream tag into this source tree and rebuild before an
  installed copy is replaced; the app never downloads or installs a public upstream binary.
- Hardened native profile switching so profile directories, markers, identities, and managed account files are validated before copy. Switch write-back now routes by live `auth.json` account identity instead of blindly trusting `.current_profile`, refuses unmanaged drift instead of overwriting a stale slot, manages only `auth.json` / `profile.json`, rejects symlink/reparse or directory-shaped managed entries, and ignores stale profile `skills` trees instead of overlaying live global Codex assets.
- Windows 账号切换不再按镜像名执行 `taskkill`：现在按 `OpenAI.Codex` Store 包发现并用进程句柄复核 `ChatGPT.exe` 外壳及包内后端，先请求正常退出，再确认连续空快照；提交期若进程重现会重试目标态写入，写盘失败先事务回滚，恢复不完整时返回 `SWITCH_RECOVERY_FAILED`、不自动重开并提示手动关闭。
- 当 live `CODEX_HOME` 账号不属于任何已保存账号卡片时，启动同步会清除 stale current-profile marker，账号页也不会继续把旧卡片显示为 current，并会暴露未管理账号提示。
- Windows 本地补丁登录现在改为显式 profile metadata 路由：`login_browser` 设置为 `edge` 的目标账号无论在哪个 profile 槽位都通过 `codex app-server` 打开用户正常 Edge 状态；profile `b` / `c` 不再自带 Chrome / Edge 强制规则。定向 Edge flow 仍不传 `--user-data-dir` / `--new-window`，并保留写入前账号身份校验。
- 账号卡片和当前账号面板现在显示本地授权缓存状态：可观察到 refresh token 到期字段时显示到期时间；官方未提供 refresh token 到期时间时，根据 Codex 官方约 8 天 token bundle stale 规则提示“建议刷新”。UI 文案改为单行状态 chip，去掉“点刷新 / 点登录”等容易误解的内联动作文字。
- 账号页 quota 读数现在显示来源与新鲜度：官方 ChatGPT API、Codex app-server fallback、本地 session 观测或未知来源。5 小时 / 周额度低于或等于 20% 时只显示低额度提示，不自动切换账号。
- 修正账号卡片中的 5 小时 / 周额度错位、账号文本换行和窄窗口布局；额度窗口现在按服务端实际时长归位。可用额度重置卡会逐张显示北京时间的发放与到期时间；详情接口失败仍保留主 quota，未报告 additional 配额时不再显示误导性的空状态。
- Windows 本地 MSVC 便携构建现在通过 `scripts/tauri-windows-msvc-build.mjs` 执行；脚本默认使用环境或系统 `rustup`，也可通过 `CODEX_SWITCH_RUSTUP_BIN`、`CODEX_SWITCH_CARGO_HOME`、`CODEX_SWITCH_RUSTUP_HOME`、`RUSTC` 或 `CARGO` 注入工具链，并在 `rustc -vV` 未报告 `x86_64-pc-windows-msvc` 时拒绝构建。
- Windows 本地补丁构建新增 `npm run deploy:windows:remote-desktop` 远端桌面覆盖入口：先解析远端 `.lnk` 的真实目标，上传同一份 `codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll`，备份旧目标文件，停止远端进程并校验 SHA-256。
- Login / refresh runtime 清理现在保留上游 `plugins` / `cache` / `sqlite` 共享行为，只 prune 隔离功能状态，拒绝不安全 prune 路径，并把 `_autosave` 保持为追加式 timestamp 授权快照。备份保留必须走显式 dry-run / apply 清理流程。
- Plan / quota 刷新不再把 `last_plan_check_ms` 超过 6 小时当作授权失效信号。卡片刷新和后台 bulk refresh 会先使用现有 `access_token` 查询官方 usage；只有 access token 过期、临近过期或 usage 返回 401 时才消耗 `refresh_token`，避免仍有效账号被提前提示重新登录。
- 新增「使用统计」模块：位于「账号」和「设置」之间，从 Codex session JSONL 重建 `account_backup/openai_usage.sqlite`，只统计可证明为官方 OpenAI Base URL 的 token 增量，按账号、模型、时间范围、趋势、请求日志和数据源证据展示，并按模型价格估算费用；未知模型费用保持未定价，不当作 `$0`。
- 使用统计筛选不再自我折叠：账号分类表移到时间选择下方、摘要卡片前；账号候选清除账号自身筛选但保留时间和模型筛选，模型候选清除模型自身筛选但保留时间和账号筛选，选择一个账号或模型后仍可切换到同一时间范围内的其他候选。
- 使用统计刷新默认间隔改为 30 秒；手动刷新会立即显示进行中或排队状态，自动刷新不会覆盖正在进行的查询。请求日志分页和筛选现在使用请求序列与查询意图防止旧响应覆盖当前页，趋势图增加顶部余量、浅色 tooltip 和移动端表格横向滚动，避免峰值、浮层和日志列互相遮挡。
- 使用统计的模型列和请求证据列现在会完整换行显示，不再因表格紧凑样式截断长模型名、请求模型、定价模型、reasoning effort 或 verbosity 证据。
- OpenAI 使用统计升级为 evidence / facts / strict accepted events 分层账本：默认总额仍只读 strict proven，历史槽位归因估算同屏可见但不冒充官方账单；账号表按账号实体聚合，`account_identity_key` 优先，legacy label 只在能唯一桥接到同一 strict identity 时合并。
- cc-switch 等只读外部证据冻结副本改放 `%CODEX_HOME%\usage_evidence\`，并显式排除 `account_backup\usage_evidence`，避免 evidence 目录被误识别成账号 profile。

## 1.5.12 - 2026-05-29

- Settings → Codex CLI path gains an **Auto-detect** button next to "Change". Unlike the existing path self-check (which trusts the cached / override path), it force-rescans every common install location plus PATH and verifies each candidate is actually runnable via `codex --version`. A lone runnable hit is applied immediately; several open the dialog with the verified candidates to pick from; none falls back to the manual dialog. Targets the two cases the self-check can't: auto-detection landed on a wrong / stale path, or the user doesn't know where to point it. Backed by a new `redetect_codex_cli_path` command that runs on the blocking pool (each candidate probe spawns a child) with a per-candidate timeout so a hung binary can't wedge the scan. macOS + Windows symmetric.
- macOS release builds are now **ad-hoc codesigned** instead of `--no-sign`. `--no-sign` left only the Rust linker's binary-level ad-hoc signature (`flags 0x20002 adhoc,linker-signed`) with no full bundle codesign, so `codesign --verify` reported "code has no resources but signature indicates they must be present" — a mismatched signature that macOS Sequoia 15+ flags as "is damaged" (no "Open anyway") once the download is quarantined. The release workflow now exports `APPLE_SIGNING_IDENTITY` (a real Developer ID when the secret is set, else ad-hoc `-`) so Tauri does a complete bundle codesign, plus a `codesign --verify --deep --strict` gate before upload. Downloads now open via the normal "unverified developer → Open anyway" flow.
- Linux `.deb` / `.AppImage` now actually attach to the release: the asset glob is recursive (`**/*`) so it matches the `bundle/deb/` and `bundle/appimage/` subdirectories the artifact preserves (#46).

## 1.5.11 - 2026-05-16

- Added experimental Linux x86_64 build to the release pipeline. Tagged releases now publish `.deb` (Debian/Ubuntu) and `.AppImage` (generic portable) artifacts alongside the existing macOS / Windows ones. Built on `ubuntu-22.04` (glibc 2.35) so binaries run on Ubuntu 22.04+ / Debian 12+ and equivalent distros. UI, profile switching, and plan / quota readout work as on the other platforms; Linux-native paths for Codex CLI discovery and `codex login` spawning are not separately adapted yet (the non-macOS code branch is currently reused), so feedback issues are welcome.

## 1.5.10 - 2026-05-16

- Profiles page redesign: account cards become single-row list items (account name + plan / 5-hour and weekly quotas / actions all in one horizontal row). Profile-list page size raised from 4 to 8 entries per page.
- Removed the placeholder Runtime tab and page (UI-only stub with no backend wiring).
- Top navigation tabs centered horizontally instead of right-aligned.
- Settings page trimmed: removed the unwired Forward port, Admin port, Launch at login, and Configuration backup rows. Codex CLI path row collapsed from two lines to a single inline row with the "Change" button on the right.

## 1.5.9 - 2026-05-10

- Bulk plan refresh (run on app launch + each local-day rollover) now skips any profile whose `last_plan_check_ms` was confirmed within the last 6 hours. On a workspace with several OAuth accounts the previous launch trickled cards updating one-by-one for 10–25 s of background work; with the gate, repeat launches inside the same working day cost zero round-trips.
- Per-card Refresh button stops forcing an OAuth token rotation on every click. Rotation now only fires when the cached `last_plan_check_ms` is older than 6 hours (or absent); within that window the click reuses the cached `id_token` claims and only refreshes the access_token if it's actually about to expire. Saves the OAuth POST round-trip (~0.5–2 s on a slow network) on a repeat-click without losing the "user-initiated → fresh plan info" guarantee — once a day per profile the heavier path still runs.
- After a successful per-card Refresh the front-end no longer re-fires `getCurrentLiveQuota`. The backend's `refresh_profile` already wrote fresh plan + quota into the profiles index; the client just re-reads the snapshot now. Drops one IPC round-trip + one JSONL cache pass per click.
- `chatgpt_api`'s blocking `reqwest::Client` is now built once per process and shared across calls. Cuts TLS-handshake / connection-setup overhead on bulk refresh (5-account batch saves 0.5–2 s) and on repeated single-card refreshes.
- `load_profiles_index` gains a 250 ms in-process result cache so the front-end's concurrent `get_profiles_snapshot` + `get_current_live_quota` IPC pair (issued every `refreshAllData` and on every dashboard mount) reconciles + writes `profiles.json` once instead of twice. Tests bypass the cache via `cfg(test)` so per-test fs setup remains observable without explicit invalidation.

## 1.5.8 - 2026-05-10

- Refresh fallback no longer burns user quota or runs an LLM round-trip. The legacy `codex exec "Reply with the single word OK."` path took 30–90 s and consumed real ChatGPT quota whenever the direct HTTP refresh failed (slow network, transient 401, GFW). It is now replaced by `codex app-server`'s JSON-RPC `account/read` + `account/rateLimits/read`, which return the same plan + rate-limit data in well under a second without touching the model. Requires `codex` ≥ 0.130.0 on this fallback path; older CLIs surface `APP_SERVER_METHOD_UNSUPPORTED` so the user can upgrade.
- Closed a same-profile race between Refresh and Login: clicking Refresh on a card whose login is already in flight is now a no-op, mirroring the existing reverse guard (`handleLoginProfile` already blocks Login when the same profile has a Refresh pending). Cross-profile Refresh during a login remains allowed.
- Tuned the app-server RPC fallback's per-method timeouts so slow-network users (the population the fallback exists for) don't trip `APP_SERVER_TIMEOUT` on a legitimately slow OAuth refresh. `account/read` (chains an OAuth refresh + account read server-side) now allows 25 s; `account/rateLimits/read` (single GET) gets 15 s to mirror `chatgpt_api`'s `HTTP_TIMEOUT`. Overall session ceiling raised to 60 s.
- Settings → Version row is now driven by `package.json` at build time (Vite-injected `__CODEX_APP_VERSION__`) instead of a hardcoded literal. The previous 1.5.0 string had drifted across many releases. A new `npm run version:check` script (also wired into the Linux CI job) refuses any future commit that puts a `\d+\.\d+\.\d+` literal back into the front-end HTML.
- 15-second dashboard quota ticker and every `refreshAllData` after a switch / login / refresh now short-circuit through a persistent JSONL parse cache at `<runtime_dir>/quota_cache.json`. On an idle dashboard with a multi-GB session corpus this drops the per-tick cost from "0.5–5 s of blocking I/O" to "~10 ms of stat calls": the cache reuses the previous winning snapshot whenever the lex-largest jsonl's `(mtime, size)` signature is unchanged. Slow-path scans also skip parsing files that haven't moved (including files previously confirmed as having no `token_count` event), so cold-cache rebuilds stay fast on big corpora. Cache is bounded at 64 entries with oldest-first pruning.
- Reorganised the Usage Guide page: the static three-card timeline is now an auto-fitting grid that scales to any step count, plus dedicated "Tips & warnings" callouts (info / warn / success accent stripes) and a collapsible "FAQ" section using native `<details>`.
- Hardened three macOS `discover_real_codex_cli_path_*` tests against parallel `HOME` / `PATH` env-var races by routing them through the existing `env_guard()` mutex.

## 1.5.7 - 2026-05-10

- **Critical** — fixed a latent hang in `codex login` cancellation. The previous PID-based cancel had a microsecond race window where a recycled PID could be SIGTERM'd between `wait_with_output` returning and the slot being cleared (worse on Windows where `taskkill /F /T` would nuke an unrelated process tree). The slot now holds the actual `Child` handle, and cancel calls `Child::kill()` directly. Both cancel and natural-exit paths funnel through a `drop_killed_child` helper that does `kill()` + `wait()` so we don't leak zombies on Unix.
- **Critical** — fixed a latent buffer-fill hang in the login poll loop. With piped stdout/stderr, a verbose `codex login` could fill its 64 KB pipe buffer and block on write, leaving our `try_wait` loop seeing `Running` forever. Stdio is now drained concurrently in dedicated threads. Regression tests cover 256 KiB through both stdout and stderr.
- Painted the per-card **Base** button red whenever a custom Base Url is set on that profile. Reuses the existing danger styling so the warning is consistent with the Delete button. Tooltip explains that ChatGPT / OAuth accounts will fail with a redirect and points to the workaround.
- Refactored `InstallState` + `RealCodexPathSource` + the four codex-CLI Tauri command wrappers into a shared module backed by a `CodexPathResolver` trait, removing the byte-identical mac/Windows duplication. No behavior change.
- a11y: login button now sets an explicit `aria-label` reflecting its dual role ("Log into <profile>" idle vs. "Cancel login for <profile>" while in flight), so screen readers that ignore `title` still announce the cancel semantics consistently.
- Bounded the macOS `suggested_codex_cli_paths` PATH walk by a 500 ms soft deadline. Fixed locations (Codex.app, Homebrew, npm-global, etc) are checked first; the remaining PATH walk bails after the deadline so the Codex CLI path dialog opens promptly even when PATH includes slow NFS / SMB drives.

## 1.5.6 - 2026-05-09

- Fixed the mojibake login toast on Windows: the previous `cmd /C codex login` fallback printed cmd.exe's GBK "command not found" message, which `String::from_utf8_lossy` mangled into U+FFFD replacement characters. `run_codex_login` now resolves the real codex path up-front and surfaces a typed `REAL_CODEX_NOT_FOUND` error; the front-end auto-opens a "Codex CLI path" dialog on this error.
- Added a manual override for the codex CLI path. The dialog lists common install locations (click to autofill), accepts a free-form path, and persists it as `user_codex_path` in `install_state.json` with priority over auto-discovery. If the override file disappears later, the resolver silently falls back to auto-discovery so users are never permanently wedged.
- Added a Settings → "Codex CLI path" row that always shows the resolved path plus its source label (manual override / cached / auto-discovered) and a Change button. The dialog is now reachable proactively, not only after a failure.
- Made the login button double as a cancel button while a login is in flight. Clicking the spinning button (now labelled "Cancel" in red) sends SIGTERM (Unix) / `taskkill /F /T` (Windows) to the codex login process, so closing the OAuth tab no longer leaves the app spinning forever.
- Fixed plan detection on downgrade so the cached quota is cleared when the API returns no quota, instead of showing stale numbers next to the new plan.

## 1.5.5 - 2026-05-08

- Added a plan-freshness tooltip on each profile card ("Plan tier confirmed N min ago"); cards stale by more than a local day are marked and trigger a bulk refresh at the next day rollover.
- Added an `unknown_paid` plan state so unrecognised paid tiers surface an explicit hint instead of a blank.
- Plan rotation is now driven by daily bulk refresh + force-rotation hooks, so accounts that aren't actively used still keep an up-to-date plan and `last_plan_check_ms`.
- Plan name now prefers the API `plan_type` field with a JWT fallback; stale "0 days" displays are dropped when the plan tier changes.
- Fixed the "switch already in progress" toast that occasionally got stuck after a switch (stale `.switch.lock` cleanup + double-click guard).
- Fixed the 5h / weekly quota windows occasionally cross-routing — windows are now selected by `window_minutes`, not slot position.
- Internal: split the plan-sync and quota-sync code paths (D1) and dropped the deprecated D2 endpoint plan.

## 1.5.4 - 2026-05-07

- Added per-card login: a profile can now be logged in directly from its card, without first switching to it. The login runs against a sandboxed `CODEX_HOME` and atomically copies the resulting `auth.json` into the target profile folder.
- Cherry-picked the direct ChatGPT-API quota refresh path from 1.6.x onto the 1.5 line so quota stays current without depending on a periodic codex CLI run.
- Fixed real-codex resolution to anchor on the live `~/.codex` instead of the sandboxed `CODEX_HOME`, so the managed-shim filter and install-state cache work correctly.

## 1.5.3 - 2026-04-27

- Added real update checks against the configured GitHub latest-release JSON endpoint.
- Added automatic new-version prompting when the latest release is newer than the running app.
- Fixed update version parsing so historical two-part tags such as `1.5` compare as `1.5.0` instead of failing.
- Added macOS `.pkg` packaging alongside `.app` and `.dmg`.
- Standardized future release records on full three-part semantic versions.

## 1.5.2 - 2026-04-21

- Windows installer uploaded as `codex_switch_1.5.2_x64-setup.exe`.
- Historical note: this asset was uploaded under the non-standard GitHub Release tag `1.5`.

## 1.5.1 - 2026-04-21

- Windows installer uploaded as `codex_switch_1.5.1_x64-setup.exe`.
- Historical note: this asset was uploaded under the non-standard GitHub Release tag `1.5`.

## 1.5.0 - 2026-04-20

- Added normalized installation and version-control release.
- Uploaded macOS DMG and Windows installer assets.
- Historical note: the GitHub Release tag was created as `1.5`; future release tags should use full semantic versions such as `1.5.3`.

## 1.4.2 - 2026-04-16

- Added the new local release after GitHub tag `1.4.1`.
- Added macOS drag-install DMG packaging and kept the generated `.dmg` beside the `.app`.
- Fixed macOS profile enumeration so the runtime `macos/` directory no longer appears as an empty account card.
- Improved macOS real `codex` CLI discovery for GUI refresh/login flows by falling back to the bundled `Codex.app` CLI when shell resolution is unavailable.
- Removed leftover preview mock account names from the shared frontend bridge.

## 1.4.1 - 2026-04-15

- Resolve merge conflicts.

## 1.4.0 - 2026-04-15

- Resolve merge conflicts.

## 1.3.2 - 2026-04-08

- 更新额度刷新流程和优化逻辑。
- 添加卡片 Base URL。
- 精简代码。

## 1.3.1 - 2026-04-08

- 更新额度刷新流程和优化逻辑。
- 添加卡片 Base URL。
- 精简代码。

## 1.2.4 - 2026-04-08

- 代码清理。

## 1.2.0 - 2026-04-08

- 移除 workflow。

## 1.1.5 - 2026-04-07

- 逻辑完善与界面美化。

## 1.1.4 - 2026-04-07

- 添加自动挂载 release。

## 1.1.0 - 2026-04-07

- Windows 优化完善。

## 0.1.1 - 2026-03-30

- Synced the repository script with the locally used production script.
- Removed the old auto-create profile behavior from the repository project.
- Kept the repository installer generic: non-switch commands continue to use the user's existing CLI.
- Updated README and implementation notes to require pre-created profile folders and `auth.json`.
- Removed local smoke-test instructions and the repository test script from the public project surface.

## 0.1.0 - 2026-03-04

- Initial standalone project extraction.

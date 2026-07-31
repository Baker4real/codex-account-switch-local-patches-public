---
type: report
title: "Cockpit Tools 第一组能力实施计划"
tags: [project-docs, report]
intent: "Cockpit Tools 第一组能力实施计划"
---

# Cockpit Tools 第一组能力实施计划

> 状态：已完成并归档（2026-07-13）；实现、测试、构建和部署证据见 [`docs/evidence/2026-07-13-cockpit-phase1-verification.md`](../evidence/2026-07-13-cockpit-phase1-verification.md)。

> **For agentic workers:** Implement this plan task-by-task. Prefer `subagent-driven-development` when available for independent tasks; otherwise use the repository's normal implementation workflow. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在已经完成的 Windows-only 基线上，补齐与 Codex Switch 定位一致的 Cockpit Tools 第一组能力：Windows GUI 下的 nvm/fnm/asdf Codex CLI 候选发现、账号搜索与动态套餐筛选、可见/总数提示，以及 additional/model-specific 配额的安全显示偏好。

**Architecture:** Windows 路径枚举继续留在 `src-tauri/win/runtime/process.rs`，并复用现有真实 `codex --version` 探测和超时上限。账号筛选作为 `src-tauri/shared/front` 的纯视图逻辑，只改变分页前的展示集合，不改变后端快照或账号写盘。additional 配额作为 `QuotaSummary` 的向后兼容中立数据契约，由 ChatGPT API 与 app-server 解析器分别填充；前端使用本地显示偏好控制是否展示，并保留来源、观测时间、未知和陈旧语义。

**Tech Stack:** Rust/Serde/Tauri IPC, TypeScript/Vite, 原生 HTML/CSS, Vitest, Cargo Windows MSVC。

## Global Constraints

- 只支持 Windows x64；不新增、恢复或验证 macOS/Linux 路径。
- 不复制 Cockpit Tools 源码、文案或组件；只参考固定提交 `da0deca4e7277d2bcfd62fcb674d56d85210bd7d` 的行为，并在研究文档中保留来源与许可证记录。
- 普通启动、筛选和显示偏好不得写入 `auth.json`、`config.toml` 或切换账号；显示偏好只允许写浏览器本地存储。
- 保留现有 Windows Store 包作用域生命周期、精确进程匹配、身份路由和事务回滚。
- 所有新数据字段使用 `serde(default)` 或可选字段兼容已有 `profile.json`、quota cache 和前端 mock。
- 不导出、记录或向子代理传播 access token、refresh token、密码、2FA 秘钥或完整 auth 内容。
- 每个任务必须先加入失败测试，再写最小实现；完成后重复 Windows 类型检查、前端测试、Rust 测试和 portable build。

## Task 1: 补齐 Windows GUI 下的 nvm/fnm/asdf 候选发现

**Files:**
- Modify: `src-tauri/win/runtime/process.rs`
- Test: `src-tauri/win/runtime/process.rs` 的 Windows 候选发现测试

**Interfaces:**
- 输入：当前进程环境中的 `NVM_HOME`、`NVM_DIR`、`NVM_BIN`、`NVM_SYMLINK`、`FNM_DIR`、`FNM_MULTISHELL_PATH`、`ASDF_DATA_DIR`、`USERPROFILE`、`LOCALAPPDATA`，以及已有 `CODEX_EXE`、`CODEX_INSTALL_DIR`、`APPDATA`。
- 输出：`suggested_codex_cli_paths` 返回去重、可执行后缀合法、排除 managed shim/WindowsApps alias 的候选；`redetect_runnable_codex_cli_paths` 仍对候选运行真实 `codex --version`。

- [x] **Step 1: 先写失败测试**

  用现有 `EnvVarGuard` 和临时目录构造 Windows 风格目录：

  - nvm-windows：`NVM_HOME\v20.11.1\codex.cmd` 与 `NVM_HOME\v22.14.0\codex.cmd`，并覆盖 `NVM_SYMLINK`/`NVM_BIN` 当前目录；
  - fnm：`FNM_DIR\node-versions\v20.11.1\installation\codex.cmd`，并覆盖 `FNM_MULTISHELL_PATH` 当前安装目录；
  - asdf：`ASDF_DATA_DIR\installs\nodejs\22.14.0\bin\codex.cmd`；
  - 同一文件通过两个环境根出现时只返回一次；空目录、文件、超出一层的目录不被递归扫描。

  断言版本目录按数值版本降序枚举、候选顺序稳定、managed shim 和 WindowsApps alias 仍被排除；让测试在尚未实现扫描时失败。

- [x] **Step 2: 实现受限的版本管理器扫描**

  在 `process.rs` 增加 Windows-only 私有 helper：

  - `push_nvm_codex_candidates`、`push_fnm_codex_candidates`、`push_asdf_codex_candidates` 分别接收 Windows 环境变量值；只读取版本根的直接子目录，不做全盘或深层递归，并设置每个版本根最多 32 个目录、全局候选仍受既有 12 个探测上限保护。
  - `collect_version_directories_newest_first` 按 `vMAJOR.MINOR.PATCH` 数值排序，无法解析的版本名按不区分大小写的稳定字典序排在后面。
  - nvm 扫描 `<NVM_HOME>\<version>\codex.{cmd,exe,bat,com}` 和 `<NVM_HOME>\<version>\node_modules\.bin\codex.{cmd,exe,bat,com}`；fnm 扫描 `<FNM_DIR>\node-versions\<version>\installation\...`；asdf 扫描 `<ASDF_DATA_DIR>\installs\nodejs\<version>\bin\...`。
  - 环境变量为空时使用 `%USERPROFILE%\.fnm`、`%LOCALAPPDATA%\fnm`、`%USERPROFILE%\.asdf` 作为 fnm/asdf 数据根；不猜测 Unix home、WSL 或 macOS 路径。
  - 将扫描结果接入现有 `suggested_codex_cli_paths`，复用既有 `resolve_windows_invokable_path`、`is_acceptable_real_codex_cli_path` 和路径去重，不改 `MAX_PROBE_CANDIDATES`、探测超时或进程隐藏逻辑。

- [x] **Step 3: 运行定向验证**

  `cargo test --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc process::tests`

  预期：新增测试和现有 `where` 超时、shim 修复测试全部通过；非 Windows 源树不被触碰。

## Task 2: 账号搜索、动态套餐筛选与可见/总数

**Files:**
- Create: `src-tauri/shared/front/profile-filters.ts`
- Create: `src-tauri/shared/front/profile-filters.test.ts`
- Modify: `src-tauri/shared/front/types.ts`, `state.ts`, `dashboard-view-model.ts`, `actions.ts`, `render.ts`, `i18n.ts`
- Modify: `src-tauri/win/front/index.html`, `src-tauri/win/front/styles.css`, `src-tauri/shared/front/base.css`

**Interfaces:**
- `ProfileFilters = { query: string; plan: string | null }`。
- 纯函数 `normalizeProfileFilterText`、`listProfilePlanOptions`、`filterProfiles`、`countActiveProfileFilters` 只读取 `ProfileCard`，不调用 IPC、不写文件。
- `DashboardViewModel` 增加 `profile_counts: { visible: number; total: number }`；`paging.total_profiles` 表示筛选后的可分页数量，`profile_counts.total` 始终表示快照中的总账号数。

- [x] **Step 1: 先写纯逻辑失败测试**

  覆盖大小写/空白归一化、账号标签/显示标题/文件夹名搜索、从完整快照动态生成套餐选项、空结果、活动筛选计数、筛选后分页总页数，以及清除筛选不改变原始 `snapshot.profiles`。

- [x] **Step 2: 实现视图模型和状态**

  在 `state` 增加 `profileFilters`，默认空搜索、`plan: null`。`buildDashboardViewModel` 先过滤后分页，并在页码越界时回到最后有效页；`applySnapshot` 只重置页码，不清理用户筛选。`rerenderDashboard` 仍只渲染当前页，`renderShellOverview` 继续使用未过滤的快照统计健康/缺失数量。

- [x] **Step 3: 接入 Windows UI**

  在 `page-profiles` 的标题和分页之间加入搜索输入、动态套餐 `select`、`显示 X / 共 Y` 文案、活动筛选 chip 和“清除”按钮。输入/change 事件更新状态并把页码设为 1；清除按钮只清理筛选状态。`applyLocale` 同步 aria-label、placeholder、选项文本和 chip 文案；CSS 只补 Windows profiles page 的布局，不新增平台选择器。

- [x] **Step 4: 运行前端定向验证**

  `npx vitest run --root . src-tauri/shared/front/profile-filters.test.ts` 与 `npx tsc --noEmit`。

  预期：筛选只影响视图，后端快照、切换、登录、刷新和写盘动作没有新增调用路径。

## Task 3: additional/model-specific 配额与显示偏好

**Files:**
- Modify: `src-tauri/shared/runtime/models.rs`, `chatgpt_api.rs`, `codex_app_server.rs`, `session_usage.rs`, `metadata.rs`, `profiles_index.rs`
- Create: `src-tauri/shared/runtime/additional_quota.rs`
- Modify: `src-tauri/shared/front/types.ts`, `render.ts`, `actions.ts`, `state.ts`, `tauri.ts`, `i18n.ts`, `quota-view-model.ts`, `quota-preferences.ts`
- Test: `src-tauri/shared/front/quota-view-model.test.ts`, `src-tauri/shared/front/quota-preferences.test.ts`
- Modify: `src-tauri/win/front/index.html`, `src-tauri/win/front/styles.css`, `src-tauri/shared/front/base.css`
- Test: parser tests in `chatgpt_api.rs`/`codex_app_server.rs` and a front-end pure rendering/normalization test module

**Interfaces:**
- 新增 `AdditionalQuotaWindow`：`id`、`label`、`limit_name`、`metered_feature`、`window_kind`、`remaining_percent`、`refresh_at`、`reset_at_timestamp`、`window_minutes`、`allowed`、`limit_reached`、`is_spark`；字段均使用 `Option` 或默认值兼容旧缓存。
- `QuotaSummary.additional: Option<Vec<AdditionalQuotaWindow>>` 默认 `None`，区分接口未返回 additional 与接口明确返回空数组；`QuotaProvenance.observed_at_ms` 继续作为观测时间，不把陈旧数据伪装成实时数据。
- ChatGPT API 解析 `additional_rate_limits[]` 的 `limit_name`、`metered_feature`、`rate_limit.allowed`、`rate_limit.limit_reached` 及 primary/secondary window；app-server 只在真实响应中出现同名 snake/camel allowlist 字段时解析，否则保留 `additional: None`，不猜测未证实的结构。

- [x] **Step 1: 先写 Rust 解析失败测试**

  使用固定 JSON 覆盖：一个普通 model-specific additional entry 的 primary/secondary 窗口、`used_percent`/reset/window duration 转换、缺失窗口字段保留未知状态、`spark`/`codex-spark`/`gpt-5.3-codex-spark` 标记为噪声、旧 payload 反序列化后 `additional` 为空。断言不保存 access token 或完整响应凭据字段。

- [x] **Step 2: 实现向后兼容数据契约和两个解析器**

  `QuotaSummary`、profile metadata、quota cache、session JSONL 归一化和 profile index 复制 `additional`，不改变 5h/weekly 的现有窗口路由。解析时只接受有限数值范围，异常百分比转为 `None` 而不是满额；unknown/缺失窗口保留为可显示的未知行。app-server 若没有 additional 字段只沿用旧路径，不因未知字段失败。

- [x] **Step 3: 实现本地显示偏好和 UI**

  增加 `showAdditionalQuota` 状态和 `codex-switch-show-additional-quota` localStorage 键，默认显示普通 additional 行；配额卡默认过滤 `is_spark` 行，关闭偏好后隐藏全部 additional 行，打开偏好时额外显示 Spark 行的“噪声”标签。读写 helper 返回成功/失败，失败时回滚内存状态并显示“偏好未持久化”提示，不触发后端写盘。数据缺失显示“未知”，`observed_at_ms` 超过 6 小时显示“过期”，不显示为 0% 或 100%。

- [x] **Step 4: 运行配额定向验证**

  `cargo test --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc chatgpt_api::tests codex_app_server::tests metadata::tests`

  `npx vitest run --root . src-tauri/shared/front/profile-filters.test.ts`（与配额视图纯函数测试一起运行）及 `npx tsc --noEmit`。

## Task 4: 全量验收、构建和远端部署

**Files:**
- Modify: `docs/evidence/2026-07-13-cockpit-phase1-verification.md`

- [x] **Step 1: 运行 Windows 验收门槛**

  `npm run version:check`

  `npx tsc --noEmit`

  `npm run test:front`

  `npm run check:windows-release-hygiene`

  `npm run check:rust:windows`

  `npm run test:rust:windows`

  `npm run tauri:build:windows:portable`

  记录每项命令的退出码、通过/失败数量和关键诊断；不得把未运行的检查写成通过。

- [x] **Step 2: 按现有安装规则替换本机 Windows 产物**

  从 release 输出解析真实安装目录，同时替换 `codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll`；启动后验证窗口响应、文件长度和 SHA-256。

- [x] **Step 3: 使用 `codex-gsd-codex-c-skill` 的 UU/Replicator 方法部署远端**

  先解析远端桌面快捷方式的真实 `TargetPath` 和 `WorkingDirectory`，再使用 `D:\Codex\projects\codex-gsd-codex-c-skill\scripts\Invoke-ReplicatorUuCommand.ps1`、对应 target manifest 和临时本地 HTTP staging 传输两个文件。远端只停止精确目标路径进程，保留备份，替换后独立探测窗口响应与哈希；完成后停止临时 HTTP 服务并确认端口关闭。不得复制到桌面路径字面值，不得修改 `remote*.ps1` 用户临时文件。

- [x] **Step 4: 固化证据并提交**

  写入本地/远端构建哈希、快捷方式解析、备份路径、探测结果、HTTP 端口关闭结果和剩余未跟踪用户文件；只提交项目文件与证据文档，不提交凭据、临时脚本或远端备份。

## 完成标准

- 第一组四项能力均有失败测试先行、实现和定向测试证据。
- `npm run test:front`、`npm run check:rust:windows`、`npm run test:rust:windows`、`npm run tauri:build:windows:portable` 全部通过。
- 账号筛选不改变底层集合；additional 缺失/异常/陈旧不会被误显示为满额；默认不显示 Spark 噪声。
- 本机和远端运行的 EXE/loader 哈希与构建产物一致，远端进程窗口可响应，传输临时服务已关闭。
- 没有恢复 macOS/Linux 代码、构建、发布入口，也没有引入 API Service、自动切号、多实例或敏感数据导出。

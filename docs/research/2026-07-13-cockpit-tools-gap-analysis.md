# Cockpit Tools 功能差距研究（2026-07-13）

## 结论先行

> 状态（2026-07-14）：第一组能力已经完成，具体实现计划与验收证据见 [`docs/plans/2026-07-13-cockpit-phase1.md`](../plans/2026-07-13-cockpit-phase1.md) 和 [`docs/evidence/2026-07-13-cockpit-phase1-verification.md`](../evidence/2026-07-13-cockpit-phase1-verification.md)；第二、三组仍是候选范围，不构成当前实施承诺。

截至 2026-07-13，`jlcodes99/cockpit-tools` 的最新稳定版是 **v1.3.0**（tag commit `da0deca4e7277d2bcfd62fcb674d56d85210bd7d`）。它已经从 Codex 切号器扩展成覆盖 17 类 AI 客户端的通用管理平台；本项目仍是刻意聚焦于 Codex 的轻量账号切换器。两者不是可以直接做“全功能同步”的同类产品。

建议只吸收与本项目现有边界一致、能独立实现和验证的 Codex 能力。第一组已经落地，具体包括：

1. Windows GUI 环境下补齐 `nvm` / `fnm` / `asdf` 的 Codex CLI 与 Node 探测。
2. 增加账号搜索、套餐筛选、可见数/总数、活动筛选提示和一键清除。
3. 将 additional / 模型专属配额纳入数据模型，并提供显示开关；默认隐藏 Spark 类噪声行。
4. 增加账号分组、标签与批量导入/导出，但必须保持现有身份路由和事务回滚约束。
5. 把会话预览、导入导出和废纸篓作为独立后续项目评估。

不建议把多平台账号管理、通用插件服务或整套 Codex API Service 直接移植进来。上游 README 声明默认采用 **CC BY-NC-SA 4.0**；直接复制源码会带来署名、非商业和相同方式共享义务。本报告只用于学习行为和独立实现，实施前应再次确认许可证兼容性。[上游 README（固定提交）](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/README.md)

## 研究基线与来源

- 最新发布：[Cockpit Tools v1.3.0](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.3.0)，发布时间为 2026-07-13（北京时间），对应[固定提交](https://github.com/jlcodes99/cockpit-tools/commit/da0deca4e7277d2bcfd62fcb674d56d85210bd7d)。
- 累计能力：[README @ v1.3.0](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/README.md)。
- 版本演进：[中文 CHANGELOG @ v1.3.0](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/CHANGELOG.zh-CN.md)、[v1.2.0](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.2.0)、[v1.1.5](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.1.5)。
- 架构证据：[package.json](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/package.json)、[Tauri 入口](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/src-tauri/src/lib.rs)、[Codex commands](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/src-tauri/src/commands/codex.rs)。

本地对比基线是当前工作区的 `README.zh-CN.md`、`LOCAL_PATCHES.md`、`docs/IMPLEMENTATION.md`、`src-tauri/shared/**` 与 `src-tauri/win/**`。Windows-only 平台裁剪已完成，当前不存在 active 的 `src-tauri/mac/**`。本地只读核查显示，当前项目已有账号身份路由切换、OAuth 登录、plan/5 小时/周额度、配额缓存、按账号 Base URL、CLI 自检、Windows Store 包作用域生命周期与事务回滚，以及证据分层的 OpenAI token/费用统计。

## 上游产品与架构

上游用户可见能力包括：17 类平台账号管理、OAuth/Token/JSON/本机导入、真实切号、配额和套餐监控、标签/筛选/批量操作、分组与备注、应用多开、仪表盘、唤醒任务、托盘/悬浮卡片、备份迁移、WebDAV 同步、自动更新和 18 种语言。[README 功能总览](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/README.md)

技术上游采用 React 19 + TypeScript/Vite/Zustand 前端和 Tauri 2 + Rust 后端，前端按 pages/components/services/stores 分层，Rust 按 commands/models/modules/utils 分层，IPC 在 `lib.rs` 集中注册。[依赖清单](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/package.json) [Tauri 入口](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/src-tauri/src/lib.rs)

与本项目不同，上游把大量 Windows/macOS/Linux 分支散布在共享 module 内的 `#[cfg(target_os = ...)]` 中，例如 [process.rs](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/src-tauri/src/modules/process.rs) 和 [codex_instance.rs](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/src-tauri/src/modules/codex_instance.rs)。本项目当前是 Windows-only，Windows 行为放在 `src-tauri/win/**`，只有磁盘格式、IPC contract 和确实相同的算法进入 shared；不存在 active 的 macOS runtime。

## 差距矩阵

| 能力 | 当前项目 | 上游 v1.3.0 | 判断与建议 |
|---|---|---|---|
| 基础账号切换 | 已有，且 Windows 身份路由和事务回滚更严格 | 已有 | 不替换现有实现；只吸收 UI/工作流改进 |
| plan、5 小时、周额度 | 已有官方 API/app-server fallback 与来源标识，并已纳入 additional/model-specific 配额与动态 plan 筛选 | 已有，并支持动态 plan、additional 配额、重置次数 | **第一组已完成**：继续保持来源、观测时间和未知状态语义 |
| CLI 探测 | 已有 PATH、官方目录、`where`、nvm-windows/fnm/asdf 扫描 | v1.3.0 明确补齐 GUI 下 nvm/fnm/asdf | **第一组已完成**：候选仍通过真实 `codex --version` 校验 |
| Windows 后台黑窗 | 已用 `CREATE_NO_WINDOW` 覆盖主要 process/login/update 路径 | v1.3.0 同样强化 | **已存在**：做覆盖审计即可，不重复开发 |
| 使用统计 | 已有 session/cc-switch 证据账本、账号归属、成本与趋势 | API Service 侧有请求日志、模型价格和估值 | **实现不同**：本项目证据约束更强；保留本地账本，未来 API source 只能作为新 adapter |
| 搜索/套餐/标签/分组/自定义排序 | 已支持账号搜索、动态套餐筛选、可见/总数、活动筛选提示和清除；标签/分组/排序仍缺失 | 完整支持，v1.3.0 增加可见/总数、筛选 chips 与清除 | **第一组已完成**：标签/分组/排序另立第二组 |
| 批量导入导出 | 缺少产品化批量工作流 | 支持批量、失败重试、可丢弃任务；导入可自动入组/API 池 | **确实缺失**：可做，但敏感字段默认不得导出，导入必须走身份去重 |
| 应用多开 | 不具备独立 Codex 实例管理 | 每实例独立账号、目录、参数与生命周期 | **确实缺失但高风险**：需单独设计，不能破坏 Windows Store 包生命周期 |
| 自动唤醒 | 无调度产品能力 | 有任务、调度、历史、验证 | **确实缺失但目标可疑**：它会主动消耗额度，不应默认加入；须用户明确选择 |
| 会话管理 | 只读取 sessions/archived_sessions 作统计，不改变会话 | 支持跨实例列表、预览、导入导出、废纸篓恢复/永久删除 | **确实缺失且涉及用户数据**：只能作为独立高风险项目，先做只读预览 |
| 模型供应商 | 仅每账号 Base URL 与基础 config | 自定义 provider、动态模型发现、别名映射、per-provider WebSocket | **部分重叠但产品范围不同**：先明确是否要成为 provider 管理器 |
| Codex API Service | 没有本地 OpenAI-compatible gateway/sidecar | 账号池、API Key、HTTP/WebSocket、路由、日志、额度保护、备用账号 | **巨大缺口，不宜顺手移植**：是独立产品级子系统 |
| 其他 AI 平台 | 不支持 | 17 类平台 | **不应移植**：违背 Codex 专用定位和平台隔离规则 |
| 托盘/悬浮卡片/WebSocket 插件 | 未形成通用宿主服务 | 已有 | **暂不移植**：常驻进程、端口与安全面显著扩大 |

## v1.3.0 中最值得吸收的细节

发布页明确记录了以下 Codex 改进：[v1.3.0 release](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.3.0)。

- 账号导入后可按偏好加入 API 池；对本项目可转化为“导入后进入当前分组”，但不应引入 API 池概念。
- API 服务支持备用账号；本项目目前明确“低额度只提示、不自动切换”，因此不能直接转化为自动切号。
- additional/model-specific 配额可显示或隐藏，默认隐藏 Spark 噪声行；这是和现有额度卡片最匹配的增量。
- plan 筛选改为动态值，而不是硬编码套餐枚举；本项目应采用相同原则。
- 账号筛选展示“显示 X / 共 Y”、活动筛选类型和一键清除；能避免用户误以为账号丢失。
- GUI 启动时扫描 nvm/fnm/asdf 的 Codex CLI/Node；能补足 Windows GUI 环境 PATH 与登录 shell 不一致的问题。
- 启动应用不再自动改写 `config.toml`/`auth.json`；这与本项目“显式动作才改变账号态”的安全边界一致，应作为回归门槛。
- 长对话线性规范化和 HTTP transport 复用属于 API Service 内部优化；本项目没有对应请求代理链路，无需移植。

## 平台边界

上游官方支持 macOS、Windows、Linux，但并非所有功能对称：[README](https://github.com/jlcodes99/cockpit-tools/blob/da0deca4e7277d2bcfd62fcb674d56d85210bd7d/README.md)。v1.2.0 还明确指出 Codex 应用内无痕 OAuth 只适用于 macOS，Windows/Linux 使用普通浏览器与手动回调。[v1.2.0 release](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.2.0)

对本项目的约束如下：

- Windows：保留 Store-package-scoped 生命周期、路径精确进程匹配和事务回滚；不要引入上游的通用进程关闭逻辑。
- macOS/Linux：当前项目不支持，active 源码、构建、CI 和发布路径已经退役；历史记录仅作为历史事实保留，不构成支持承诺。
- shared：仅放账号/配额 DTO、IPC 参数、磁盘 schema、纯筛选算法等中立 contract。

## 推荐实施分组

第一组已经完成；以下顺序保留后续能力的优先级。第二、三组必须分别设计、实现和验收，不能因为研究报告列出能力就视为已交付。

### 第一组：低风险、与现有产品一致

1. Windows `nvm` / `fnm` / `asdf` CLI/Node 候选发现；验证仍使用真实 `codex --version`。
2. 账号搜索、动态套餐筛选、可见/总数、活动筛选 chips、清除筛选。
3. additional 配额数据模型与显示偏好；保持来源、观测时间和未知状态表达。

### 第二组：中风险账号组织能力

1. 标签、分组、自定义排序。
2. JSON 批量导入/导出、预览、失败条目重试。
3. 导入的账号身份去重、空占位允许规则，以及敏感 token 的默认排除。

### 第三组：必须另立设计与验收的功能

1. 多实例管理。
2. 会话预览、导入导出和废纸篓。
3. 唤醒任务。
4. 模型供应商管理。
5. Codex API Service。

后续各组不能同时实施。第一组完成后，应重新核对用户真正需要的是“更好用的 Codex Switch”，还是要把项目转成通用 Codex 控制台；后者会实质改变产品、数据与安全边界。

## 不应移植的内容

- Grok、ZCode、Trae、Cursor、Windsurf 等非 Codex 平台管理。
- 上游 React/Tailwind/DaisyUI 前端栈整体替换；当前项目的原生 Windows 前端已有稳定结构。
- 上游散布在共享 modules 中的 `cfg(target_os)` 组织方式。
- 未经许可证评估直接复制上游源码、文案、图标或组件。
- 自动低额度切号。当前项目把低额度定义为风险提示，不是授权自动变更用户会话。
- 为追求“功能一致”而放松现有身份路由、原子写入、Store 包作用域生命周期或回滚保证。

## 建议验收门槛

- 所有新写盘动作必须先证明目标账号身份，不能只依赖 `.current_profile` / `.active_profile`。
- 应用普通启动不得改写 `auth.json` 或 `config.toml`。
- Windows 新进程路径必须隐藏控制台、限定目标进程并覆盖失败/回滚测试。
- 筛选不能改变底层账号集合；UI 必须同时显示可见数与总数。
- additional 配额缺失、异常或陈旧时必须显示未知/过期，不能当作满额。
- 批量导出默认不得包含 access token、refresh token、密码或 2FA 秘钥。
- 任何会话删除能力必须先提供预览、可恢复废纸篓和明确的永久删除确认。

---
type: report
title: "OpenAI 官方 Token 使用统计 Implementation Plan"
tags: [project-docs, report]
intent: "OpenAI 官方 Token 使用统计 Implementation Plan"
---

# OpenAI 官方 Token 使用统计 Implementation Plan

> 状态：已完成并归档；所有实施任务已勾选，本文保留功能、约束和验收记录。

> **For agentic workers:** Implement this plan task-by-task. Prefer `subagent-driven-development` when available for independent tasks; otherwise use the repository's normal implementation workflow. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增一个只统计 OpenAI 官方 Codex token 消耗的“使用统计”模块，并在 UI 中按账号、模型和费用展示。

**Architecture:** 后端新增只读 usage 聚合模块，扫描本地 Codex session JSONL，先通过 base URL 官方门禁和 profile 归属，再做累计差分、去重、价格估算。前端在现有 TypeScript DOM 渲染体系中新增 `usage` route，调用 Tauri command 获取聚合结果并渲染数据密集 dashboard。

**Tech Stack:** Rust/Tauri command、serde JSONL 解析、现有 `shared/runtime` profile/index 工具、现有 `shared/front` TypeScript DOM 渲染、CSS。

**Status:** Completed on 2026-07-07 in commit `c0d3098` and superseded for CC Switch parity details by `docs/plans/2026-07-07-openai-usage-ccswitch-parity.md`.

## Global Constraints

- 只统计 OpenAI 官方 token 消耗；不使用 `model_provider` 判定官方来源。
- 以有效 `base_url` 作为官方来源门禁；无法证明官方来源的历史记录不计入官方总额。
- 保留去重机制，重复扫描不能改变 token 或价格。
- 价格按模型分开计算，未知模型 token 可展示但费用必须标记为未定价。
- 新模块导航位置为“账号”和“设置”之间。
- Windows/macOS 平台隔离规则保持不变；本功能优先放在 shared 层，仅做中性只读统计。

---

### Task 1: 后端模型、价格表和 token 计费内核

**Files:**
- Modify: `src-tauri/shared/runtime/models.rs`
- Create: `src-tauri/shared/runtime/openai_usage.rs`
- Modify: `src-tauri/shared/runtime/mod.rs`

**Interfaces:**
- Produces: `load_openai_usage_summary(codex_home: Option<&Path>) -> AppResult<OpenAiUsageSummary>`
- Produces serializable front-end types: `OpenAiUsageSummary`, `OpenAiUsageAccountSummary`, `OpenAiUsageModelSummary`, `OpenAiUsageExclusions`

- [x] 添加 usage 响应结构体和价格结构。
- [x] 添加内置 OpenAI 价格表，使用 micro-dollar per million tokens。
- [x] 添加费用公式：fresh input、cached input、output 分开计费。
- [x] 添加单元测试覆盖 cached input、reasoning 不重复计费、未知模型未定价。

### Task 2: session JSONL 解析、差分和去重

**Files:**
- Modify: `src-tauri/shared/runtime/openai_usage.rs`

**Interfaces:**
- Consumes: Task 1 的价格与响应结构。
- Produces: session event aggregation internals。

- [x] 扫描 `sessions` 和 `archived_sessions` 下 JSONL。
- [x] 从 `session_meta` 读取 session id。
- [x] 从 `turn_context` 读取当前 model。
- [x] 从 `event_msg` / `token_count` 读取累计 token 并按 session 差分。
- [x] 用 `request_id` 和内容指纹去重。
- [x] 添加单元测试覆盖重复扫描、累计差分、负差分跳过。

### Task 3: 官方 base URL 门禁和账号归属

**Files:**
- Modify: `src-tauri/shared/runtime/openai_usage.rs`

**Interfaces:**
- Consumes: profile index、profile metadata、profile auth identity。
- Produces: account/model aggregation and exclusion counters。

- [x] 加载 profiles index，默认按已管理 profile 归属。
- [x] 官方判定只看 profile 的有效 `openai_base_url` 和 ChatGPT/OAuth 默认路径。
- [x] 自定义非官方 base URL 进入排除计数。
- [x] 无法归属或无法证明官方的历史记录不计入官方统计。
- [x] 添加测试覆盖官方空 base URL、非官方自定义 base URL、未知归属排除。

### Task 4: Tauri command 和前端 API mock

**Files:**
- Create: `src-tauri/shared/commands/usage.rs`
- Modify: `src-tauri/shared/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/shared/front/types.ts`
- Modify: `src-tauri/shared/front/tauri.ts`

**Interfaces:**
- Produces command: `get_openai_usage_summary`
- Produces front-end API: `getOpenAiUsageSummary(): Promise<OpenAiUsageSummary>`

- [x] 注册 Tauri command。
- [x] 在非 Tauri preview 环境提供 mock usage 数据。
- [x] TypeScript 类型与 Rust serde 字段保持 snake_case 对齐。

### Task 5: 前端路由、页面渲染和样式

**Files:**
- Modify: `src-tauri/shared/front/types.ts`
- Modify: `src-tauri/shared/front/state.ts`
- Modify: `src-tauri/shared/front/i18n.ts`
- Modify: `src-tauri/shared/front/render.ts`
- Modify: `src-tauri/shared/front/actions.ts`
- Modify: `src-tauri/win/front/index.html`
- Modify: `src-tauri/shared/front/base.css` 或 `src-tauri/win/front/styles.css`

**Interfaces:**
- Consumes: `OpenAiUsageSummary`
- Produces: `usage` route between `profiles` and `settings`

- [x] 将 `ShellRoute` 扩展为 `usage`。
- [x] 导航 HTML 插入“使用统计”。
- [x] 页面包含总 token、总价、缓存命中率、输入/缓存/输出、去重/排除摘要、模型表、账号表。
- [x] 数据为空时展示“暂无可证明的官方 OpenAI 使用记录”。
- [x] 样式遵守现有浅色桌面工具风格和 44px 触控目标。

### Task 6: 验证和对抗性审查

**Files:**
- No direct production file unless review finds bugs.

**Interfaces:**
- Consumes: Tasks 1-5 implementation.
- Produces: test and review evidence.

- [x] 运行 `npm run build`。
- [x] 运行 `npm run test:rust:windows` 或可用的窄范围 Rust test。
- [x] 启用子代理对抗性审查：重点攻击 base URL 官方门禁、历史归属、去重、价格公式、UI 误导性展示。
- [x] 修复审查发现的问题。

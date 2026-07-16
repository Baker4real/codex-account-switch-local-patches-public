# Usage deep modules 设计

## 状态

- 日期：2026-07-10
- 结论：已实施并完成验证；本文保留为两项 deep module 的历史设计依据。当前产品仅支持 Windows x64，涉及 macOS runtime 的表述只反映实施当时的架构背景。
- 范围：完成架构扫描中全部“强烈推荐”候选

## 目标

在不改变公开上游核心功能和现有 Usage 用户语义的前提下，完成两项定向深化：

1. 把 Usage 前端的状态、异步时序、分页和渲染绑定收进一个纵向 deep `module`。
2. 把 Usage evidence 的 session JSONL、cc-switch 与 future response 来源接入真实 adapter `seam`。

完成后，调用者通过更小的 interface 获得相同行为；Usage 变化的知识、错误和测试集中到所属 module，提高 `locality` 与 `leverage`。

## 硬约束

- 公开上游账号快照、quota、登录、刷新、增删改、Base URL、CLI 路径与账号切换 command interface 不得删减、禁用、回退或被本地实现替换。
- 不改变现有 Tauri Usage command 名称、参数与返回模型。
- 不改变 `openai_usage.sqlite` schema、重建条件、证据分层、去重、定价或 official Base URL 判定语义。
- 不把 provider sync 接入账号切换或 Usage 默认流程。
- 本设计不扩大共享平台行为；当前 Windows-only runtime 边界以仓库 `AGENTS.md` 为准。
- 不修改账号、设置、主题、语言、登录和切换页面的用户行为。
- 不引入新的前端状态框架或通用插件系统。

## 第一性原理

### 可观察目标

- Usage 页面在相同输入下展示与当前版本等价的摘要、筛选项、趋势、日志、数据源和错误状态。
- 快速连续筛选时，过期响应不能覆盖最新意图。
- 手动刷新进行中再次刷新时只排队一次；自动刷新不得制造额外队列。
- 日志页超出总页数时自动纠偏并重新加载正确页。
- 每种 evidence 来源都通过同一个 ingestion seam 产生统一 evidence，ledger 不再直接知道来源发现和采集细节。
- 现有 Rust ledger/dashboard 回归测试全部继续通过。

### 失败模式

- 把 Usage 字段从全局 state 移走后，渲染或事件处理仍读取旧字段，造成页面静默失效。
- timer 或异步请求在路由离开后继续写状态。
- stale request 判定遗漏分页或筛选意图，旧数据覆盖新数据。
- adapter 重构改变 source priority、错误归类、source locator/hash 或证据顺序。
- 为了拆文件而扩大 interface，使调用者仍需知道 implementation 的排序约束。
- 修改共享前端文件时误伤上游账号页、设置页或公共渲染流程。

## 方案比较

### 方案 A：定向深化（采用）

新增 Usage 纵向 module 与真实 evidence adapters，保持现有 IPC、Rust 数据模型和其他页面行为不变。改动集中、可逐步回滚，能真实缩小 interface。

### 方案 B：横向拆文件（拒绝）

仅把 state、actions、render、parser 分到更多文件。文件会变短，但调用顺序、全局字段和时序知识仍跨 module 泄漏，无法通过 deletion test。

### 方案 C：整体重写（拒绝）

引入新 store、前端框架或通用 evidence 插件注册系统。扩展面大于当前真实变化，增加依赖和迁移风险，不符合 YAGNI。

## 前端架构

### Module 位置

新增 `src-tauri/shared/front/usage/`。目录作为一个 module，对外由 `index.ts` 暴露最小 interface；目录内文件是 implementation 细节。

建议内部职责：

- `workspace.ts`：拥有 Usage 状态、异步请求、stale response、刷新队列、分页纠偏和 timer 生命周期。
- `model.ts`：保存仅供 module 内部使用的状态与只读 view snapshot 类型。
- `render.ts`：拥有 Usage 专属 markup、图表、表格、筛选器和证据视图生成。
- `index.ts`：唯一公开入口，隐藏内部文件布局。

如果实施时能在不扩大 interface 的情况下进一步合并文件，应优先保持 module 简单，而不是机械遵守文件数量。

### Frontend interface

对外 interface 只表达调用者真实需要知道的事实：

- 创建 workspace，并注入 Usage dashboard adapter、时钟、timer 与变更通知。
- 派发用户或生命周期事件。
- 读取不可变 view snapshot 供渲染使用。
- 销毁 workspace，停止 timer 并使未完成请求失效。

实施计划使用以下稳定形状；允许根据现有命名风格调整类型字段，但不得扩大调用者必须理解的状态：

```ts
export interface UsageWorkspace {
  dispatch(event: UsageWorkspaceEvent): Promise<void>;
  snapshot(): Readonly<UsageWorkspaceView>;
  dispose(): void;
}

export function createUsageWorkspace(
  dependencies: UsageWorkspaceDependencies,
): UsageWorkspace;
```

`UsageWorkspaceDependencies` 只包含 dashboard loader、clock/timer adapters、状态变更回调与用户可见错误回调。它不得暴露 Tauri 全局对象或共享全局 state。

事件覆盖：范围 preset、自定义范围草稿与提交、筛选、scope、日志分页、手动刷新、自动刷新、路由进入和路由离开。具体字段、request serial、刷新排队标记和分页纠偏顺序全部属于 implementation，不进入调用者 interface。

### Adapter seam

现有 Tauri IPC 与 preview mock 是两个真实 adapters，继续复用现有 `getOpenAiUsageDashboard` 返回模型。Workspace 只依赖“加载 dashboard”能力，不依赖 Tauri 全局对象或 mock 分支细节。

### 数据流

1. 路由或 DOM handler 把意图转换成 workspace 事件。
2. Workspace 更新内部状态并生成当前查询。
3. Workspace 通过注入的 adapter 加载 dashboard。
4. 返回时同时校验 request serial 与完整 intent；过期响应直接丢弃。
5. 有效响应写入内部 state；若日志页越界，纠偏后只重试一次。
6. Workspace 发布只读 view snapshot；Usage renderer 只消费 snapshot。
7. 路由离开时销毁 workspace 或停用自动刷新，未完成响应不得再更新视图。

### 错误处理

- 加载错误写入 view snapshot，保留现有可见错误文案来源。
- 手动刷新失败继续允许外层显示 toast；自动刷新失败不重复打扰用户。
- 手动刷新进行中收到新的手动刷新时合并为一次队列；自动刷新直接跳过。
- 销毁后的 promise 完成不得触发状态或 DOM 更新。

## 后端架构

### Module 位置

新增 `src-tauri/shared/runtime/openai_usage/ingestion.rs`，作为 `openai_usage.rs` 的内部 module。外部仍只通过现有 Usage load functions 和 Tauri commands 使用 ledger。

### Ingestion interface

Ingestion module 负责：

- 定义统一 evidence adapter interface。
- 按既有确定顺序调用所有真实 adapters。
- 汇总统一 `UsageEvidence`，并把来源级错误映射为现有 `AppError` 行为。

Ledger 只调用一个父 module 可见的入口：

```rust
pub(super) fn ingest_usage(
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
    cache_probe: impl FnOnce(
        &str,
        &mut OpenAiUsageSummary,
    ) -> AppResult<bool>,
) -> AppResult<UsageLoadResult>;
```

父 ledger 先打开 SQLite 并完成 schema/migration，再向 ingestion 注入只接收 `source_signature` 的窄 cache probe。Ingestion 仍独占 source discovery/signature，并在任何 adapter collect 前调用 probe；cache hit 时立即返回。该注入不暴露 `Connection`，也不允许 ingestion 直接操作 schema、migration、sync state 或 projection。

具体 `UsageEvidenceAdapter` trait、adapter 列表和 ingestion context 均留在 ingestion implementation 内，不进入 `openai_usage` 的外部 interface。

三个 adapters：

- session JSONL adapter：发现 session 文件、读取事件并生成 session evidence。
- cc-switch adapter：定位并冻结数据库，读取 proxy/rollup 数据并生成 evidence。
- future response adapter：当前返回空集合，但真实实现同一 interface，为未来直接 response usage 保留位置。

### Ledger 职责

Ledger implementation 继续独占：

- evidence 分类与 layer 决策；
- request id / fingerprint 去重与 priority resolution；
- SQLite schema、migration、sync state 与 projection；
- account attribution、summary、trend、logs 和 pricing。

来源发现、冻结、采集、source type 和 source priority 从 ledger orchestration 移入 ingestion module。不得创建通用动态注册系统。

### 兼容要求

- Adapter 顺序和 source priority 与当前实现一致。
- `source_locator`、`source_hash`、`evidence_id` 与 decision reason 的现有稳定语义保持不变。
- cc-switch snapshot 路径、只读冻结和 strict/candidate 判定保持不变。
- Future adapter 继续不产生事实，但必须与其他 adapters 走同一 seam。

## 测试设计

### 前端 test surface

新增 Vitest 作为 devDependency。测试只通过 workspace public interface 和 fake adapters 观察行为，不读取私有字段、不 mock 内部函数。

必须覆盖：

1. preset 与自定义范围生成正确查询。
2. 较早请求较晚返回时不能覆盖最新 intent。
3. 手动刷新在 in-flight 时合并为一次排队刷新；自动刷新不排队。
4. 日志页越界时纠偏并只重载一次。
5. 销毁后忽略未完成响应并停止 timer。

渲染 implementation 由 TypeScript 类型检查、production build 与必要的纯 markup 测试验证；本轮不引入 Playwright 或 DOM 模拟依赖。

### 后端 test surface

保留现有 ledger/dashboard tests，并通过 ingestion interface 增加 fixture tests：

1. 所有三个 adapters 按固定顺序参与采集。
2. session JSONL adapter 保持现有 source metadata 与 official attribution 行为。
3. cc-switch proxy/rollup adapter 保持冻结、identity 与 candidate/strict 语义。
4. future response adapter 当前返回空 evidence。
5. adapter 汇总后现有 priority/dedupe 回归结果不变。

### 验证命令

- 前端定向测试：新增 `npm` test script 运行 Vitest。
- 前端类型与 production build：`npm run build -- --mode windows`。
- Rust 定向测试：对 ingestion 与 openai usage 相关测试使用 `cargo test --manifest-path src-tauri/Cargo.toml <filter>`。
- Rust 完整测试：`npm test`。
- Windows 相关前端/Tauri 构建遵守项目规则，使用 Windows mode；本设计不要求 macOS build。

## 实施顺序

1. 先用 Vitest 建立 workspace tracer bullet，再迁移查询、异步与刷新状态机。
2. 迁移 Usage 专属渲染和事件绑定，删除全局 Usage state 泄漏。
3. 用 Rust fixture 建立 adapter tracer bullet，再接入 session、cc-switch 与 future adapters。
4. 保持 ledger public interface 不变，运行完整回归。
5. 对共享前端、证据语义和上游兼容性进行独立对抗性审查。

## 非目标

- 不实现新的 Usage 功能、图表、筛选或证据来源。
- 不改变数据库 schema 或迁移版本。
- 不重构账号切换、profile mutation、Edge 登录或 portable artifact pipeline。
- 不引入 Redux、Zustand、Vue、React、Playwright、jsdom 或动态插件注册系统。
- 不发布、部署或替换本机/远端安装版。

## 完成标准

- Usage 前端不再把专属状态平铺在共享全局 `state.ts`。
- 查询、stale response、刷新队列、分页纠偏和 timer 只由 Usage workspace implementation 拥有。
- Usage 专属渲染不再占据共享 `render.ts` 的大段实现。
- session JSONL、cc-switch 和 future response 都实现同一 evidence adapter interface。
- Ledger orchestration 不再直接调用来源特定 collector。
- 新增 Vitest 与 Rust adapter tests 先红后绿，现有测试全部通过。
- `npm run build -- --mode windows` 与 `npm test` 通过。
- 公开上游核心 command、磁盘格式和非 Usage 页面行为无回退。

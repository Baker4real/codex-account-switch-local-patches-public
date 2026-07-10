# Usage Deep Modules 实施计划

> **For agentic workers:** Implement this plan task-by-task. Prefer `subagent-driven-development` when available for independent tasks; otherwise use the repository's normal implementation workflow. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在保持公开上游核心功能和现有 Usage 语义不变的前提下，完成 Usage 前端纵向 deep module 与真实 evidence adapter seam。

**Architecture:** 前端新增 `@front-shared/usage` module，以 `UsageWorkspace` 小 interface 隐藏范围状态、异步时序、刷新队列、分页和 timer，并把 Usage 专属 DOM binding/rendering 收进同一 module。后端新增 `openai_usage::ingestion` 内部 module，以一个父 module 可见入口隐藏 session JSONL、cc-switch、future response 三个真实 adapters，ledger 继续拥有分类、去重、schema、projection 与 pricing。

**Tech Stack:** TypeScript 5.9、Vite 7.3.6、Vitest 4.1.10、Rust 2021、Tauri 2.10、rusqlite 0.32。

## Global Constraints

- 公开上游账号快照、quota、登录、刷新、增删改、Base URL、CLI 路径与账号切换 command interface 不得删减、禁用、回退或被本地实现替换。
- 不改变现有 Tauri Usage command 名称、参数与返回模型。
- 不改变 `openai_usage.sqlite` schema、重建条件、证据分层、去重、定价或 official Base URL 判定语义。
- 不把 provider sync 接入账号切换或 Usage 默认流程。
- Windows 与 macOS 平台 runtime 继续独立；本设计不扩大共享平台行为。
- 不修改账号、设置、主题、语言、登录和切换页面的用户行为。
- 不引入新的前端状态框架、DOM 模拟依赖或通用插件系统。
- `npm test` 继续表示完整 Rust suite；新增前端入口必须命名为 `npm run test:front`。
- 只提交本任务文件；不得暂存或提交用户现有 `.gitignore`、`AGENTS.md` 改动。
- Windows 前端构建使用 `npm run build -- --mode windows`；不运行需要编译 `src-tauri/mac/**` 的验证路径。

---

### Task 1：用 Vitest 建立 Usage workspace interface

**风险：** 中。新增测试工具和本地纯状态 module，尚不接管现有 DOM。

**Files:**

- Modify: `package.json`
- Modify: `package-lock.json`
- Create: `src-tauri/shared/front/usage/model.ts`
- Create: `src-tauri/shared/front/usage/workspace.ts`
- Create: `src-tauri/shared/front/usage/index.ts`
- Test: `src-tauri/shared/front/usage/workspace.test.ts`

**Interfaces:**

- Consumes: `getOpenAiUsageDashboard(query, page, pageSize): Promise<OpenAiUsageDashboard>` 的既有数据形状。
- Produces:

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

- `UsageWorkspaceDependencies` 只包含 `loadDashboard`、`now`、`setInterval`、`clearInterval`、`onChange`、`onError`。
- `UsageWorkspaceEvent` 至少覆盖 `set-context`、`set-range`、`set-filters`、`set-page`、`set-refresh-interval`、`refresh`。
- `refresh.mode` 固定为 `initial | manual | auto`；只有 `manual` 错误要求用户可见通知。

- [x] **Step 1: 安装并固定 Vitest 4.1.10**

Run:

```powershell
npm install --save-dev --save-exact vitest@4.1.10
```

修改 `package.json` scripts，保留现有 `test` 不变：

```json
{
  "test": "cargo test --manifest-path src-tauri/Cargo.toml",
  "test:front": "vitest run"
}
```

Expected: `package-lock.json` 锁定 Vitest；`npm test` 仍调用 Cargo。

- [x] **Step 2: 写第一个失败测试——preset/custom range 只通过 workspace interface 生成 query**

在 `workspace.test.ts` 使用 fake loader 捕获调用参数：

```ts
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const zeroTokens = {
  input_tokens: 0,
  cached_input_tokens: 0,
  output_tokens: 0,
  reasoning_output_tokens: 0,
  real_total_tokens: 0,
};

const zeroCost = {
  input_cost_micros: 0,
  cached_input_cost_micros: 0,
  output_cost_micros: 0,
  total_cost_micros: 0,
  unpriced_tokens: 0,
};

function emptyDashboard(): OpenAiUsageDashboard {
  return {
    summary: {
      generated_at_ms: 0,
      attribution_since_ms: null,
      range_start_ms: null,
      range_end_ms: null,
      total_requests: 0,
      tokens: { ...zeroTokens },
      cost: { ...zeroCost },
      cache_hit_rate: null,
      accounts: [],
      models: [],
      exclusions: {
        duplicate_events: 0,
        non_official_base_url: 0,
        unproven_history: 0,
        unattributed: 0,
        missing_model: 0,
        invalid_delta: 0,
        malformed_lines: 0,
        accepted_events: 0,
        candidate_events: 0,
        rejected_events: 0,
        superseded_events: 0,
        strict_proven_events: 0,
        legacy_profile_attributed_events: 0,
        candidate_unattributed_events: 0,
        rejected_invalid_events: 0,
      },
      scope: "strict",
      layer_summaries: [],
      pricing_source: "OpenAI API pricing",
      pricing_source_url: "https://developers.openai.com/api/docs/pricing",
      pricing_checked_at: "2026-07-08",
    },
    account_summary: [],
    model_options: [],
    trends: [],
    logs: { data: [], total: 0, page: 1, page_size: 20 },
    data_sources: [],
  };
}

function dashboardWithRequests(totalRequests: number): OpenAiUsageDashboard {
  const dashboard = emptyDashboard();
  dashboard.summary.total_requests = totalRequests;
  return dashboard;
}

function dashboardWithPage(input: { page: number; pageSize: number; total: number }): OpenAiUsageDashboard {
  const dashboard = emptyDashboard();
  dashboard.logs = { data: [], total: input.total, page: input.page, page_size: input.pageSize };
  return dashboard;
}

function fakeTimers() {
  let nextId = 1;
  const active = new Map<number, () => void>();
  return {
    setInterval: (handler: () => void) => {
      const id = nextId++;
      active.set(id, handler);
      return id;
    },
    clearInterval: (id: unknown) => active.delete(Number(id)),
    activeCount: () => active.size,
  };
}

function fakeDependencies(
  overrides: Partial<UsageWorkspaceDependencies> = {},
): UsageWorkspaceDependencies {
  const timers = fakeTimers();
  return {
    loadDashboard: async () => emptyDashboard(),
    now: () => new Date(2026, 6, 10, 9, 0, 0).getTime(),
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
    onChange: vi.fn(),
    onError: vi.fn(),
    ...overrides,
  };
}

it("把 preset 与自定义范围转换成 dashboard query", async () => {
  const calls: OpenAiUsageQuery[] = [];
  const nowMs = new Date(2026, 6, 10, 9, 0, 0).getTime();
  const workspace = createUsageWorkspace(fakeDependencies({
    now: () => nowMs,
    loadDashboard: async (query) => {
      calls.push(query);
      return emptyDashboard();
    },
  }));

  await workspace.dispatch({ type: "set-range", preset: "7d" });
  await workspace.dispatch({ type: "refresh", mode: "initial" });
  expect(calls.at(-1)).toMatchObject({
    scope: "strict",
    start_date: Math.floor(new Date(2026, 6, 4, 0, 0, 0).getTime() / 1000),
    end_date: Math.floor(nowMs / 1000),
  });

  await workspace.dispatch({
    type: "set-range",
    preset: "custom",
    startMs: 1_700_000_000_000,
    endMs: 1_700_003_600_000,
    liveEnd: false,
  });
  await workspace.dispatch({ type: "refresh", mode: "manual" });
  expect(calls.at(-1)).toMatchObject({
    start_date: 1_700_000_000,
    end_date: 1_700_003_600,
  });
});
```

- [x] **Step 3: 运行测试并确认 RED**

Run: `npm run test:front -- workspace.test.ts`

Expected: FAIL，原因是 `createUsageWorkspace`/module 尚不存在，而不是测试基础设施错误。

- [x] **Step 4: 实现最小 workspace state、query 与只读 snapshot**

`model.ts` 定义内部状态和 view：

```ts
export interface UsageWorkspaceView {
  readonly summary: OpenAiUsageSummary | null;
  readonly accountSummary: readonly OpenAiUsageAccountSummary[];
  readonly modelOptions: readonly OpenAiUsageModelSummary[];
  readonly trends: readonly OpenAiUsageTrendPoint[];
  readonly logs: OpenAiUsageRequestLogPage | null;
  readonly dataSources: readonly OpenAiUsageDataSourceSummary[];
  readonly rangePreset: OpenAiUsageRangePreset;
  readonly scope: OpenAiUsageScope;
  readonly logsPage: number;
  readonly logsPageSize: number;
  readonly loaded: boolean;
  readonly loading: boolean;
  readonly loadError: string | null;
  readonly refreshQueued: boolean;
}
```

`workspace.ts` 内部保存可变 state，通过 `snapshot()` 返回新对象和新数组；不得把 internal state 引用交给调用者。

- [x] **Step 5: 逐个 TDD stale response、刷新队列、分页纠偏与 dispose**

每个行为执行一个 red → green cycle，测试必须通过 public interface：

```ts
it("忽略晚到的旧 intent 响应", async () => {
  const first = deferred<OpenAiUsageDashboard>();
  const second = deferred<OpenAiUsageDashboard>();
  const loadDashboard = vi.fn()
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise);
  const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
  const older = workspace.dispatch({ type: "refresh", mode: "initial" });
  await workspace.dispatch({ type: "set-filters", model: "gpt-5.3-codex" });
  const newer = workspace.dispatch({ type: "refresh", mode: "initial" });
  second.resolve(dashboardWithRequests(2));
  await newer;
  first.resolve(dashboardWithRequests(1));
  await older;
  expect(workspace.snapshot().summary?.total_requests).toBe(2);
});

it("合并 in-flight 手动刷新且自动刷新不排队", async () => {
  const first = deferred<OpenAiUsageDashboard>();
  const loadDashboard = vi.fn()
    .mockReturnValueOnce(first.promise)
    .mockResolvedValue(emptyDashboard());
  const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
  const running = workspace.dispatch({ type: "refresh", mode: "manual" });
  await workspace.dispatch({ type: "refresh", mode: "manual" });
  await workspace.dispatch({ type: "refresh", mode: "auto" });
  first.resolve(emptyDashboard());
  await running;
  expect(loadDashboard).toHaveBeenCalledTimes(2);
});

it("日志页越界时纠偏并只重载一次", async () => {
  const loadDashboard = vi.fn()
    .mockResolvedValueOnce(dashboardWithPage({ page: 9, pageSize: 20, total: 25 }))
    .mockResolvedValueOnce(dashboardWithPage({ page: 2, pageSize: 20, total: 25 }));
  const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
  await workspace.dispatch({ type: "set-page", page: 9 });
  await workspace.dispatch({ type: "refresh", mode: "manual" });
  expect(loadDashboard.mock.calls.map((call) => call[1])).toEqual([9, 2]);
  expect(workspace.snapshot().logsPage).toBe(2);
});

it("dispose 后忽略完成响应并停止 timer", async () => {
  const pending = deferred<OpenAiUsageDashboard>();
  const timers = fakeTimers();
  const workspace = createUsageWorkspace(fakeDependencies({
    loadDashboard: () => pending.promise,
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
  }));
  await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
  await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
  const running = workspace.dispatch({ type: "refresh", mode: "manual" });
  workspace.dispose();
  pending.resolve(dashboardWithRequests(7));
  await running;
  expect(timers.activeCount()).toBe(0);
  expect(workspace.snapshot().summary).toBeNull();
});
```

`workspace.ts` 必须满足：

```ts
const stale = disposed || requestSerial !== activeRequestSerial || intentKey !== currentIntentKey();
if (stale) return;
```

手动排队使用布尔合并，不保存无界队列；分页纠偏最多二次 loader 调用。`initial` load 允许被新的 intent load 取代，并依赖 request serial 丢弃旧响应；`manual`/`auto` 使用刷新队列规则。

- [x] **Step 6: 验证任务 1**

Run:

```powershell
npm run test:front -- workspace.test.ts
npx tsc --noEmit
```

Expected: 5 个 workspace 行为测试 PASS；TypeScript 0 errors。

- [x] **Step 7: 提交任务 1**

```powershell
git add -- package.json package-lock.json src-tauri/shared/front/usage
git commit -m "test: define usage workspace interface"
```

---

### Task 2：把 Usage DOM、状态和渲染迁入纵向 module

**风险：high-risk。** 修改上游已有共享前端文件；常规 task review 后必须增加独立 adversarial review，攻击其他页面回退、timer 泄漏、stale DOM 写入与错误提示差异。

**Files:**

- Create: `src-tauri/shared/front/usage/render.ts`
- Create: `src-tauri/shared/front/usage/dom.ts`
- Modify: `src-tauri/shared/front/usage/model.ts`
- Modify: `src-tauri/shared/front/usage/workspace.ts`
- Modify: `src-tauri/shared/front/usage/index.ts`
- Modify: `src-tauri/shared/front/usage/workspace.test.ts`
- Modify: `src-tauri/shared/front/actions.ts`
- Modify: `src-tauri/shared/front/render.ts`
- Modify: `src-tauri/shared/front/state.ts`

**Interfaces:**

- Consumes: 任务 1 的 `UsageWorkspace` interface 与现有 `getOpenAiUsageDashboard` adapter。
- Produces: `mountUsageWorkspace({ loadDashboard, getContext, onError }): UsageWorkspace`，供共享 `actions.ts` bootstrap 一次；调用者只派发 `set-context` 和 `refresh`，不读取 Usage internal state。
- `render.ts` 不再导出 `renderOpenAiUsage`；`state.ts` 不再包含任何 `usage*` 字段。

- [x] **Step 1: 写失败测试——locale/context 与 picker/timer 仍属于 workspace**

扩展 public-interface tests：

```ts
it("切出 Usage route 时停止自动刷新并关闭 range picker", async () => {
  const workspace = createUsageWorkspace(fakeDependencies());
  await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
  await workspace.dispatch({ type: "open-range-picker" });
  await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
  await workspace.dispatch({ type: "set-context", active: false, locale: "zh-CN" });
  expect(workspace.snapshot().rangePickerOpen).toBe(false);
  expect(fakeTimers.activeCount()).toBe(0);
});
```

Run: `npm run test:front -- workspace.test.ts`

Expected: FAIL，因为 context/picker 生命周期尚未进入 workspace。

- [x] **Step 2: 先让 renderer 消费 `UsageWorkspaceView`，不再读全局 state**

把 `render.ts` 中 Usage 专属实现（当前约 848-1938）移动到 `usage/render.ts`。所有 formatter/render helper 接收 `view` 与 `locale`，例如：

```ts
export function renderUsage(elements: UsageElements, view: UsageWorkspaceView): void {
  renderUsageControls(elements, view);
  if (view.loadError) {
    if (elements.accountPanel) elements.accountPanel.innerHTML = "";
    elements.summaryPanel!.innerHTML = `<article class="usage-empty-state usage-empty-state--error"><strong>${t(view.locale, "usageLoadFailed")}</strong><p>${escapeHtml(view.loadError)}</p></article>`;
    elements.trendPanel!.innerHTML = "";
    elements.detailPanel!.innerHTML = "";
    return;
  }
  renderUsageSummary(elements, view);
  renderUsageTrends(elements, view);
  renderUsageDetails(elements, view);
}
```

不得从 `usage/render.ts` 反向导入共享 `render.ts`；需要的 `escapeHtml`、数字/日期 formatter 在 Usage module 内保持纯函数。

- [x] **Step 3: 把 Usage DOM elements 与事件 binding 移入 `usage/dom.ts`**

`dom.ts` 只查询现有 ID/data attributes，不改 HTML contract：

```ts
export interface UsageElements {
  refreshButton: HTMLButtonElement | null;
  rangeButton: HTMLButtonElement | null;
  rangePopover: HTMLDivElement | null;
  scopeSelect: HTMLSelectElement | null;
  accountFilter: HTMLSelectElement | null;
  modelFilter: HTMLSelectElement | null;
  refreshIntervalSelect: HTMLSelectElement | null;
  accountPanel: HTMLDivElement | null;
  summaryPanel: HTMLDivElement | null;
  trendPanel: HTMLDivElement | null;
  detailPanel: HTMLDivElement | null;
}
```

`mountUsageWorkspace` 安装 click/change/focusin/document click/Escape listeners，并在 `dispose()` 中逐一移除。所有 DOM intent 转换成 `workspace.dispatch(event)`。

- [x] **Step 4: 迁移 actions 中的 Usage 状态机与 timer**

删除 `actions.ts` 当前约 134-590、760-840 的 Usage-only implementation，以及约 1598-1685 的 Usage-only listeners。共享代码只保留：

```ts
const usageWorkspace = mountUsageWorkspace({
  loadDashboard: getOpenAiUsageDashboard,
  getContext: () => ({ active: state.route === "usage", locale: state.locale }),
  onError: (error, notifyUser) => {
    if (notifyUser) showToast(error.message, true);
  },
});
```

`rerenderDashboard()` 只派发 context，不调用 Usage renderer/timer helper；`refreshAllData()` 使用 `await usageWorkspace.dispatch({ type: "refresh", mode: "initial" })`。

- [x] **Step 5: 删除共享全局 Usage state 与 renderer surface**

从 `state.ts` 删除全部 `usage*` 字段和对应 imports。从共享 `render.ts` 删除 Usage-only imports、elements、formatter 和 `renderOpenAiUsage` export；保留 1938 行后的账号/设置/主题 renderer 行为。

Run: `rg -n 'state\.usage|renderOpenAiUsage|syncUsageAutoRefresh|syncUsageRangeLiveEndTimer' src-tauri/shared/front`

Expected: 0 matches；允许 `usage` module 内部 state 名称，但不得出现 `state.usage*`。

- [x] **Step 6: 验证任务 2**

Run:

```powershell
npm run test:front
npm run build -- --mode windows
```

Expected: Vitest 全绿；`tsc` 与 Windows Vite production build 成功；`dist/web` 生成。

- [x] **Step 7: 提交任务 2**

```powershell
git add -- src-tauri/shared/front/usage src-tauri/shared/front/actions.ts src-tauri/shared/front/render.ts src-tauri/shared/front/state.ts
git commit -m "refactor: deepen usage frontend module"
```

---

### Task 3：把 Usage evidence 来源迁入真实 ingestion adapters

**风险：high-risk。** 触及 token/费用证据分类链；常规 task review 后必须增加独立 adversarial review，攻击 source priority、identity proof、dedupe、冻结失败与 schema/cache 行为漂移。

**Files:**

- Create: `src-tauri/shared/runtime/openai_usage/ingestion.rs`
- Modify: `src-tauri/shared/runtime/openai_usage.rs`
- Test: `src-tauri/shared/runtime/openai_usage/ingestion.rs`
- Test: existing `src-tauri/shared/runtime/openai_usage.rs` tests

**Interfaces:**

- Consumes: parent module 的 `UsageEvidence`、`UsageLoadResult`、`OpenAiUsageSummary` 与既有 evidence/classifier helpers。
- Produces only:

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

- `UsageEvidenceAdapter` trait、adapter list 与 ingestion context 必须 private to `ingestion.rs`。
- Parent `sync_openai_usage_ledger` 只调用 `ingestion::ingest_usage`；不得直接调用来源 collector。

- [x] **Step 1: 写失败测试——三个真实 adapters 以固定顺序参与**

在 `ingestion.rs` tests 通过内部 adapter seam 注入 recording adapters：

```rust
#[test]
fn adapters_collect_in_stable_priority_order() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    collect_from_adapters(
        &[
            &RecordingAdapter::new(SOURCE_CODEX_SESSION_JSONL, Rc::clone(&calls)),
            &RecordingAdapter::new(SOURCE_OPENAI_RESPONSE_USAGE, Rc::clone(&calls)),
            &RecordingAdapter::new(SOURCE_CCSWITCH_PROXY, Rc::clone(&calls)),
        ],
        Path::new("unused"),
        &mut test_context(),
    ).unwrap();
    assert_eq!(calls.borrow().as_slice(), [
        SOURCE_CODEX_SESSION_JSONL,
        SOURCE_OPENAI_RESPONSE_USAGE,
        SOURCE_CCSWITCH_PROXY,
    ]);
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml adapters_collect_in_stable_priority_order -- --exact`

Expected: FAIL，因为 ingestion module/真实 adapter seam 尚不存在。

- [x] **Step 2: 建立 private trait 与三个 adapters，不改变 collector 实现**

```rust
trait UsageEvidenceAdapter {
    fn source_type(&self) -> &'static str;
    fn collect(
        &self,
        codex_home: &Path,
        context: &mut IngestionContext<'_>,
    ) -> AppResult<Vec<UsageEvidence>>;
}

struct SessionJsonlAdapter;
struct FutureOpenAiResponseUsageAdapter;
struct CcSwitchAdapter;
```

先让 adapters 调用现有 collector，跑测试变绿；随后 Step 3 再移动 implementation，避免大爆炸式重写。

- [x] **Step 3: 机械移动来源 implementation 到 ingestion module**

移动并保持代码主体不变：

- session roots/file discovery、`SessionEvidenceInput`、session evidence identity/build、JSONL parse/delta 与 `collect_session_file_events`；
- `live_ccswitch_db_path`、`freeze_ccswitch_db`、proxy/rollup collectors 与 cc-switch identity normalization；
- future adapter 与 source type/priority mapping。

Parent 可保留 attribution、classification、dedupe、pricing、schema、projection helpers；child 通过 `super::*` 使用这些 implementation details。不要复制逻辑，不改变字符串常量、SQL、evidence id/hash 或错误码。

- [x] **Step 4: 让 ledger 只调用一个 ingestion interface**

在 `openai_usage.rs` 顶部声明：

```rust
mod ingestion;
```

将：

```rust
let usage = load_usage_events(codex_home, summary)?;
```

替换为：

```rust
let mut conn = open_usage_ledger(codex_home)?;
ensure_usage_ledger_schema(&conn)?;
let usage = ingestion::ingest_usage(codex_home, summary, |source_signature, summary| {
    apply_cached_usage_sync_state(&conn, source_signature, summary)
})?;
```

删除 parent 中旧 trait、future adapter 和所有来源直接 collector。Parent 独占 SQLite open、schema/migration、sync state 和 projection；ingestion 独占 source discovery/signature/priority，并在 adapter collect 前调用注入的窄 cache probe。Parent 保留 `finalize_usage_load`，除非移动它能进一步缩小 interface 且不复制 classification。

- [x] **Step 5: 增加 adapter fixture 回归**

把 parent tests 中以下已有 fixture 与断言机械移动到 `ingestion.rs`，函数体保持不变，只调整 module path：

- `ccswitch_rows_are_candidate_evidence_not_accepted_events`
- `ccswitch_proxy_rows_with_account_identity_are_strict_proven`
- `future_response_source_has_highest_priority_without_collecting_rows`
- `official_default_profile_counts_only_post_attribution_deltas` 所依赖的 session fixture 保留在 parent interface test，不移动。

Future adapter 的内部 interface test 保持完整断言：

```rust
#[test]
fn future_adapter_collects_no_evidence() {
    let adapter = FutureOpenAiResponseUsageAdapter;
    assert_eq!(adapter.source_type(), SOURCE_OPENAI_RESPONSE_USAGE);
    assert!(source_priority(adapter.source_type()) > source_priority(SOURCE_CCSWITCH_PROXY));
    let mut context = IngestionContext::empty_for_test();
    assert!(adapter.collect(Path::new("unused"), &mut context).unwrap().is_empty());
}
```

保留 parent 现有 `future_response_source_has_highest_priority_without_collecting_rows` 的行为断言；如果测试移动，名称与断言语义保持一致。

- [x] **Step 6: 验证任务 3**

Run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml openai_usage::ingestion
cargo test --manifest-path src-tauri/Cargo.toml openai_usage
```

Expected: ingestion tests 与现有 openai usage tests 全部 PASS；无 schema/version 常量变化。

- [x] **Step 7: 提交任务 3**

```powershell
git add -- src-tauri/shared/runtime/openai_usage.rs src-tauri/shared/runtime/openai_usage/ingestion.rs
git commit -m "refactor: deepen usage evidence ingestion"
```

---

### Task 4：同步项目知识、全量验证与收尾准备

**风险：** 中。文档与集成验证；不构建或部署安装版。

**Files:**

- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/IMPLEMENTATION.md`
- Modify: `docs/plans/2026-07-10-usage-deep-modules.md`（只勾选完成项）

**Interfaces:**

- Consumes: 任务 1-3 已评审通过的实现。
- Produces: 项目结构说明、验证证据和可合并提交；不改变 runtime behavior。

- [x] **Step 1: 更新仓库结构与 implementation 文档**

README 两份同步说明：

```text
src-tauri/shared/front/usage/：Usage workspace deep module，内聚状态、异步时序、DOM binding 与渲染。
src-tauri/shared/runtime/openai_usage/ingestion.rs：Usage evidence 内部 adapter seam，负责 session JSONL、cc-switch 与 future response 来源采集。
```

`docs/IMPLEMENTATION.md` 记录外部 Tauri interface、ledger schema 与 evidence 语义未变。

- [x] **Step 2: 运行格式与静态检查**

Run:

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run test:front
npm run build -- --mode windows
npm run check:windows-release-hygiene
```

Expected: 全部 exit 0。

- [x] **Step 3: 运行完整 Rust suite**

Run: `npm test`

Expected: 全部 Rust tests PASS。

- [x] **Step 4: 验证架构完成信号与 Git 范围**

Run:

```powershell
rg -n 'state\.usage|renderOpenAiUsage|syncUsageAutoRefresh|syncUsageRangeLiveEndTimer' src-tauri/shared/front
rg -n 'collect_future_response_evidence|collect_ccswitch_evidence|collect_session_file_events' src-tauri/shared/runtime/openai_usage.rs
git status --short
```

Expected: 两组 `rg` 均 0 matches；Git status 只包含计划内文件和用户原有 `.gitignore`、`AGENTS.md`。

- [x] **Step 5: 提交任务 4**

```powershell
git add -- README.md README.zh-CN.md docs/IMPLEMENTATION.md docs/plans/2026-07-10-usage-deep-modules.md
git commit -m "docs: describe usage deep modules"
```

- [ ] **Step 6: 全分支 review 与对抗性审查**

从 `main` merge-base 生成 review package。最终 reviewer 必须同时给出 spec compliance 和 code quality 结论；独立 adversarial reviewer 必须攻击：

- 旧响应覆盖新 intent；
- route/dispose 后 timer 与 promise 写入；
- 手动/自动刷新通知差异；
- 非 Usage 页面与上游 commands 回退；
- source priority、identity proof、official gate、dedupe、schema/cache 漂移；
- 文档、测试与 Git 状态不一致。

所有 Critical/Important findings 必须修复并复审通过，才允许合并。

---

## 最终集成与临时分支删除

此阶段由主 agent 执行，不委托实现子代理。

- [ ] 确认 `codex/deepen-usage-modules` 上所有任务提交存在，完整测试和最终 review 通过。
- [ ] 记录用户未提交 `.gitignore`、`AGENTS.md` diff，确认切换分支不会覆盖。
- [ ] 切换 `main`，以 `git merge --ff-only codex/deepen-usage-modules` 合入临时分支；不得 squash 掉任务级审查提交。
- [ ] 在 `main` 上重新运行 `npm run test:front`、`npm test` 与 `git status --short --branch`。
- [ ] 删除本地 `codex/deepen-usage-modules` 分支。
- [ ] 验证 `main` 含所有实现提交，临时分支不存在，用户原有未提交改动仍在。

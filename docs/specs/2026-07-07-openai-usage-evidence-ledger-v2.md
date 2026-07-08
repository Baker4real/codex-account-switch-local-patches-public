# OpenAI Usage Evidence Ledger v2 设计

日期：2026-07-07  
状态：设计草案  
前置材料：

- `docs/specs/2026-07-07-openai-usage-statistics-design.md`
- `docs/research/2026-07-07-high-star-account-usage-projects.md`
- `docs/research/2026-07-07-historical-usage-attribution.md`

## 设计目标

把当前 OpenAI Usage 统计从“可证明事件直接进入统计账本”升级为“先保存证据，再由规则采纳为统计事件”。

目标不是追求更多历史覆盖率，而是在覆盖率、准确性和可审计性之间建立硬边界：

1. 后续新 usage 能按账号准确归类。
2. 历史 usage 能尽量补充证据，但不能把无账号证明的数据硬归类。
3. 每一条未计入、被替代、低置信或未定价记录都有可追溯原因。
4. 后续接入 cc-switch、proxy、OpenAI response usage 或其他来源时，不需要把判断规则散落到多个调用点。

## 高分项目核心设计

最高分、最接近本问题的是 LiteLLM。Langfuse、Portkey、Helicone 提供相同方向的旁证。

它们的共同设计不是“事后猜账号”，而是：

```text
请求进入自己的 gateway / SDK / trace adapter
  -> 请求时写入 user / key / team / metadata / header 身份维度
  -> 响应时记录 usage / cost / model / provider
  -> 写入可审计事实行
  -> dashboard / report 只读事实行做聚合
```

对应到本项目：

- LiteLLM 的 `api_key owner / user_id / team_id` 对应我们的 `account_identity_key / profile`。
- LiteLLM 的 `end_user` 警告对应我们的“不信任请求体或当前状态自报身份”规则。
- Langfuse 的 trace/observation 对应我们的 `UsageEvidence`。
- Helicone/Portkey 的 header/metadata 对应未来请求路径上的 account identity 写入。
- cc-switch 的 `proxy_request_logs` 是可借鉴的请求事实表，但本机 `codex_session` 数据不是账号证明。

## 第一性原理

一次 usage 要进入严格账号统计，必须同时证明三件事：

1. **Token fact**：这次事件实际消耗了多少 input、cached input、output、reasoning token。
2. **Official source fact**：这次事件发生时有效 OpenAI source 是官方路径。
3. **Account fact**：这次事件发生时使用的是哪个已管理账号。

当前实现已经在严格性上方向正确：不能证明 official/account 的事件不会进入 `openai_usage_events`。问题是它把“证据收集、分类、去重、采纳、聚合”压在同一个实现路径里，未采纳数据只剩排除计数。

因此 v2 的关键改进是：把 `UsageEvent` 从唯一事实形态拆成两层。

## 目标 Module

### 1. `UsageEvidenceCollector`

**Interface**：

```text
collect_usage_evidence(codex_home, source_snapshot) -> UsageEvidenceBatch
```

**Adapter**：

- `CodexSessionJsonlAdapter`
- `AttributionLogAdapter`
- `CcSwitchSqliteAdapter`
- `FutureOpenAiResponseUsageAdapter`

**规则**：

- Adapter 只忠实读取来源，不判断最终是否计入统计。
- Adapter 可以标记来源自身置信度，例如 `codex_session_jsonl`、`ccswitch_codex_session`、`ccswitch_proxy`。
- Adapter 不直接写 `AcceptedUsageEvent`。

这是一个真实 seam，因为至少有当前 Codex session JSONL、当前 attribution log、cc-switch DB、未来 response/proxy usage 四类来源会变化。

### 2. `UsageEvidenceLedger`

**Interface**：

```text
rebuild_evidence(source_signature) -> EvidenceLedgerSnapshot
load_evidence(query) -> Vec<UsageEvidence>
```

**职责**：

- 保存 accepted、rejected、superseded、candidate 的所有最小审计字段。
- 保存 source hash、parser version、source locator。
- 支持 source signature 变化后重建。
- 不面向 UI 直接展示复杂细节，UI 通过 summary/query module 读取。

### 3. `EvidenceClassifier`

**Interface**：

```text
classify(evidence, attribution_context, official_policy) -> EvidenceDecision
```

**职责**：

- 判定 `account_proven`、`official_proven`、`token_proven`。
- 输出 `proven / candidate / unproven / rejected`。
- 生成明确 rejection reason。
- 不做跨来源去重。

关键规则：

- `model_provider='openai'` 最多是 `official_bucket_proven`，不能升级为 `account_proven`。
- 当前 active profile 不能反推历史账号。
- 请求体自报 user、cc-switch 当前 provider 名称、session-only provider id 都不能作为账号证明。
- `last_token_usage`、rollup-only、推断 token 必须降低 token confidence。

### 4. `UsageDedupeEngine`

**Interface**：

```text
resolve(decisions) -> Vec<AcceptedUsageEvent | SupersededEvidence>
```

**职责**：

- 将高置信来源覆盖低置信来源。
- 避免 proxy/response usage 与 session fallback 重复计数。
- 保存 `superseded_by` 和原因。

建议优先级：

```text
openai_response_usage
  > ccswitch_proxy
  > codex_session_jsonl + account_proven attribution
  > ccswitch_codex_session
  > ccswitch_rollup
```

只有 `account_proven + official_proven + token_proven` 的结果才能进入严格统计。

### 5. `AcceptedUsageEventLedger`

**Interface**：

```text
load_accepted_events(query) -> Vec<AcceptedUsageEvent>
```

**职责**：

- 替代当前单层 `openai_usage_events` 的角色。
- 只保存严格可计入统计的事件。
- 聚合层只读这里，不读取 raw evidence。

### 6. `UsageRollupQuery`

**Interface**：

```text
load_dashboard(query) -> OpenAiUsageDashboard
load_request_logs(query) -> OpenAiUsageRequestLogPage
load_evidence_summary(query) -> OpenAiUsageEvidenceSummary
```

**职责**：

- 生成账号、模型、趋势、日志、来源、排除/候选摘要。
- 保持 UI 查询简单。
- 不重新执行业务归因规则。

## SQLite 结构草案

### `openai_usage_evidence`

保存来源证据，不代表已计入统计。

```text
evidence_id TEXT PRIMARY KEY
source_type TEXT NOT NULL
source_priority INTEGER NOT NULL
source_locator TEXT NOT NULL
source_hash TEXT NOT NULL
parser_version TEXT NOT NULL

event_started_at_ms INTEGER
event_completed_at_ms INTEGER
observed_at_ms INTEGER

session_id TEXT
turn_index INTEGER
request_id TEXT
response_id TEXT
previous_response_id TEXT

model TEXT
request_model TEXT
pricing_model TEXT
usage_shape TEXT NOT NULL
usage_confidence TEXT NOT NULL
input_tokens INTEGER
cached_input_tokens INTEGER
output_tokens INTEGER
reasoning_output_tokens INTEGER
raw_usage_json TEXT

profile TEXT
account_identity_key TEXT
account_label TEXT
identity_source TEXT
identity_observed_at_ms INTEGER
account_confidence TEXT NOT NULL

effective_base_url TEXT
official_source TEXT
official_confidence TEXT NOT NULL

status_code INTEGER
decision TEXT NOT NULL
decision_reason TEXT
superseded_by TEXT
created_at_ms INTEGER NOT NULL
```

### `openai_usage_events`

当前表可以演进为 accepted-only 表，或者新增 `openai_accepted_usage_events` 后兼容迁移。

必须新增或等价表达：

```text
accepted_event_id TEXT PRIMARY KEY
evidence_id TEXT NOT NULL
account_identity_key TEXT NOT NULL
profile TEXT NOT NULL
account_label TEXT
official_source TEXT NOT NULL
source_type TEXT NOT NULL
source_priority INTEGER NOT NULL
usage_confidence TEXT NOT NULL
pricing_model TEXT
pricing_version TEXT NOT NULL
...
```

### `openai_usage_sync_state`

继续保留，但需要扩展：

```text
evidence_source_signature TEXT
classifier_version TEXT
dedupe_version TEXT
pricing_version TEXT
accepted_events INTEGER
candidate_events INTEGER
rejected_events INTEGER
superseded_events INTEGER
```

## 与当前设计的差异

当前设计：

```text
Codex JSONL
  -> attribution_for_event
  -> UsageEvent
  -> openai_usage_events
  -> dashboard

未采纳数据 -> OpenAiUsageExclusions 计数
```

v2 设计：

```text
Codex JSONL / attribution log / cc-switch DB / future response usage
  -> UsageEvidence
  -> EvidenceDecision
  -> Dedupe resolution
  -> AcceptedUsageEvent
  -> dashboard

未采纳数据 -> openai_usage_evidence 中的 rejected/candidate/superseded 行
```

收益：

- **Leverage**：新增来源时只写 adapter，不重写 dashboard 或账号规则。
- **Locality**：official/account/token/dedupe 规则集中在 classifier 和 dedupe engine。
- **可测试性**：测试穿过 `EvidenceClassifier` 和 `UsageDedupeEngine` 的 Interface，而不是依赖 UI 聚合结果反推。
- **可审计性**：每条未归因历史都有原因，不只是计数。

## 账号身份规则

账号身份应从 profile slot 名升级为稳定 key。

建议：

```text
account_identity_key = local_hash(account_id || email)
```

规则：

- `profile` 是槽位，不是账号身份。
- `account_label` 是展示标签，不是账号身份；只能在没有 identity 的 legacy 记录上作为桥接线索使用。
- 同一账号跨多个 profile 时应能按 `account_identity_key` 合并。
- profile 改名不应切裂历史账号统计。
- email/account id 原文不应作为 join 主键散落到查询层。
- strict identity 永远优先于 label。两个不同 `account_identity_key` 即使 label 相同也不能合并。
- legacy label 只有在当前 fact 集合中能唯一映射到一个 strict identity 时，才桥接到该账号实体；否则保留为 legacy/label 分组。
- 查询层使用 `account_group_key` 做账号实体筛选，不能把合并账号再降级成单个 profile 筛选。

## cc-switch 的位置

cc-switch 只能作为 source adapter，不是主账本。

导入规则：

- 导入前先冻结 DB 副本到 `%CODEX_HOME%\usage_evidence\` 下的 D 盘 evidence 目录，记录 hash、mtime、SQL；不能写入 `account_backup`，避免被 profile 扫描识别成账号。
- `data_source='proxy'` 且能映射 provider/account 的 Codex 行，可以进入高置信 candidate 或 proven 检查。
- `data_source='codex_session'`、`provider_id='_codex_session'` 只能提供 token/model/session/time 证据；除非 cc-switch 在请求发生时额外记录稳定 `account_identity_key`、profile 归属和 effective Base URL，否则不能提升为账号证明。
- `usage_daily_rollups` 只能做 day/model/provider 聚合候选，不做请求级账号归因。

## UI 改进

UI 不应只展示“排除数量”，应展示证据层状态：

- 严格计入：`accepted/proven`
- 候选：`candidate`
- 未证明：`unproven`
- 已被高优先级来源替代：`superseded`
- 未定价：`unpriced`

默认账号统计仍只读 accepted/proven。candidate 应有单独视图或摘要卡片用于对账，但不能混入严格或历史费用总额。

## 分阶段迁移

### Phase 1：隐藏 evidence ledger

- 新增 `openai_usage_evidence`。
- 先让当前 Codex JSONL + attribution 产生 evidence。
- 当前 UI 不变。
- `openai_usage_events` 仍作为 accepted-only 表。

### Phase 2：把 classification/dedupe 从 parser 中抽出

- `collect_session_file_events` 不再直接产出 accepted `UsageEvent`。
- 引入 `EvidenceClassifier` 和 `UsageDedupeEngine`。
- `OpenAiUsageExclusions` 从纯计数变为 evidence summary。

### Phase 3：加入 cc-switch 只读 adapter

- 只读冻结副本。
- 默认不把 session-only 数据计入账号总额。
- 提供 token 校验差异和 candidate/unproven 摘要；`candidate_unattributed` 应作为独立对账层展示，帮助解释与 cc-switch 总额的差异。

### Phase 4：未来 response/proxy usage adapter

- 如果能捕获 OpenAI response usage，作为最高优先级 token fact。
- 保存 raw usage JSON、request/response id、stream final 状态。
- 与 session fallback 做 supersede，不重复计数。

## 测试矩阵

必须覆盖：

- official 判定：默认 OpenAI、HTTPS `api.openai.com`、HTTP、子域、自定义 proxy。
- account 判定：根 auth 与 profile auth 匹配、不匹配、缺失、profile 改名、同账号多 profile。
- token confidence：`total_token_usage`、`last_token_usage`、负 delta、cached clamp、rollup-only。
- source priority：response/proxy 覆盖 session fallback。
- cc-switch：proxy 行、codex_session 行、rollup 行分别进入正确决策。
- evidence rebuild：source signature、classifier version、dedupe version、pricing version 变化会重建。
- UI：accepted/candidate/unproven/superseded 数量不混入严格账号总额，`candidate_unattributed` 只能作为独立对账层展示。

## 决策

应采用 LiteLLM 式“身份绑定到事实行”的核心设计，但不能照搬它的 API key 模型。

本项目的等价身份不是 end user 参数，而是切换器在事件发生时能证明的 OpenAI account identity。也就是说：

- `account_identity_key` 是我们的 key owner。
- `profile` 是本地槽位维度。
- `UsageEvidence` 是 Langfuse observation / LiteLLM spend log 的本地版本。
- `AcceptedUsageEvent` 是严格统计行。

这能保留当前已实现的保守官方统计，同时为 cc-switch 导入和未来 response usage 捕获留出干净 seam。

# Usage 统计账号准确性实施计划

> **For agentic workers:** Implement this plan task-by-task. Prefer `subagent-driven-development`，每个任务完成后先做规格符合性检查，再做代码质量检查。

**Goal:** 让 Usage 统计以可验证的账号身份合并数据，保持 token 守恒，明确展示严格/推定归属和不可定价原因，并把刷新圆圈替换为稳定的三点脉冲状态。

**Architecture:** 保持现有 session 解析、fact 去重和 token delta 不变；为新 attribution/evidence/fact 增加两个不可逆身份别名，在查询阶段用一个 canonical resolver 将同 account_id、同邮箱、兼容旧 key 以及 latest-prior 历史事件归入同一连通分量。dashboard、趋势、日志和账号筛选共享同一 resolver。前端只消费新增的归属拆分契约，不自行重新归并账号。

**Tech Stack:** Rust、rusqlite、serde、TypeScript、Vitest、Tauri/Vite、现有 shared runtime/front 模块。

## 全局约束

- 以 `docs/specs/2026-07-10-usage-account-accuracy-design.md` 为唯一产品口径；若实现需要改变其中规则，先停下并重新确认。
- 只改 shared usage 契约和 Windows 可验证路径，不引入 macOS 专属代码，也不扩张平台共享边界。
- 不重写现有 ledger；SQLite 只做可空列的增量迁移。
- `real_total_tokens` 永远等于 `input_tokens + output_tokens`；cached input 和 reasoning output 只能是子项。
- 同一 fact 在一次聚合中最多累加一次；resolver 只决定归属键，不复制 fact。
- 历史缺失身份时只允许使用事件发生前、同 profile 最近一次已确认身份，禁止未来或当前槽位倒灌。
- 未知价格不近似、不显示为零；全未定价显示“不可定价”，部分未定价显示已知金额并标记“部分定价”。
- UI 阶段必须按顺序实际使用 `awesome-design-md`、`design-taste-frontend`、`ui-ux-pro-max`、`prototype`、`frontend-design`，并把每个 skill 的实际影响记入 `progress.md`。
- 每次声称完成前使用 `verification-before-completion`；重大归属逻辑完成后启用多子代理进行对抗性审查，攻击边界、回归、迁移和守恒缺口。

## 执行修订（2026-07-10，用户已确认）

- 原 Tasks 1–5 不再作为“先批量提交红灯、后批量实现”的水平阶段执行。
- 后端按行为垂直切片：每个切片都完成一个公开 seam 测试的 RED、对应最小实现的 GREEN、定向验证和任务审查后才进入下一切片。
- 不创建只包含失败测试的提交；原 Tasks 1–5 的所有验收项、文件范围和最终契约保持不变。
- 垂直切片顺序：身份别名持久化与直接归并 → 传递归并与旧 key 恢复 → latest-prior 历史归属 → 跨视图 canonical 筛选 → 归属拆分与 token 守恒。

---

## Task 1：建立后端公开 seam 的失败测试

**Files:**

- Modify: `src-tauri/shared/runtime/openai_usage.rs`（现有 `#[cfg(test)] mod tests`）

- [ ] 1.1 在现有测试 helper 旁增加仅用于 fixture 的 auth/session/attribution 构造器；测试只调用 `load_openai_usage_dashboard`、公开 trends/logs 入口，不直接调用 resolver 私有方法。
- [ ] 1.2 把现有 `dashboard_account_summary_does_not_merge_distinct_strict_identities_with_same_label` 拆成两个行为：同邮箱即使旧组合 key 不同也合并；真正不同邮箱且不同 account_id 保持两行。
- [ ] 1.3 增加 email-only ↔ 双字段、account-id-only ↔ 双字段、A-email→B-both→C-account-id 的传递归并测试；断言 `account_summary.len() == 1`、请求数守恒、profiles 集合完整。
- [ ] 1.4 增加旧 v1 key 恢复测试：profile auth snapshot 的重算 key 匹配时可附加 aliases；不匹配时禁止仅凭 profile 名或当前账号合并。
- [ ] 1.5 增加 latest-prior 时间线测试：同 profile 先账号 A、后账号 B，夹在中间的 legacy event 属于 A；B 之后的属于 B；第一条确认身份之前的 legacy event 保持归属不明。
- [ ] 1.6 增加 token 守恒断言：每个账号 `strict_proven + legacy_profile_attributed == account.tokens`，所有账号加归属不明 bucket 等于全局 total，且 `real_total == input + output`。
- [ ] 1.7 增加 canonical filter 回归：用旧 identity key、email alias key、canonical group key 查询，dashboard、trends 和 logs 必须返回同一 fact 集合。

测试命名固定为：

```rust
#[test]
fn dashboard_merges_identity_aliases_transitively() { /* fixture + public API assertions */ }

#[test]
fn dashboard_uses_latest_prior_identity_without_future_backfill() { /* fixture + public API assertions */ }

#[test]
fn dashboard_preserves_token_conservation_across_attribution_layers() { /* fixture + public API assertions */ }

#[test]
fn canonical_account_filter_accepts_previous_aliases_across_views() { /* fixture + public API assertions */ }
```

- [ ] 1.8 运行定向测试并确认红灯来自缺失的 alias/resolver/拆分行为，而不是 fixture 错误：

```powershell
npm run test:rust:windows -- openai_usage::tests::dashboard_merges_identity_aliases_transitively -- --nocapture
npm run test:rust:windows -- openai_usage::tests::dashboard_uses_latest_prior_identity_without_future_backfill -- --nocapture
```

Expected: 新测试失败；失败信息明确显示账号仍被拆分或历史事件归错，现有编译不应因测试数据本身损坏。

- [ ] 1.9 提交测试红灯：

```powershell
git add src-tauri/shared/runtime/openai_usage.rs
git commit -m "test: define canonical usage attribution behavior"
```

---

## Task 2：增加向后兼容的身份别名持久化

**Files:**

- Create: `src-tauri/shared/runtime/openai_usage/identity.rs`
- Modify: `src-tauri/shared/runtime/openai_usage.rs`
- Modify: `src-tauri/shared/runtime/openai_usage/ingestion.rs`

- [ ] 2.1 在 `identity.rs` 定义规范化与不可逆 key。禁止持久化原始 account_id；邮箱继续只作为现有 UI label 使用，resolver 比较的是 hash alias。

```rust
pub(super) fn account_id_alias(account_id: Option<&str>) -> Option<String> {
    normalized_non_empty(account_id, false)
        .map(|value| format!("acct:{:x}", Sha256::digest(value.as_bytes())))
}

pub(super) fn email_alias(email: Option<&str>) -> Option<String> {
    normalized_non_empty(email, true)
        .map(|value| format!("email:{:x}", Sha256::digest(value.as_bytes())))
}
```

`normalized_non_empty` 必须 trim；邮箱转 ASCII lowercase；account_id 保持大小写，避免改变服务端标识符语义。仓库当前没有 SHA-256 helper，因此允许增加最小直接依赖 `sha2`，禁止手写密码学实现。

- [ ] 2.2 给 `AttributionContext`、`AttributionRecord`、`UsageEvidence`、`UsageFact` 增加：

```rust
account_id_alias: Option<String>,
email_alias: Option<String>,
```

所有 serde 读取位置使用 `#[serde(default)]` 或等价缺省，保证旧 JSON/ledger 可读。

- [ ] 2.3 在 profile auth identity 被确认时一次性生成两个 alias，并沿 `AttributionContext → AttributionRecord/UsageEvidence → UsageFact` 传递；legacy 无身份记录保持 `None`，不得从 profile 名推导 alias。
- [ ] 2.4 在 `openai_usage_evidence` 和 `openai_usage_facts` schema 增加可空 `account_id_alias TEXT`、`email_alias TEXT`；在 `ensure_usage_evidence_columns` 和 `ensure_usage_facts_columns` 使用现有 additive migration 模式补列。
- [ ] 2.5 同步修改 evidence/facts INSERT、SELECT 和 row mapping。逐列核对 placeholder 数量、SELECT 顺序、`row.get(index)` 下标，避免静默列错位。
- [ ] 2.6 更新 SQLite 迁移测试：旧 schema 打开后新列存在、旧行读取为 `None`、新行 round-trip 保留两个 alias，重复 sync 仍幂等。
- [ ] 2.7 运行迁移与归并定向测试：

```powershell
npm run test:rust:windows -- openai_usage::tests::sqlite -- --nocapture
npm run test:rust:windows -- openai_usage::tests::dashboard_merges_identity_aliases_transitively -- --nocapture
```

Expected: SQLite 测试通过；传递归并测试可能仍红，因为 resolver 尚未接入，但新 facts 已携带 aliases。

- [ ] 2.8 提交：

```powershell
git add src-tauri/shared/runtime/openai_usage.rs src-tauri/shared/runtime/openai_usage/identity.rs
git commit -m "feat: persist usage identity aliases"
```

---

## Task 3：实现 CanonicalAccountResolver 与 latest-prior 归属

**Files:**

- Modify: `src-tauri/shared/runtime/openai_usage/identity.rs`
- Modify: `src-tauri/shared/runtime/openai_usage.rs`

- [ ] 3.1 在 `identity.rs` 定义 resolver 的窄接口，resolver 内部可用并查集，但不得泄露到公开模型：

```rust
pub(super) struct CanonicalAccountResolver {
    fact_group_keys: BTreeMap<String, String>,
    alias_to_group_key: BTreeMap<String, String>,
}

impl CanonicalAccountResolver {
    pub(super) fn from_facts(
        facts: &[UsageFact],
        verified_profile_aliases: &BTreeMap<String, VerifiedIdentityAliases>,
    ) -> Self;

    pub(super) fn group_key_for(&self, fact: &UsageFact) -> String;
    pub(super) fn matches_group_filter(&self, fact: &UsageFact, requested: &str) -> bool;
}
```

- [ ] 3.2 为每条严格 fact 建身份节点，并按相同 `account_id_alias`、相同 `email_alias`、相同 legacy `account_identity_key` 连接；规范化邮箱 label 只作为旧记录兼容 alias，不覆盖更强证据。
- [ ] 3.3 canonical key 采用确定性优先级：连通分量内字典序最小 account-id alias → email alias → legacy identity key → `legacy_account_key` → `profile:<profile>`。同一输入无论遍历顺序如何都得到相同 key。
- [ ] 3.4 从当前 profile auth snapshot 生成 `VerifiedIdentityAliases`，但只有旧 fact 的 v1 `account_identity_key` 与 snapshot 重算值完全相等时才附加；不匹配 snapshot 不得连接。
- [ ] 3.5 对无身份 fact 按 `(profile, created_at_ms)` 排序，查找 `created_at_ms <= event_time` 的最近已确认节点；相同时间以已持久化 attribution observation/window 顺序稳定决胜。没有 prior 节点时保留独立“归属不明”组。
- [ ] 3.6 删除或收窄旧 `account_group_key`/`strict_label_identities` 分支，使所有账号 key 都经 resolver；`add_fact_usage` 接收已解析 group key，避免内部再次猜身份。
- [ ] 3.7 运行 Task 1 全部后端新测试：

```powershell
npm run test:rust:windows -- openai_usage::tests::dashboard_merges_identity_aliases_transitively -- --nocapture
npm run test:rust:windows -- openai_usage::tests::dashboard_uses_latest_prior_identity_without_future_backfill -- --nocapture
npm run test:rust:windows -- openai_usage::tests::dashboard_preserves_token_conservation_across_attribution_layers -- --nocapture
```

Expected: 三个测试通过；不同真实账号仍保持两行。

- [ ] 3.8 提交：

```powershell
git add src-tauri/shared/runtime/openai_usage.rs src-tauri/shared/runtime/openai_usage/identity.rs
git commit -m "feat: resolve canonical usage accounts"
```

---

## Task 4：让 dashboard、趋势、日志与筛选共用 canonical 结果

**Files:**

- Modify: `src-tauri/shared/runtime/openai_usage.rs`

- [ ] 4.1 增加统一加载容器，明确“resolver universe”和“当前视图 facts”是两件事：

```rust
struct ResolvedUsageFacts {
    facts: Vec<UsageFact>,
    resolver: CanonicalAccountResolver,
}
```

resolver universe 使用 `without_model_filter(account_visibility_query(query))`，保留时间/status/profile 边界并覆盖 strict + legacy；然后在内存中应用 model 和 canonical account filter。禁止先用旧 SQL `account_label/account_group_key` 缩小数据，导致 resolver 看不到连接节点。

- [ ] 4.2 修改 `load_openai_usage_dashboard`：summary、account summary、model options、trends、logs 都共享同一个 resolver；scope 仍按现有产品语义决定所展示 facts。
- [ ] 4.3 修改 standalone trends/logs 加载入口，复用同一 helper；旧 key 通过 `alias_to_group_key` 映射到当前 component，避免筛选静默空结果。
- [ ] 4.4 `fact_matches_query` 只负责时间/profile/model/status/scope，账号组过滤只由 resolver 负责；删除重复且不一致的账号判断。
- [ ] 4.5 添加/更新查询回归：同一 canonical 账号跨 profile 只显示一项，但 profile 显式过滤仍只统计该 profile 的 facts；model filter 不改变账号 canonical key。
- [ ] 4.6 运行：

```powershell
npm run test:rust:windows -- openai_usage::tests::canonical_account_filter_accepts_previous_aliases_across_views -- --nocapture
npm run test:rust:windows -- openai_usage::tests::dashboard -- --nocapture
```

Expected: dashboard/trends/logs 的 request ID 集合一致，旧 alias 与新 canonical key 返回相同结果。

- [ ] 4.7 提交：

```powershell
git add src-tauri/shared/runtime/openai_usage.rs
git commit -m "fix: share canonical account filters across usage views"
```

---

## Task 5：增加严格/推定拆分契约与守恒检查

**Files:**

- Modify: `src-tauri/shared/runtime/models.rs`
- Modify: `src-tauri/shared/runtime/openai_usage.rs`
- Modify: `src-tauri/shared/front/types.ts`

- [ ] 5.1 复用现有 `OpenAiUsageLayerSummary` 作为向后兼容的账号归属拆分类型，避免再造一份字段完全相同的 DTO；给 `OpenAiUsageAccountSummary` 增加：

```rust
#[serde(default)]
pub attribution_breakdown: Vec<OpenAiUsageLayerSummary>,
```

保持现有 snake_case JSON 风格；TypeScript 对应字段为 `attribution_breakdown: OpenAiUsageLayerSummary[]`。

- [ ] 5.2 在 TypeScript `OpenAiUsageAccountSummary` 增加同名字段；fixture 为兼容旧数据可以使用 `attribution_breakdown?: OpenAiUsageLayerSummary[]`，render 时缺省为空数组。
- [ ] 5.3 `AccountAggregate` 按 `ledger_layer` 累加请求与 token；只接受 `strict_proven`、`legacy_profile_attributed` 两个展示 bucket，未知值保留原值但不丢数据。
- [ ] 5.4 在测试中显式断言：拆分 request/token 之和等于账号总计；`real_total_tokens == input_tokens + output_tokens`；cached/reasoning 不被二次加总。
- [ ] 5.5 增加 debug/test-only 守恒 helper，失败信息包含 account group key、expected、actual，便于定位重复 fact。
- [ ] 5.6 运行：

```powershell
npm run test:rust:windows -- openai_usage::tests::dashboard_preserves_token_conservation_across_attribution_layers -- --nocapture
npm run test:rust:windows -- openai_usage::tests::calculates_cost_without_double_counting_reasoning_tokens -- --nocapture
```

Expected: 全部通过。

- [ ] 5.7 提交：

```powershell
git add src-tauri/shared/runtime/models.rs src-tauri/shared/runtime/openai_usage.rs src-tauri/shared/front/types.ts
git commit -m "feat: expose usage attribution breakdown"
```

---

## Task 6：建立前端公开 render seam 的失败测试

**Files:**

- Create: `src-tauri/shared/front/usage/render.test.ts`
- Test: `src-tauri/shared/front/usage/render.ts`

- [ ] 6.1 在新测试文件创建最小 fake DOM 元素，实现 `innerHTML`、`textContent`、`disabled`、`classList.toggle`、`setAttribute/removeAttribute`；不要依赖完整浏览器或复制 listener-heavy 的 `dom.test.ts`。
- [ ] 6.2 通过公开 `renderUsage(elements, view)` 构造一条 canonical account，带 strict/legacy 拆分；断言账号只一行、表头无“来源槽位”、出现“严格”和“推定”徽标、TOKENS 是合计。
- [ ] 6.3 构造全未定价 cost（`unpriced_tokens > 0`、`total_cost_micros == 0`、unknown model），断言出现“不可定价”、未定价 token、模型名和“未按近似模型估算”。
- [ ] 6.4 构造部分定价 cost，断言显示已知金额和“部分定价”，不显示 `N/A` 或 `$0.0000` 代替未知部分。
- [ ] 6.5 分别 render idle/busy/queued，断言刷新按钮包含独立 `.usage-refresh-label` 和三个 `.usage-refresh-dot`，状态文案正确，busy/queued 时 `aria-busy="true"` 且 disabled，idle 恢复。
- [ ] 6.6 运行红灯：

```powershell
npm run test:front -- src-tauri/shared/front/usage/render.test.ts
```

Expected: 因当前来源槽位列、`N/A` 和圆圈/textContent 实现而失败。

- [ ] 6.7 提交：

```powershell
git add src-tauri/shared/front/usage/render.test.ts
git commit -m "test: define usage account presentation states"
```

---

## Task 7：依次执行五个 UI skill 并固化小型原型

**Files:**

- Modify: `progress.md`
- Optional temporary evidence only: `D:/Codex/.codex/run-evidence/codex-account-switch/usage-refresh-prototype/`

- [ ] 7.1 读取并执行 `awesome-design-md`：选择与当前桌面工具一致的“紧凑、中性、数据优先”视觉约束；不引入新设计系统或大面积品牌色。把选定约束写入 `progress.md`。
- [ ] 7.2 读取并执行 `design-taste-frontend`：给出账号表密度、徽标层级、按钮几何、动效幅度的具体审查结论；至少形成 4 条可直接映射到 CSS 的约束。
- [ ] 7.3 读取并执行 `ui-ux-pro-max`：检查 idle/busy/queued/error 状态机、键盘可用性、`aria-busy`、禁用语义、窄宽度表格和 reduced motion；把发现写入 `progress.md`。
- [ ] 7.4 读取并执行 `prototype`：在一次性原型中比较 idle/busy/queued 三态，验证文字与三点独立 DOM、按钮不抖动、中文不压缩。原型只回答状态和布局问题，不进入生产 bundle。
- [ ] 7.5 用本地浏览器或截图检查原型；若三态宽度或视觉中心不稳定，先调整原型，再记录最终参数。
- [ ] 7.6 读取 `frontend-design`，用前四个 skill 的结论作为生产实现 brief；明确禁止重新设计整个 Usage 页面。

本任务不提交一次性 evidence；只提交对后续实现有持续价值且属于 project/agent 分类的文档。`progress.md` 保持本地计划状态，不纳入产品提交。

---

## Task 8：实现账号表、不可定价说明和三点刷新状态

**Files:**

- Modify: `src-tauri/shared/front/usage/render.ts`
- Modify: `src-tauri/shared/front/i18n.ts`
- Modify: `src-tauri/shared/front/base.css`
- Test: `src-tauri/shared/front/usage/render.test.ts`

- [ ] 8.1 在 `i18n.ts` 增加中英双语 key：`usageAttributionBreakdown`、`usageAttributionStrict`、`usageAttributionLegacy`、`usageCostUnpriceable`、`usageCostPartiallyPriced`、`usageCostUnpriceableReason`。中文主文案固定为“不可定价”“部分定价”“严格”“推定”。
- [ ] 8.2 重构 `formatUsageCost` 为返回可访问的 HTML/展示模型；全未定价输出主标签和解释 title，部分定价输出金额+状态。模型名从 account/model 明细中收集、去重并转义。

```html
<span class="usage-cost usage-cost--unpriced" title="未定价 12,345 tokens；模型 gpt-x-unknown；未按近似模型估算">不可定价</span>
```

- [ ] 8.3 修改 `buildUsageAccountTable` 列顺序为账号、归属拆分、官方来源、请求数、TOKENS、费用；彻底移除来源槽位单元格和表头。每个 breakdown 使用紧凑 badge，0 值 bucket 不渲染。
- [ ] 8.4 修改 `renderUsageRefreshButton`，不再直接覆盖 `textContent`，而是设置：

```html
<span class="usage-refresh-label">刷新中</span>
<span class="usage-refresh-dots" aria-hidden="true">
  <span class="usage-refresh-dot"></span>
  <span class="usage-refresh-dot"></span>
  <span class="usage-refresh-dot"></span>
</span>
```

idle 隐藏 dots；busy/queued 显示 dots。可读状态来自 label/`aria-label`，dots 必须 `aria-hidden`。

- [ ] 8.5 删除 `.usage-refresh-button::before` 和 `usage-refresh-spin`；将按钮设为 inline-flex、固定最小宽度、居中 gap。三点使用错峰透明度/轻微位移动画；`prefers-reduced-motion: reduce` 下移除位移，仅保留低幅透明度或静态点。
- [ ] 8.6 为 attribution badges 和不可定价状态添加复用 CSS；颜色保持中性高对比，不把推定伪装成错误，也不让费用状态只靠颜色表达。
- [ ] 8.7 运行前端定向测试：

```powershell
npm run test:front -- src-tauri/shared/front/usage/render.test.ts
```

Expected: 新测试全部通过。

- [ ] 8.8 运行 TypeScript/Vite 构建：

```powershell
npm run build
```

Expected: `tsc` 与 Vite build 均成功。

- [ ] 8.9 提交：

```powershell
git add src-tauri/shared/front/usage/render.ts src-tauri/shared/front/usage/render.test.ts src-tauri/shared/front/i18n.ts src-tauri/shared/front/base.css
git commit -m "feat: clarify usage attribution and refresh states"
```

---

## Task 9：全链路回归、浏览器验收与对抗性审查

**Files:**

- Modify if review finds defects: files touched in Tasks 1–8
- Evidence: `D:/Codex/.codex/run-evidence/codex-account-switch/usage-account-accuracy/`

- [ ] 9.1 运行格式化与 Windows 定向检查：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run test:front
npm run test:rust:windows
npm run check:rust:windows
npm run build
npm run check:windows-release-hygiene
```

Expected: 全部 exit 0。若 `cargo fmt --check` 失败，只运行 `cargo fmt --manifest-path src-tauri/Cargo.toml` 做机械格式化，再重跑检查。

- [ ] 9.2 启动本地应用，使用真实 Usage 页面验证：相同账号只有一行、无来源槽位列、严格/推定相加正确、未知价格不再显示 N/A、刷新三态无圆圈压字。
- [ ] 9.3 分别以正常宽度和窄窗口截图；验证表格可横向滚动而不遮挡关键字段，按钮 idle/busy/queued 宽度稳定，中文 label 不跳动。
- [ ] 9.4 开启系统 reduced-motion 或 DevTools media emulation，验证三点无位移动画且状态仍可识别。
- [ ] 9.5 启用多子代理进行对抗性审查：
  - 账号身份审查：攻击传递归并、碰撞、不同真实账号误合并、旧 snapshot 倒灌。
  - 数据守恒审查：攻击 fact 重复累计、scope/model/profile 过滤差异、dashboard/trends/logs 不一致。
  - 迁移/UI 审查：攻击旧 SQLite 列缺失、SELECT 下标、未知价格误报零、ARIA/reduced-motion。
- [ ] 9.6 对每条有效发现先写回归测试，再做最小修复；无效发现也在 `progress.md` 写清理由，避免重复调查。
- [ ] 9.7 运行 `verification-before-completion`，保存最终命令、exit code、关键截图路径和 git diff 摘要到 evidence 目录。
- [ ] 9.8 检查变更范围：

```powershell
git status --short
git diff --check
git diff --stat HEAD~1
```

Expected: 无空白错误；无无关文件；`task_plan.md`、`findings.md`、`progress.md` 仍为本地工作文件，除非收尾 workflow 明确决定处理。

- [ ] 9.9 提交审查修复（仅当确有改动）：

```powershell
git add src-tauri/shared/runtime src-tauri/shared/front
git commit -m "fix: close usage accuracy review gaps"
```

---

## Task 10：收尾与交付选择

- [ ] 10.1 使用 `finishing-a-development-branch` 检查提交序列、未跟踪文件和集成方式；未经用户授权不 push、不建 PR、不替换安装目录。
- [ ] 10.2 如果本次确实生成了新的 Windows `codex_switch.exe`，严格按 `LOCAL_PATCHES.md` 和项目 AGENTS 的远端部署规则处理；仅完成源码测试/build 不触发安装替换。
- [ ] 10.3 向用户报告：账号归并口径、token 守恒证据、N/A 的原始含义与新展示、刷新按钮结果、测试命令，以及仍存在的旧 ledger 无法验证身份边界。

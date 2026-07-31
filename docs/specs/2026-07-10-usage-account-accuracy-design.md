---
type: reference
title: "Usage 统计、账号归并与刷新状态设计"
tags: [project-docs, reference]
intent: "Usage 统计、账号归并与刷新状态设计"
---

# Usage 统计、账号归并与刷新状态设计

日期：2026-07-10

> 状态（2026-07-14）：已实施并归档；本文保留为 Usage 账号归并与刷新状态的历史设计依据。涉及 macOS/Windows 共享边界的表述仅反映实施当时的架构背景；当前产品仅支持 Windows x64。

## 目标

本次变更解决五个用户可见问题：

1. token 总量与账号归属可解释、可验证，不重复计算。
2. 同一 OpenAI 账号跨槽位、跨历史身份形态时只显示一行。
3. 严格证明与历史推定可以合并进同一账号，同时保留各自 token 子计。
4. 费用无法可靠计算时显示中文“不可定价”及原因，不把未知费用显示为零。
5. 刷新按钮移除与文字冲突的旋转圆圈，改为稳定的文字脉冲点状态。

## 非目标

- 不重写或迁移现有 usage ledger。
- 不用近似模型价格估算未知费用。
- 不改变 macOS/Windows 平台专属运行时边界；本次 usage 契约保持在现有 shared 模块内。
- 不把 cached input 或 reasoning output 再次加入 token 总量。

## 已确认术语与口径

### 同一账号

两条身份记录共享以下任一稳定字段时，视为同一账号：

- 相同的非空 `account_id`；或
- 忽略大小写后相同的非空邮箱。

该关系允许传递归并。例如 A 与 B 共享邮箱、B 与 C 共享 account_id，则 A、B、C 属于同一 canonical account。

### Token 总量

`real_total_tokens = input_tokens + output_tokens`。

- `cached_input_tokens` 是 input 的子项。
- `reasoning_output_tokens` 是 output 的子项。
- 两个子项只用于明细与成本拆分，不再次加入总量。

### 历史推定

usage 事件缺少 account_id/email 时，允许按相同 profile 的 attribution 时间线归属到事件发生前最近一次已确认账号。禁止使用事件之后的登录或当前槽位账号回填历史。若事件之前没有已确认账号，事件保持“归属不明”。

## 当前问题机制

当前 `account_identity_key` 同时哈希 account_id 与邮箱。相同账号在 email-only、account-id-only、双字段三种证据形态下可能产生不同 key。当前 label fallback 遇到同一邮箱对应多个 identity key 时会判为歧义，最终把相同账号拆成多行。

`UsageFact`、`UsageEvidence` 与 `AttributionRecord` 当前只保存组合 identity hash 和账号标签，不保存可独立比较的身份别名。汇总阶段因此无法稳定连接 account-id-only 与 email-only 的证据。

费用列当前在 `unpriced_tokens > 0 && total_cost_micros == 0` 时显示 `N/A`。其含义是模型没有匹配到可靠价格，不是费用为零。

刷新按钮当前通过绝对定位的 `::before` 圆环和不对称左 padding 拼接 busy 状态。中文文案下，图标、文字与按钮视觉中心发生冲突。

## 架构

### 1. 保持事实采集与去重不变

现有 session 解析、usage delta、fingerprint/request 去重和模型定价流程继续作为事实来源。账号归并只改变 fact 的归属键，不复制 fact，也不重新计算 delta。

每条 accepted fact 在一次汇总中最多累加一次。

### 2. 增加向后兼容的身份别名

Attribution/Evidence 增加可选 identity alias 字段：

- account-id alias：由规范化 account_id 生成不可逆内部键；
- email alias：由小写、去首尾空白后的邮箱生成不可逆内部键；
- legacy identity key：保留现有 v1 组合 hash，兼容旧 ledger。

不持久化 access token、id token 或其他 OAuth 凭证。新增字段均为可选字段，旧 ledger 无需迁移即可读取。

### 3. CanonicalAccountResolver

汇总前创建 resolver，输入为当前查询范围内的 usage facts 与 attribution 证据。

resolver 按以下顺序建立连接：

1. 相同 account-id alias；
2. 相同 email alias；
3. 相同 legacy identity key；
4. 旧记录中可识别为邮箱的规范化 account label；
5. 无身份记录按相同 profile 的 latest-prior attribution 归入已存在组件。

共享任一 alias 的身份节点进入同一连通分量。canonical key 优先使用 account-id alias，其次 email alias，再次 legacy identity key。查询收到旧 alias 时，resolver 先映射到当前 canonical component，保证账号筛选、趋势和日志与账号表使用同一归并结果。

对于旧 ledger，resolver 可以读取当前仍存在的 profile auth snapshot：只有当该 snapshot 重新计算出的 v1 identity key 与历史记录完全一致时，才把其 account-id/email aliases 附加到历史节点。不能通过 hash 验证的旧记录不得仅凭当前槽位名称强行连接；它们只适用已批准的 latest-prior 规则。

### 4. 汇总与归属拆分

每个 canonical account 生成一个 `OpenAiUsageAccountSummary`。

账号总计继续包含：

- 请求数；
- input/cached input/output/reasoning output；
- `real_total_tokens`；
- cost breakdown；
- 模型明细。

同时增加向后兼容的归属拆分：

- `strict_proven` 请求数与 token；
- `legacy_profile_attributed` 请求数与 token。

全局 total 必须等于各已归属账号与归属不明 bucket 的无重叠总和。严格与历史推定子计之和必须等于该账号总计。

## 界面设计

### 账号汇总表

删除“来源槽位”列。槽位是本地存储位置，不是账号身份。

新列顺序：

1. 账号；
2. 归属拆分；
3. 官方来源；
4. 请求数；
5. TOKENS；
6. 费用。

同一 canonical account 永远只显示一行。“归属拆分”用紧凑徽标展示，例如：

- `严格 180M`
- `推定 19M`

TOKENS 列显示合计，不重复加入 cached/reasoning 子项。

### 不可定价

全部 token 无可靠价格时，费用列显示“不可定价”。辅助说明或 `title` 至少包含：

- 未定价 token 数；
- 未匹配价格的模型名称；
- “未按近似模型估算”的说明。

部分 token 可定价时显示已知金额并标注“部分定价”。不得显示 `$0.0000` 来代表未知费用。

### 刷新按钮

采用已批准的方案 C：

- idle：`刷新`
- busy：`刷新中` + 三个脉冲点
- queued：`已排队` + 三个脉冲点

label 与脉冲点使用独立 DOM 元素，按钮三种状态保持稳定宽度和居中对齐。保留 `aria-busy` 和可读文案。`prefers-reduced-motion` 下脉冲点停止位移，仅允许低幅透明度变化。

## 失败处理与可观察性

- resolver 不得改变现有 fact 去重结果。
- 事件之前没有已确认账号时保持归属不明。
- identity alias 连接出现冲突时记录诊断计数；按已确认规则，共享 account_id 或邮箱仍归为同账号。
- 未知模型不阻断 token 统计，只影响费用可用性。
- 刷新失败时停止脉冲并恢复可操作状态，沿用现有错误提示。
- 账号筛选使用 canonical key；旧 key 通过 alias 映射解析，不能静默返回空结果。

## TDD seam

### 后端公开 seam

通过现有 dashboard/summary 加载入口，使用临时 session、attribution 和 ledger fixtures 验证：

- email-only 与双字段证据合并；
- account-id-only 与双字段证据合并；
- 可由匹配 profile auth snapshot 验证的旧 v1 identity key 获得 aliases；不匹配 snapshot 不得强行连接；
- 传递 identity 归并；
- 同一 fact 不重复累计；
- legacy 事件归入事件前最近账号；
- 无前序账号的事件保持归属不明；
- 严格/推定子计之和等于账号总计；
- 全局 token 与账号/bucket 合计守恒；
- 未知模型 token 计入总量但费用不可定价。

测试只调用公开加载入口，不直接测试并查集或 resolver 私有方法。

### 前端公开 seam

通过 usage DOM/render 入口输入完整 dashboard view，验证：

- 同一账号只渲染一行；
- 不再渲染来源槽位表头；
- 严格/推定徽标与 token 子计正确；
- 全未定价显示“不可定价”与解释；
- 部分定价显示金额与“部分定价”；
- idle/busy/queued 使用稳定的 label + 脉冲点 DOM；
- `aria-busy` 与 disabled 语义正确。

CSS 不做像素级单元测试；视觉通过专业 UI skills 与最终截图检查验证。

## UI 专业 Skill 执行顺序

设计批准后，UI 工作必须实际依次使用：

1. `awesome-design-md`：选择适配当前应用的视觉语言约束；
2. `design-taste-frontend`：审查密度、字体、间距、动效与反模板化质量；
3. `ui-ux-pro-max`：审查状态模型、响应式、可访问性和交互反馈；
4. `prototype`：先验证 idle/busy/queued 小型状态原型；
5. `frontend-design`：把批准方案落为生产实现。

每个 skill 必须对最终改动产生可说明的实际影响，不能只读取或挂名。

## 验收标准

- 相同 account_id 或邮箱的 usage 在账号表中只出现一行。
- 跨 profile 的同账号 usage 合并，且来源槽位列不再显示。
- token 总量严格等于 input + output；cached/reasoning 不重复计数。
- 合并前后的 accepted fact 数和全局 token 守恒。
- 历史推定只使用事件时间之前最近的已确认账号。
- 严格/推定子计清晰可见且合计等于账号总量。
- 未知模型显示“不可定价”及原因，不显示为零费用。
- 刷新按钮无圆圈压字，三种状态尺寸稳定，并支持 reduced motion。
- 后端与前端公开 seam 的新回归测试通过；Windows 定向检查、前端测试与构建通过。

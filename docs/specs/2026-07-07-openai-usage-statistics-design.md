# OpenAI 官方 Token 使用统计设计

## 目标

在 Codex Switch 中新增“使用统计”模块，导航位置放在“账号”和“设置”之间。该模块只统计 OpenAI 官方路径产生的 Codex token 消耗，默认按账号/profile 归属展示，并提供按模型拆分的 token 与费用估算。

## 第一性原理边界

使用统计只有在三件事同时成立时才可信：

1. 该事件来自 Codex 本地 session 记录中的 `token_count`，且能从累计 token 中算出非负增量。
2. 该事件发生时的有效 `base_url` 能证明是 OpenAI 官方路径。
3. 该事件能归属到一个已管理 profile，或明确进入“未归属/不计入官方消耗”。

准确性优先级高于覆盖率：无法证明官方来源或账号归属时，不把历史记录硬塞进官方统计。

## 官方 OpenAI 判定

不使用 `model_provider` 判定官方来源。`model_provider` 可能被自定义 provider、兼容层或历史记录误导，只能作为诊断信息。

官方判定使用有效 `base_url`：

- `openai_base_url` 为空，且当前 profile 是 ChatGPT/OAuth 官方登录路径时，视为官方默认路径。
- `base_url` host 明确属于官方 allowlist 时，视为官方路径。
- 自定义中转、第三方兼容 API、反向代理、未知历史 `base_url` 一律不进入官方 OpenAI 消耗。

初始 allowlist：

- `api.openai.com`
- Codex/ChatGPT 官方默认 OAuth/app-server 路径；这类路径在本项目里表现为 ChatGPT/OAuth profile 且没有自定义 `openai_base_url`。

## 历史归属规则

历史统计以保守归属为准：

- 优先使用 session 时间点附近的已管理 profile 状态：`.current_profile`、`.active_profile`、profile `auth.json` 身份、profile `profile.json` 的 `openai_base_url`。
- 如果 live root 账号能通过 `auth.json` 身份匹配到某个 profile，则归属到该 profile。
- 如果存在 profile 身份漂移或无法证明当时有效 `base_url`，该事件进入排除计数，不进入官方统计总额。
- 统计结果必须展示排除数量，避免用户误以为“没有消耗”。

## Token 解析规则

Codex session JSONL 中 `token_count` 是累计值，因此必须按同一 session 内相邻事件做差分：

- 优先读取 `payload.info.total_token_usage`。
- 缺少累计值时可读取 `payload.info.last_token_usage`，但标记为较低信心。
- 字段映射：
  - `input_tokens`：原始 input tokens。
  - `cached_input_tokens`：缓存命中 input tokens。
  - `output_tokens`：输出 tokens。
  - `reasoning_output_tokens`：展示为 output 子集，不额外计费。
- `cached_input_tokens` 必须 clamp 到 `input_tokens` 范围内。
- 差分出现负数时视为 session 状态重置，跳过该事件并记录排除原因。

## 去重规则

去重是准确性的核心要求，不能删除。

第一层：稳定事件 ID。

- session 事件使用 `codex_session:<session_id>:<event_index>`。
- 同一 ID 重复出现只保留一次。

第二层：内容指纹。

- 指纹字段：`profile_key + official_source_key + model + input_delta + cached_delta + output_delta + time_bucket + data_source`。
- 指纹命中时标记为 `deduped_by=fingerprint`。

第三层：未来代理日志 overlap。

- 如果以后接入真实 proxy/request 日志，proxy 日志优先。
- session fallback 在相近时间窗口内与 proxy 日志 token、model、profile 匹配时跳过。

## 价格估算规则

价格只在通过官方来源门禁后计算。模型名只用于价格表匹配，不能反推官方来源。

费用公式：

```text
fresh_input_tokens = max(input_tokens - cached_input_tokens, 0)
input_cost = fresh_input_tokens / 1_000_000 * input_price_per_million
cached_input_cost = cached_input_tokens / 1_000_000 * cached_input_price_per_million
output_cost = output_tokens / 1_000_000 * output_price_per_million
total_cost = input_cost + cached_input_cost + output_cost
```

OpenAI prompt cache 写入不单独收费；因此 cache creation 在 UI 中显示为 `N/A`，不是 0 token。

价格表要求：

- 使用十进制整数 micro-dollar 或 Decimal 语义，避免浮点累积误差。
- 每个价格项保存 `model_id`、input/cached input/output 每百万 token 价格、来源 URL、`checked_at`。
- 未命中价格表的模型仍统计 token，但费用显示为“未定价”，不得当作 `$0`。

## UI 要求

新模块导航顺序：

```text
仪表盘 / 账号 / 使用统计 / 设置 / 引导
```

默认视图按账号聚合，包含：

- 真实消耗 tokens：fresh input + cached input + output。
- 新增输入、缓存命中输入、输出、reasoning output。
- 总成本估算和未定价提示。
- 缓存命中率。
- 按模型拆分表。
- 排除/去重摘要，说明有多少记录因为非官方 base URL、历史无法证明、未知模型价格或重复而未进入成本总额。

视觉方向采用现有 Codex Switch 风格：浅色、本地桌面工具、数据密集、低动效。避免营销式 hero，不使用装饰性渐变或大卡片堆叠。

## 验收标准

- 自定义 `https://example.com/v1` profile 的 session 不进入官方 OpenAI 统计。
- 官方 ChatGPT/OAuth profile 且无自定义 base URL 的 session 可统计。
- `token_count` 累计值被正确差分，不把累计总量重复相加。
- 同一事件重复扫描不会改变总 token 和总价。
- cached input 按缓存价格计费，fresh input 按 input 价格计费。
- reasoning tokens 只展示，不重复加到 output 费用之外。
- 未知模型显示未定价，不显示为 `$0`。
- UI 中“使用统计”位于“账号”和“设置”之间。

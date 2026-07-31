---
type: reference
title: "OpenAI 官方模型定价目录调研"
tags: [project-docs, reference]
intent: "OpenAI 官方模型定价目录调研"
---

# OpenAI 官方模型定价目录调研

调研日期：2026-07-15  
范围：只采用 OpenAI 官方网站与官方开发者文档，目标是为本项目的动态 `PricingCatalog` 确定可审计的一手来源和失败策略。

## 结论

推荐把 [OpenAI API Pricing](https://developers.openai.com/api/docs/pricing) 作为标准价总表，把 [Models 目录](https://developers.openai.com/api/docs/models/all) 用作模型发现入口，再用 `https://developers.openai.com/api/docs/models/<model-id>` 详情页校验单个模型的标准价格、停用状态、alias 与 snapshot。不要把第三方价格库、搜索摘要或本地旧常量当成“最新官方价格”。

OpenAI 当前没有在公开文档中承诺一个稳定、结构化的 Pricing API。上述来源都是 HTML 文档，因此实现必须把它们当作会发生 DOM 漂移的外部输入：严格解析、全量校验、成功后原子发布；任何网络或解析失败都继续使用最后一次验证成功的目录（LKG），不能把失败结果覆盖到本地。

## 官方来源及结构

### 价格总表

[Pricing 页面](https://developers.openai.com/api/docs/pricing) 明确以“每 100 万 token”为单位，并按以下维度分区：

- 处理层级：Standard、Batch、Flex、Priority；
- 上下文：Short context、Long context；
- token 类型：Input、Cached input、Cache writes、Output；
- 特殊模型及工具调用另有独立分区。

因此价格记录不能只有 `model -> 三个数字`。至少应显式包含处理层级和上下文档位；首版若只支持 Standard/Short context，就必须把其他档位标为不支持，不能假设 Batch、Flex、Priority、长上下文或区域处理永远是固定倍数。

### 模型目录与详情页

[Models 总览](https://developers.openai.com/api/docs/models/all) 可发现当前及 deprecated 模型。每个 [模型详情页](https://developers.openai.com/api/docs/models/gpt-5.4) 同时给出：

- canonical model id；
- Text tokens 下的 Input、Cached input、Output 标准价；
- Snapshots 与 aliases；
- Deprecated 状态（如适用）；
- 长上下文、区域处理等模型专属计费说明（如适用）。

解析详情页时只能读取 `Pricing -> Text tokens -> Per 1M tokens` 的当前模型字段，不能从 `Quick comparison` 读取相邻模型，也不能把 UI 中 Batch/Flex/Priority 标签下的内容误当成 Standard。

## 2026-07-15 官方价格事实

以下均为 Standard、短上下文、美元/100 万 text tokens；“输入 / 缓存输入 / 输出”依次列示。

| canonical model | 输入 | 缓存输入 | 输出 | 官方依据 |
|---|---:|---:|---:|---|
| `gpt-5.6-sol` | $5.00 | $0.50 | $30.00 | [Pricing 总表](https://developers.openai.com/api/docs/pricing) |
| `gpt-5.6-terra` | $2.50 | $0.25 | $15.00 | [Pricing 总表](https://developers.openai.com/api/docs/pricing) |
| `gpt-5.6-luna` | $1.00 | $0.10 | $6.00 | [Pricing 总表](https://developers.openai.com/api/docs/pricing) |
| `gpt-5.4` | $2.50 | $0.25 | $15.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.4) |
| `gpt-5.3-codex` | $1.75 | $0.175 | $14.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.3-codex) |
| `gpt-5.2` | $1.75 | $0.175 | $14.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.2) |
| `gpt-5.2-codex` | $1.75 | $0.175 | $14.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.2-codex) |
| `gpt-5.1` | $1.25 | $0.125 | $10.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.1) |
| `gpt-5.1-codex` | $1.25 | $0.125 | $10.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.1-codex) |
| `gpt-5.1-codex-max` | $1.25 | $0.125 | $10.00 | [模型页](https://developers.openai.com/api/docs/models/gpt-5.1-codex-max) |
| `gpt-5` / `gpt-5-codex` | $1.25 | $0.125 | $10.00 | [GPT-5](https://developers.openai.com/api/docs/models/gpt-5)、[GPT-5-Codex](https://developers.openai.com/api/docs/models/gpt-5-codex) |

当前 Pricing 总表还列出 `gpt-5.5`、`gpt-5.4-mini`、`gpt-5.4-nano` 等模型；实现应通过目录发现与详情页校验更新，而不是把这张调研表重新抄成另一套长期硬编码。

`gpt-5.6-sol`、`gpt-5.6-terra`、`gpt-5.6-luna` 的上述金额来自 Pricing 页 Standard 表的独立行，不是从 5.5 或 5.4 推算。Pricing 总表本身没有为这三行提供可验证的 snapshot 或 alias；实现只有在同一轮抓取的官方 Models 目录与 GPT-5.6 Sol 详情页明确声明映射、且两个响应也进入 catalog validator 后，才允许加入 `gpt-5.6 -> gpt-5.6-sol`。任一页面缺失或结构变化都拒绝整轮目录。

`gpt-5.4` 的官方模型页列出 alias `gpt-5.4` 与 snapshot `gpt-5.4-2026-03-05`。该模型在输入超过 272K 时存在整次会话输入 2 倍、输出 1.5 倍的长上下文计费；符合条件的区域处理还可能有 10% uplift。这些规则证明单纯按模型名套一个三元价格不足以覆盖全部账单语义。

模型生命周期也必须进入目录：例如 [Models 总览](https://developers.openai.com/api/docs/models/all) 当前将 `gpt-5.2-codex` 列为 Deprecated。Deprecated 不等于立即删除价格；历史使用记录仍需按该记录发生时适用且已验证的 catalog 版本计价。当前 catalog schema 尚未表达 GPT-5.6 的显式长上下文费率，因此输入达到 272K 的 GPT-5.6 记录必须标记为 `unpriced_long_context`，不能套用短上下文价格或借用 GPT-5.4 的倍率。

## HTTP 缓存能力实测

2026-07-15 对 `https://developers.openai.com/api/docs/pricing` 执行 HEAD 得到：

```text
Cache-Control: public, max-age=0, must-revalidate
ETag: "970a4b7591bb737b1ffd78f1953401c8"
Last-Modified: Tue, 14 Jul 2026 17:40:00 GMT
```

携带 `If-None-Match` 或 `If-Modified-Since` 的后续请求均返回 `304 Not Modified`。但同日抽查多个 `/models/<id>` 详情页时只有 `Cache-Control`，没有稳定返回 `ETag` 或 `Last-Modified`。因此：

- 持久化每个 URL 的 `etag` 和 `last_modified`，服务端提供时发送条件请求；
- 不能依赖这两个响应头一定存在；200 响应还要计算原始 body SHA-256 和规范化 catalog hash；
- 304 只表示该 URL 未变，不代表整个多页面目录一致；只有一轮所有必需页面成功并通过校验后才能发布新版本。

响应头是运行时观测，不是 OpenAI 的长期兼容承诺；实现和测试不得写死上述具体值。

## 推荐更新协议

1. 应用启动后异步刷新，Usage 首屏只读本地已验证目录，不等待网络。
2. 正常周期为 24 小时：先条件请求 Pricing 与 Models 目录，再按发现结果请求所需模型详情页。
3. 每个响应只允许 `https://developers.openai.com/` 官方来源；限制响应体大小、重定向目标和超时。
4. 解析为版本化结构，例如：

   ```text
   PricingCatalog {
     schema_version, fetched_at, published_at,
     source_urls[], source_validators[], catalog_hash,
     models[] { canonical_id, aliases[], snapshots[], deprecated, rates[] }
   }
   Rate { service_tier, context_tier, input, cached_input, output, currency, unit }
   ```

5. 发布前执行全量校验：canonical id 唯一；alias/snapshot 无冲突、无环；金额非负且精度可无损转换为项目的 micro-dollar 表示；单位、币种、层级明确；本轮必需页面完整；不允许空目录或异常规模突变静默通过。
6. 将 catalog 写入临时文件并 `fsync`，再原子替换 active LKG；保留最近两个已验证版本及其来源元数据，便于回滚和解析器回归测试。
7. `pricing_version` 使用规范化 catalog 内容 hash，而不是抓取时间。只有内容真的变化时才触发账本重算。

canonical 匹配必须是显式映射：先精确 canonical id，再匹配由同一 catalog 验证的 alias/snapshot。未知模型保持 `unpriced`；禁止前缀、编辑距离或“最相近模型”猜价。

## 失败与 LKG 策略

- 网络错误、超时、403/429/5xx、非官方重定向、DOM 漂移、字段缺失、校验失败：拒绝整轮候选目录，继续使用 active LKG。
- 刷新失败按 15 分钟、1 小时、6 小时指数退避，之后最多每 24 小时重试；成功后恢复正常周期。
- LKG 超过 72 小时仅标记 `stale=true` 并在 UI 显示来源、最近成功时间和错误摘要，不停止历史成本计算，也不伪称“最新”。
- 首次运行无 LKG 时允许使用随安装包发布、经测试验证的 seed catalog，但必须显示 `source=bundled_seed`、`stale=true`，并立即后台刷新。
- 没有可靠价格的模型继续累计 token，但成本显示“不可定价/部分定价”，不得清零或套用旧模型价格。
- 解析器升级不能直接覆盖 LKG。先用保存的最近两个官方响应 fixture 做回归，再接受在线候选。

## 实现边界建议

把网络刷新、HTML adapter、catalog 校验与 Usage 计价分层：请求路径只能读取一个不可变 `PricingCatalog` 快照。动态刷新失败不会拖慢刷新统计，也不会使同一次 Usage 聚合混用两个价格版本。UI 至少展示官方来源链接、catalog 内容版本、最近成功更新时间、fresh/stale/seed 状态和最近刷新错误。

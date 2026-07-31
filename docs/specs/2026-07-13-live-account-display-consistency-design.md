---
type: reference
title: "当前账号显示一致性设计"
tags: [project-docs, reference]
intent: "当前账号显示一致性设计"
---

# 当前账号显示一致性设计

日期：2026-07-13

> 状态（2026-07-14）：已实施并归档；本文保留为实时身份解析的历史设计依据。当前产品仅支持 Windows x64，涉及 macOS 共享契约的表述仅反映实施当时的架构背景。

## 目标

Codex Switch 显示的“当前账号”必须与实时 `CODEX_HOME/auth.json` 所属的托管 profile 一致，不能仅依赖可能过期的 `.current_profile` marker。

## 范围

- 修复账号列表快照中的当前 profile、当前账号卡片和当前额度卡片。
- 修复实时额度读取所返回的当前 profile。
- 保持 Windows 与 macOS 共用的身份契约一致，不增加平台专属路径。
- 不处理、检测或兼容 `CC Switch`；它与本项目功能边界无关。

## 已知事实与机制

- `resolve_current_profile` 只读取 `.current_profile` 或旧 active marker，可能返回过期状态。
- `resolve_backup_target` 会比较实时 `auth.json` 与各托管 profile 的 `account_id`/邮箱；实时身份属于另一个 profile 时，它能返回真实所有者。
- `detect_unmanaged_live_account` 只区分“是否属于任意托管 profile”，不能指出实时身份具体属于哪个 profile。
- 当前 `load_profiles_snapshot` 和 `load_current_live_quota` 在实时身份已托管时仍使用 profile index 中的旧 marker，因此会显示错误账号。
- `profile.json` 只是展示元数据，可能因历史写回而落在错误的 profile 目录；它不能覆盖同目录 `auth.json` 所证明的账号身份。

## 方案

采用只读身份解析：账号快照和实时额度在每次加载时使用 `resolve_backup_target` 计算 effective current profile。

- 实时身份唯一匹配某个托管 profile：使用该 profile，即使 marker 指向另一个 profile。
- 实时身份完全未托管：不显示任何托管 profile 为当前账号；账号快照继续返回 unmanaged 提示。
- 实时身份不可解析：保留现有兼容行为，回退到合法 marker。
- 读取快照不修改 marker、不复制账号文件，也不触发 Codex 生命周期操作。
- 读取 profile 元数据时，以目录名和同目录 `auth.json` 的完整身份共同校验归属；元数据标签匹配 `account_id` 或 email 任一稳定标识即可，不一致或无法绑定的账号标签、套餐、订阅和额度字段全部失效，并从 auth 重新派生，设备配置字段仍保留。
- 对 API Key 或其他无法从 auth 派生账号标签的 profile，只允许保留不含账号绑定字段的纯设备配置；已有账号标签、套餐、订阅或额度一律视为无法验证并清空。
- 根状态写回托管 profile 时，只有目录声明与实时 auth 身份均匹配的 `profile.json` 才能覆盖目标；明确属于其他 profile 的元数据跳过，无效 JSON 或读取错误直接失败并保留目标文件。

不采用以下方案：

- 在每次读取前执行启动同步：会把只读操作变成磁盘写入和备份操作。
- 在前端重新实现身份比较：会复制 Rust 后端规则并产生新的状态分叉点。

## TDD seam

测试只调用现有公开加载入口：

- `load_profiles_snapshot`：构造 `marker=a`、实时身份属于 `b`，断言只有 `b` 状态为 `current`，`current_card` 也属于 `b`。
- `load_current_live_quota`：同一 fixture 下断言返回 profile `b`，并读取 `b` 的额度，而不是 `a`。
- 保留并运行已有 unmanaged drift 测试，确保实时身份未托管时仍不显示伪造的当前账号。
- 构造 `account_backup/b/profile.json` 声明 `folder_name=c` 的污染 fixture，断言读取 `b` 时不会显示 `c` 的标签、套餐或额度。
- 构造根 `profile.json` 声明 `c`、写回目标为 `b` 的 fixture，断言 `b/auth.json` 正常更新而 `b/profile.json` 不被覆盖。
- 覆盖 `folder_name=b` 与旧格式缺失 `folder_name`、但 `account_label` 属于 `c` 的绕过路径；另验证无效根 JSON 会 fail closed。
- 覆盖 auth 无账号标签时的读取与写回，并验证 profile 重命名只能持久化清洗后的身份字段。
- 覆盖同一账号在 `account_id` 标签与 email 标签之间切换的兼容路径，避免误清空正确套餐和额度。

## 失败模式

- 多个 profile 共享同一身份时，沿用现有 profile 枚举的确定性所有者选择；本次不扩展重复账号治理。
- profile index 缺少解析出的所有者时，不构造虚假 current card；返回 profile 名但额度为空，沿用现有失败语义。
- 不因 marker 过期而把实时根状态写回错误 profile。
- 不信任与目录身份冲突的展示元数据；宁可暂时不显示套餐或额度，也不能把另一个账号的数据拼到当前账号卡片。

## 验收标准

- managed drift fixture 下，账号快照和实时额度都选择实时身份所有者。
- unmanaged drift、正常 marker 匹配和身份不可解析行为不回归。
- 已污染的跨 profile 元数据读取时失效，后续写回不会继续传播污染。
- Windows 定向 Rust 测试通过。
- 不新增依赖，不修改前端，不触碰 `CC Switch`。

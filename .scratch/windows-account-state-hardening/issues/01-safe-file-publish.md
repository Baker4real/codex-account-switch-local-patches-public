# 统一 Windows 关键文件的安全发布与替换语义

Status: archived/closed

对应私有 GitHub issue：[#1](https://github.com/Baker4real/codex-account-switch-local-patches/issues/1)（已关闭）。本文件仅保留为历史验收记录。

## What to build

为 Windows 账号状态文件建立统一的安全发布机制，让 `auth.json`、`profile.json`、`profiles.json`、root `config.toml`、quota cache 等关键文件在刷新、切换、备份和 metadata/index 更新过程中不会因为中断、复制失败、杀软占用或并发写入而丢失旧内容。

这个 slice 要覆盖两类现有风险：一类是 `copy_entry` 对文件先删除目标再复制，影响 switch overlay、live state backup、refresh copy-back 和 live auth sync；另一类是现有多个“atomic rename”/直接写 helper 在 Windows 目标已存在时语义不统一，导致失败恢复和并发 tmp 行为不可预测。

## Acceptance criteria

- [ ] 关键文件发布不再使用 remove-then-copy；旧目标文件必须保留到新文件成功发布之后。
- [ ] switch overlay、live state backup、refresh app-server copy-back、active live auth sync 都使用同一类 staged copy / safe replace 语义。
- [ ] profile metadata、root metadata、profiles index、root config、chatgpt API auth 写入、quota cache 写入要么复用统一 helper，要么明确留下安全等价的实现和测试。
- [ ] 临时文件名是唯一 sibling temp，不使用固定 `profiles.json.tmp` 这类会被并发 writer 共享的路径。
- [ ] 目标文件已存在时，Windows 替换行为可用；替换失败时旧文件仍可读。
- [ ] 有 fault-injection 或等价测试覆盖：目标已有旧文件、stage 成功但 publish 失败、并发 writer 使用不同 tmp 名。

## Blocked by

None - can start immediately

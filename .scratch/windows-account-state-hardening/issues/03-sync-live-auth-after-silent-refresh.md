# silent refresh 后同步 active profile 的 live auth

Status: ready-for-agent

## What to build

当 Windows dashboard 的 silent active quota refresh 或 bulk plan refresh 触发 OAuth token rotation 时，如果被刷新的 profile 是当前 active profile，要把刷新后的 profile `auth.json` 安全发布到 live `~/.codex/auth.json`。这样后台刷新不会让 active profile 的备份 auth 与实际 CLI/Codex 使用的 live auth 发生漂移。

## Acceptance criteria

- [ ] `chatgpt_api` refresh 结果能让调用方判断本次是否发生 OAuth token rotation，或提供等价的 post-refresh 同步 hook。
- [ ] silent active quota refresh 发生 token rotation 且目标是 current profile 时，会同步 live auth。
- [ ] bulk plan refresh 遍历到 current profile 且发生 token rotation 时，会同步 live auth。
- [ ] inactive profile 的 silent/bulk refresh 不会修改 live `auth.json`。
- [ ] live auth 同步使用 Issue 1 的安全发布语义，不使用 remove-then-copy。
- [ ] 相关写入遵守 Issue 2 的 shared lock 约束，避免与 switch/login/explicit refresh 并发写状态。

## Blocked by

- 01-safe-file-publish.md
- 02-lock-refresh-switch-prework.md

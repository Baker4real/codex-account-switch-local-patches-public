# Windows refresh 与 switch prework 纳入 shared lock

Status: archived/closed

对应私有 GitHub issue：[#2](https://github.com/Baker4real/codex-account-switch-local-patches/issues/2)（已关闭）。本文件仅保留为历史验收记录。

## What to build

让 Windows 中所有会修改账号状态的 refresh、switch prework、login/switch 路径都遵守同一个 `.switch.lock` 互斥边界。完成后，refresh 不会与 switch/login 并发写 profile auth、live auth、quota、metadata 或 index；Windows switch 的 quota prework 也不会在 shared switch core 加锁前提前写入状态。

## Acceptance criteria

- [ ] Windows `refresh_profile` 在进入任何 auth/quota/metadata/index 写入前获取 shared process lock。
- [ ] Windows switch 的 current quota sync 和 target quota prime 移入持锁区，或由 shared switch core 在持锁状态下调用。
- [ ] target profile 不存在时返回 `PROFILE_NOT_FOUND`，且不会提前创建或写入 target metadata。
- [ ] target profile 缺少 auth 时返回 `PROFILE_AUTH_MISSING`，且不会提前写入 target metadata。
- [ ] refresh 遇到 switch/login 正在进行时不会写 profile auth、live auth、quota、metadata 或 index。
- [ ] switch/login 遇到 refresh 正在进行时按设计返回 busy 或等待，但不能并发写账号状态。

## Blocked by

None - can start immediately

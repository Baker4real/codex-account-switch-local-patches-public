# 统一 Windows Base URL 对 API key 与 OAuth profile 的语义

Status: ready-for-agent

## What to build

统一 Windows Base URL 行为，避免 ChatGPT/OAuth profile 的 metadata Base URL 被无条件写入 root `config.toml`，导致账号登录、切换或刷新走到错误 endpoint。完成后，产品语义、Windows hook、profile action、UI 文案和测试都应一致。

建议语义：Base URL 只对 API key profile 写入 root config；非 API key profile 要么拒绝保存 Base URL，要么仅保存为 metadata 注记并确保 root config 被清理。

## Acceptance criteria

- [ ] Windows switch hook 不再无条件把 profile metadata 的 Base URL 写入 root config。
- [ ] 当前 active OAuth profile 保存 Base URL 时，不会让 root `config.toml` 保留 `openai_base_url`。
- [ ] 当前 active API key profile 保存 Base URL 时，root `config.toml` 正确写入。
- [ ] 清空 Base URL 会从 root config 移除对应 assignment。
- [ ] UI 文案与最终语义一致，不再出现“代码允许但文案说会失效”的矛盾。
- [ ] 更新或替换现有允许非 API key profile 写 root Base URL 的测试。

## Blocked by

None - can start immediately

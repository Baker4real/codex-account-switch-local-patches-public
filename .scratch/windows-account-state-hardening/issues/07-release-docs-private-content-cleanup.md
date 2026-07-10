# 清理 Windows 发布与用户可见说明中的私有和过期内容

Status: archived/closed

对应私有 GitHub issue：[#7](https://github.com/Baker4real/codex-account-switch-local-patches/issues/7)（已关闭）。本文件仅保留为历史验收记录。

## What to build

清理 Windows 运行时、发布脚本和用户可见 Guide 中的私有路径、真实账号标识、TODO 和过期注释。完成后，发布产物不应携带维护者本机路径或真实邮箱，Guide 页面不显示 TODO，bulk refresh 注释与真实行为一致，Windows build scripts 可被其他 Windows 环境复用。

## Acceptance criteria

- [ ] 移除源码中的真实邮箱硬编码；登录浏览器路由改为本机配置或 profile metadata 显式字段，默认不按邮箱字符串路由。
- [ ] 更新相关测试，使用非真实 `.example` 测试账号或配置字段，不再引用真实邮箱。
- [ ] 更新检查不再默认优先尝试 `D:\Codex\...` 这类本机绝对 PowerShell 路径；如需本机路径，必须通过 env/config 注入。
- [ ] Windows MSVC build/check scripts 不硬编码 `D:\Codex\.cargo` / `D:\Codex\.rustup`；默认从环境或系统 rustup 解析。
- [ ] Windows Guide HTML fallback 和 i18n 文案无用户可见 TODO，并解释 refresh 成本、`.switch.lock`、`account_backup`、Base URL 语义。
- [ ] bulk plan refresh 注释说明真实行为：启动/换日触发、6 小时新鲜度跳过、仅 access token 过期或 401 时 OAuth token rotation。
- [ ] 加入或更新静态检查，至少覆盖 Windows front/i18n 中无 `TODO`，以及 runtime/release scripts 中无真实邮箱和本机绝对路径。

## Blocked by

- 03-sync-live-auth-after-silent-refresh.md
- 05-base-url-auth-mode-semantics.md

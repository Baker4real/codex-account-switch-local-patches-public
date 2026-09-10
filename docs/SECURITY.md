---
type: reference
title: "Windows 安全边界"
tags: [project-docs, reference]
intent: "Windows 安全边界"
---

# Windows 安全边界

## 敏感文件

以下文件可能包含有效的认证 token：

- `%CODEX_HOME%\auth.json`
- `%CODEX_HOME%\account_backup\<profile>\auth.json`

按密钥处理，不要复制到 issue、截图、日志或公共仓库。

## 建议保护

1. 在 Windows 上限制备份目录和 CLI shim 目录的 NTFS 权限：
   - `icacls "%CODEX_HOME%\account_backup" /inheritance:r /grant:r "%USERNAME%":F`
   - `icacls "%CODEX_HOME%\bin" /inheritance:r /grant:r "%USERNAME%":F`
2. 永远不要把 token 文件提交到 Git 仓库。
3. 避免把账号备份目录同步到公共云盘。

## 威胁模型摘要

本项目在本机保存和切换 token；登录与 quota 刷新会通过 Codex CLI、ChatGPT Web API 或 `codex app-server` 访问官方 OpenAI / ChatGPT endpoint。按设计不会把 token 发送到第三方服务。

额度重置卡详情请求只把当前刷新后的 access token 和账号作用域 header 发往官方 ChatGPT backend。实现不得记录 Authorization、cookie、token、完整账号 ID、完整卡片 ID 或失败响应正文；持久化与前端模型只保留可用数量、发放时间和到期时间。详情接口失败不能让已经成功取得的主 quota 一并失败。

主要风险是 Git、截图、共享终端、不安全备份或宽松 Windows ACL 导致 token 意外暴露。

本机 Windows Edge 浏览器登录补丁只对 `profile.json` 明确设置 `login_browser` 为 `edge` 的 profile 复用用户正常 Edge 状态，不创建独立浏览器 profile。这移除了旧的浏览器 profile 缓存面，但也意味着 Edge 当前登录的 ChatGPT 账号就是 OAuth 返回的身份。应用必须保留写入前身份检查，避免已知 profile 被另一个浏览器账号静默覆盖。

profile 切换有同样的身份边界：`.current_profile` 和 `.active_profile` 只是本地提示，不能证明根目录 `auth.json` 当前属于哪个 ChatGPT 账号。把根目录账号状态写回 profile 前，应用必须将 live `auth.json` 身份与已知 profile 身份比较；发现未管理漂移时拒绝写入，不能覆盖过期槽位。

profile 切换使用的 Windows Store 桌面生命周期也有独立的身份边界。镜像名或 PID 只能作为发现提示：发送 `WM_CLOSE` 或终止 Store 应用进程前，应用必须打开进程，并用 `GetPackageFullName` 验证句柄仍属于 `OpenAI.Codex_*__2p2nqsd0c76g0`。`taskkill /IM Codex.exe`、`taskkill /IM ChatGPT.exe` 这类按镜像名的命令，以及未验证的 Store 应用进程树终止，都不符合这条边界。登录取消是独立流程：可以清理已由应用持有的 live `Child` 句柄 PID 所根的 shim 进程树。

手动“重启 Codex”（主窗口按钮、CLI `restart`、桌面「重启 Codex」快捷方式）同样受这条边界约束，并与账号切换共用生命周期锁。它必须在稳定确认受管包进程全部退出后只启动一次，并等待受管进程重新出现；不能用按镜像名终止、并发重启或未验证启动成功来替代。

桌面应用静默后，账号切换按事务处理。失败时必须在重新启动 hook 前恢复根账号/配置文件、按身份路由的源 profile、活动标记和 profiles index。恢复不完整时返回 `SWITCH_RECOVERY_FAILED`，不自动重启 Codex。静默失败可能留下存活的 Store 进程；此时用户应按错误提示手动关闭 Codex，再修复账号文件。

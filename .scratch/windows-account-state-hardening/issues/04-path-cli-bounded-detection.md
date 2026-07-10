# Windows PATH 与 CLI 检测有界且非破坏性

Status: archived/closed

对应私有 GitHub issue：[#3](https://github.com/Baker4real/codex-account-switch-local-patches/issues/3)（已关闭）。本文件仅保留为历史验收记录。

## What to build

修复 Windows 安装和 CLI 检测中的两个失败模式：读取 User PATH 失败时不能把 PATH 当成空字符串写回；`where codex` 不能因为慢 PATH、网络路径或损坏目录无限阻塞启动、安装状态刷新、设置页 redetect 或建议路径扫描。

## Acceptance criteria

- [ ] PowerShell 读取 User PATH 的命令非 0 退出时返回 `PATH_READ_FAILED`，不会继续写 User PATH。
- [ ] 只有命令成功且 stdout 为空时，才把 User PATH 视为真实空值。
- [ ] 安装流程在 PATH 读取失败时不会写 `path_added_by_installer` 成功状态。
- [ ] `discover_real_codex_cli_path` 中的 `where codex` 使用硬超时；超时只跳过 where 结果，不阻塞整个发现链。
- [ ] `suggested_codex_cli_paths` 中的 `where codex` 使用同样的有界执行策略。
- [ ] `codex --version` 现有 timeout 语义保持不退化。
- [ ] 测试覆盖：PATH read 非 0 不写 PATH；真实空 PATH 可写 managed bin；fake `where` 卡住时 redetect 不无限等待。

## Blocked by

None - can start immediately

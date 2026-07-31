---
type: reference
title: "Windows 双机部署环境"
tags: [project-docs, reference]
intent: "Windows 双机部署环境"
---

# Windows 双机部署环境

> 状态：设备专属配置快照；核对日期为 2026-07-17。本文不保存密码或私钥，通用传输规则以 `D:\Codex\.codex\docs\agent-guidance\ssh-mesh.md` 为准。

## 实时状态优先

路径和快捷方式目标可能随安装、桌面重定向或远端维护变化。每次替换前都必须经严格 SSH
重新解析快捷方式、主机身份和两项 SHA-256；若与本文不同，以实时结果为准。家里到自习室
的正式路径是 OpenSSH/SCP over Tailscale，必须显式指定该方向专用私钥与 `known_hosts`，并
固定 `BatchMode=yes`、`IdentitiesOnly=yes`、`StrictHostKeyChecking=yes` 和短连接超时。不得
通过密码、`SSH_ASKPASS`、自动接纳主机密钥或 UU 搬运正式 payload。

UU 仅在 SSH 不可用时用于 bootstrap、诊断或受限救援；它不是日常命令或文件传输路径。

## 当前本机

- Tailscale 节点：`jia-li` / `<jia-li-tailscale-ip>`
- 桌面快捷方式：`D:\Desktop\codex_switch.exe.lnk`
- 已确认 `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`
- 替换时同时更新同目录的 `codex_switch.exe` 与 x64 `WebView2Loader.dll`，并在替换前后核对 SHA-256。

## 当前远端

- 正式 SSH 目标：`14224@<zi-xi-shi-tailscale-ip>`（主机 `BAKER`）
- Tailscale 节点：`zi-xi-shi` / `<zi-xi-shi-tailscale-ip>`
- 桌面快捷方式：`D:\桌面\codex_switch.lnk`
- 已确认 `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`
- 远端 staging 根目录：`D:\Codex\.codex\run-evidence\replicator\manual-artifacts\codex-account-switch`
- 本机部署证据根目录：`D:\Codex\.codex\run-evidence\codex-account-switch`

设备 `.lnk` 的真实 `TargetPath` 和 `WorkingDirectory` 仍必须每次实时确认；上面的值只用于
检测漂移，不能替代检查。部署时的具体哈希、备份目录和进程状态属于一次性 evidence，不写入
本文件。

## 本项目部署路径

以下路径只约束 `codex-account-switch` 的 Windows 产物部署，不是其他项目的主机级规则。

1. 本机从 release 输出取得 `codex_switch.exe`、匹配 x64 `WebView2Loader.dll` 及两者 SHA-256。
2. 用严格 SSH 的只读探针确认远端 `hostname`、`whoami`、快捷方式 `TargetPath`、`WorkingDirectory`
   和现有文件哈希；不以本文件快照替代检查。
3. 通过 SCP 先把产物写入远端 D 盘 staging，再在两端核对 SHA-256。
4. 远端脚本必须成对备份 exe/DLL，只停止完整路径等于快捷方式 `TargetPath` 的进程，等待句柄
   释放后成对覆盖并复核哈希；任何失败均成对回滚。
5. 非交互 SSH 会话只能证明目标文件与进程路径，不能证明可见桌面窗口。需要 UI 可用性时，
   在自习室实际桌面从快捷方式启动并观察窗口；不要把 SSH 会话的窗口句柄超时误判为哈希部署失败。

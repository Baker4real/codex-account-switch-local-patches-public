# Windows 双机部署环境

> 状态：设备专属配置快照；核对日期为 2026-07-14。本文不是通用部署契约，也不保存密码或密钥。

## 实时状态优先

路径和快捷方式目标可能随安装、桌面重定向或远端维护变化。每次替换前都必须重新解析快捷方式；当前设备可直接复制以下 PowerShell 初始化与 inspect 命令：

不要继续使用旧快照里的 `WINDOWS@<jia-li-tailscale-ip>` 作为远端：2026-07-11
实时 Tailscale 状态确认该地址是当前机器自身的 `jia-li` 节点。自习室设备必须先通过
UU 设备列表锁定，再用 UU 终端读取其主机身份、快捷方式目标和哈希。

如果实时解析结果与本文不同，以实时结果为准，并在确认设备状态稳定后更新本文。

## 当前本机

- Tailscale 节点：`jia-li` / `<jia-li-tailscale-ip>`
- 桌面快捷方式：`D:\Desktop\codex_switch.exe.lnk`
- 2026-07-11 已收敛到稳定 `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`
- 替换时同时更新同目录的 `codex_switch.exe` 与 x64 `WebView2Loader.dll`，并在替换前后核对 SHA-256。

## 当前远端

- UU 设备名 / ID：`自习室` / `aeawutfrjia4s5i6`
- 已验证身份：主机 `BAKER`、用户 `14224`
- Tailscale 节点：`zi-xi-shi` / `<zi-xi-shi-tailscale-ip>`
- 桌面快捷方式：`D:\桌面\codex_switch.lnk`
- 2026-07-11 解析到的 `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`
- UU CLI：`D:\Program Files\Netease\GameViewer\bin\uuycmgr.exe`
- UU 远端 PowerShell：`D:\Codex\tools\PowerShell\7.6.3\pwsh.exe`
- 远端 staging 根目录：`D:\Codex\.codex\run-evidence\replicator\manual-artifacts\codex-account-switch`
- 本机部署证据根目录：`D:\Codex\.codex\run-evidence\codex-account-switch`
- 当前从本机到自习室的 Windows OpenSSH 没有可用私钥、agent 或 askpass；不要把 SSH 失败误判成设备离线。

设备 `.lnk` 的真实 `TargetPath` 和 `WorkingDirectory` 仍必须每次实时确认；上面的值只用于检测漂移，不能替代检查。

## 当前设备部署命令

先实时读取 UU 设备列表并确认 `自习室` 在线：

```powershell
& 'D:\Program Files\Netease\GameViewer\bin\uuycmgr.exe' device list
```

远端命令检查使用 `codex-gsd-codex-c-skill` 项目中已验证的 UU wrapper；该项目名只保留历史名称，不代表重新启用 GSD workflow：

```powershell
& 'D:\Codex\tools\PowerShell\7.6.3\pwsh.exe' -NoLogo -NoProfile `
  -File 'D:\Codex\projects\codex-gsd-codex-c-skill\scripts\Invoke-ReplicatorUuCommand.ps1' `
  -ManifestPath 'D:\Codex\projects\codex-gsd-codex-c-skill\config\targets.example.json' `
  -TargetId home -DeviceId 'aeawutfrjia4s5i6' -CleanupNewSession -Command '<read-only probe>'
```

大文件不能使用 UU Base64 小文件通道。已验证的救援路径是：本机只在 Tailscale IP
上临时提供只读 HTTP，UU 远端 PowerShell 下载到 D 盘隔离 staging，逐文件校验
SHA-256，随即关闭临时服务。正式覆盖前必须再次锁定 `BAKER`、快捷方式目标、
WorkingDirectory 和旧哈希；远端成对备份 exe/DLL，只停止完整路径等于目标的进程，
失败时成对回滚，成功后从快捷方式启动并独立复核进程路径、窗口响应和两文件哈希。

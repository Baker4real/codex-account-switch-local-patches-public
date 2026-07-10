# Windows 双机部署环境

> 状态：设备专属配置快照；最近核对日期为 2026-07-10。本文不是通用部署契约，也不保存密码或密钥。

## 实时状态优先

路径和快捷方式目标可能随安装、桌面重定向或远端维护变化。每次替换前都必须重新解析快捷方式；当前设备可直接复制以下 PowerShell 初始化与 inspect 命令：

```powershell
$env:CODEX_SWITCH_REMOTE_TARGET = 'WINDOWS@<jia-li-tailscale-ip>'
$env:CODEX_SWITCH_REMOTE_DESKTOP_LINK = 'D:\Desktop\codex_switch.exe.lnk'
$env:CODEX_REMOTE_STAGE_ROOT = 'D:\Codex\.tmp\codex-account-switch-deploy'
$env:CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT = 'D:\Codex\.codex\run-evidence\codex-account-switch'
npm run deploy:windows:remote-desktop -- -InspectOnly
```

`-InspectOnly` 本身不需要 `CODEX_REMOTE_STAGE_ROOT`；上面一并设置是为了 inspect 通过后可以直接执行正式部署，不必再次初始化环境。

如果实时解析结果与本文不同，以实时结果为准，并在确认设备状态稳定后更新本文。

## 当前本机

- 桌面快捷方式：`D:\桌面\codex_switch.lnk`
- 2026-07-10 解析到的 `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`
- 2026-07-10 解析到的 `WorkingDirectory`：`D:\Codex\apps\codex_switch`
- 替换时同时更新同目录的 `codex_switch.exe` 与 x64 `WebView2Loader.dll`，并在替换前后核对 SHA-256。

## 当前远端

- SSH 目标：`WINDOWS@<jia-li-tailscale-ip>`
- 桌面快捷方式参数：`D:\Desktop\codex_switch.exe.lnk`
- 远端 staging 根目录：`D:\Codex\.tmp\codex-account-switch-deploy`
- 本机部署证据根目录：`D:\Codex\.codex\run-evidence\codex-account-switch`
- 当前 askpass helper：`D:\Codex\.codex\run-evidence\codex-account-switch\ssh-askpass-home.cmd`
- 当前 known-hosts 文件：`D:\Codex\.codex\run-evidence\codex-account-switch\home_known_hosts`

askpass helper 只记录路径，不在仓库中复制其内容。脚本优先采用显式 `-AskPassPath`，其次采用调用环境的 `SSH_ASKPASS`，最后才检查 evidence 根目录下的 helper。远端 `.lnk` 的真实 `TargetPath` 和 `WorkingDirectory` 不在本文固化，必须由 `-InspectOnly` 实时确认。

## 当前设备部署命令

完成上面的环境变量初始化且确认 inspect 输出后，正式部署运行：

```powershell
npm run deploy:windows:remote-desktop
```

不想保留进程级环境变量时，也可以分别复制以下完整显式命令：

```powershell
npm run deploy:windows:remote-desktop -- -RemoteTarget 'WINDOWS@<jia-li-tailscale-ip>' -RemoteDesktopLink 'D:\Desktop\codex_switch.exe.lnk' -EvidenceRoot 'D:\Codex\.codex\run-evidence\codex-account-switch' -InspectOnly
npm run deploy:windows:remote-desktop -- -RemoteTarget 'WINDOWS@<jia-li-tailscale-ip>' -RemoteDesktopLink 'D:\Desktop\codex_switch.exe.lnk' -RemoteStageRoot 'D:\Codex\.tmp\codex-account-switch-deploy' -EvidenceRoot 'D:\Codex\.codex\run-evidence\codex-account-switch'
```

正式部署会在远端 staging 根目录下创建时间戳目录，备份旧文件，只停止可执行路径等于快捷方式 `TargetPath` 的进程，替换 exe 与 loader，并在设备两端复核哈希。复制或哈希校验失败时会自动恢复部署前备份；其他目录中的同名进程不会被停止。参数和失败语义以 [`LOCAL_PATCHES.md`](../../LOCAL_PATCHES.md) 为准。

# Windows-only 裁剪最终验证证据

日期：2026-07-13
仓库：`D:\Codex\projects\codex-account-switch`
验证基线：`d392d26 refactor: remove retired platform cfg branches`

## 1. 仓库状态

`git status --short --branch`：

```text
## main...origin/main [ahead 22]
?? remote-deploy-temp.ps1
?? remote-download-temp.ps1
?? run-remote-download-temp.ps1
```

工作区没有已跟踪文件改动；上面三个未跟踪文件是既有的外部远端操作临时脚本，未读取、未修改、未提交。最终验证提交只包含本证据文件。

验证前最近提交（按时间倒序）：

```text
d392d26 refactor: remove retired platform cfg branches
b36ae0a fix: keep update links on private releases
6f4c0d2 docs: point releases to private patch repository
504c882 docs: document Windows-only product boundary
536a4ea docs: correct retired icon paths
e091538 refactor: remove retired mobile icons
3b03ad0 refactor: retire non-Windows platform sources
6f219fe ci: publish Windows-only releases
07d23c7 refactor: collapse frontend build to Windows
5d233b9 refactor: make Rust runtime Windows-only
```

## 2. 活动源闭合扫描

执行范围为 `src-tauri`、`scripts`、`package.json`、`vite.config.ts` 和 `.github`，命令为计划中指定的退役平台引用扫描。第一次扫描发现 4 处仅用于 `allow(dead_code)` 的残留 `target_os = "macos"` 属性；已在 `d392d26` 删除，并重新扫描。

最终扫描结果：退出码 1（`rg` 表示无匹配），无禁止命中：

```text
target_os = "macos"       0
not(target_os = "macos")  0
crate::macos               0
src-tauri/mac              0
macOS-backup               0
tauri.macos / tauri.linux  0
tauri:build:linux          0
CODEX_UI_TARGET            0
@mac-front                 0
```

`LEGACY_MACOS_RUNTIME_DIRNAME = "macos"`、`account_backup\\macos` 及其回归测试仍保留，仅用于旧磁盘目录防误识别；它们不是活动平台实现或构建入口。活动源码中剩余的 macOS 文本仅为历史注释或兼容性测试说明。

## 3. Windows 全量检查

以下命令均在本机 Windows x64/MSVC 环境成功：

| 命令 | 结果 |
| --- | --- |
| `npm ci` | 成功；安装 46 个包，审计 47 个包，0 vulnerabilities |
| `npm run version:check` | 通过；版本 `1.5.12` 无漂移 |
| `npx tsc --noEmit` | 通过 |
| `npm run test:front` | 4 个测试文件、48/48 通过 |
| `npm run check:windows-release-hygiene` | 通过 |
| `npm run check:rust:windows` | 通过；`x86_64-pc-windows-msvc` |
| `npm run test:rust:windows` | 285/285 通过；0 failed、0 ignored |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | 通过 |

## 4. Windows portable 构建产物

执行：`npm run tauri:build:windows:portable`。构建脚本明确使用 `x86_64-pc-windows-msvc`，并输出 release profile：

```text
rustc 1.95.0 (59807616e 2026-04-14)
host: x86_64-pc-windows-msvc
Built application at:
D:\Codex\projects\codex-account-switch\src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe
```

| 文件 | 绝对路径 | 大小 | SHA-256 |
| --- | --- | ---: | --- |
| `codex_switch.exe` | `D:\Codex\projects\codex-account-switch\src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe` | 25,432,576 bytes | `60E10EED61023082548D5E3B3F163A72165F227B47DD41EA3AF05CDA3024B4CA` |
| x64 `WebView2Loader.dll` | `D:\Codex\projects\codex-account-switch\src-tauri\target\x86_64-pc-windows-msvc\release\build\webview2-com-sys-23577d6972a55637\out\x64\WebView2Loader.dll` | 160,320 bytes | `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C` |

loader 取自同一次 MSVC release 构建目录的 `out\\x64`，不是 arm64/x86 变体。

## 5. CodeGraph

执行：

```text
D:\Codex\tools\codegraph\current\bin\codegraph.cmd sync
D:\Codex\tools\codegraph\current\bin\codegraph.cmd status
```

结果：同步退出码 0（Already up to date）；状态退出码 0，索引为 up to date。

```text
Files:  77
Nodes:  2,575
Edges:  9,558
DB Size: 11.18 MB
Backend: node:sqlite - built-in (full WAL)
Index is up to date
```

没有创建新的 per-project index。

## 6. 远端实时只读 inspect

先执行 UU 设备列表，目标 `自习室`（DeviceId `aeawutfrjia4s5i6`）在线；随后通过
`D:\Codex\projects\codex-gsd-codex-c-skill\scripts\Invoke-ReplicatorUuCommand.ps1`，使用 `TargetId home`、远端 `D:\Codex\tools\PowerShell\7.6.3\pwsh.exe` 和 `cmd` 终端，对远端进行只读探针。没有把历史快照当作当前事实。实时身份与目标如下：

- 主机：`BAKER`；用户：`14224`。
- 快捷方式：`D:\桌面\codex_switch.lnk`。
- `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`。
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`。
- 当前目标进程存在，且 `ExecutablePath` 精确等于上述 `TargetPath`；本次只读探针未停止该进程。

实时目标文件哈希：

| 文件 | 旧大小 | 旧 SHA-256 |
| --- | ---: | --- |
| `codex_switch.exe` | 25,399,808 bytes | `2E4E20F4E36D45B5EBB393C9456AB28A8787BE2E944C3E3AF941C4D3314FC0BE` |
| `WebView2Loader.dll` | 160,320 bytes | `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C` |

与本地产物比较：loader 哈希一致；exe 为新构建哈希 `60E10EED61023082548D5E3B3F163A72165F227B47DD41EA3AF05CDA3024B4CA`，因此远端仍是旧 exe。

SSH `-InspectOnly` 也已尝试，但本机没有可用的远端认证，返回 `Permission denied (publickey,password,keyboard-interactive)`；该失败不覆盖 UU 的实时结果。

## 7. 部署边界

本次**未执行实际部署**：主代理要求在覆盖前完成最终对抗审查；且实时目标进程仍在运行。没有上传、覆盖、备份、回滚或启动远端文件，因此不存在“部署成功”声明，也没有可记录的部署后哈希/启动验证。待主代理批准后，下一步应使用已解析的快捷方式目标目录，成对备份并替换 exe + x64 loader，只停止完整路径匹配的进程，再校验两文件哈希并从快捷方式启动复核。

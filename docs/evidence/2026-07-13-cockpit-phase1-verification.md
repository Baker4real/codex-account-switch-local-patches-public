---
type: reference
title: "Cockpit 第一组能力验收证据"
tags: [project-docs, reference]
intent: "Cockpit 第一组能力验收证据"
---

# Cockpit 第一组能力验收证据

日期：2026-07-13/14

仓库：`D:\Codex\projects\codex-account-switch`
实现计划：[`docs/plans/2026-07-13-cockpit-phase1.md`](../plans/2026-07-13-cockpit-phase1.md)

## 1. 本轮实现范围

- Windows GUI 下受限枚举 nvm-windows、fnm、asdf 的 Codex CLI 候选，并继续使用真实 `codex --version` 探测、路径去重和候选上限。
- 账号页增加本地搜索、动态套餐筛选、可见/总数提示、活动筛选 chip 和清除操作；筛选只影响视图，不修改后端快照或账号文件。
- 增加 additional/model-specific 配额的中立数据契约、ChatGPT API/app-server allowlist 解析、session/cache/index 传递，以及普通行 / Spark 噪声 / 未知 / 过期显示语义。
- 增加本地显示偏好：默认显示普通 additional 行、默认隐藏 Spark 行；localStorage 写入失败时回滚内存状态并提示，未写入账号凭据或 profile 配置。
- 保持 Windows-only 边界；没有恢复 macOS/Linux 源树、构建、发布入口，也没有引入 API Service、自动切号或多实例功能。

## 2. Windows 全量门禁

以下命令均在 `D:\Codex\projects\codex-account-switch`、Windows x64、MSVC target 下执行：

| 命令 | 结果 |
| --- | --- |
| `npm run version:check` | 退出码 0；版本 `1.5.12` 无漂移 |
| `npx tsc --noEmit` | 退出码 0 |
| `npm run test:front` | 退出码 0；7 个测试文件、58/58 通过 |
| `npm run check:windows-release-hygiene` | 退出码 0 |
| `cargo fmt --check` | 退出码 0 |
| `npm run check:rust:windows` | 退出码 0；`x86_64-pc-windows-msvc` |
| `npm run test:rust:windows` | 退出码 0；292/292 通过，0 failed |
| `npm run tauri:build:windows:portable` | 退出码 0；release 构建完成 |

portable 构建同时执行了 `tsc` 和 Vite production build（27 个模块），产物路径为：

`D:\Codex\projects\codex-account-switch\src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe`

## 3. 构建产物哈希

| 文件 | 长度 | SHA-256 |
| --- | ---: | --- |
| `codex_switch.exe` | 25,532,416 bytes | `A0695A355CE080A944B7FCE3F39F0766630B189FE06161EEE6FFA8FA3375A163` |
| x64 `WebView2Loader.dll` | 160,320 bytes | `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C` |

loader 来源为同一次 MSVC release 构建的：

`D:\Codex\projects\codex-account-switch\src-tauri\target\x86_64-pc-windows-msvc\release\build\webview2-com-sys-23577d6972a55637\out\x64\WebView2Loader.dll`

## 4. 本机安装替换

- 目标目录：`D:\Codex\apps\codex_switch`
- 替换前只停止了 `ExecutablePath` 精确等于目标的本机进程；证据目录：
  `D:\Codex\.codex\run-evidence\codex-account-switch\local-deploy-20260713-234600`
- 替换后 exe/loader 哈希分别与上表两份构建哈希一致。
- 重启探针 PID `45452`，路径为 `D:\Codex\apps\codex_switch\codex_switch.exe`。

## 5. 远端 Windows 部署

本轮运行标识：`remote-deploy-20260713-234700`。本机证据目录：

`D:\Codex\.codex\run-evidence\codex-account-switch\remote-deploy-20260713-234700`

远端 staging / 备份根目录：

`D:\Codex\.codex\run-evidence\replicator\manual-artifacts\codex-account-switch\remote-deploy-20260713-234700`

传输沿用了 `codex-gsd-codex-c-skill` 的 `Invoke-ReplicatorUuCommand.ps1` UU wrapper 和临时只读 HTTP staging：大文件通过 HTTP 下载，UU 只负责远端命令控制、长度 / SHA-256 校验和部署；没有走 UU Base64 大文件通道。exe、loader、部署脚本均在远端逐项校验。

部署脚本报告确认：

- 设备身份：`BAKER` / `14224` / `AMD64`。
- 快捷方式：`D:\桌面\codex_switch.lnk`。
- `TargetPath`：`D:\Codex\apps\codex_switch\codex_switch.exe`。
- `WorkingDirectory`：`D:\Codex\apps\codex_switch`。
- 部署前 exe：25,431,552 bytes，SHA-256 `E2531A6F06C2954299840E3B275447C0A51504CB67AAA935357223399BE6A0F1`。
- 部署前 loader：160,320 bytes，SHA-256 `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C`。
- 部署脚本只停止精确目标 PID `30784`，并创建成对备份；新进程 PID `38520`，`MainWindowHandle=529038`、`Responding=True`。

独立后探针 `uu-post-probe-retry.json` 退出码为 0，并再次确认：

- 远端 exe：25,532,416 bytes，SHA-256 `A0695A355CE080A944B7FCE3F39F0766630B189FE06161EEE6FFA8FA3375A163`。
- 远端 x64 loader：160,320 bytes，SHA-256 `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C`。
- 快捷方式目标和工作目录未漂移，目标进程路径精确匹配。

临时 HTTP 服务使用 `<jia-li-tailscale-ip>:21303`，传输和部署完成后已停止；`http-shutdown.json` 显示无残留服务进程、无监听端口。此前工作区中提到的三个未跟踪临时脚本在本轮继续保持未重建、未加入提交，传输改用 D 盘 evidence 中已校验的副本。

## 6. 结论

计划中的第一组能力已通过前端、Rust、发布卫生、格式检查、portable build、本机启动和远端成对文件 / 窗口探针门禁。当前本机安装与远端快捷方式目标均指向同一份最终构建哈希；未执行 push。

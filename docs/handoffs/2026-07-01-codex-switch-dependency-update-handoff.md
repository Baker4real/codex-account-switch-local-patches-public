# codex-account-switch 依赖更新接手交接

生成时间：2026-07-01 08:30 +08:00

## 当前目标

从 `D:\Codex\projects\统一环境` 中整理出的 `codex-account-switch` 后续处理项，供下一轮在本项目 `D:\Codex\projects\codex-account-switch` 内继续。

当前建议的下一步不是继续改 runtime cleanup，而是处理 npm audit 指向的 Vite 安全补丁，并保持变更范围尽量小。

## 当前仓库状态

- 仓库：`D:\Codex\projects\codex-account-switch`
- 分支：`codex/clean-codex-switch-local-patches`
- 当前状态：`git status --short --branch` clean
- 当前 HEAD：`37f35c9 docs: record runtime cleanup boundaries`
- runtime cleanup 实质修复提交：`d6eea82 fix: harden runtime cleanup without pruning auth backups`

## 已整理出的外部证据

来自 `D:\Codex\projects\统一环境` 的只读 dry-run：

- Issue：`D:\Codex\projects\统一环境\.scratch\unified-env\issues\10-codex-account-switch-dependency-update-dry-run.md`
- Markdown 报告：`D:\Codex\projects\统一环境\reports\codex-account-switch-dependency-update-dry-run\PLAN.md`
- JSON 报告：`D:\Codex\projects\统一环境\reports\codex-account-switch-dependency-update-dry-run\plan.json`

该 dry-run 只读取状态，没有修改本项目，没有执行 `npm install`、`npm ci`、`npm audit fix`，也没有删除或重建 `node_modules`。

## dry-run 摘要

- 项目版本：`codex_switch@1.5.12`
- Node：`v24.15.0`
- npm：`11.12.1`
- npm cache：`D:\Codex\tools\npm-cache`
- `package-lock.json`：lockfileVersion 3，package entries 78
- 项目根和 `node_modules` 都不是 reparse point

当前顶层依赖状态：

| 包 | 当前 | latest | 建议 |
| --- | --- | --- | --- |
| `vite` | `7.1.4` | `8.1.2` | 先只做 audit fix 到 `7.3.6` |
| `typescript` | `5.9.2` | `6.0.3` | 暂不跳大版本 |
| `@tauri-apps/api` | `2.10.1` | `2.11.1` | 后续单独评估 |
| `@tauri-apps/cli` | `2.10.1` | `2.11.4` | 后续单独评估 |

`npm audit` 当前摘要：

- `vite` direct high，`fixAvailable` 指向 `vite@7.3.6`
- `postcss` transitive moderate，`fixAvailable=true`

## 建议执行范围

优先只处理安全补丁：

```powershell
Set-Location -LiteralPath 'D:\Codex\projects\codex-account-switch'
git status --short --branch
npm outdated --json --depth=0
npm audit --json
npm install vite@7.3.6 --save-dev --package-lock-only --ignore-scripts
git diff -- package.json package-lock.json
```

这一步预期只改 `package.json` 和 `package-lock.json`。不要把 `vite 8`、`typescript 6`、Tauri minor 更新和安全补丁混在同一次变更里。

不要用 `npm ci` 作为更新手段；它只会按当前 lockfile 重建旧依赖，不能让依赖保持最新。

## 验证建议

真实执行后至少跑：

```powershell
npm ls --depth=0
npm audit
npm run version:check
npm run build
npm run check:rust:windows
git diff --check
```

如果后续要替换本机安装版，再按 `LOCAL_PATCHES.md` 的已验证 Windows 编译路径执行：

```powershell
npm run build -- --mode windows
npm run tauri:build:windows:portable
```

替换 `D:\Codex\apps\codex_switch\` 时必须同时复制 `codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll`。

## 不要做的事

- 不要回退 `d6eea82` / `37f35c9` 记录的 runtime cleanup 边界。
- 不要把 `plugins`、`cache`、`sqlite` 从 runtime 共享列表移除。
- 不要把 `plugins`、`.tmp`、`cache`、`sqlite` 或 `_autosave` 加入自动删除范围。
- 不要在依赖更新 issue 里删除或重建 `node_modules`。
- 不要直接跳 `vite 8`、`typescript 6` 或合并 Tauri minor 更新。

## runtime cleanup 边界备忘

当前源码和文档已经明确：

- `plugins`、`cache`、`sqlite` 继续作为 login / refresh runtime 的共享资产。
- runtime prune 只清理隔离状态：`AGENTS.md`、`rules`、`skills`、`vendor_imports`、`memories`。
- `auth.json` autosave 只追加唯一 timestamp 快照，不在 autosave/login/refresh 路径里做保留清理。
- 备份保留策略属于独立 dry-run / apply 清理流程，不属于本项目 runtime 生命周期函数。

## 建议 skills

- `implement`：用于真实执行 Vite 安全补丁和最小验证。
- `review`：用于提交前检查依赖更新 diff 是否越界。
- `karpathy-guidelines`：保持小改动、先验证事实、避免 patch-on-patch。
- `handoff`：如果真实更新后需要切换会话，继续写本目录下的新交接。

# Windows-only 裁剪前基线

日期：2026-07-13

仓库：`D:\Codex\projects\codex-account-switch`

基线提交：`039834b`（`docs: plan Windows-only platform retirement`）

分支：`main`

## 1. 仓库与 CodeGraph 状态

执行：

```text
git -C "D:\Codex\projects\codex-account-switch" status --short --branch
## main...origin/main [ahead 11]
```

没有后续文件行，因此基线工作区干净；本地 `main` 比 `origin/main` 多 11 个提交。

最近 6 个提交：

```text
039834b (HEAD -> main) docs: plan Windows-only platform retirement
2b547a1 docs: record local account display verification
5ecbb5b fix: match profile metadata against full identity
10a7db0 fix: distinguish missing and unbound auth metadata
bf78024 fix: bind profile metadata to auth identity
1863a99 fix: reject cross-profile display metadata
```

`D:\Codex\tools\codegraph\current\bin\codegraph.cmd status`：

```text
Project: D:\Codex\projects\codex-account-switch
Files: 92
Nodes: 2,876
Edges: 10,431
DB Size: 10.85 MB
Backend: node:sqlite - built-in (full WAL)
Journal: wal
[OK] Index is up to date
```

CodeGraph `files` 显示 Windows、macOS、shared 三套运行时和多平台图标；`query macos` 找到 `scripts/macos-artifacts.mjs`、`scripts/create-macos-pkg.mjs`、`src-tauri/mac/runtime` 中的 macOS 符号。

## 2. 平台文件清单

补充使用 `rg --files`（按目录分别统计，避免 Windows 反斜杠路径造成漏报）：

| 分类 | 数量 | 路径/说明 |
| --- | ---: | --- |
| macOS 原生运行时与前端 | 13 | `src-tauri/mac/**` |
| macOS legacy shell | 9 | `macOS-backup/**` |
| iOS 图标 | 18 | `src-tauri/icons/ios/**` |
| Android 图标 | 17 | `src-tauri/icons/android/**` |
| macOS Tauri 配置 | 1 | `src-tauri/tauri.macos.conf.json` |
| Linux Tauri 配置 | 1 | `src-tauri/tauri.linux.conf.json` |
| macOS 打包脚本 | 2 | `scripts/macos-artifacts.mjs`、`scripts/create-macos-pkg.mjs` |

## 3. 引用分类

### 3.1 活跃生产/构建引用

- `package.json:14-18,30,33-35`：macOS artifact/package、Linux build、macOS app/release 命令仍可执行；`tauri:build` 和 `tauri:build:portable` 尚未固定为 Windows。
- `vite.config.ts`：根据 `process.platform`/`CODEX_UI_TARGET` 选择 UI，并注册 `@mac-front` 别名。
- `src-tauri/src/lib.rs`、`src-tauri/shared/commands/**`、`src-tauri/shared/platform/**`、`src-tauri/shared/runtime/**`：使用 `target_os = "macos"`、`not(target_os = "macos")` 和 `crate::macos` 进行平台分派。
- `src-tauri/mac/**`：macOS bootstrap、安装、刷新、进程、切换和窗口运行时；`src-tauri/win/**` 同时存在 Windows 实现。
- `src-tauri/Cargo.toml:30`：`cfg(any(target_os = "macos", windows, target_os = "linux"))` 仍声明跨平台依赖。
- `.github/workflows/build.yml:24-70`：CI 运行在 `ubuntu-latest` 并安装 Linux Tauri 依赖；`:80-107` 的 release matrix 同时构建 macOS arm64/x64、Windows x64、Linux x64；`:157-200` 仍包含 Apple signing 和 DMG 验证；release artifact 仍收集三平台产物。
- Tauri 配置：`src-tauri/tauri.macos.conf.json`、`src-tauri/tauri.linux.conf.json` 与 Windows 配置并存。

按 brief 指定表达式在生产/构建路径上的补充统计（`rg -l/-n -i`）：

| 表达式 | 文件数 | 命中行数 |
| --- | ---: | ---: |
| `target_os = "macos"` | 12 | 26 |
| `not(target_os = "macos")` | 9 | 14 |
| `crate::macos` | 8 | 16 |
| `CODEX_UI_TARGET` | 3 | 4 |
| `@mac-front` | 2 | 2 |
| `tauri:build:(macos|linux)` | 2 | 7 |
| `.dmg` | 2 | 7 |
| `.pkg` | 3 | 6 |
| `AppImage` | 2 | 2 |
| `.deb` | 1 | 1 |

### 3.2 历史文档/规划材料

- `README.md`、`README.zh-CN.md`、`docs/IMPLEMENTATION.md` 仍描述 macOS/Linux 安装、目录和下载产物；属于当前文档中的产品承诺，裁剪时需更新。
- `docs/plans/2026-04-16-macos-desktop-app.md`、`docs/plans/2026-04-20-macos-sync-latest-windows-updates.md` 是已完成或历史 macOS 规划，应在 Windows-only 裁剪时删除或明确归档。
- `CHANGELOG.md` 中旧版本 macOS/Linux 条目是历史事实，不应改写；应仅新增平台退役说明。
- `docs/specs/2026-07-13-windows-only-cockpit-alignment-design.md` 和 `docs/plans/2026-07-13-windows-only-platform-retirement.md` 是本次裁剪的现行设计/计划，不能当作待删除的产品引用。

### 3.3 lockfile 的可选平台依赖

- `package-lock.json` 包含 npm 工具的可选平台包：`@esbuild/*`（android、darwin、linux）、`@rollup/rollup-*`（android、darwin、linux）以及 `@tauri-apps/cli-*` 的 darwin/linux 构建包。这些来自跨平台工具分发，不是应用平台入口。
- `src-tauri/Cargo.lock` 仅发现 `android_system_properties`、`linux-raw-sys` 等传递依赖。
- 裁剪时不手工删 lockfile 可选条目；应由 npm/Cargo 根据 Windows 构建重新生成或保留，以避免破坏依赖完整性。

### 3.4 保留的旧磁盘格式防误识别规则

- `src-tauri/shared/runtime/paths.rs:22` 定义 `MACOS_RUNTIME_DIRNAME = "macos"`；`:148-151` 在 `is_profile_dir` 中排除 `_autosave`、`windows`、`macos`、`usage_evidence`；`:211-226` 测试确认 `account_backup/macos` 不会被识别为账号 profile。
- 这是旧磁盘格式兼容护栏，不代表继续支持 macOS。Windows-only 实施应保留排除行为（可把常量重命名为 `LEGACY_MACOS_RUNTIME_DIRNAME`），并继续保留回归测试。

## 4. 基线疑虑

1. brief 中的文件清单正则 `(^|[/\\])(src-tauri/mac|... )` 没有在目录名内部同时允许反斜杠；在 Windows 上实际只列出了 `macOS-backup/**`，漏报 `src-tauri/mac/**`、`src-tauri/icons/ios/**`、`src-tauri/icons/android/**` 和 `scripts/macos/**`。上面的目录计数使用按目录补充扫描校正了这一点。
2. brief 的内容正则包含 `tauri\.(macos|linux)`，它匹配内容中的点号而非配置文件名；因此指定命令没有列出两个配置文件。配置文件存在性已用 `rg --files ... | rg 'tauri\\.(macos|linux)\\.conf\\.json$'` 复核。
3. 本基线只冻结事实，没有修改产品代码或删除平台文件；完成裁剪前必须以这些分类区分活跃入口、历史文档、工具 lockfile 和磁盘护栏。

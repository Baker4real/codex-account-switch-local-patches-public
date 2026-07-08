# 本地补丁更新流程

当前安装版是本地补丁构建。更新检查提示有新版上游发布时，不要直接用上游安装包覆盖。

## 私有补丁栈分支策略

- 本仓库对应的私有远端 `Baker4real/codex-account-switch-local-patches` 是本地补丁栈仓库。
- 必须保留并维护 `patch-stack/*` 累计回滚检查点；不要删除、改名或迁移到 public fork。
- 普通独立私有项目不使用 `patch-stack/*`，但本仓库属于明确的 `*-local-patches` 场景。

先把上游 tag 合并进这份源码树，保留下面的本地补丁，再重新构建并替换已安装的可执行文件：

## 已验证 Windows 编译路径（优先使用）

已在 2026-07-01 于本仓库验证通过。下次需要本机补丁构建或替换
`CODEX_SWITCH_INSTALL_DIR` 指向的安装目录时，优先从这条路径开始；除非任务明确要求
CI release、GNU/MinGW 诊断或上游对比，不要先试裸 `tauri build`、
`npm run tauri:build:portable` 或其它临时路径。

```powershell
$repo = if ($env:CODEX_SWITCH_REPO) { $env:CODEX_SWITCH_REPO } else { (Get-Location).Path }
Set-Location -LiteralPath $repo
npm run build -- --mode windows
npm run tauri:build:windows:portable
```

- 前提是 Rust 已安装 `x86_64-pc-windows-msvc` target，且 Visual Studio Build
  Tools 的 C++ 工具链和 Windows SDK 可用；如果 Git for Windows 的 `link.exe`
  抢先进入 `PATH`，从 VS developer shell 运行，或确保 MSVC `link.exe` 优先。
- `npm run build -- --mode windows` 是前端 Windows mode 预检，输出到 ignored
  的 `dist\web\`。
- `npm run tauri:build:windows:portable` 是本机替换安装版的主路径。该 wrapper
  默认使用当前环境或系统 `rustup`；需要固定工具链时，通过
  `CODEX_SWITCH_RUSTUP_BIN`、`CODEX_SWITCH_CARGO_HOME`、
  `CODEX_SWITCH_RUSTUP_HOME`、`RUSTC` 或 `CARGO` 注入。它会在
  `rustc -vV` 未报告 `host: x86_64-pc-windows-msvc` 时拒绝构建。
- 已验证的 exe 产物路径：
  `src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe`。
- MSVC `--no-bundle` 输出目录下通常没有相邻的根级 `WebView2Loader.dll`；
  复制安装版时，从
  `src-tauri\target\x86_64-pc-windows-msvc\release\build\webview2-com-sys-*\out\x64\WebView2Loader.dll`
  取匹配的 x64 loader。

替换本机安装版时，只复制这两个文件到安装目录：

```powershell
$repo = if ($env:CODEX_SWITCH_REPO) { $env:CODEX_SWITCH_REPO } else { (Get-Location).Path }
$release = Join-Path $repo 'src-tauri\target\x86_64-pc-windows-msvc\release'
$install = $env:CODEX_SWITCH_INSTALL_DIR
if (-not $install) { throw 'Set CODEX_SWITCH_INSTALL_DIR to the installed Codex Switch directory.' }
Copy-Item -LiteralPath "$release\codex_switch.exe" -Destination "$install\codex_switch.exe" -Force
$loader = Get-ChildItem -LiteralPath "$release\build" -Recurse -Filter 'WebView2Loader.dll' |
  Where-Object { $_.FullName -like '*\out\x64\WebView2Loader.dll' } |
  Select-Object -First 1
Copy-Item -LiteralPath $loader.FullName -Destination "$install\WebView2Loader.dll" -Force
```

每次本项目构建或替换出新的 Windows `codex_switch.exe` 后，还必须把同一份
`codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll` 同步到远端 Windows
电脑对应安装目录，替换远端桌面入口实际指向的旧文件。远端桌面项可能是快捷方式，
不是 exe 本体；先解析远端桌面 `codex_switch*.lnk` 的 `TargetPath` 和
`WorkingDirectory`，再替换目标目录里的 `codex_switch.exe` 与
`WebView2Loader.dll`。不要把文件直接复制到 `.lnk` 路径上，也不要只复制 exe。
替换前停止远端正在运行的 `codex_switch` 进程，替换后校验远端目标文件 SHA-256
与本机构建产物一致。已知远端 `home` 的桌面入口曾位于
`D:\Desktop\codex_switch.exe.lnk`，验证过的安装根目录是
该快捷方式 `TargetPath` 所在目录；仍要以实时解析到的快捷方式目标为准。

已沉淀的远端桌面覆盖入口：

```powershell
npm run deploy:windows:remote-desktop -- -InspectOnly
npm run deploy:windows:remote-desktop
```

- 默认远端是 `WINDOWS@<jia-li-tailscale-ip>`，默认桌面入口是
  `D:\Desktop\codex_switch.exe.lnk`；需要临时覆盖时传
  `-RemoteTarget` 或 `-RemoteDesktopLink`。
- 脚本先从
  `src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe`
  和 `release\build\webview2-com-sys-*\out\x64\WebView2Loader.dll`
  读取本机构建产物并计算 SHA-256。
- `-InspectOnly` 只解析远端快捷方式和当前哈希，不上传、不覆盖；正式执行会
  上传到远端 `$env:CODEX_REMOTE_STAGE_ROOT\<timestamp>`，
  由远端脚本备份旧文件、停止 `codex_switch`、覆盖 exe+dll 并复核哈希。
- 脚本不保存密码。它复用调用环境里的 `SSH_ASKPASS`，或本机 D 盘 evidence
  目录中已有的 `ssh-askpass-home.cmd`；缺少非交互认证时，先按
  `codex-gsd-codex-c-skill` 里跑通的 DPAPI/SSH_ASKPASS 模式恢复连接。
- 每次运行的 stdout/stderr、inspect 和 deploy JSON 会落在
  `$env:CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT\remote-desktop-deploy-*`，
  用于事后追踪和回滚定位。

`npm run test:rust:windows-gnu` 只用于当前 `rustc` host 是
`x86_64-pc-windows-gnu` 的 GNU/MinGW 链接路径诊断；它不是替换安装版的主构建路径。

## 必须保留的本地补丁

- Windows 定向浏览器登录不再按 profile `b` / `c` 槽位选浏览器。仅当目标
  profile 的 `profile.json` 显式设置 `login_browser` 为 `edge` 时，登录使用
  `codex app-server`，并用用户正常 Edge 状态打开返回的 auth URL。
- 其他 profile 不强制 Chrome 或 Edge，可继续走普通 `codex login` /
  上游默认浏览器行为；不要恢复按槽位名路由浏览器的旧规则。
- 定向 Edge flow 不要再传 `--user-data-dir`、`--new-window`，也不要创建
  独立浏览器 profile。
- 由于这个 flow 复用用户正常浏览器状态，必须保留写入前账号身份校验：如果
  现有 profile 已有已知 ChatGPT 身份，而浏览器完成登录的是另一个账号，
  则必须在替换 `auth.json` 前失败。
- 不要把显式 Edge 登录路径替换成普通 `codex login`；上游 CLI 会打开
  默认浏览器，并且没有给这个 flow 暴露受支持的浏览器选择参数。
- 更新检查默认尝试 `pwsh` 再尝试 `powershell`；需要本机固定 PowerShell
  路径时，通过 `CODEX_SWITCH_POWERSHELL` 注入。
- 更新对话框必须提示：替换前需要先合并本地补丁。
- Windows real-Codex discovery 必须用 `codex --version` 和 Codex 版本行校验
  缓存路径及用户指定路径，跳过 stale shim，并探测官方 OpenAI versioned
  runtime 目录，避免 Codex 升级后应用继续指向已删除 runtime。
- 账号页 quota 读数必须保留 provenance 元数据，避免把低可信度 fallback
  或本地 session 观测结果展示得像官方 ChatGPT API 读数。
- OpenAI 使用统计必须以有效 `base_url` 作为官方来源门禁，不得用
  `model_provider` 反推官方来源；只统计默认官方路径或 HTTPS
  `api.openai.com`。`account_backup\openai_usage.sqlite` 是可重建分层 ledger，
  保存 evidence、facts 和 strict accepted events。默认总额、趋势和日志只读
  strict proven；历史槽位归因只能进入 `legacy_profile_attributed` 估算层；
  candidate / rejected / superseded 不得进入费用总额。source signature 必须覆盖
  session JSONL、归属记录、profile metadata、active marker、schema/layer/backfill
  和价格版本，确保历史归属或价格表变化后能重新同步。未知模型 token 可以展示，
  但费用必须保持未定价，不得显示为 `$0`。
- 使用统计账号/模型 facet 必须按当前时间范围重算：账号候选清除账号自身筛选
  但保留时间、模型和状态筛选；模型候选清除模型自身筛选但保留时间、账号和
  状态筛选。账号分类表应位于时间选择下方、摘要卡片前，选择一个账号后不能
  让其他同范围账号从下拉中消失。账号筛选使用 `account_group_key`，不是单个
  profile；账号聚合必须以 `account_identity_key` 为最高优先级，legacy label
  只能在唯一映射到同一 strict identity 时桥接，不能合并两个不同 strict identity。
- cc-switch 等外部只读证据冻结副本必须写到 `%CODEX_HOME%\usage_evidence\`，
  不得写入 `account_backup`。`account_backup\usage_evidence` 是系统目录污染，
  必须迁出或排除，不能出现在账号列表、Usage 账号表或 profile 扫描中。
- 本地 Windows MSVC 构建必须使用 `npm run tauri:build:windows:portable`；
  该 wrapper 默认使用环境或系统 `rustup`，需要本机固定路径时通过
  `CODEX_SWITCH_RUSTUP_BIN` / `CODEX_SWITCH_CARGO_HOME` /
  `CODEX_SWITCH_RUSTUP_HOME` / `RUSTC` / `CARGO` 注入，并在 `rustc -vV`
  报告非 `x86_64-pc-windows-msvc` host 时拒绝构建。
- 向另一台 Windows 主机同步 auth/profile 状态时，必须保留目标主机自己的
  `%CODEX_HOME%\account_backup\windows\install_state.json`；只同步 root/profile
  auth 和 metadata。一次性的 `login_runtime`、`refresh_runtime`、stale
  `.switch.lock` 可以删除，因为应用会重新生成。旧本地构建留下的
  `login_browser_profiles` 目录也可以删除；当前构建不再创建它们。
- profile 切换只能同步账号态文件 `auth.json` 和 `profile.json`；不得从
  profile 目录整棵覆盖 live `skills`、`AGENTS.md`、`rules`、`memories`、
  `vendor_imports`、插件/cache/sqlite 或其它全局运行时资产。即使旧 profile
  目录里残留这些污染条目，切换和反向备份也必须忽略它们。
- login / refresh runtime 目录是可再生运行态，但运行期间必须保持上游共享资产
  `plugins`、`cache`、`sqlite` 可用；runtime prune 只清理 `AGENTS.md`、
  `rules`、`skills`、`vendor_imports`、`memories` 等隔离状态。不要把
  `plugins`、`.tmp`、`cache`、`sqlite` 或 `_autosave` 加入自动删除范围；
  备份保留策略必须走显式 dry-run / apply 清理流程。
- 切换前的 root 写回必须按账号身份路由，不能盲信 `.current_profile` 或
  `.active_profile`。先用 live `auth.json` 的 `account_id` / `email` 匹配
  已知 profile；只有身份匹配或目标槽位是真空占位时才写入。若 live root
  已漂移到未管理账号，必须拒绝写回旧槽位并清理 active marker，防止串号覆盖。

验收目标：已安装的 `codex_switch.exe` 必须从这份已合并源码树重新构建，不能直接复制纯上游 release。做本地便携 Tauri 替换时，必须把匹配的 x64 `WebView2Loader.dll` 复制到可执行文件同一安装目录；否则桌面端可能在加载 UI 前启动失败。MSVC target 构建中，loader 可能来自目标架构的 `webview2-com-sys` build output，而不是 release 根目录。

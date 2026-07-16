# 本地补丁更新流程

当前安装版是 Windows x64 本地补丁构建。更新检查提示有新版上游发布时，不要直接用
公共上游安装包覆盖，也不要把更新检查当作自动升级器。

## Windows-only 更新边界

- 应用内更新检查只读取版本 JSON、返回 release notes URL 并打开发布页；不会下载、安装
  或覆盖任何公共二进制。
- 发现新版本时，先在源码树合并上游 tag，再保留本节列出的本地补丁，完成 Windows MSVC
  检查/测试后重新构建。只有新构建通过 exe + `WebView2Loader.dll` 哈希校验，才可替换
  已安装目录。
- 当前发布产物只有 Windows x64 NSIS `.exe`；本地便携构建仍必须走下方已验证脚本。

## 私有主分支发布策略

- 本仓库的用户自有远端是 `private`（`Baker4real/codex-account-switch-local-patches`）；正常修改只提交并推送到 `private/main`。
- 本地和远端当前只保留 `main`；提交历史与本文补丁清单共同承载累计补丁记录。不再创建、更新、推送或重新建立长期 `patch-stack/*` 分支。
- 若其他位置仍存在遗留 patch-stack refs，只作历史，默认不读取、不修改、不迁移、不删除。
- 如确有必要隔离高风险修改，Agent 必须先推荐具体临时分支并取得 owner 事前确认；owner 验收后才推进到 `private/main`，随后删除本地和远端临时分支，除非 owner 明确要求保留。
- `origin` 是第三方上游，只读且禁止推送；目标远端、归属或 `main` 基线无法证明时停止发布。

先把上游 tag 合并进这份源码树，保留下面的本地补丁，再重新构建并替换已安装的可执行文件：

## 已验证 Windows 编译路径（优先使用）

已在 2026-07-09 于本仓库验证通过。下次需要本机补丁构建或替换
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
`codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll` 同步到已配置的远端 Windows
安装目录。远端桌面入口可能是快捷方式，不是 exe 本体；必须先实时解析 `.lnk` 的
`TargetPath` 和 `WorkingDirectory`，再替换目标目录里的 `codex_switch.exe` 与
`WebView2Loader.dll`。不要把文件直接复制到 `.lnk` 路径上，也不要只复制 exe。
替换前停止远端正在运行的 `codex_switch` 进程，替换后校验远端目标文件 SHA-256
与本机构建产物一致。当前维护设备的目标、路径和认证配置快照见
[`docs/environments/windows-dual-host.md`](docs/environments/windows-dual-host.md)；快照
只用于恢复默认值，执行部署时仍以 `-InspectOnly` 的实时解析结果为准。

远端桌面覆盖入口使用显式参数；下面是通用形状，尖括号内容必须替换为真实值：

```powershell
npm run deploy:windows:remote-desktop -- -RemoteTarget '<user>@<host>' -RemoteDesktopLink '<remote-desktop-link>' -IdentityFile '<direction-private-key>' -KnownHostsPath '<direction-known-hosts>' -EvidenceRoot '<local-evidence-root>' -InspectOnly
npm run deploy:windows:remote-desktop -- -RemoteTarget '<user>@<host>' -RemoteDesktopLink '<remote-desktop-link>' -IdentityFile '<direction-private-key>' -KnownHostsPath '<direction-known-hosts>' -RemoteStageRoot '<remote-stage-root>' -EvidenceRoot '<local-evidence-root>'
```

- `RemoteTarget` 与 `RemoteDesktopLink` 对 inspect 和 deploy 都是必填项，可分别由
  `-RemoteTarget` / `CODEX_SWITCH_REMOTE_TARGET` 与
  `-RemoteDesktopLink` / `CODEX_SWITCH_REMOTE_DESKTOP_LINK` 提供。
- `RemoteStageRoot` 只有正式 deploy 必填，可由 `-RemoteStageRoot` 或
  `CODEX_REMOTE_STAGE_ROOT` 提供；`-InspectOnly` 不读取、不创建 staging，因而不要求该值。
- `IdentityFile` 与 `KnownHostsPath` 对 inspect 和 deploy 都是必填项，且必须是该方向独立的
  已存在文件。脚本固定使用 `BatchMode=yes`、`IdentitiesOnly=yes`、
  `StrictHostKeyChecking=yes` 和短连接超时；不自动接纳主机密钥，也不支持密码或 `SSH_ASKPASS` 回退。
- 本地 npm 部署入口固定调用 PATH 中的最新版 PowerShell 7（`pwsh`），不再默认回退到 Windows
  PowerShell 5.1；缺少 `pwsh` 时应修复本机 runtime，而不是改用旧宿主绕过。
- `RemotePowerShellPath` 默认使用远端 PATH 中的 `pwsh.exe`；仅当目标未暴露该命令时，才通过
  `-RemotePowerShellPath` 或 `CODEX_SWITCH_REMOTE_POWERSHELL_PATH` 指定其固定路径。
- `EvidenceRoot` 必须由 `-EvidenceRoot`、`CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT`
  或 `CODEX_HOME` 派生路径提供。脚本不依赖仓库内 `.scratch` 作为部署证据 fallback。
- 设备专属默认值不得复制到通用部署契约；当前维护设备的可复制命令见
  `docs/environments/windows-dual-host.md`。
- 脚本先从
  `src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe`
  和 `release\build\webview2-com-sys-*\out\x64\WebView2Loader.dll`
  读取本机构建产物并计算 SHA-256。
- `-InspectOnly` 只解析远端快捷方式和当前哈希，不上传、不覆盖，也不要求
  `RemoteStageRoot`；正式执行会
  上传到 `-RemoteStageRoot`（默认读取 `CODEX_REMOTE_STAGE_ROOT`）下的时间戳目录，
  由远端脚本备份旧文件，只停止可执行路径等于快捷方式 `TargetPath` 的
  `codex_switch` 进程，覆盖 exe+dll 并复核哈希；其他目录中的同名进程不受影响。
- 远端替换开始后，如任一复制或哈希校验失败，脚本会自动用部署前备份恢复
  `codex_switch.exe` 与 `WebView2Loader.dll`，避免目标目录停留在混合版本。
- 有效 evidence 根依次取 `-EvidenceRoot`、`CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT`、
  `CODEX_HOME\run-evidence\codex-account-switch`；三者都不可用时立即失败。每次运行的
  stdout/stderr、inspect 和 deploy JSON 会写入其下的 `remote-desktop-deploy-*`
  目录，用于事后追踪和回滚定位。

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
- 套餐也必须记录来源：`/wham/usage` 的 `plan_type` 优先于 OAuth `id_token` claim，
  app-server 返回的套餐只可用于没有 ChatGPT API 套餐时，`id_token` 仅用于初始或无 live 数据时的
  fallback。已确认的 ChatGPT API 套餐不得被 app-server 或之后 metadata hydration 中的旧 token
  覆盖；普通刷新保持 OAuth 惰性，只在 token 过期、临近过期或 usage 返回 401 时轮换授权。
- 5 小时与周额度必须按服务端 `window_minutes` / `limit_window_seconds` 的实际窗口时长
  归位，不能假设 primary / secondary 的固定顺序。额度重置卡数量来自 `/wham/usage`，
  发放与到期时间来自 `/wham/rate-limit-reset-credits`；详情请求必须复用刷新后的 OAuth
  身份和账号作用域 header，失败时保留主 quota。不得记录 token、cookie、完整账号或卡片
  ID、失败响应正文；前端固定按北京时间显示时间，服务端没有 additional 配额时不显示
  “接口未报告附加额度”一类空占位。较新的本地 session JSONL 若未包含重置卡，合并和切换写回
  必须保留已存的官方卡片，不得把可用卡清零。
- OpenAI 使用统计必须以有效 `base_url` 作为官方来源门禁，不得用
  `model_provider` 反推官方来源；只统计默认官方路径或 HTTPS
  `api.openai.com`。`account_backup\openai_usage.sqlite` 是可重建分层 ledger，
  保存 evidence、facts 和 strict accepted events。默认总额、趋势和日志只读
  strict proven；历史槽位归因只能进入 `legacy_profile_attributed` 估算层；
  `candidate_unattributed` 必须作为单独对账层可见，方便核对 cc-switch
  session-only 差额，但 candidate / rejected / superseded 不得进入费用总额。source signature 必须覆盖
  session JSONL、归属记录、profile metadata、active marker 和 schema/layer/backfill，
  确保历史归属变化后能重新同步。模型价格使用独立、版本化的官方目录；价格变化必须在
  单个 SQLite 事务中只重算费用与 pricing metadata，不得重新解析 session 或改变 token。
  未知模型 token 可以展示，但费用必须保持未定价，不得显示为 `$0`；无法验证长上下文
  档位时必须标记 `unpriced_long_context`，不得套用短上下文价格。
- Usage 热路径必须保留 session 文件 offset/parser checkpoint 和 cc-switch 内容 checkpoint；
  只有追加且边界可证明安全时才走增量事务，更新、删除、VACUUM、schema 漂移或同时变化时
  回退全量重建。前端相同意图的刷新必须合并，后台页不得重复读取。
- 手动“重启 Codex”必须复用账号切换的 Windows Store 包作用域生命周期锁：先验证并稳定
  退出受管包进程，再且仅再启动一次；不得退化为按镜像名 `taskkill`。执行期间前端按钮必须
  禁用并设置 busy 状态，成功或失败后恢复。
- 使用统计账号/模型 facet 必须按当前时间范围重算：账号候选清除账号自身筛选
  但保留时间、模型和状态筛选；模型候选清除模型自身筛选但保留时间、账号和
  状态筛选。账号分类表应位于时间选择下方、摘要卡片前，选择一个账号后不能
  让其他同范围账号从下拉中消失。账号筛选使用 `account_group_key`，不是单个
  profile；账号实体按相同的非空 `account_id` 或规范化后相同的 email 建立传递
  闭包；不存在上述身份别名路径的两个已确认组件不能仅凭 label 合并。缺少身份的
  历史事件只能归到同一 profile 中事件时间之前最近的已确认账号，绝不使用当前
  快照或未来身份；同毫秒、候选
  歧义或缺少此前证据时必须回落到 unknown。旧 ledger 身份只有在旧版 v1 key 或
  alias 可验证时才允许桥接；strict proven 与 `legacy_profile_attributed` 必须分账。
- 使用统计自定义时间范围必须使用应用内日历和分钟级文本输入；不要恢复原生
  `date` / `time` 输入控件，避免系统日期浮层压住应用内弹层和统计卡片。
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
- Windows profile 切换必须按 `OpenAI.Codex_*__2p2nqsd0c76g0` Store 包身份发现
  整棵应用进程，并在采取退出动作前用已打开的进程句柄再次核对包身份；不得恢复
  按镜像名执行 `taskkill /IM Codex.exe`、`taskkill /IM ChatGPT.exe` 或 `/T` 的旧逻辑。
  先向已验证的 `ChatGPT.exe` 顶层窗口发送 `WM_CLOSE`，超时后才能强制结束仍属于
  同一包的句柄；退出和初始空状态都必须经连续空快照确认。切换事务要在快照前后
  复核进程稳定性，若提交期 Store 进程重现则重新覆盖目标账号态；任何失败必须先
  恢复 root、原槽位、活动标记、索引和归因文件，再至多重开一次 Codex。恢复不完整
  时返回 `SWITCH_RECOVERY_FAILED` 并保持 Codex 不自动重开，避免半切换状态继续写盘。
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

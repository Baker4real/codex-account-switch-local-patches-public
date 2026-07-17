# Windows 实现说明

本文描述当前私有补丁版的 Windows x64 运行契约。仓库只构建和发布
`x86_64-pc-windows-msvc`；`src-tauri/shared/` 只承载真正中性的文件格式、协议和
账号生命周期逻辑，Windows 行为放在 `src-tauri/win/`。

## 数据位置

- Codex 根目录：`%CODEX_HOME%`；未设置时使用 `%USERPROFILE%\.codex`。
- 当前账号：`%CODEX_HOME%\auth.json` 与 `%CODEX_HOME%\profile.json`。
- 账号备份：`%CODEX_HOME%\account_backup\<profile>\`。
- 当前账号提示：`account_backup\.current_profile`；profile 活跃提示：各 profile 下的
  `.active_profile`。两者都只是路由提示，不是账号身份证明。
- 自动快照：`account_backup\_autosave\<timestamp>\auth.json`。
- Windows runtime：`account_backup\windows\`，包括 `install_state.json`、
  `quota_cache.json`、`login_runtime`、`refresh_runtime` 和 `codex_switch_cli.exe`。
- Windows 命令 shim：`%CODEX_HOME%\bin\codex.cmd`。
- Usage ledger：`account_backup\openai_usage.sqlite`；外部只读证据冻结到
  `%CODEX_HOME%\usage_evidence\`，不属于 profile。

`account_backup\macos` 仍被 `paths.rs` 排除，是为了让旧数据目录不会重新显示为账号；
它是兼容旧数据的过滤项，不是当前运行时目录，也不会被创建或写入。

## 运行时边界

- Windows UI 壳位于 `src-tauri/win/front/`，共享前端桥接和 Usage workspace 位于
  `src-tauri/shared/front/`。
- Windows runtime adapter 位于 `src-tauri/win/runtime/`，负责安装、CLI spawn、登录、
  Store 进程生命周期和窗口控制。
- `src-tauri/shared/runtime/` 负责 profile、metadata、路径、ChatGPT API、app-server、
  quota cache、Usage ledger 和错误模型；`src-tauri/shared/commands/` 是 Tauri command
  边界。
- 应用不启动本地 HTTP backend；更新检查使用 Windows PowerShell 请求版本 JSON。

## 首次安装与初始化

`codex_switch.exe install` 和桌面首次启动都会确保 `account_backup` 存在，创建默认
`a`–`d` profile，并从 `examples/account_backup/demo/auth.json.example` 填充占位文件。
若 live `%CODEX_HOME%\auth.json` 存在，会先保存到 `a\auth.json`，再以身份检查结果设置
当前 profile。安装过程记录可运行的真实 Codex CLI 到 `install_state.json`，不会把管理的
shim 当成真实 CLI。

## Profile 切换

切换前必须满足：profile 名称通过 `validate_profile_name`；目录是真实目录而非 symlink
或 Windows reparse point；目标含普通文件 `auth.json`；被管理文件不是目录或其他特殊文件。

事务顺序如下：

1. 按 Store 包身份发现 Codex 进程。对已验证的 `ChatGPT.exe` 先发 `WM_CLOSE`，再在有界
   等待后处理同一包的剩余句柄；退出和初始空状态都要求连续两个空快照。
2. 读取 live `auth.json` 身份，按 `account_id` / 规范化 email 查找实际所属 profile。
   marker 只有在身份匹配、或确实是空占位槽位时才可使用；身份已知但未管理时拒绝写回并
   清理活动提示。
3. 快照 root 的 `auth.json`、`profile.json`、`config.toml`、markers、profiles index、
   Usage attribution 和目标 profile 的受管文件。
4. 把旧账号态写回身份解析出的 profile，保存时间戳 autosave；再把目标 profile 的
   `auth.json` / `profile.json` 覆盖到 live root，并同步其 Base URL 和 markers。
5. 提交后再次观察 Store 进程。若进程在提交期间重现，在有界重试内重新静默并写入目标态；
   超出预算返回 `APP_RESTART_RACE`。
6. 任一步失败时先再次静默，再恢复完整快照和 profiles index。恢复不完整返回
   `SWITCH_RECOVERY_FAILED`，不自动重开 Codex；成功提交或完整回滚后最多调用一次 relaunch。

profile 切换只管理 `auth.json`、`profile.json` 和按 profile 配置的 Base URL。不会整棵复制
`skills`、插件/cache/sqlite、sessions/jsonl、`AGENTS.md`、`rules`、`memories` 或其他
全局运行态；profile 中残留这些条目也会被忽略。login/refresh runtime 可以重建，但必须
保留上游共享的 `plugins`、`cache`、`sqlite`，自动 prune 只清理隔离状态。

## 登录、CLI 与 quota

- `login_browser: "edge"` 是唯一的定向浏览器开关：通过 `codex app-server` 打开用户正常
  Edge 状态，不传 `--user-data-dir` / `--new-window`，写入前必须比较 OAuth 身份，防止
  浏览器当前账号覆盖已知 profile。
- 未设置 Edge metadata 的 profile 使用普通 `codex login`，保留上游默认行为。
- CLI resolver 按手动 override、`install_state`、PATH、nvm-windows、fnm、asdf 和常见
  Windows runtime 顺序探测，每个候选都用 `codex --version` 验证；失效的缓存 shim 不会
  继续使用。版本管理器扫描只读取受限的直接版本目录，不做全盘递归。
- plan 持久化来源：官方 `/wham/usage` 的 `plan_type` 是首选 live 值，app-server 返回的套餐
  只可作为没有官方 API 套餐时的 fallback；OAuth `id_token` claim 只用于初始或没有 live 结果时。
  已确认的 ChatGPT API 套餐不会被 app-server 或后续 metadata hydration 中滞后的 `id_token` 覆盖。
  卡片刷新和后台 bulk refresh 保持 OAuth
  惰性，只在 access token 过期、临近过期或 usage 返回 401 时轮换授权。quota 优先走 ChatGPT
  Web API，必要时使用 app-server `account/rateLimits/read` fallback。UI 显示 provenance 和
  观测时间，低于阈值只告警，不自动切换。`/wham/usage` 的 5 小时与周额度按
  `window_minutes` 或 `limit_window_seconds` 的实际窗口时长归位，不依赖 primary/secondary
  槽位顺序。
- `/wham/usage` 提供重置卡可用数量，`/wham/rate-limit-reset-credits` 补充每张卡的
  `granted_at` / `expires_at`；前端固定按 `Asia/Shanghai` 显示到秒。session JSONL 快照只能在当前
  profile 激活后且严格更新时才能合并；即使它缺少重置卡字段，也不能覆盖已存的官方重置卡。切换前写回遵循同一合并规则。
  详情接口失败不丢弃已成功取得的主 quota，401 明确归因为凭证失效或 Authorization header 无效。响应中的
  卡片 ID 不进入模型、缓存或日志。
- 账号页支持本地搜索、动态套餐筛选和显示数/总数；additional/model-specific 配额保留
  unknown/stale 语义，Spark 噪声行默认隐藏，显示偏好只写浏览器 localStorage，不写入账号
  凭据或 profile 配置。服务端没有报告 additional 配额时不显示空占位状态。

## 更新检查

`check_update` 只请求版本 JSON，比较语义化版本并返回 `release_url`、notes 和
`local_patch_required`。Windows 请求默认尝试 `pwsh`、再尝试 `powershell`，可由
`CODEX_SWITCH_POWERSHELL` 指定路径。

更新弹窗只提供“稍后”和“查看发布说明”。本地补丁版发现上游新版本时明确提示：先把上游
tag 合并到本地源码、保留补丁并重新构建，再替换已安装文件。应用没有 updater 依赖，不
下载公共 exe，不安装或覆盖上游二进制。

## Usage ledger

`openai_usage.sqlite` 是可重建的本机事实表，不是不可变账单。ingestion 冻结本机 session
JSONL 和外部只读证据，并以持久化 checkpoint 对可证明安全的追加变化执行增量事务；检测到
更新、删除、VACUUM、schema 漂移或不安全边界时回退全量重建。官方价格目录独立刷新，
价格变化只在 SQLite 事务中重算费用与 pricing metadata，不重新解析 session。

- `openai_usage_evidence` 保存 accepted、candidate、rejected、superseded 证据。
- `openai_usage_facts` 投影可计量层；`openai_usage_events` 保持 strict accepted-only
  兼容表。
- 默认摘要、趋势和日志只读 strict proven；历史槽位归因单独显示，未归属候选用于对账，
  都不冒充官方账单。
- 只有可证明的官方 OpenAI `base_url` 才进入严格统计；未知模型保留 token，但费用保持未
  定价，不显示为 `$0`。
- token 摘要使用万/亿等紧凑单位；同一刷新意图在前端合并，非活动页面不触发重复读取。
- 价格目录后台读取 OpenAI 官方 Pricing、Models 目录和必需详情页；候选目录全量验证成功后
  才原子发布，失败继续使用最后一次验证成功的目录。无法确认的 GPT-5.6 长上下文记录保持
  `unpriced_long_context`。

## Codex 手动重启

控制区的“重启 Codex”与账号切换共用 Windows Store 包作用域生命周期锁。后端先验证并退出
受管包进程，确认稳定空快照后只启动一次，并等待受管进程重新出现；任一步失败都返回明确错误。
前端执行期间禁用按钮并设置 busy 状态，防止重复点击。

## Windows 构建与远端部署

本机替换安装版的主路径是：

```powershell
npm run build -- --mode windows
npm run tauri:build:windows:portable
```

构建脚本会确认 `rustc -vV` 的 host 为 `x86_64-pc-windows-msvc`。替换时必须把同一构建
的 `codex_switch.exe` 和匹配的 x64 `WebView2Loader.dll` 一起写入安装目录；loader 通常
位于 `target\x86_64-pc-windows-msvc\release\build\webview2-com-sys-*\out\x64\`。

远端桌面入口可能是 `.lnk` 而非 exe。部署必须先以 `-InspectOnly` 解析实时 `TargetPath`
和 `WorkingDirectory`，确认旧哈希后再把 exe + loader 上传到目标目录。正式替换只停止
路径等于目标的 `codex_switch` 进程；复制或哈希校验失败会恢复部署前的成对备份。通用参数
契约和回滚细节见 [`LOCAL_PATCHES.md`](../LOCAL_PATCHES.md)，设备快照见
[`windows-dual-host.md`](environments/windows-dual-host.md)。

本地 `npm run deploy:windows:remote-desktop` 默认通过 PATH 中的 `pwsh`（PowerShell 7）执行；
远端默认通过其 PATH 中的 `pwsh.exe` 执行部署脚本；没有该入口的设备必须显式传入
`-RemotePowerShellPath`，或设置 `CODEX_SWITCH_REMOTE_POWERSHELL_PATH`。不要把某台设备的
PowerShell 绝对路径写回通用发布脚本。

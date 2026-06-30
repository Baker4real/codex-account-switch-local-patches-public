# 本地补丁更新流程

当前安装版是本地补丁构建。更新检查提示有新版上游发布时，不要直接用上游安装包覆盖。

先把上游 tag 合并进这份源码树，保留下面的本地补丁，再重新构建并替换已安装的可执行文件：

- Windows 定向浏览器登录不再按 profile `b` / `c` 槽位选浏览器。仅当目标
  profile 已有账号标签匹配 `FORCED_EDGE_LOGIN_ACCOUNT_LABEL` 时，登录使用
  `codex app-server`，并用用户正常 Edge 状态打开返回的 auth URL。
- 其他 profile 不强制 Chrome 或 Edge，可继续走普通 `codex login` /
  上游默认浏览器行为；不要恢复按槽位名路由浏览器的旧规则。
- 定向 Edge flow 不要再传 `--user-data-dir`、`--new-window`，也不要创建
  独立浏览器 profile。
- 由于这个 flow 复用用户正常浏览器状态，必须保留写入前账号身份校验：如果
  现有 profile 已有已知 ChatGPT 身份，而浏览器完成登录的是另一个账号，
  则必须在替换 `auth.json` 前失败。
- 不要把 forced-Edge 账号路径替换成普通 `codex login`；上游 CLI 会打开
  默认浏览器，并且没有给这个 flow 暴露受支持的浏览器选择参数。
- 更新检查优先使用 `D:\Codex\tools\PowerShell\7.6.3\pwsh.exe`，再回退到
  其他 PowerShell 可执行文件。
- 更新对话框必须提示：替换前需要先合并本地补丁。
- Windows real-Codex discovery 必须用 `codex --version` 和 Codex 版本行校验
  缓存路径及用户指定路径，跳过 stale shim，并探测官方 OpenAI versioned
  runtime 目录，避免 Codex 升级后应用继续指向已删除 runtime。
- 账号页 quota 读数必须保留 provenance 元数据，避免把低可信度 fallback
  或本地 session 观测结果展示得像官方 ChatGPT API 读数。
- 本地 Windows MSVC 构建必须使用 `npm run tauri:build:windows:portable`；
  该 wrapper 优先使用 `D:\Codex\.cargo\bin` 下的 rustup shim，并在
  `rustc -vV` 报告非 `x86_64-pc-windows-msvc` host 时拒绝构建。
- 向另一台 Windows 主机同步 auth/profile 状态时，必须保留目标主机自己的
  `%CODEX_HOME%\account_backup\windows\install_state.json`；只同步 root/profile
  auth 和 metadata。一次性的 `login_runtime`、`refresh_runtime`、stale
  `.switch.lock` 可以删除，因为应用会重新生成。旧本地构建留下的
  `login_browser_profiles` 目录也可以删除；当前构建不再创建它们。
- profile 切换只能同步账号态文件 `auth.json` 和 `profile.json`；不得从
  profile 目录整棵覆盖 live `skills`、`AGENTS.md`、`rules`、`memories`、
  `vendor_imports`、插件/cache/sqlite 或其它全局运行时资产。即使旧 profile
  目录里残留这些污染条目，切换和反向备份也必须忽略它们。

验收目标：已安装的 `D:\Codex\apps\codex_switch\codex_switch.exe` 必须从这份已合并源码树重新构建，不能直接复制纯上游 release。做本地便携 Tauri 替换时，必须把匹配的 x64 `WebView2Loader.dll` 复制到可执行文件同一安装目录；否则桌面端可能在加载 UI 前启动失败。MSVC target 构建中，loader 可能来自目标架构的 `webview2-com-sys` build output，而不是 release 根目录。

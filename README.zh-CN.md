# Codex 账号切换工具

<p align="center">
  <a href="README.md">简体中文</a> |
  <a href="README.zh-CN.md">中文（备份）</a> |
  <a href="CHANGELOG.md">Changelog</a> |
  <a href="https://github.com/Cmochance/Codex_Account_Switch/releases">Releases</a>
</p>

<p align="center">
  <a href="https://github.com/Cmochance/Codex_Account_Switch/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/Cmochance/Codex_Account_Switch?style=social"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/Cmochance/Codex_Account_Switch"></a>
  <a href="https://www.rust-lang.org/"><img alt="Rust" src="https://img.shields.io/badge/Rust-1.80%2B-orange?logo=rust"></a>
  <a href="https://v2.tauri.app/"><img alt="Tauri" src="https://img.shields.io/badge/Tauri-2.x-24C8DB?logo=tauri"></a>
  <a href="https://github.com/Cmochance/Codex_Account_Switch/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/Cmochance/Codex_Account_Switch/total?label=downloads"></a>
</p>

**Codex 账号切换工具**（Codex Switch）是一个面向 **OpenAI Codex CLI** 的本地桌面账号管理器。它把每个账号的 `auth.json` 与 `profile.json` 元数据保存在独立账号备份目录，一键切换"当前 Codex 账号"，并把每个账号的 plan / 5 小时额度 / 周额度直接渲染在桌面端。切换只同步这些账号态文件；`skills`、插件缓存、sessions、AGENTS.md 等全局或运行时资产不会被 profile 覆盖。

跟纯账号切换脚本不同，本项目内置 **plan / quota 直读**：登录后通过 ChatGPT OAuth 的 `id_token` + ChatGPT Web API `account/rateLimits` / Codex app-server JSON-RPC fallback 拉取真实计划等级和 rate-limit 窗口剩余比例，并在 UI 标明额度来源与观测时间；只有没有实时来源时才退回本地 sessions/jsonl 观测。

当前版本 **v1.5.12**（详见 [CHANGELOG](CHANGELOG.md) 与 [Releases](https://github.com/Cmochance/Codex_Account_Switch/releases)）。

## 能做什么

- 同一台机器多账号管理：每个账号一个备份目录（含独立的 `auth.json` / `profile.json` 元数据），切换时把目标账号态写入 `~/.codex/`；旧账号的账号态会先按 `auth.json` 身份路由回真实所属 profile，无法识别时拒绝写回旧槽位。
- **账号页**：每页 8 个账号条目，横向单行布局展示「账号名 + 套餐」「5 小时额度 + 刷新时间」「周额度 + 刷新时间」「删除 / 重命名 / 刷新 / 登录 / Base / 切换」按钮，并用来源 badge 区分官方 API、app-server fallback、本地观测或未知数据。
- **授权缓存状态**：账号卡片和当前账号面板会显示 refresh token / 授权缓存健康状态、可观察到的到期时间或上次刷新时间；当官方登录缓存没有暴露 refresh token 到期时间时，只提示授权缓存建议刷新，不把它误写成固定到期时间。
- **登录与浏览器控制**：普通账号仍使用 `codex login`；Windows 本地补丁版仅对 `FORCED_EDGE_LOGIN_ACCOUNT_LABEL` 指定账号使用 `codex app-server` 打开 Edge，并复用用户当前常用浏览器状态，不再创建独立浏览器 profile。
- **plan / quota 智能缓存**：bulk plan refresh 在 6 小时窗口内跳过已确认账号，per-card 刷新按钮也共享同一缓存；切换 / 登录 / 刷新后直接复用 backend 写回的 snapshot，不重复发 IPC。5 小时或周额度低于 20% 时只提示风险，不自动切换账号。
- **Custom Base URL**：每个账号可独立配置 `OPENAI_BASE_URL`；配置后按钮变红警示（自定义 Base 与 ChatGPT OAuth 账号互斥）。
- **Codex CLI 路径自检**：自动定位 `codex` 可执行（PATH / `~/.codex/bin` / Homebrew / nvm），找不到或路径错误时设置页可手动指定，结果写入 `install_state.json` 优先生效。设置页还提供「自动检测」按钮：忽略可能出错的缓存重新扫描所有常见位置，并用 `codex --version` 验证候选确实是可运行的 Codex CLI——唯一命中直接应用，多个命中时让你选。
- **跨平台原生 Tauri**：macOS arm64 / x64 与 Windows x64 提供原生窗口、原生标题栏 / 关闭按钮，配套 5 套浅色 / 深色主题与中英文界面。
- **本地预览模式**：没有 Tauri 运行时（直接 `vite` 跑前端）时自动使用 mock snapshot，方便单纯调样式。

## 下载

最新版：<https://github.com/Cmochance/Codex_Account_Switch/releases/latest>

资产命名：

```text
codex_switch_<版本>_aarch64.dmg            macOS Apple Silicon DMG（拖拽到 Applications）
codex_switch_<版本>_aarch64.pkg            macOS Apple Silicon PKG 安装包
codex_switch_<版本>_x64.dmg                macOS Intel DMG（拖拽到 Applications）
codex_switch_<版本>_x64.pkg                macOS Intel PKG 安装包
codex_switch_<版本>_x64-setup.exe          Windows x64 NSIS 安装包
codex_switch_<版本>_amd64.deb              Linux x86_64 Debian/Ubuntu 包（sudo dpkg -i 安装）
codex_switch_<版本>_amd64.AppImage         Linux x86_64 通用便携包（chmod +x 后直接运行）
```

> macOS / Windows 都暂未做代码签名，首次启动可能提示「未知开发者 / 未知发布者」；macOS 可在「系统设置 → 隐私与安全」放行，Windows 可在 SmartScreen 弹窗点「更多信息 → 仍要运行」。
>
> Linux 为实验性发布（Ubuntu 22.04 / glibc 2.35 基线构建）：UI、账号切换、plan/quota 查看可用；与 Codex CLI 的部分交互（路径自检、`codex login` spawn 等）当前走 Windows 路径分支，Linux-native 体验尚未单独适配，欢迎反馈 issue。

## 快速开始

1. 从 Releases 下载并安装 Codex Switch，启动桌面窗口。
2. 进入「设置」页，确认 **Codex CLI 路径** 已自动填好（默认探测 PATH / `~/.codex/bin`）；缺失时手动指定，状态变绿即可。
3. 进入「账号」页，点「添加账号」→ 输入账号别名（落地为 `~/.codex/account_backup/<别名>/`）。
4. 在新账号条目点「登录」→ 浏览器完成 ChatGPT OAuth → 回到应用，套餐 + 额度自动加载。Windows 本地补丁版中，指定账号会打开 Edge；其他账号走普通登录浏览器行为。
5. 多账号场景：在目标条目点「切换」即可一键替换当前 `~/.codex/` 的账号态文件，原账号的 `auth.json` / `profile.json` 会按账号身份写回匹配的账号备份目录；无法确认归属时不会覆盖旧槽位。
6. 需要清空 quota cache、重新拉计划或更新授权缓存状态时，点对应账号的「刷新」（≥ 6h 走完整 OAuth rotation，< 6h 复用缓存）。

## 仓库结构

- `src-tauri/`：Rust + Tauri 应用根目录。
- `src-tauri/mac/front/`：macOS HTML 壳、样式、原生窗口控制。
- `src-tauri/win/front/`：Windows HTML 壳、样式、自绘标题栏与窗口按钮。
- `src-tauri/shared/front/`：跨平台前端 — state / render / actions / theme / i18n / Tauri bridge / mock preview。
- `src-tauri/mac/runtime/` / `src-tauri/win/runtime/`：各自平台的 runtime adapter（profile_actions / switch / login spawn / Windows 定向浏览器登录）。
- `src-tauri/shared/runtime/`：共享 runtime — `chatgpt_api`、`codex_app_server`、`models`、`switch_core`、`profiles_index`、`paths`、`metadata`、`process_lock`、`fs_ops`。
- `src-tauri/shared/commands/`：前端可见的 Tauri command 层（dashboard / switch / profile lifecycle）。
- `macOS-backup/`：兼容老的 shell 流程（`install.sh` / `uninstall.sh` / `codex-switch.sh`），用于 desktop 不可用的回退场景。
- `scripts/`：version-sync、macOS 产物布局、`.pkg` 生成、CHANGELOG 校验。
- `examples/account_backup/demo/`：`auth.json` / `profile.json` 占位模板，演示账号备份目录结构。
- `docs/`：发布、实现笔记、安全说明、上游协议参考。

构建输出（`dist/`、`src-tauri/target/`、`node_modules/`、`.app` / `.dmg` / `.pkg` / `.exe`）全部 .gitignore，不入仓。

## 本地开发

```bash
npm install
npm run tauri:dev                  # 起 Tauri 桌面 dev 窗口 + Vite HMR
npm run build                      # 仅前端 production build（vite + tsc）
npm run test                       # cargo test --manifest-path src-tauri/Cargo.toml
npm run check:rust:windows         # cross check Windows target（macOS 本地交叉验证）
npm run test:rust:windows          # 在 Windows runner 上跑（CI 主用）
npm run test:rust:windows-gnu      # 在 Windows GNU/MinGW host 上验证本地链接与单测启动路径
```

## 桌面端打包

Tauri 2 没暴露 bundle 输出目录配置，macOS 打包脚本在 `tauri build` 前把 `src-tauri/target/release/bundle/macos` 与 `bundle/dmg` symlink 到 `dist/`，把最终 `.app` / `.dmg` / `.pkg` 直接落到仓库根目录的 `dist/`。

本地仅打包 `.app`（用于手测，不签名）：

```bash
npm run tauri:build:macos-app
```

发布构建（`.dmg` + `.pkg`）：

```bash
npm run tauri:build:macos-release
```

发布构建的 `.app` 会作为打包输入参与签名 / notarization 链路（CI 跑），随后从 `dist/` 移除，只保留 `.dmg` / `.pkg`。旧版本产物会移动到 `dist/history/v<旧版本>/` 下归档；重复构建当前版本只替换根目录的当前文件，不会再额外备份。

期望产物结构：

```text
dist/
  codex_switch_<版本>_aarch64.dmg
  codex_switch_<版本>_aarch64.pkg
  codex_switch_<版本>_x64.dmg            （macOS Intel runner）
  codex_switch_<版本>_x64.pkg            （macOS Intel runner）
  codex_switch_<版本>_x64-setup.exe      （Windows runner）
  history/
    v<旧版本>/
      ...
  web/
    ...
```

Windows 正式发布构建走 CI 的 `tauri build --target x86_64-pc-windows-msvc`；本地 GNU 补丁验证可用 `npm run tauri:build:portable` 生成 `src-tauri/target/release/codex_switch.exe`。替换本机安装版时，必须同时把同目标架构的 `WebView2Loader.dll` 放到 `D:\Codex\apps\codex_switch\`，否则桌面端会因为找不到 WebView loader 而启动失败。

Windows 本地 MSVC 便携构建可用 `npm run tauri:build:windows:portable`，产物为 `src-tauri/target/x86_64-pc-windows-msvc/release/codex_switch.exe`。该命令会优先使用 `D:\Codex\.cargo\bin` 里的 rustup shim，并在构建前确认 `rustc -vV` 的 host 是 `x86_64-pc-windows-msvc`，避免独立安装的 GNU Rust 抢先进入 PATH。该命令仍需要 Rust 的 `x86_64-pc-windows-msvc` target，以及 Visual Studio Build Tools 的 C++ 工具链和 Windows SDK；如果 Git for Windows 的 `link.exe` 抢先进入 PATH，请从 VS developer shell 运行，或确保 MSVC `link.exe` 优先。

## 版本与发布

- 版本号源头是 `package.json`，`npm run version:sync` / `npm run version:set -- <semver>` 把同一版本写到 `package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`，并在 `src-tauri/mac/front/index.html` / `win/front/index.html` 通过 Vite-injected `__CODEX_APP_VERSION__` 渲染到设置页。
- GitHub Release tag 用完整 semver（如 `v1.5.12`）。push tag → `.github/workflows/build.yml` 自动跑 macOS arm64 + macOS x64 + Windows x64 构建并把产物上传到一个 **draft release**，不会自动转 Latest。
- 不要把补丁版本的 asset 上传到旧的两段式 tag（如 `1.5`）。
- macOS 安装包仅作 Release asset 发布，不提交到 Git。

常用命令：

```bash
npm run version:sync              # 把 package.json 当前 version 同步到 Cargo / lock
npm run version:set -- 1.5.12     # 一次性 bump 到指定版本
npm run version:check             # CI 用：拒绝把 semver 字面量写回 *.html
```

## 常见问题

### 没有 Codex CLI 怎么办

应用启动后会探测 `codex` 路径；探测失败时设置页会标红「Codex CLI 路径未找到」，可手动指定（写入 `install_state.json` 的 `user_codex_path` 优先级最高）。也可以点设置页「Codex CLI 路径」行的「自动检测」按钮强制重新扫描，并用 `codex --version` 验证候选确实是 Codex CLI——比启动时的自检更主动，适合自动定位出错或不清楚 `codex` 装在哪的情况。Windows 会同时检查 `CODEX_EXE`、`CODEX_INSTALL_DIR` 和官方 OpenAI versioned runtime 目录；缓存路径或手动路径如果不能运行 `codex --version`，或输出的版本行不像 Codex CLI，会被跳过并重新发现。未装 Codex CLI 也能用 plan / quota 查看（走 ChatGPT OAuth token 直接拉 rate-limits），但「登录」按钮和切换后启动 Codex 等动作需要 CLI 存在。

### 切换账号会改动 skills / sessions / 历史吗

不会。当前原生 runtime 的 profile 切换只管理 `auth.json` 和 `profile.json` 两个账号态文件，并会按 profile 元数据同步当前账号需要的 Base URL 配置。写回当前 root 状态时，`.current_profile` / `.active_profile` 只作为候选提示，实际归属要用 `auth.json` 中可识别的账号身份匹配；如果 root 已经漂移到未管理账号，应用不会把它写进旧槽位。`skills`、插件缓存、`AGENTS.md`、`rules`、`memories`、`vendor_imports`、sqlite 状态和 sessions/jsonl 历史都不会从 profile 目录覆盖到 live `~/.codex/`。如果旧 profile 目录里残留这些全局资产，它们会被切换流程忽略。

### 「刷新」按钮和「切换」之后会不会消耗 quota

不会真发 ChatGPT 对话请求。Plan 数据来自 `id_token` claims；quota 走 ChatGPT Web API `account/rateLimits/read`（HTTP GET），或 Codex 0.130+ 的 `app-server` JSON-RPC fallback。没有 LLM 调用，不消耗用户额度。

### per-card「登录」按钮为什么有时变成「取消」

普通 `codex login` 流程启动后如果 OAuth 回调没回来（用户关浏览器 / 网络异常），点同一按钮会向登录子进程发 SIGTERM / taskkill 中止它，释放 `.switch.lock` 全局锁。Windows 本地补丁版中，账号标签匹配 `FORCED_EDGE_LOGIN_ACCOUNT_LABEL` 的 profile 会改用 `codex app-server` 取得 `account/login/start` 返回的 `authUrl`，再由应用用 Edge 的正常用户状态打开登录页，并等待 `account/login/completed` 通知。

因为定向 Edge 登录复用的是 Edge 当前正常登录状态，点击登录前请确认 Edge 里的 ChatGPT 账号就是目标 profile 要保存的账号。若目标 profile 已有可识别账号，应用会在写入前校验新旧账号标识，不一致时拒绝覆盖旧 `auth.json`。

### Custom Base URL 配置后能继续用 ChatGPT OAuth 吗

不能。Custom Base URL（指向第三方 OpenAI 兼容反代）和 ChatGPT 官方 OAuth 是互斥的 — OAuth `id_token` 校验只对官方 endpoint 生效。配置 Base 之后该卡片的「Base」按钮会变红警示。

### macOS 提示「无法打开，因为无法验证开发者」

App 暂未做 Apple Developer 代码签名 / notarization。第一次启动按住 Control 点应用图标 → 选「打开」一次即可放行；后续直接双击就行。或在「系统设置 → 隐私与安全」放行。

### Windows SmartScreen 提示「Windows 已保护你的电脑」

同上，未做 Authenticode 签名。SmartScreen 弹窗点「更多信息 → 仍要运行」即可。

### 日志和账号数据存在哪里

- 账号备份：`~/.codex/account_backup/<别名>/`（macOS / Linux）/ `%USERPROFILE%\.codex\account_backup\<别名>\`（Windows）
- 应用运行时状态：Windows 为 `%CODEX_HOME%\account_backup\windows\`（`install_state.json`、`quota_cache.json`、可再生成的 login runtime / refresh runtime）；macOS 为 `~/.codex/account_backup/macos/`
- Codex 当前账号状态：`~/.codex/auth.json` + `~/.codex/profile.json`；`config.toml` 中的 Base URL 可能由 profile 元数据同步

## 技术栈

- **后端 / runtime**：Rust 1.80+ · Tauri 2.x · tokio + reqwest（rustls-tls）· chrono · serde
- **协议适配**：`shared/runtime/chatgpt_api.rs`（ChatGPT Web API `account/{read,rateLimits/read}`）+ `shared/runtime/codex_app_server.rs`（Codex app-server JSON-RPC，用于 quota fallback 和 Windows 定向 Edge 登录）
- **前端**：HTML + CSS + 原生 TypeScript（`shared/front/{render,state,actions,theme,i18n,tauri}.ts`），无前端框架；Vite 7 打包
- **跨平台 runtime**：mac / win runtime 各自实现 spawn / process control / OS-native title bar；shared runtime 处理协议、profile lifecycle、JSON-RPC、缓存、文件锁
- **构建 / 发布**：`tauri build` 单命令出 dmg / pkg / exe；`.github/workflows/build.yml` 出 draft release，全部 asset 由 GitHub Actions 上传

## 平台支持

- **macOS**：原生 Tauri 桌面端（Apple Silicon + Intel），同时保留 `macOS-backup/` 下的 legacy shell 流程兼容。
- **Windows**：原生 Tauri 桌面端（x64），通过 Release 中的 `.exe` 分发。
- **Linux**：**实验性发布**（Ubuntu 22.04 / glibc 2.35 基线，x86_64），通过 Release 中的 `.deb` / `.AppImage` 分发。前端 UI、账号切换、plan / quota 查看与 macOS / Windows 一致；与 Codex CLI 的交互（路径自检、`codex login` spawn）当前复用 Windows runtime 分支，尚未做 Linux-native 适配，欢迎踩到具体问题后开 issue 反馈。

平台专属逻辑放在 `src-tauri/mac/**` 或 `src-tauri/win/**`，跨平台逻辑都在 `src-tauri/shared/**`。

## macOS 兼容脚本

老的 shell 流程仍然保留，作为桌面 GUI 不可用 / SSH 终端场景的兜底：

```bash
bash macOS-backup/install.sh
source ~/.zshrc
```

三种模式：

- `auto`：优先尝试原生桌面 runtime，找不到回退到 legacy shell。
- `desktop`：强制走原生桌面。
- `legacy`：强制走 `codex-switch.sh`。

卸载：

```bash
bash macOS-backup/uninstall.sh                  # 只移除命令接入层
bash macOS-backup/uninstall.sh --remove-script  # 连脚本一起删
source ~/.zshrc
```

卸载脚本不会删 `~/.codex/account_backup/`，账号目录需要时手动清理。

## 免责声明

本项目专注 **OpenAI Codex CLI 多账号管理**，**不是** OpenAI 官方项目，也不复用其商标 / Logo / 发布身份。

所有账号凭据 / OAuth token 仅保存在本机 `~/.codex/account_backup/`（Unix 0600 + atomic write）；plan / quota 通过 HTTPS 直连 `chatgpt.com` / `api.openai.com`，不经任何第三方中转。

ChatGPT OAuth `id_token` / `access_token` 仅用于读取计划与 rate-limit 元数据，不会向模型发起任何对话请求，因此不会消耗用户额度。

## 许可证

MIT License。完整文本见 [LICENSE](LICENSE).

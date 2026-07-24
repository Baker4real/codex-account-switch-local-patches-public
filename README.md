# Codex 账号切换工具（Windows）

<p align="center">
  <a href="README.zh-CN.md">简体中文备份</a> |
  <a href="CHANGELOG.md">Changelog</a> |
  <a href="https://github.com/Baker4real/codex-account-switch-local-patches/releases">私有补丁 Releases</a>
</p>

Codex Switch 是一个面向 OpenAI Codex CLI 的本地 Windows x64 桌面账号管理器。它为每个账号保存独立的 `auth.json` 与 `profile.json`，支持安全切换、plan/quota 查看、Codex CLI 路径检测和 Codex 控制。

当前产品契约只有 Windows x64。仓库中的运行时、前端入口、构建、CI 和发布流程都以 Windows MSVC 为准；不再提供其他操作系统的安装包或运行路径。

## 功能

- 多账号管理：账号备份位于 `%CODEX_HOME%\account_backup\<profile>\`，切换时只同步 `auth.json` 和 `profile.json`。
- 身份路由写回：`.current_profile` 与 `.active_profile` 只是提示，实际按 live `auth.json` 的 `account_id` / email 识别归属；无法证明归属时拒绝覆盖旧槽位。
- Windows Store 生命周期：切换前按 `OpenAI.Codex_*__2p2nqsd0c76g0` 包身份退出并确认空快照，提交失败会事务回滚，避免半切换状态。
- plan/quota：优先读取官方 ChatGPT API，必要时使用 Codex app-server fallback；live 套餐不会被滞后的 OAuth `id_token` 覆盖，正常刷新只在授权确有需要时轮换 token。5 小时与周额度按服务端窗口时长归位，界面显示来源和观测时间，不会因为低额度自动切换账号。支持 additional/model-specific 配额与额度重置卡；重置卡逐张显示北京时间的发放、到期时间，较新的本地 session 快照缺卡时仍保留已确认卡片，接口未返回附加额度时不渲染误导性占位文案。未知或过期数据不会伪装成满额，Spark 噪声行默认隐藏。
- 账号筛选：账号页支持本地搜索、动态套餐筛选、显示数/总数提示和一键清除；筛选不修改账号快照或凭据。
- 设备码登录：每个 profile 卡片都可显示验证地址和一次性代码，在另一台能够登录 OpenAI 的设备上完成授权；它与浏览器登录共用隔离运行目录、账号身份校验、取消和原子写入流程。
- 定向浏览器登录：只有 profile metadata 明确设置 `login_browser: "edge"` 时才使用 Edge + app-server，并复用用户正常 Edge 状态；不会创建独立浏览器 profile。
- 授权方式标记：浏览器或设备码授权成功后，账号卡片显示最近一次实际授权方式；旧 profile 没有可靠历史时不显示，也不会根据 `login_browser` 猜测。
- Codex CLI 自动检测：扫描 PATH、官方 Windows runtime、nvm-windows、fnm、asdf 和常见安装目录，并以 `codex --version` 验证可运行性；也可在设置页手动指定路径。
- Codex 控制：仪表盘前列可直接重启 Codex，控制区仍可打开 Codex；重启沿用 Windows Store 包身份校验和稳定退出检查，按钮执行期间会锁定，避免重复触发。
- Custom Base URL：每个账号可单独配置第三方 OpenAI 兼容地址；它与 ChatGPT OAuth 账号互斥。

## 安装与使用

1. 私有补丁仓库已有 Release 时，从 [私有补丁 Releases](https://github.com/Baker4real/codex-account-switch-local-patches/releases/latest) 下载 Windows x64 NSIS 安装包 `codex_switch_<版本>_x64-setup.exe` 并安装；没有私有 Release 时，按下方本地构建命令生成，不要从公共上游下载。
2. 打开应用，在“设置”确认 Codex CLI 路径；缺失时点击“自动检测”或手动选择可运行的 `codex.exe` / `codex.cmd`。
3. 在“账号”页添加 profile；本机浏览器可登录时点击“浏览器”，否则点击“设备码”，再在能够登录 OpenAI 的设备上打开验证页面并输入一次性代码。
4. 在目标账号卡片点击“切换”。应用会先保存当前账号，再按身份路由写回目标账号；全局 `skills`、插件缓存、sessions、`AGENTS.md`、`rules`、`memories` 等不会被 profile 覆盖。
Windows SmartScreen 可能提示未签名；确认来源后可选择“更多信息 → 仍要运行”。账号凭据和 OAuth token 只写入本机，请勿把 `auth.json` 上传到公共位置。

历史版本说明：旧版仓库曾发布过 macOS/Linux 资产，那些 Release 只用于保留历史记录，当前不再支持；不要安装旧平台或公共上游二进制。

## 仓库结构

- `src-tauri/win/`：Windows Tauri 前端壳与原生 runtime。
- `src-tauri/shared/`：账号生命周期、身份路由、ChatGPT API、app-server 和前端桥接；只保留必要的中性契约。
- `scripts/`：版本同步、Windows MSVC 构建、发布卫生检查和远端桌面部署。
- `examples/account_backup/demo/`：账号目录模板。
- `docs/`：实现说明、Windows 双主机部署和研究/设计记录。

## 本地开发与验证

在已安装并处于 PATH 中的最新版 PowerShell 7（`pwsh`）中执行：

```powershell
npm install
npm run tauri:dev
npm run build
npm run test:front
npm run check:rust:windows
npm run test:rust:windows
npm run check:windows-release-hygiene
```

前端生产构建输出到 ignored 的 `dist\web\`。Windows MSVC 便携构建使用已验证脚本：

```powershell
npm run build
npm run tauri:build:windows:portable
```

产物为 `src-tauri\target\x86_64-pc-windows-msvc\release\codex_switch.exe`。替换安装目录时必须同时复制同一构建产生的 x64 `WebView2Loader.dll`；loader 通常位于 `release\build\webview2-com-sys-*\out\x64\`，不保证在 release 根目录。

GitHub Actions 的 Windows CI 和构建只在 `windows-latest` 上执行；tag release 仅附加 Windows NSIS `.exe`，中性的 release 编排 runner 不参与平台构建。版本号源头是 `package.json`，常用命令：

```powershell
npm run version:sync
npm run version:set -- 1.5.12
npm run version:check
```

## 网络代理

账号刷新和 `codex login` / `codex app-server` 子进程按以下顺序选择代理：
`CODEX_SWITCH_PROXY_URL` → `HTTPS_PROXY` / `https_proxy` → `ALL_PROXY` / `all_proxy` →
`HTTP_PROXY` / `http_proxy`；未设置或值为空时使用 `http://127.0.0.1:10808`。如需固定其他端口，
请设置用户级环境变量并完全退出后重启 Codex Switch：

```powershell
[Environment]::SetEnvironmentVariable(
  "CODEX_SWITCH_PROXY_URL",
  "http://127.0.0.1:19090",
  "User"
)
```

该覆盖只作用于 Rust HTTP 请求和 Codex 子进程；内嵌 WebView 当前仍使用构建时配置的
`127.0.0.1:10808`。若 WebView 自身也必须使用其他端口，需要同步修改 Tauri 配置并重新构建。

## 更新与本地补丁

更新检查只读取版本 JSON、显示 release notes 并打开发布页。它不会下载、安装或覆盖公共上游二进制。当前安装版含本地 Windows patch；发现上游新版本时，先把上游 tag 合并到源码、重新保留本地补丁并通过 Windows 验证，再构建和替换安装目录。完整流程见 [`LOCAL_PATCHES.md`](LOCAL_PATCHES.md)。

远端 Windows 桌面部署必须使用严格 OpenSSH/SCP over Tailscale，并显式提供方向专用私钥与 `known_hosts`；先用 `-InspectOnly` 实时解析快捷方式的 `TargetPath`，再上传并替换目标目录中的 exe + loader。不可把文件复制到 `.lnk` 路径、只替换 exe、自动接纳新主机密钥或回退到密码认证。设备专属路径与传输证据见 [`docs/environments/windows-dual-host.md`](docs/environments/windows-dual-host.md)。

## 数据位置与安全边界

- Codex 当前账号：`%CODEX_HOME%\auth.json`、`%CODEX_HOME%\profile.json`。
- Profile 备份：`%CODEX_HOME%\account_backup\<profile>\`。
- Windows runtime：`%CODEX_HOME%\account_backup\windows\`（安装状态、quota cache、login/refresh runtime）。
- 已停用统计功能的历史文件可能仍位于 `account_backup\openai_usage.sqlite`、归因 JSONL 和
  `%CODEX_HOME%\usage_evidence\`；当前应用不会读取、更新或自动删除它们，也不会把这些目录列为账号 profile。

切换只管理账号态文件，不整棵复制 profile。更新、plan/quota 和 OAuth 流程不向第三方中转。该项目不是 OpenAI 官方项目。

## 许可证

MIT License，完整文本见 [`LICENSE`](LICENSE)。本地补丁说明和上游合并要求见 [`LOCAL_PATCHES.md`](LOCAL_PATCHES.md)。

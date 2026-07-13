# Windows-only 裁剪与 Cockpit Tools 功能对齐设计

> 状态（2026-07-14）：阶段一 Windows-only 裁剪与阶段二第一组能力均已完成。实现计划见 [`docs/plans/2026-07-13-cockpit-phase1.md`](../plans/2026-07-13-cockpit-phase1.md)，验收证据见 [`docs/evidence/2026-07-13-cockpit-phase1-verification.md`](../evidence/2026-07-13-cockpit-phase1-verification.md)；后续候选能力仍须单独设计和验收。

## 目标

把项目从跨平台 Codex 账号切换器收敛为仅面向 Windows 的个人本地工具，并在平台裁剪完成后，分阶段吸收 Cockpit Tools 中与现有产品定位一致的 Codex 功能。

完成后的项目只承诺 Windows x64：源码、构建、测试、CI、发布产物和用户文档不再提供 macOS、Linux、iOS 或 Android 支持。历史版本记录仍保留原有事实。

## 约束

- 保留现有 Windows 账号身份路由、Store 包作用域生命周期、事务回滚和路径精确进程控制。
- 保留 OpenAI 使用统计的证据分层、账号归属和未知模型未定价语义。
- 新功能不得在普通应用启动时自动改写 `auth.json` 或 `config.toml`。
- 当前用途是个人、本机、非商业使用，目标 GitHub 仓库保持 private。允许参考和复用 Cockpit Tools 源码，但复制内容必须记录固定来源提交、作者和 CC BY-NC-SA 4.0 许可，不得把复制部分误标为纯 MIT 原创。
- 不创建新的共享平台抽象；Windows 专属行为留在 `src-tauri/win/**`。
- 不在同一实施切片中同时进行平台裁剪和 Cockpit 功能开发。

## 方案选择

采用“聚焦裁剪”方案：删除其他平台的产品实现和构建路径，但保留 Windows 仍使用的 shared 业务模块、测试 seam 与旧磁盘格式护栏。

不采用以下方案：

- 不做激进扁平化：当前 `PlatformHooks` 影响大量生产调用和测试替身；在平台退役时一并消除会扩大回归面。
- 不只停用 CI：用户要求现有代码中也移除 macOS 内容，保留休眠平台源码不符合目标。

## 阶段一：Windows-only 平台裁剪

### 删除内容

- `src-tauri/mac/**`
- `macOS-backup/**`
- `scripts/macos-artifacts.mjs`
- `scripts/create-macos-pkg.mjs`
- `src-tauri/tauri.macos.conf.json`
- `src-tauri/tauri.linux.conf.json`
- `src-tauri/icons/icon.icns`
- `src-tauri/icons/ios/**`
- `src-tauri/icons/android/**`
- 纯 macOS 实施计划：
  - `docs/plans/2026-04-16-macos-desktop-app.md`
  - `docs/plans/2026-04-20-macos-sync-latest-windows-updates.md`

通用 Tauri、Windows ICO、Windows Store 图标、shared front/runtime/commands 和 Windows runtime 不属于删除范围。

### Rust 结构

- `src-tauri/src/lib.rs` 只注册 Windows runtime 和 windowing。
- 用明确的 `cfg(windows)` 取代 `not(target_os = "macos")`。非 Windows target 通过清晰的编译错误拒绝构建，不能再静默落入 Windows runtime。
- `shared/commands/{actions,dashboard,switch}.rs`、`shared/runtime/cli.rs` 和 `shared/platform/mod.rs` 直接连接 Windows adapter。
- `src-tauri/examples/live_quota_check.rs` 使用 Windows profiles index。
- `tauri-plugin-single-instance` 的 target dependency 只保留 `cfg(windows)`。
- 删除仅服务于 Unix/macOS/Linux 的生产 fallback 和测试 helper，但保留被这些 helper 覆盖的中立业务核心。
- `PlatformHooks` 暂时保留。它仍是切换、刷新、登录和失败路径测试的隔离接口。

### 旧磁盘格式兼容

`account_backup/macos` 即使不再是有效运行目录，也必须继续作为保留目录名从 profile 扫描中排除。常量可以改名为 `LEGACY_MACOS_RUNTIME_DIRNAME`，并用测试证明该目录不会生成账号卡。

这是旧磁盘格式防误识别规则，不代表继续支持 macOS。

### 前端与构建

- Vite root 固定为 `src-tauri/win/front`。
- 删除 `CODEX_UI_TARGET`、Darwin 分支和 `@mac-front` alias。
- Windows UI 条件固化为 Windows 行为，删除 macOS 专属 selector 与文案。
- `scripts/version-sync.mjs` 只检查 Windows HTML 版本。
- `package.json` 删除 macOS/Linux 构建和打包脚本；现有通用 `tauri:build` 与 `tauri:build:portable` 明确转发到对应 Windows 构建入口，避免产生平台不明的产物，同时保留已有调用兼容性。
- `tauri.conf.json` 图标列表只引用仍存在的 Windows 资源。
- lockfile 只通过包管理器和 Cargo 正常更新；不按平台关键字手工删除可选依赖。

### CI 与发布

- CI 改用 Windows runner。
- 保留前端类型检查、前端测试、版本检查和 Windows release hygiene 的既有顺序约束。
- Rust 检查和测试使用 Windows MSVC 路径。
- tag 和手动发布只构建 Windows x64 安装包。
- 删除 Linux apt 依赖、无用途的 Go setup、Apple signing、DMG 验证及 macOS/Linux artifact globs。
- Release 只附加 Windows artifact。

### 文档

- `README.md`、`README.zh-CN.md`、`docs/IMPLEMENTATION.md` 改为 Windows-only 当前事实。
- `AGENTS.md` 将平台隔离规则替换为 Windows-only 规则。
- `LOCAL_PATCHES.md` 记录这是主动的 Windows-only private fork；未来同步上游时不得复活 macOS/Linux 目录、构建矩阵或发布承诺。
- `CHANGELOG.md` 保留历史 macOS/Linux 条目，并在当前版本记录平台退役；不改写历史。
- 混合历史 spec/plan 中的平台描述保留为历史上下文，必要时增加“当前已退役”提示，不批量篡改过去的设计记录。

## 阶段二：Cockpit Tools 第一组功能

平台裁剪单独验收通过后，第一组已按下列顺序实现；第二、三组仍需独立计划和验收：

1. Windows GUI 下补充 `nvm`、`fnm`、`asdf` 的 Codex CLI/Node 候选发现；最终候选仍必须通过真实 `codex --version` 校验。
2. 账号搜索和动态套餐筛选；筛选只影响视图，不改变底层账号集合。
3. 显示可见账号数/总账号数、活动筛选条件和一键清除。
4. additional/model-specific 配额数据与显示偏好；缺失、异常或陈旧数据必须显示未知或过期，Spark 类噪声配额默认隐藏。

允许复用 Cockpit Tools 的局部实现时，优先移植纯算法和 Windows 路径枚举；不得用其通用多平台进程管理替换本项目的 Windows Store 生命周期。

## 后续候选功能

账号标签、分组、自定义排序和批量导入导出属于第二组独立设计。导出默认排除 access token、refresh token、密码和 2FA 秘钥；导入必须经过现有身份去重和占位规则。

下列能力不在本设计实施范围：

- Codex API Service、账号池和自动路由
- 自动低额度切号
- 多实例管理
- 会话删除、废纸篓和跨实例迁移
- 自动唤醒任务
- Grok、Cursor、Windsurf 等非 Codex 平台

## 更新语义

公开上游 release 只能提示“存在可研究/合并的新版本”，不能引导用户直接下载安装并覆盖本地补丁构建。private 仓库 release API 需要认证，不作为无凭据客户端更新源。

保留应用内上游版本检查，但界面明确展示“发现上游版本，需要合并并重新构建”，只允许打开上游发布说明，不提供下载或安装动作，不能把上游安装包当成本地补丁版升级包。

## 错误处理与失败边界

- 删除平台文件前先移除所有生产、构建和版本检查引用；不得通过 stub 或空文件维持错误配置。
- 任何非 Windows target 构建都应尽早以明确错误失败。
- CLI 新候选扫描有数量和超时上限；单个损坏 shim 不得阻塞整个应用。
- 筛选与显示偏好写盘失败必须报告错误，不得只更新内存造成假成功。
- 普通启动、筛选和配额显示不得触发账号切换或凭据写盘。

## 验收标准

阶段一必须满足：

- 活跃代码、配置、脚本和现行文档中不存在 macOS/Linux 产品路径或发布入口；允许保留 CHANGELOG 历史、lockfile 可选依赖和旧磁盘目录保留字。
- `npm run version:check` 通过。
- `npx tsc --noEmit` 通过。
- `npm run test:front` 通过。
- `npm run check:windows-release-hygiene` 通过。
- `npm run check:rust:windows` 通过。
- `npm run test:rust:windows` 通过。
- `npm run tauri:build:windows:portable` 通过。
- Windows tag workflow 只生成 Windows x64 安装包。
- CodeGraph 同步后索引健康，关键 Windows 调用链可解析。

阶段二每项功能必须先有失败测试，再实现最小通过改动；完成后重复阶段一的 Windows 检查和 portable build。

任何新 Windows `codex_switch.exe` 都必须连同匹配的 x64 `WebView2Loader.dll` 部署到本机安装位置和远端桌面快捷方式的真实目标，并核对哈希。

## 来源

功能差距和上游永久链接见 `docs/research/2026-07-13-cockpit-tools-gap-analysis.md`。设计基线使用 Cockpit Tools v1.3.0、tag commit `da0deca4e7277d2bcfd62fcb674d56d85210bd7d`。

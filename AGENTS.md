# Codex Account Switch Agent 入口

本仓库维护 Windows x64 的本地补丁版本。`origin` 是只读上游；产品、构建、CI、安装与发布只支持 Windows x64。

## 启动顺序

1. 涉及代码、上游同步、构建、安装或发布时，先读 `LOCAL_PATCHES.md` 中与任务相关的章节；它是必须保留补丁、Windows 构建路径、身份/生命周期合同和私有发布边界的真源。
2. 只加载当前分支需要的文档：
   - **远端 Windows 部署**：读 `LOCAL_PATCHES.md` 的部署合同、`docs/environments/windows-dual-host.md` 和 `%CODEX_HOME%\docs\agent-guidance\ssh-mesh.md`；先运行只读 inspect，现场解析快捷方式目标、身份、host key 和哈希。
   - **Issue、PRD 或 triage**：读 `docs/agents/issue-tracker.md` 与 `docs/agents/triage-labels.md`。
   - **领域词汇或架构决策**：读 `docs/agents/domain.md`，再按其中规则读取仓库内实际存在且相关的决策记录；缺失的可选上下文文件不构成创建文档的理由。
3. 命令、环境变量、产物路径和测试入口以 `package.json`、当前脚本及其帮助输出为真源。Windows Vite/Tauri 构建需要允许 `esbuild` 创建子进程；出现 `spawn EPERM` 时修复执行环境，再运行现役 wrapper。

## 产品与安全不变量

- 现役 runtime、前端入口、资产、构建和发布保持 Windows x64。`src-tauri/shared/**` 只放不可避免的中性磁盘格式、协议模型与账号/quota 逻辑；Windows 进程、登录、窗口和安装行为留在 `src-tauri/win/**`。
- 上游更新先合并 tag，再逐项保留 `LOCAL_PATCHES.md` 的现役补丁并通过 Windows MSVC 验证；历史非 Windows 源树和发布资产只作证据。
- 浏览器路由只认显式 profile metadata；账号写回按 live `auth.json` identity 路由，marker 只作提示。无法证明 identity 或命中未管理漂移时失败闭锁。
- 切换与重启复用 Windows Store package identity、稳定空快照、生命周期锁和事务回滚；失败先恢复 root、槽位、marker 与相关索引，再决定是否重开 Codex。
- Profile 复制只管理账号态文件；全局 skills、plugins、cache、sqlite、rules、memories、runtime 与机器本地资产保持隔离。凭据、token、cookie 和完整账号标识不进入日志或远端复用。
- 安装替换把同一次 Windows MSVC 构建的 `codex_switch.exe` 与 x64 `WebView2Loader.dll` 视为不可拆分产物。远端替换只针对实时解析的快捷方式目标，前后验证双文件 SHA-256，失败时恢复成套旧版本。

## Git 发布边界

普通修改只提交并推送到用户自有 `private/main`。`origin` 和其它第三方 source/upstream 只读；历史 `patch-stack/*` 只作证据。临时分支须由 Agent 提出确切用途并取得 owner 事前确认，验收后再进入 `private/main`。

项目长期知识写入项目文档，设备事实写入 `docs/environments/`，本机事实留在 local 范围，一次性证据写入获准的 D 盘 evidence 或 reports 路径。

## 完成标准

按改动范围运行 `package.json` 和现役脚本声明的窄测试及失败路径，最后运行 `git diff --check`。若任务构建或替换了新 EXE，只有同构建 loader、本机安装校验和已登记远端的 inspect/备份/成套替换/哈希复核均完成时才算发布完成；当前授权不含远端写入时，停在已验证构建并明确报告发布尚未完成。

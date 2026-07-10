# macOS 与 Windows 拆分历史说明

> 状态：已完成并归档。本文记录原生 macOS runtime 落地前的历史设计背景，不是待执行计划；其中提到的旧源码路径也只代表当时布局。

早期 Tauri 桌面实现曾只覆盖 Windows，并把 Windows 行为简化为：

- 通过 Microsoft Store shell target 启动 Codex；
- 不再把桌面端 `app_path` 持久化到 `install_state.json`；
- 从 Windows process layer 移除本地 `.exe` discovery fallback。

后续原生 macOS 桌面端已经实现独立的应用解析和生命周期路径，覆盖：

- 在 `/Applications/Codex.app` 查找 Codex；
- 兼容 `~/Applications/Codex.app`；
- 通过 AppleScript 激活已运行的应用；
- profile 切换后重新打开 app bundle。

当前代码以 `src-tauri/mac/runtime/`、`src-tauri/win/runtime/` 和各自 platform frontend 为准。本文保留的意义仅是解释为什么两个平台不再从旧 Windows 路径推断彼此行为。

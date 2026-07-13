# 当前账号显示一致性 Implementation Plan

> 状态：已完成并归档（2026-07-14）；实现、Windows 回归、远端部署、独立探针和成对哈希核验均已通过。

> **For agentic workers:** Implement this plan task-by-task. Prefer `subagent-driven-development` when available for independent tasks; otherwise use the repository's normal implementation workflow. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让账号列表快照和实时额度始终以实时 `auth.json` 的托管身份所有者作为当前 profile。

**Architecture:** 复用 `profiles::resolve_backup_target` 作为唯一 effective-current 解析规则。公开读取入口保持只读：它们选择正确 profile，但不修改 marker、不复制文件、不触发 Codex 生命周期。

**Tech Stack:** Rust 2021、Tauri 2、现有 `cargo test` Windows MSVC 测试链。

## Global Constraints

- 账号身份规则只修改 shared Rust 运行时；Windows profile 重命名流程可做必要的兼容顺序调整，不修改前端或 macOS 模块。
- 不新增依赖。
- 不处理、检测或兼容 `CC Switch`。
- 测试必须通过公开 seam `load_profiles_snapshot` 与 `load_current_live_quota` 验证行为。
- Windows-only 验证使用 `x86_64-pc-windows-msvc`，不编译已退役非 Windows runtime。

---

### Task 1: 按实时身份解析当前 profile

**Files:**
- Modify: `src-tauri/shared/runtime/profiles_index.rs:17-21`
- Modify: `src-tauri/shared/runtime/profiles_index.rs:390-463`
- Test: `src-tauri/shared/runtime/profiles_index.rs:475-588`

**Interfaces:**
- Consumes: `profiles::resolve_backup_target(backup_root: &Path, codex_home: &Path) -> Option<String>`。
- Produces: `load_profiles_snapshot` 和 `load_current_live_quota` 保持原签名，但返回的 current profile 由 live identity 决定。

- [x] **Step 1: 写 managed drift 失败测试**

在现有 tests 模块增加：

```rust
#[test]
fn snapshot_and_live_quota_follow_managed_live_identity_instead_of_stale_marker() {
    let codex_home = temp_codex_home("snapshot-managed-drift");
    let backup_root = get_backup_root(Some(&codex_home));
    let profile_a_dir = backup_root.join("a");
    let profile_b_dir = backup_root.join("b");
    fs::create_dir_all(&profile_a_dir).unwrap();
    fs::create_dir_all(&profile_b_dir).unwrap();
    fs::write(profile_a_dir.join("auth.json"), auth_with_account("acct_A")).unwrap();
    fs::write(profile_b_dir.join("auth.json"), auth_with_account("acct_B")).unwrap();
    fs::write(
        profile_a_dir.join("profile.json"),
        r#"{"folder_name":"a","account_label":"a@example.com","quota":{"five_hour":{"remaining_percent":42},"weekly":{"remaining_percent":88}}}"#,
    )
    .unwrap();
    fs::write(
        profile_b_dir.join("profile.json"),
        r#"{"folder_name":"b","account_label":"b@example.com","quota":{"five_hour":{"remaining_percent":73},"weekly":{"remaining_percent":91}}}"#,
    )
    .unwrap();
    fs::write(backup_root.join(".current_profile"), "a\n").unwrap();
    fs::write(codex_home.join("auth.json"), auth_with_account("acct_B")).unwrap();

    let snapshot = load_profiles_snapshot(Some(&codex_home)).unwrap();
    let current_quota = load_current_live_quota(Some(&codex_home)).unwrap();

    assert_eq!(snapshot.unmanaged_live_account, None);
    assert_eq!(
        snapshot.current_card.as_ref().map(|card| card.folder_name.as_str()),
        Some("b")
    );
    assert_eq!(
        snapshot
            .profiles
            .iter()
            .find(|card| card.status == "current")
            .map(|card| card.folder_name.as_str()),
        Some("b")
    );
    assert_eq!(current_quota.profile.as_deref(), Some("b"));
    assert_eq!(
        current_quota
            .quota
            .as_ref()
            .and_then(|quota| quota.five_hour.remaining_percent),
        Some(73)
    );

    let _ = fs::remove_dir_all(&codex_home);
}
```

- [x] **Step 2: 运行测试并确认红灯**

Run:

```powershell
cargo test --manifest-path 'D:\Codex\projects\codex-account-switch\src-tauri\Cargo.toml' snapshot_and_live_quota_follow_managed_live_identity_instead_of_stale_marker --lib --target x86_64-pc-windows-msvc -- --nocapture
```

Expected: 运行 1 个测试并失败；旧实现把 `a` 而不是 `b` 作为 current profile。

- [x] **Step 3: 写最小实现**

把 `resolve_backup_target` 加入现有 profiles import，并在两个公开入口使用同一个解析结果：

```rust
let backup_root = get_backup_root(Some(&codex_home));
let unmanaged_live_account = detect_unmanaged_live_account(&backup_root, &codex_home);
let current_profile = if unmanaged_live_account.is_some() {
    None
} else {
    resolve_backup_target(&backup_root, &codex_home)
};
let current_profile_ref = current_profile.as_deref();
```

账号快照用 `current_profile_ref` 构造 cards。实时额度入口直接处理：

```rust
let backup_root = get_backup_root(Some(&codex_home));
let Some(current_profile) = resolve_backup_target(&backup_root, &codex_home) else {
    return Ok(CurrentQuotaResponse {
        profile: None,
        quota: None,
    });
};
```

- [x] **Step 4: 运行新测试并确认绿灯**

Run Step 2 的同一命令。

Expected: `1 passed; 0 failed`。

- [x] **Step 5: 运行相关回归测试**

Run:

```powershell
cargo test --manifest-path 'D:\Codex\projects\codex-account-switch\src-tauri\Cargo.toml' snapshot_and_live_quota --lib --target x86_64-pc-windows-msvc -- --nocapture
```

Expected: managed 与 unmanaged 两个测试全部通过。

- [x] **Step 6: 运行格式、Windows 全量 Rust 测试与差异检查**

Run:

```powershell
cargo fmt --manifest-path 'D:\Codex\projects\codex-account-switch\src-tauri\Cargo.toml' -- --check
npm run test:rust:windows
git diff --check
```

Expected: 格式检查退出 0；Windows Rust 测试 0 failures；diff check 退出 0。

- [x] **Step 7: 提交实现**

```powershell
git add -- 'src-tauri/shared/runtime/profiles_index.rs'
git commit -m 'fix: align current profile with live identity'
```

- [x] **Step 8: 对抗性代码评审**

以本计划提交为 fixed point，按 `code-review` skill 并行执行 Standards 与 Spec 两轴审查。若发现问题，先修复并重跑 Step 6，再提交修复。

---

### Task 2: 阻断跨 profile 展示元数据污染

**Files:**
- Modify: `src-tauri/shared/runtime/metadata.rs`
- Modify: `src-tauri/shared/runtime/fs_ops.rs`
- Test: 同文件 tests 模块

- [x] **Step 1: 写两个失败回归测试**

一个验证读取 `b` 时拒绝显式声明 `folder_name=c` 的元数据；另一个验证把根状态写回 `b` 时不会用声明为 `c` 的根 `profile.json` 覆盖 `b/profile.json`。

- [x] **Step 2: 确认旧实现红灯**

两个定向测试均在旧实现上失败，分别暴露错误标签读取和跨目录元数据覆盖。

- [x] **Step 3: 写最小防护并确认绿灯**

读取端同时校验目录声明与 auth 派生账号标签，冲突时只清空账号绑定的标签、套餐、订阅和额度字段，保留设备配置；写回端仅复制与目标身份匹配的根 `profile.json`，冲突时跳过，无效 JSON 或读取错误时 fail closed。

对 auth 无法提供账号标签的路径，带任何账号绑定字段的元数据均不可验证；读取时清洗、写回时跳过。重命名流程必须先走同一清洗入口再持久化新目录名。

- [x] **Step 4: 运行全量 Windows 验证并提交**

- [x] **Step 5: 重建、部署本机并验证实际界面**

- [x] **Step 6: 对抗性复审后部署远端并核验哈希**

历史暂缓记录（2026-07-13）：对抗性复审已完成；用户于 2026-07-13 指示暂不进行部署。2026-07-13 的传输窗口完成脚本预置并短暂生成 staging payload；临时 HTTP 已停止，staging payload 已删除，未进入 exe/loader 安装替换；远端保留当日已有安装状态。

本次完成（2026-07-14）：设备“自习室”（`aeawutfrjia4s5i6`）在线，UU 控制通道执行 `remote-deploy-20260714-005800`。远端快捷方式目标仍为 `D:\Codex\apps\codex_switch\codex_switch.exe`，工作目录为 `D:\Codex\apps\codex_switch`；替换后的 `codex_switch.exe` 为 `25,541,632` 字节、SHA-256 `FBA221AF9163516A42DCE84DCA15DCE809B9438DCB0D6DF3F64AE0037584470E`，`WebView2Loader.dll` 为 `160,320` 字节、SHA-256 `8427B1FC58EC707813E5C0A51EB5D69397BB333250A7B891BE4D3B123F1E0F1C`。部署保留成对旧文件备份，独立 cmd 探针确认精确目标进程 PID `35156` 且 `Responding=True`；远端 staging 仅保留成对备份与部署报告，HTTP 服务已停止、UU 会话已退出。完整证据位于 `D:\Codex\.codex\run-evidence\codex-account-switch\remote-deploy-20260714-005800\`。

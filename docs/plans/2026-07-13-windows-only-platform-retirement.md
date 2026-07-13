# Windows-only Platform Retirement Implementation Plan

> 状态：已完成并归档（2026-07-13）。Windows-only 源码裁剪、构建/发布收敛、文档更新、CodeGraph 同步和远端部署证据均已落地；本文仅保留为历史实施记录。

> **For agentic workers:** Implement this plan task-by-task. Prefer \`subagent-driven-development\` when available for independent tasks; otherwise use the repository's normal implementation workflow. Steps use checkbox (\`- [ ]\`) syntax for tracking.

**Goal:** Remove macOS and Linux product support from source, build, release, and current documentation while preserving Windows account-switching behavior and legacy disk safety guards.

**Architecture:** Select only the Windows adapter with explicit \`cfg(windows)\`; make non-Windows targets fail clearly. Change all references before deleting platform trees. Keep shared business modules and \`PlatformHooks\` as the Windows test seam, and keep \`account_backup/macos\` as a retired reserved directory name.

**Tech Stack:** Rust/Tauri 2.10.1, TypeScript/Vite 7.3.6, Vitest 4.1.10, npm, Cargo, GitHub Actions, CodeGraph, Windows MSVC.

## Global Constraints

- Only Windows x64 is supported; macOS, Linux, iOS, and Android are retired.
- Preserve Windows Store-package lifecycle, identity-routed writes, transactional rollback, and matching EXE/loader deployment.
- Do not delete shared runtime, commands, front-end, or \`PlatformHooks\` merely because they are under \`shared\`.
- Replace \`not(target_os = "macos")\` selection with explicit \`cfg(windows)\`; Linux must not silently compile the Windows adapter.
- Keep \`account_backup/macos\` excluded from profile discovery as a retired disk-format directory.
- Do not hand-prune platform optional dependencies from lockfiles.
- Keep historical CHANGELOG entries; update only current platform facts and add a retirement entry.
- Use Windows-only validation commands.

---

### Task 1: Freeze baseline and reference inventory

**Files:**
- Create: \`docs/evidence/2026-07-13-windows-only-baseline.md\`
- Read: CodeGraph index, \`package.json\`, \`.github/workflows/build.yml\`, \`scripts/check-windows-release-hygiene.mjs\`

**Interfaces:**
- Consumes: repository at the approved design commit plus any existing user commits.
- Produces: read-only baseline evidence and a classified reference inventory.

- [x] **Step 1: Capture repository and CodeGraph state**

    git -C "D:\Codex\projects\codex-account-switch" status --short --branch
    git -C "D:\Codex\projects\codex-account-switch" log -6 --oneline --decorate
    & "D:\Codex\tools\codegraph\current\bin\codegraph.cmd" status

    Expected: worktree state is recorded and CodeGraph is up to date.

- [x] **Step 2: Capture the platform inventory**

    rg --files "D:\Codex\projects\codex-account-switch" | rg "(^|[/\\])(src-tauri/mac|macOS-backup|src-tauri/icons/(ios|android)|scripts/macos|src-tauri/tauri\.(macos|linux))"
    rg -n -i 'target_os = "macos"|not\(target_os = "macos"\)|crate::macos|CODEX_UI_TARGET|@mac-front|macOS-backup|tauri:build:(macos|linux)|tauri\.(macos|linux)|\.dmg|\.pkg|AppImage|\.deb' "D:\Codex\projects\codex-account-switch\src-tauri" "D:\Codex\projects\codex-account-switch\scripts" "D:\Codex\projects\codex-account-switch\package.json" "D:\Codex\projects\codex-account-switch\vite.config.ts" "D:\Codex\projects\codex-account-switch\.github"

    Classify hits as active production/build references, historical docs, lockfile optional dependencies, or the retained legacy disk guard.

- [x] **Step 3: Write and commit the evidence**

    git -C "D:\Codex\projects\codex-account-switch" add -- "docs/evidence/2026-07-13-windows-only-baseline.md"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "docs: record Windows-only retirement baseline"

### Task 2: Make Rust platform selection explicitly Windows-only

**Files:**
- Modify: \`src-tauri/src/lib.rs\`, \`src-tauri/src/main.rs\`
- Modify: \`src-tauri/shared/platform/mod.rs\`
- Modify: \`src-tauri/shared/commands/actions.rs\`, \`dashboard.rs\`, \`switch.rs\`
- Modify: \`src-tauri/shared/runtime/cli.rs\`
- Modify: \`src-tauri/examples/live_quota_check.rs\`
- Modify: \`src-tauri/shared/runtime/paths.rs\`
- Modify: \`src-tauri/Cargo.toml\`, regenerate \`src-tauri/Cargo.lock\`
- Test: Windows Rust tests and the retired-directory test

**Interfaces:**
- Consumes: existing \`crate::windows\`, \`PlatformHooks\`, command signatures, and Windows profiles index.
- Produces: one Windows production adapter; non-Windows compilation fails explicitly.

- [x] **Step 1: Add the compile guard and remove macOS module declarations**

    #[cfg(not(windows))]
    compile_error!("codex_switch supports Windows only");

    In \`src-tauri/src/lib.rs\`, remove \`pub mod macos\` and replace all \`not(target_os = "macos")\` module selection with direct Windows modules.

- [x] **Step 2: Route platform hooks and commands directly to Windows**

    Replace each macOS/non-macOS pair with direct calls to:
    crate::windows::process::platform_hooks()
    crate::windows::bootstrap::ensure_backup_initialized(None)?
    crate::windows::bootstrap::ensure_refresh_runtime_config_initialized(None)?
    crate::windows::bootstrap::sync_root_state_to_current_profile(None)?
    crate::windowing::install(app)

    Keep \`PlatformHooks\` signatures unchanged.

- [x] **Step 3: Simplify command/runtime aliases**

    In \`shared/commands/{actions,dashboard,switch}.rs\` and \`shared/runtime/cli.rs\`, replace conditional imports with the Windows module import. Keep public Tauri command names and return types unchanged.

- [x] **Step 4: Restrict target dependencies and the example**

    Change \`live_quota_check.rs\` to import \`codex_switch_lib::windows::profiles_index\`. Change the single-instance target dependency to \`cfg(windows)\`, keep the existing \`windows-sys\` feature list, and remove the old macOS/Linux target table.

- [x] **Step 5: Preserve the legacy disk guard**

    Rename \`MACOS_RUNTIME_DIRNAME\` to \`LEGACY_MACOS_RUNTIME_DIRNAME\` in \`shared/runtime/paths.rs\`; keep it in \`is_profile_dir\` exclusion and assert that \`account_backup/macos\` is not a profile. This is disk compatibility, not macOS support.

- [x] **Step 6: Run the narrow checks**

    npm run check:rust:windows
    npm run test:rust:windows

    Expected: both commands exit 0. A failure blocks the next task.

- [x] **Step 7: Commit the Rust seam**

    git -C "D:\Codex\projects\codex-account-switch" add -- "src-tauri/src/lib.rs" "src-tauri/src/main.rs" "src-tauri/shared/platform/mod.rs" "src-tauri/shared/commands" "src-tauri/shared/runtime/cli.rs" "src-tauri/shared/runtime/paths.rs" "src-tauri/examples/live_quota_check.rs" "src-tauri/Cargo.toml" "src-tauri/Cargo.lock"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "refactor: make Rust runtime Windows-only"

### Task 3: Collapse front-end and package entrypoints to Windows

**Files:**
- Modify: \`vite.config.ts\`
- Modify: \`src-tauri/shared/front/vite-env.d.ts\`, \`render.ts\`, \`base.css\`
- Modify: \`scripts/version-sync.mjs\`
- Modify: \`package.json\`, \`package-lock.json\`
- Modify: \`src-tauri/tauri.conf.json\`, \`tsconfig.json\`

**Interfaces:**
- Consumes: \`src-tauri/win/front\` and \`__CODEX_APP_VERSION__\`.
- Produces: deterministic Windows Vite root and Windows-only npm/Tauri entrypoints.

- [x] **Step 1: Fix Vite root and aliases**

    Set \`const root = "src-tauri/win/front";\` in \`vite.config.ts\`. Remove \`CODEX_UI_TARGET\`, \`__CODEX_UI_TARGET__\`, Darwin selection, and \`@mac-front\); keep shared and Windows aliases.

- [x] **Step 2: Remove target conditionals from the front end**

    Delete \`__CODEX_UI_TARGET__\` from \`vite-env.d.ts\`. Make the Windows branches in \`render.ts\` unconditional and remove \`isWindowsUiTarget\`. Remove only macOS selectors from \`base.css\`.

- [x] **Step 3: Restrict version-sync to Windows HTML**

    Set \`versionHardcodeForbiddenPaths\` to exactly:
    const versionHardcodeForbiddenPaths = [
      'src-tauri/win/front/index.html',
    ]

    A missing Windows HTML file must still fail the check.

- [x] **Step 4: Keep generic commands but make them Windows-only**

    Set:
    "tauri:build": "npm run tauri:build:windows"
    "tauri:build:portable": "npm run tauri:build:windows:portable"

    Remove macOS package commands and \`tauri:build:linux\`; retain Windows checks, builds, and deployment.

- [x] **Step 5: Remove retired icon references**

    Remove \`icons/icon.icns\` from \`tauri.conf.json\`; retain \`icons/icon.ico\` and \`icons/icon.png\`.

- [x] **Step 6: Run the front-end checks**

    npm run version:check
    npx tsc --noEmit
    npm run test:front

    Expected: all commands exit 0.

- [x] **Step 7: Commit the front-end/package seam**

    git -C "D:\Codex\projects\codex-account-switch" add -- "vite.config.ts" "src-tauri/shared/front" "scripts/version-sync.mjs" "package.json" "package-lock.json" "src-tauri/tauri.conf.json" "tsconfig.json"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "refactor: collapse frontend build to Windows"

### Task 4: Convert CI and release publishing to Windows-only

**Files:**
- Modify: \`.github/workflows/build.yml\`
- Test: \`scripts/check-windows-release-hygiene.mjs\`

**Interfaces:**
- Consumes: Windows-only scripts from Tasks 2–3.
- Produces: PR/main CI and tag/manual releases that build only Windows x64.

- [x] **Step 1: Convert CI runner and checks**

    Use \`windows-latest\` for CI. Keep Node 22, npm ci, type-check, front tests, Windows release hygiene, version check, and Windows MSVC check/test in their required order. Remove Ubuntu apt setup and Linux-only dependency installation.

- [x] **Step 2: Reduce the release matrix**

    Delete macOS-arm64, macOS-x64, and Linux entries. Keep Windows target \`x86_64-pc-windows-msvc\`, script \`tauri:build:windows\`, and the NSIS artifact glob. Remove Apple signing, DMG verification, Go setup, and non-Windows globs.

- [x] **Step 3: Restrict draft release assets**

    Keep only \`artifacts/codex-switch-windows-x86_64/**/*\` in the release job and keep the dependency on the single Windows build.

- [x] **Step 4: Run and commit the workflow contract**

    npm run check:windows-release-hygiene
    git -C "D:\Codex\projects\codex-account-switch" add -- ".github/workflows/build.yml"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "ci: publish Windows-only releases"

    Expected: hygiene checker exits 0 before the commit.

### Task 5: Delete retired source, assets, scripts, and historical-only plans

**Files:**
- Delete: \`src-tauri/mac/**\`, \`macOS-backup/**\`
- Delete: \`scripts/macos-artifacts.mjs\`, \`scripts/create-macos-pkg.mjs\`
- Delete: \`src-tauri/tauri.macos.conf.json\`, \`src-tauri/tauri.linux.conf.json\`
- Delete: \`src-tauri/icons/icon.icns\`, \`src-tauri/icons/ios/**\`, \`src-tauri/icons/android/**\`
- Delete: \`docs/plans/2026-04-16-macos-desktop-app.md\`, \`2026-04-20-macos-sync-latest-windows-updates.md\`

**Interfaces:**
- Consumes: all active references removed in Tasks 2–4.
- Produces: no retired platform implementation remains.

- [x] **Step 1: Verify references before deletion**

    rg -n -i 'src-tauri/mac|macOS-backup|scripts/macos|tauri\.(macos|linux)|tauri:build:(macos|linux)|@mac-front|CODEX_UI_TARGET' "D:\Codex\projects\codex-account-switch\src-tauri" "D:\Codex\projects\codex-account-switch\scripts" "D:\Codex\projects\codex-account-switch\package.json" "D:\Codex\projects\codex-account-switch\vite.config.ts" ".github"

    Expected: no active production/config/build hits.

- [x] **Step 2: Delete exact paths after boundary verification**

    Verify every resolved target is below D:\Codex\projects\codex-account-switch, then use literal-path Remove-Item for only the listed paths. Do not use a keyword-based recursive delete.

- [x] **Step 3: Regenerate metadata normally**

    npm install --package-lock-only
    cargo metadata --manifest-path "D:\Codex\projects\codex-account-switch\src-tauri\Cargo.toml" --format-version 1 --no-deps | Out-Null

    Do not hand-edit platform optional dependencies from either lockfile.

- [x] **Step 4: Commit the deletion**

    git -C "D:\Codex\projects\codex-account-switch" add -A -- "src-tauri" "macOS-backup" "scripts" "docs/plans" "package-lock.json"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "refactor: retire non-Windows platform sources"

### Task 6: Rewrite current documentation and update behavior

**Files:**
- Modify: \`README.md\`, \`README.zh-CN.md\`
- Modify: \`docs/IMPLEMENTATION.md\`, \`AGENTS.md\`, \`LOCAL_PATCHES.md\`, \`CHANGELOG.md\`
- Modify: \`src-tauri/shared/runtime/paths.rs\`
- Modify: active update action/UI files found by \`rg\`

**Interfaces:**
- Consumes: final Windows-only tree and existing update-check IPC.
- Produces: current documentation that promises Windows only and an informational upstream check that cannot install a public upstream binary over the private patch build.

- [x] **Step 1: Rewrite current README and implementation facts**

    Keep Windows x64 install/build/deploy, account paths, security, and Usage statistics. Remove current macOS/Linux download rows, platform sections, macOS shell instructions, and multi-platform claims. State that old releases are historical and unsupported.

- [x] **Step 2: Update project contracts**

    Change \`AGENTS.md\` and \`LOCAL_PATCHES.md\` to make Windows-only an explicit contract. Future upstream merges must not resurrect retired platform directories, targets, release matrices, or generic non-Windows fallbacks.

- [x] **Step 3: Preserve historical changelog truth**

    Add an Unreleased breaking-change entry; do not delete old macOS/Linux entries.

- [x] **Step 4: Make update behavior informational**

    Keep the upstream version check but show “upstream version available; merge and rebuild required”. Keep opening release notes; remove any direct download/install action that could replace this private patched build.

- [x] **Step 5: Run documentation checks**

    rg -n -i 'macOS-backup|src-tauri/mac|tauri:build:(macos|linux)|\.dmg|\.pkg|AppImage|\.deb|aarch64-apple|x86_64-apple' "D:\Codex\projects\codex-account-switch\README.md" "D:\Codex\projects\codex-account-switch\README.zh-CN.md" "D:\Codex\projects\codex-account-switch\docs\IMPLEMENTATION.md" "D:\Codex\projects\codex-account-switch\AGENTS.md" "D:\Codex\projects\codex-account-switch\LOCAL_PATCHES.md"

    Expected: only explicitly marked historical references remain.

- [x] **Step 6: Commit docs and update behavior**

    git -C "D:\Codex\projects\codex-account-switch" add -- "README.md" "README.zh-CN.md" "docs/IMPLEMENTATION.md" "AGENTS.md" "LOCAL_PATCHES.md" "CHANGELOG.md" "src-tauri/shared/runtime/paths.rs"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "docs: document Windows-only product boundary"

### Task 7: Full Windows verification, build, and deployment evidence

**Files:**
- Create: \`docs/evidence/2026-07-13-windows-only-verification.md\`
- Read: final diff, active file tree, CodeGraph, build outputs

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces: fresh evidence for source closure, tests, Windows build, and deployment readiness.

- [x] **Step 1: Run active-source closure scan**

    rg -n -i 'target_os = "macos"|not\(target_os = "macos"\)|crate::macos|src-tauri/mac|macOS-backup|tauri\.macos|tauri\.linux|tauri:build:linux|CODEX_UI_TARGET|@mac-front' "D:\Codex\projects\codex-account-switch\src-tauri" "D:\Codex\projects\codex-account-switch\scripts" "D:\Codex\projects\codex-account-switch\package.json" "D:\Codex\projects\codex-account-switch\vite.config.ts" ".github"

    Expected: no active references; permitted historical text is recorded separately.

- [x] **Step 2: Run the full Windows checks**

    npm ci
    npm run version:check
    npx tsc --noEmit
    npm run test:front
    npm run check:windows-release-hygiene
    npm run check:rust:windows
    npm run test:rust:windows

    Expected: every command exits 0. Any failure blocks completion.

- [x] **Step 3: Build the installed-app artifact**

    npm run tauri:build:windows:portable

    Expected: MSVC \`codex_switch.exe\` and matching x64 \`WebView2Loader.dll\` exist at the documented target paths.

- [x] **Step 4: Synchronize CodeGraph**

    & "D:\Codex\tools\codegraph\current\bin\codegraph.cmd" sync
    & "D:\Codex\tools\codegraph\current\bin\codegraph.cmd" status

    Expected: index is up to date and no retired platform adapter nodes remain.

- [x] **Step 5: Inspect and deploy the real remote shortcut target**

    Run the deployment script with \`-InspectOnly\` first. Only after confirming the real shortcut target, deploy the new EXE and matching loader using the documented remote parameters. Verify SHA-256 hashes on both hosts and preserve rollback backups.

- [x] **Step 6: Record evidence and review the final diff**

    Write command results, artifact paths, hashes, and permitted historical references to \`docs/evidence/2026-07-13-windows-only-verification.md\`. Then run:
    git -C "D:\Codex\projects\codex-account-switch" diff origin/main...HEAD --check
    git -C "D:\Codex\projects\codex-account-switch" status --short --branch

    Expected: no whitespace errors and only intentional changes remain.

- [x] **Step 7: Commit the evidence**

    git -C "D:\Codex\projects\codex-account-switch" add -- "docs/evidence/2026-07-13-windows-only-verification.md"
    git -C "D:\Codex\projects\codex-account-switch" commit -m "test: verify Windows-only platform retirement"

## Execution Order and Review Gates

Tasks 1–2 may be reviewed together, but Task 2 must pass Windows Rust checks before Task 3. Tasks 3–4 must pass front-end and workflow checks before deletion in Task 5. Task 5–6 must complete before Task 7.

Because this is destructive, cross-module, user-data, and lifecycle-sensitive, mandatory adversarial gates are:
1. After Task 2: review Rust cfg changes and the old disk directory guard.
2. After Task 5: review deletion completeness, build references, and rollback safety.
3. Before Task 7 deployment: review Windows artifact identity and remote target resolution.

Cockpit Tools 第一组能力已在独立设计/计划 [`docs/specs/2026-07-13-windows-only-cockpit-alignment-design.md`](../specs/2026-07-13-windows-only-cockpit-alignment-design.md) 与 [`docs/plans/2026-07-13-cockpit-phase1.md`](2026-07-13-cockpit-phase1.md) 中完成；后续能力仍须独立设计和验收。

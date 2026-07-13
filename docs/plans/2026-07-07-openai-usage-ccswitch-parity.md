# OpenAI Usage CC Switch Parity Implementation Plan

> 状态：已完成并归档；所有实施任务已勾选，本文保留功能、约束和验收记录。

> **For agentic workers:** Implement this plan task-by-task. Prefer inline execution in this repository because backend and frontend types must move together. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Usage page to CC Switch feature parity for time filtering, trends, stats tables, request logs, and near-real-time refresh while keeping Codex Switch's OpenAI-official accounting rules.

**Architecture:** Parse Codex JSONL into canonical `UsageEvent` rows, sync them into a local SQLite ledger at `account_backup/openai_usage.sqlite`, then derive summary, trends, account stats, model stats, logs, and data-source stats from one dashboard query. The ledger is a rebuildable fact table, not an immutable first-write cache: source file signatures include session JSONL, attribution records, profile metadata, active markers, and the pricing version, so rule or source changes rebuild the rows instead of freezing historical attribution or pricing.

**Tech Stack:** Rust Tauri commands, existing shared TypeScript frontend, existing CSS shell, Codex JSONL session files.

**Status:** Initial parity implementation completed on 2026-07-07 in commit `c0d3098`; usage facet self-collapse follow-up completed in commit `61db517`; refresh/pagination/chart hardening completed on 2026-07-08 in commit `8a193e4`. Desktop/mobile browser smoke and Windows portable deploy were verified for the 2026-07-08 follow-up. A later 2026-07-08 hardening pass made the `candidate_unattributed` layer visible as a separate reconciliation card and replaced native date/time controls with app-owned minute inputs in the range popover. On 2026-07-09, commit `70df10d` fixed model and request-evidence table truncation and was deployed through the Windows portable local/remote replacement path.

## Global Constraints

- Only count OpenAI official usage: default OpenAI path or `api.openai.com`; do not use `model_provider`.
- Explicit Base URL must be HTTPS and host `api.openai.com`; `http://api.openai.com` is excluded.
- Preserve account attribution ledger and dedupe.
- Unknown model prices must remain unpriced, not `$0.0000`.
- Time filters use epoch seconds and local-time display.
- Default refresh is 30 seconds while the Usage page is open; manual refresh must provide immediate busy or queued feedback.
- One frontend refresh should call one backend dashboard command, so summary, trends, logs, and source stats share the same sync/query snapshot.

---

### Task 1: Backend Event Model And Query Surface

**Files:**
- Modify: `src-tauri/shared/runtime/models.rs`
- Modify: `src-tauri/shared/runtime/openai_usage.rs`
- Modify: `src-tauri/shared/commands/usage.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces `OpenAiUsageQuery`, `OpenAiUsageTrendPoint`, `OpenAiUsageRequestLog`, `OpenAiUsageRequestLogPage`, and `OpenAiUsageDataSourceSummary`.
- Replaces `load_openai_usage_summary(codex_home)` with `load_openai_usage_summary(codex_home, query)`.

- [x] Add query/filter/result structs.
- [x] Refactor JSONL parsing to return canonical events before aggregation.
- [x] Add summary, trends, account/model stats, logs, details, and data-source commands.
- [x] Add SQLite ledger rebuild, source-signature invalidation, and unit tests for time filters, official HTTPS filtering, dashboard aggregation, and idempotent dedupe.

### Task 2: Frontend State, API, And Controls

**Files:**
- Modify: `src-tauri/shared/front/types.ts`
- Modify: `src-tauri/shared/front/tauri.ts`
- Modify: `src-tauri/shared/front/state.ts`
- Modify: `src-tauri/shared/front/actions.ts`
- Modify: `src-tauri/win/front/index.html`

**Interfaces:**
- Consumes backend query/result structs.
- Produces Usage page state: range, account filter, model filter, refresh interval, logs page.

- [x] Add Usage query types and Tauri wrappers.
- [x] Add time range, live-end, account/model filters, refresh interval, and logs state.
- [x] Wire usage controls and route-scoped 30s auto-refresh with manual busy/queued feedback.
- [x] Keep account/model filter options scoped to the selected time range and other filters without collapsing on their own selected value.

### Task 3: Frontend Render And Styling

**Files:**
- Modify: `src-tauri/shared/front/render.ts`
- Modify: `src-tauri/shared/front/base.css`
- Modify: `src-tauri/shared/front/i18n.ts`

**Interfaces:**
- Consumes `OpenAiUsageSummary`, trend points, logs, account/model arrays, data-source summaries.

- [x] Render CC Switch parity controls.
- [x] Render hero, trend chart, logs/account/model tabs, detail rows, and evidence panel.
- [x] Move the account classification table below the time controls and before the summary cards.
- [x] Show `candidate_unattributed` as a separate reconciliation layer without mixing it into strict or legacy cost totals.
- [x] Keep the custom range popover app-owned: text date/time fields plus app calendar, no native OS date picker overlay.
- [x] Keep model and request-evidence cells readable by allowing long values to wrap instead of truncating them with table ellipsis styling.
- [x] Preserve compact desktop styling and responsive behavior.

### Task 4: Verification And Review

**Files:**
- Modify only if tests reveal a bug.

- [x] Run `cargo fmt --check`.
- [x] Run `cargo test --manifest-path src-tauri/Cargo.toml openai_usage --lib -- --nocapture`.
- [x] Run `npm run build`.
- [x] Run `npm run check:rust:windows`.
- [x] Run `npm run tauri:build:windows:portable` before replacing installed Windows artifacts.
- [x] Run browser visual smoke if dev server is available.
- [x] Run multi-agent adversarial review before commit.

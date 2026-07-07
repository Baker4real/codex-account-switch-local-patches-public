# OpenAI Usage CC Switch Parity Implementation Plan

> **For agentic workers:** Implement this plan task-by-task. Prefer inline execution in this repository because backend and frontend types must move together. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Usage page to CC Switch feature parity for time filtering, trends, stats tables, request logs, and near-real-time refresh while keeping Codex Switch's OpenAI-official accounting rules.

**Architecture:** Parse Codex JSONL into canonical `UsageEvent` rows, sync them into a local SQLite ledger at `account_backup/openai_usage.sqlite`, then derive summary, trends, account stats, model stats, logs, and data-source stats from one dashboard query. The ledger is a rebuildable fact table, not an immutable first-write cache: source file signatures include session JSONL, attribution records, profile metadata, active markers, and the pricing version, so rule or source changes rebuild the rows instead of freezing historical attribution or pricing.

**Tech Stack:** Rust Tauri commands, existing shared TypeScript frontend, existing CSS shell, Codex JSONL session files.

## Global Constraints

- Only count OpenAI official usage: default OpenAI path or `api.openai.com`; do not use `model_provider`.
- Explicit Base URL must be HTTPS and host `api.openai.com`; `http://api.openai.com` is excluded.
- Preserve account attribution ledger and dedupe.
- Unknown model prices must remain unpriced, not `$0.0000`.
- Time filters use epoch seconds and local-time display.
- Default refresh should make new usage visible within roughly one second while the Usage page is open.
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

- [ ] Add query/filter/result structs.
- [ ] Refactor JSONL parsing to return canonical events before aggregation.
- [ ] Add summary, trends, account/model stats, logs, details, and data-source commands.
- [ ] Add SQLite ledger rebuild, source-signature invalidation, and unit tests for time filters, official HTTPS filtering, dashboard aggregation, and idempotent dedupe.

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

- [ ] Add Usage query types and Tauri wrappers.
- [ ] Add time range, live-end, account/model filters, refresh interval, and logs state.
- [ ] Wire usage controls and route-scoped 1s auto-refresh.

### Task 3: Frontend Render And Styling

**Files:**
- Modify: `src-tauri/shared/front/render.ts`
- Modify: `src-tauri/shared/front/base.css`
- Modify: `src-tauri/shared/front/i18n.ts`

**Interfaces:**
- Consumes `OpenAiUsageSummary`, trend points, logs, account/model arrays, data-source summaries.

- [ ] Render CC Switch parity controls.
- [ ] Render hero, trend chart, logs/account/model tabs, detail rows, and evidence panel.
- [ ] Preserve compact desktop styling and responsive behavior.

### Task 4: Verification And Review

**Files:**
- Modify only if tests reveal a bug.

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml openai_usage --lib -- --nocapture`.
- [ ] Run `npm run build`.
- [ ] Run `npm run check:rust:windows`.
- [ ] Run `npm run tauri:build:windows:portable` before replacing installed Windows artifacts.
- [ ] Run Playwright/Edge visual smoke if dev server is available.
- [ ] Run multi-agent adversarial review before commit.

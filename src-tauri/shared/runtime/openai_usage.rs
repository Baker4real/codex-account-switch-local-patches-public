use std::collections::{hash_map::DefaultHasher, BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{AppError, AppResult};
use crate::models::{
    OpenAiUsageAccountSummary, OpenAiUsageCostBreakdown, OpenAiUsageDashboard,
    OpenAiUsageDataSourceSummary, OpenAiUsageExclusions, OpenAiUsageModelSummary, OpenAiUsageQuery,
    OpenAiUsageRequestLog, OpenAiUsageRequestLogPage, OpenAiUsageSummary, OpenAiUsageTokenTotals,
    OpenAiUsageTrendPoint, ProfileIndexEntry,
};

use super::metadata::load_account_identity_from_path;
use super::paths::{
    get_backup_root, get_codex_home, get_current_profile_file, get_openai_usage_attribution_path,
    get_openai_usage_ledger_path,
};
use super::profiles_index::load_profiles_index;
use super::session_files::{collect_jsonl_files, file_modified_ms};

const PRICING_SOURCE: &str = "OpenAI API pricing";
const PRICING_SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing";
const PRICING_CHECKED_AT: &str = "2026-07-07";

#[derive(Clone, Copy)]
struct ModelPricing {
    model: &'static str,
    input_micros_per_million: u64,
    cached_input_micros_per_million: u64,
    output_micros_per_million: u64,
    source_url: &'static str,
    checked_at: &'static str,
}

const MODEL_PRICES: &[ModelPricing] = &[
    price("gpt-5.5", 5_000_000, 500_000, 30_000_000),
    price("gpt-5.4", 2_500_000, 250_000, 15_000_000),
    price("gpt-5.4-mini", 750_000, 75_000, 4_500_000),
    price("gpt-5.4-nano", 200_000, 20_000, 1_250_000),
    price("gpt-5.3-codex", 1_750_000, 175_000, 14_000_000),
];

const fn price(
    model: &'static str,
    input_micros_per_million: u64,
    cached_input_micros_per_million: u64,
    output_micros_per_million: u64,
) -> ModelPricing {
    ModelPricing {
        model,
        input_micros_per_million,
        cached_input_micros_per_million,
        output_micros_per_million,
        source_url: PRICING_SOURCE_URL,
        checked_at: PRICING_CHECKED_AT,
    }
}

#[derive(Default)]
struct Aggregate {
    tokens: OpenAiUsageTokenTotals,
    cost: OpenAiUsageCostBreakdown,
    requests: u64,
}

#[derive(Default)]
struct ModelAggregate {
    aggregate: Aggregate,
    priced: bool,
    pricing_note: Option<String>,
}

#[derive(Default)]
struct AccountAggregate {
    account_label: Option<String>,
    official_source: String,
    aggregate: Aggregate,
    models: BTreeMap<String, ModelAggregate>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UsageCounts {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
}

#[derive(Deserialize)]
struct SessionLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    line_type: String,
    payload: Option<Value>,
}

struct AttributionContext {
    profile: String,
    account_label: Option<String>,
    official_source: String,
    attribution_since_ms: u64,
}

#[derive(Clone)]
struct UsageEvent {
    request_id: String,
    fingerprint: String,
    session_id: String,
    profile: String,
    account_label: Option<String>,
    official_source: String,
    model: String,
    delta: UsageCounts,
    cost: CalculatedCost,
    status_code: u16,
    created_at_ms: u64,
    data_source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AttributionRecord {
    observed_at_ms: u64,
    profile: Option<String>,
    account_label: Option<String>,
    openai_base_url: Option<String>,
    official_source: Option<String>,
    is_official_openai: bool,
    exclusion_reason: Option<String>,
}

impl AttributionRecord {
    fn context(&self) -> Option<AttributionContext> {
        self.is_official_openai.then(|| AttributionContext {
            profile: self
                .profile
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            account_label: self.account_label.clone(),
            official_source: self
                .official_source
                .clone()
                .unwrap_or_else(|| "OpenAI default".to_string()),
            attribution_since_ms: self.observed_at_ms,
        })
    }
}

pub fn load_openai_usage_summary(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
) -> AppResult<OpenAiUsageSummary> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let events = load_ledger_events(&conn, &query)?;
    Ok(build_summary_from_events(summary, &events, &query))
}

pub fn load_openai_usage_dashboard(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
    page: u32,
    page_size: u32,
) -> AppResult<OpenAiUsageDashboard> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let events = load_ledger_events(&conn, &query)?;
    let account_query = without_account_filter(&query);
    let account_events = load_ledger_events(&conn, &account_query)?;
    let account_summary = build_summary_from_events(
        new_usage_summary(&account_query),
        &account_events,
        &account_query,
    )
    .accounts;
    let model_query = without_model_filter(&query);
    let model_events = load_ledger_events(&conn, &model_query)?;
    let model_options =
        build_summary_from_events(new_usage_summary(&model_query), &model_events, &model_query)
            .models;
    let summary = build_summary_from_events(summary, &events, &query);
    let trends = build_trends(&events, &query);
    let logs = build_request_log_page(&events, page, page_size);
    let data_sources = build_data_source_summaries(&events);
    Ok(OpenAiUsageDashboard {
        summary,
        account_summary,
        model_options,
        trends,
        logs,
        data_sources,
    })
}

fn new_usage_summary(query: &OpenAiUsageQuery) -> OpenAiUsageSummary {
    OpenAiUsageSummary {
        generated_at_ms: current_time_ms(),
        pricing_source: PRICING_SOURCE.to_string(),
        pricing_checked_at: PRICING_CHECKED_AT.to_string(),
        range_start_ms: query.start_date.and_then(seconds_to_ms),
        range_end_ms: query.end_date.and_then(end_seconds_to_ms),
        ..OpenAiUsageSummary::default()
    }
}

fn without_account_filter(query: &OpenAiUsageQuery) -> OpenAiUsageQuery {
    let mut next = query.clone();
    next.profile = None;
    next.account_label = None;
    next
}

fn without_model_filter(query: &OpenAiUsageQuery) -> OpenAiUsageQuery {
    let mut next = query.clone();
    next.model = None;
    next
}

fn build_summary_from_events(
    mut summary: OpenAiUsageSummary,
    events: &[UsageEvent],
    query: &OpenAiUsageQuery,
) -> OpenAiUsageSummary {
    let mut accounts: BTreeMap<String, AccountAggregate> = BTreeMap::new();
    let mut global_models: BTreeMap<String, ModelAggregate> = BTreeMap::new();

    for event in events
        .iter()
        .filter(|event| event_matches_query(event, &query))
    {
        add_event_usage(&mut accounts, &mut global_models, event);
    }

    let total = total_account_aggregate(&accounts);
    summary.total_requests = total.requests;
    summary.tokens = total.tokens;
    summary.cost = total.cost;
    summary.cache_hit_rate = cache_hit_rate(&summary.tokens);
    summary.accounts = accounts
        .into_iter()
        .map(|(profile, account)| account_to_summary(profile, account))
        .collect();
    summary.models = models_to_summary(global_models);
    summary
}

pub fn load_openai_usage_trends(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
) -> AppResult<Vec<OpenAiUsageTrendPoint>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let events = load_ledger_events(&conn, &query)?;
    Ok(build_trends(&events, &query))
}

pub fn load_openai_usage_request_logs(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
    page: u32,
    page_size: u32,
) -> AppResult<OpenAiUsageRequestLogPage> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let events = load_ledger_events(&conn, &query)?;
    Ok(build_request_log_page(&events, page, page_size))
}

pub fn load_openai_usage_request_detail(
    codex_home: Option<&Path>,
    request_id: &str,
) -> AppResult<Option<OpenAiUsageRequestLog>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let mut summary = new_usage_summary(&OpenAiUsageQuery::default());
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    Ok(load_ledger_events(&conn, &OpenAiUsageQuery::default())?
        .iter()
        .find(|event| event.request_id == request_id)
        .map(event_to_log))
}

pub fn load_openai_usage_data_sources(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
) -> AppResult<Vec<OpenAiUsageDataSourceSummary>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let events = load_ledger_events(&conn, &query)?;
    Ok(build_data_source_summaries(&events))
}

fn build_request_log_page(
    events: &[UsageEvent],
    page: u32,
    page_size: u32,
) -> OpenAiUsageRequestLogPage {
    let mut logs = events.iter().map(event_to_log).collect::<Vec<_>>();
    logs.sort_by(|left, right| {
        right
            .created_at_ms
            .cmp(&left.created_at_ms)
            .then_with(|| right.request_id.cmp(&left.request_id))
    });
    let total = u32::try_from(logs.len()).unwrap_or(u32::MAX);
    let page_size = page_size.clamp(1, 200);
    let page = page.max(1);
    let start = usize::try_from((page - 1).saturating_mul(page_size)).unwrap_or(usize::MAX);
    let end = start.saturating_add(usize::try_from(page_size).unwrap_or(usize::MAX));
    let data = if start >= logs.len() {
        Vec::new()
    } else {
        logs[start..logs.len().min(end)].to_vec()
    };
    OpenAiUsageRequestLogPage {
        data,
        total,
        page,
        page_size,
    }
}

fn build_data_source_summaries(events: &[UsageEvent]) -> Vec<OpenAiUsageDataSourceSummary> {
    let mut sources: BTreeMap<String, Aggregate> = BTreeMap::new();
    for event in events {
        add_to_aggregate(
            sources.entry(event.data_source.clone()).or_default(),
            event.delta,
            &event.cost.breakdown,
        );
    }
    sources
        .into_iter()
        .map(|(data_source, aggregate)| OpenAiUsageDataSourceSummary {
            data_source,
            request_count: aggregate.requests,
            cost: aggregate.cost,
        })
        .collect()
}

fn load_usage_events(
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
) -> AppResult<Vec<UsageEvent>> {
    let fallback_attribution = build_attribution_context(codex_home, &mut summary.exclusions)?;
    let attribution_records = load_attribution_records(codex_home);
    summary.attribution_since_ms = attribution_records
        .iter()
        .filter(|record| record.is_official_openai)
        .map(|record| record.observed_at_ms)
        .min()
        .or_else(|| {
            fallback_attribution
                .as_ref()
                .map(|record| record.attribution_since_ms)
        });
    if fallback_attribution.is_none() && attribution_records.is_empty() {
        return Ok(Vec::new());
    }

    let mut request_ids = HashSet::new();
    let mut fingerprints = HashSet::new();
    let mut events = Vec::new();

    for sessions_root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        let mut files = Vec::new();
        collect_jsonl_files(&sessions_root, &mut files);
        files.sort();
        for file in files {
            collect_session_file_events(
                &file,
                fallback_attribution.as_ref(),
                &attribution_records,
                &mut request_ids,
                &mut fingerprints,
                &mut events,
                summary,
            );
        }
    }
    Ok(events)
}

fn sync_openai_usage_ledger(
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
) -> AppResult<Connection> {
    let mut conn = open_usage_ledger(codex_home)?;
    ensure_usage_ledger_schema(&conn)?;
    let source_signature = usage_source_signature(codex_home);
    if apply_cached_usage_sync_state(&conn, &source_signature, summary)? {
        return Ok(conn);
    }
    let events = load_usage_events(codex_home, summary)?;
    rebuild_usage_events(&mut conn, &events, &source_signature, summary)?;
    Ok(conn)
}

fn open_usage_ledger(codex_home: &Path) -> AppResult<Connection> {
    let path = get_openai_usage_ledger_path(Some(codex_home));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_LEDGER_DIR_FAILED",
                format!(
                    "Failed to create usage ledger dir {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    let conn = Connection::open(&path).map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_LEDGER_OPEN_FAILED",
            format!("Failed to open usage ledger {}: {error}", path.display()),
        )
    })?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    Ok(conn)
}

fn ensure_usage_ledger_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS openai_usage_events (
            request_id TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            profile TEXT NOT NULL,
            account_label TEXT,
            official_source TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            cached_input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            reasoning_output_tokens INTEGER NOT NULL,
            real_total_tokens INTEGER NOT NULL,
            input_cost_micros INTEGER NOT NULL,
            cached_input_cost_micros INTEGER NOT NULL,
            output_cost_micros INTEGER NOT NULL,
            total_cost_micros INTEGER NOT NULL,
            unpriced_tokens INTEGER NOT NULL,
            priced INTEGER NOT NULL,
            pricing_note TEXT,
            pricing_source TEXT NOT NULL,
            pricing_checked_at TEXT NOT NULL,
            status_code INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            data_source TEXT NOT NULL,
            inserted_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_created_at
            ON openai_usage_events(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_profile
            ON openai_usage_events(profile);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_model
            ON openai_usage_events(model);
        CREATE TABLE IF NOT EXISTS openai_usage_sync_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            source_signature TEXT NOT NULL,
            pricing_checked_at TEXT NOT NULL,
            synced_at_ms INTEGER NOT NULL,
            attribution_since_ms INTEGER,
            duplicate_events INTEGER NOT NULL DEFAULT 0,
            non_official_base_url INTEGER NOT NULL DEFAULT 0,
            unproven_history INTEGER NOT NULL DEFAULT 0,
            unattributed INTEGER NOT NULL DEFAULT 0,
            missing_model INTEGER NOT NULL DEFAULT 0,
            invalid_delta INTEGER NOT NULL DEFAULT 0,
            malformed_lines INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_LEDGER_SCHEMA_FAILED",
            format!("Failed to initialize usage ledger schema: {error}"),
        )
    })?;
    ensure_usage_sync_state_columns(conn)
}

fn ensure_usage_sync_state_columns(conn: &Connection) -> AppResult<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(openai_usage_sync_state)")
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_SCHEMA_PREPARE_FAILED",
                format!("Failed to inspect usage sync state schema: {error}"),
            )
        })?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_SCHEMA_QUERY_FAILED",
                format!("Failed to query usage sync state schema: {error}"),
            )
        })?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_SCHEMA_ROW_FAILED",
                format!("Failed to read usage sync state schema: {error}"),
            )
        })?;

    for (name, definition) in [
        ("attribution_since_ms", "attribution_since_ms INTEGER"),
        (
            "duplicate_events",
            "duplicate_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "non_official_base_url",
            "non_official_base_url INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "unproven_history",
            "unproven_history INTEGER NOT NULL DEFAULT 0",
        ),
        ("unattributed", "unattributed INTEGER NOT NULL DEFAULT 0"),
        ("missing_model", "missing_model INTEGER NOT NULL DEFAULT 0"),
        ("invalid_delta", "invalid_delta INTEGER NOT NULL DEFAULT 0"),
        (
            "malformed_lines",
            "malformed_lines INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE openai_usage_sync_state ADD COLUMN {definition}"),
                [],
            )
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_SYNC_STATE_SCHEMA_ALTER_FAILED",
                    format!("Failed to add usage sync state column {name}: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

fn apply_cached_usage_sync_state(
    conn: &Connection,
    source_signature: &str,
    summary: &mut OpenAiUsageSummary,
) -> AppResult<bool> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT source_signature, pricing_checked_at, attribution_since_ms,
                   duplicate_events, non_official_base_url, unproven_history,
                   unattributed, missing_model, invalid_delta, malformed_lines
            FROM openai_usage_sync_state
            WHERE id = 1
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_PREPARE_FAILED",
                format!("Failed to prepare usage sync state query: {error}"),
            )
        })?;
    let result = stmt.query_row([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            OpenAiUsageExclusions {
                duplicate_events: u64_from_i64(row.get::<_, i64>(3)?),
                non_official_base_url: u64_from_i64(row.get::<_, i64>(4)?),
                unproven_history: u64_from_i64(row.get::<_, i64>(5)?),
                unattributed: u64_from_i64(row.get::<_, i64>(6)?),
                missing_model: u64_from_i64(row.get::<_, i64>(7)?),
                invalid_delta: u64_from_i64(row.get::<_, i64>(8)?),
                malformed_lines: u64_from_i64(row.get::<_, i64>(9)?),
            },
        ))
    });
    match result {
        Ok((stored_signature, stored_pricing_checked_at, attribution_since_ms, exclusions)) => {
            let current = stored_signature == source_signature
                && stored_pricing_checked_at == PRICING_CHECKED_AT;
            if current {
                summary.attribution_since_ms =
                    attribution_since_ms.and_then(|value| u64::try_from(value).ok());
                summary.exclusions = exclusions;
            }
            Ok(current)
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(AppError::new(
            "OPENAI_USAGE_SYNC_STATE_QUERY_FAILED",
            format!("Failed to read usage sync state: {error}"),
        )),
    }
}

fn usage_source_signature(codex_home: &Path) -> String {
    let mut entries = Vec::new();
    collect_usage_source_entries(codex_home, &mut entries);
    entries.sort();
    let mut hasher = DefaultHasher::new();
    PRICING_CHECKED_AT.hash(&mut hasher);
    for entry in entries {
        entry.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn collect_usage_source_entries(codex_home: &Path, entries: &mut Vec<String>) {
    for sessions_root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        let mut files = Vec::new();
        collect_jsonl_files(&sessions_root, &mut files);
        for file in files {
            push_file_signature(codex_home, &file, entries);
        }
    }

    for file in [
        get_openai_usage_attribution_path(Some(codex_home)),
        get_current_profile_file(Some(codex_home)),
        get_backup_root(Some(codex_home)).join(".current_profile"),
    ] {
        push_file_signature(codex_home, &file, entries);
    }

    let backup_root = get_backup_root(Some(codex_home));
    for profile_dir in super::paths::list_profile_dirs(&backup_root) {
        push_file_signature(codex_home, &profile_dir.join("profile.json"), entries);
        push_file_signature(
            codex_home,
            &profile_dir.join(super::paths::ACTIVE_MARKER_FILE),
            entries,
        );
    }
}

fn push_file_signature(codex_home: &Path, file: &Path, entries: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(file) else {
        return;
    };
    let relative = file.strip_prefix(codex_home).unwrap_or(file);
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or_default();
    entries.push(format!(
        "{}:{}:{}",
        relative.display(),
        metadata.len(),
        modified_ms
    ));
}

fn rebuild_usage_events(
    conn: &mut Connection,
    events: &[UsageEvent],
    source_signature: &str,
    summary: &OpenAiUsageSummary,
) -> AppResult<()> {
    let tx = conn.transaction().map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_LEDGER_TRANSACTION_FAILED",
            format!("Failed to start usage ledger transaction: {error}"),
        )
    })?;
    tx.execute("DELETE FROM openai_usage_events", [])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_LEDGER_CLEAR_FAILED",
                format!("Failed to clear usage ledger events: {error}"),
            )
        })?;
    {
        let mut stmt = tx
            .prepare(
                r#"
            INSERT INTO openai_usage_events (
                request_id, fingerprint, session_id, profile, account_label,
                official_source, model, input_tokens, cached_input_tokens,
                output_tokens, reasoning_output_tokens, real_total_tokens,
                input_cost_micros, cached_input_cost_micros, output_cost_micros,
                total_cost_micros, unpriced_tokens, priced, pricing_note,
                pricing_source, pricing_checked_at, status_code, created_at_ms,
                data_source, inserted_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
            )
            "#,
            )
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_LEDGER_PREPARE_FAILED",
                    format!("Failed to prepare usage ledger insert: {error}"),
                )
            })?;
        let inserted_at_ms = i64_from_u64(current_time_ms());
        for event in events {
            stmt.execute(params![
                &event.request_id,
                &event.fingerprint,
                &event.session_id,
                &event.profile,
                event.account_label.as_deref(),
                &event.official_source,
                &event.model,
                i64_from_u64(event.delta.input_tokens),
                i64_from_u64(event.delta.cached_input_tokens),
                i64_from_u64(event.delta.output_tokens),
                i64_from_u64(event.delta.reasoning_output_tokens),
                i64_from_u64(
                    event
                        .delta
                        .input_tokens
                        .saturating_add(event.delta.output_tokens)
                ),
                i64_from_u64(event.cost.breakdown.input_cost_micros),
                i64_from_u64(event.cost.breakdown.cached_input_cost_micros),
                i64_from_u64(event.cost.breakdown.output_cost_micros),
                i64_from_u64(event.cost.breakdown.total_cost_micros),
                i64_from_u64(event.cost.breakdown.unpriced_tokens),
                if event.cost.priced { 1_i64 } else { 0_i64 },
                event.cost.pricing_note.as_deref(),
                PRICING_SOURCE,
                PRICING_CHECKED_AT,
                i64::from(event.status_code),
                i64_from_u64(event.created_at_ms),
                &event.data_source,
                inserted_at_ms,
            ])
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_LEDGER_INSERT_FAILED",
                    format!(
                        "Failed to insert usage ledger event {}: {error}",
                        event.request_id
                    ),
                )
            })?;
        }
    }
    tx.execute(
        r#"
        INSERT INTO openai_usage_sync_state (
            id, source_signature, pricing_checked_at, synced_at_ms,
            attribution_since_ms, duplicate_events, non_official_base_url,
            unproven_history, unattributed, missing_model, invalid_delta,
            malformed_lines
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(id) DO UPDATE SET
            source_signature = excluded.source_signature,
            pricing_checked_at = excluded.pricing_checked_at,
            synced_at_ms = excluded.synced_at_ms,
            attribution_since_ms = excluded.attribution_since_ms,
            duplicate_events = excluded.duplicate_events,
            non_official_base_url = excluded.non_official_base_url,
            unproven_history = excluded.unproven_history,
            unattributed = excluded.unattributed,
            missing_model = excluded.missing_model,
            invalid_delta = excluded.invalid_delta,
            malformed_lines = excluded.malformed_lines
        "#,
        params![
            source_signature,
            PRICING_CHECKED_AT,
            i64_from_u64(current_time_ms()),
            summary.attribution_since_ms.map(i64_from_u64),
            i64_from_u64(summary.exclusions.duplicate_events),
            i64_from_u64(summary.exclusions.non_official_base_url),
            i64_from_u64(summary.exclusions.unproven_history),
            i64_from_u64(summary.exclusions.unattributed),
            i64_from_u64(summary.exclusions.missing_model),
            i64_from_u64(summary.exclusions.invalid_delta),
            i64_from_u64(summary.exclusions.malformed_lines)
        ],
    )
    .map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_SYNC_STATE_UPDATE_FAILED",
            format!("Failed to update usage sync state: {error}"),
        )
    })?;
    tx.commit().map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_LEDGER_COMMIT_FAILED",
            format!("Failed to commit usage ledger rebuild: {error}"),
        )
    })?;
    Ok(())
}

fn load_ledger_events(conn: &Connection, query: &OpenAiUsageQuery) -> AppResult<Vec<UsageEvent>> {
    let start_ms = query.start_date.and_then(seconds_to_ms).map(i64_from_u64);
    let end_ms = query.end_date.and_then(end_seconds_to_ms).map(i64_from_u64);
    let profile = query
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account_label = query
        .account_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model = query
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_model);
    let status_code = query.status_code.map(i64::from);

    let mut stmt = conn
        .prepare(
            r#"
            SELECT request_id, fingerprint, session_id, profile, account_label,
                   official_source, model, input_tokens, cached_input_tokens,
                   output_tokens, reasoning_output_tokens, input_cost_micros,
                   cached_input_cost_micros, output_cost_micros, total_cost_micros,
                   unpriced_tokens, priced, pricing_note, status_code, created_at_ms,
                   data_source
            FROM openai_usage_events
            WHERE (?1 IS NULL OR created_at_ms >= ?1)
              AND (?2 IS NULL OR created_at_ms <= ?2)
              AND (?3 IS NULL OR profile = ?3)
              AND (?4 IS NULL OR account_label = ?4)
              AND (?5 IS NULL OR model = ?5)
              AND (?6 IS NULL OR status_code = ?6)
            ORDER BY created_at_ms ASC, request_id ASC
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_LEDGER_QUERY_PREPARE_FAILED",
                format!("Failed to prepare usage ledger query: {error}"),
            )
        })?;

    let rows = stmt
        .query_map(
            params![start_ms, end_ms, profile, account_label, model, status_code],
            |row| {
                let input_tokens = u64_from_i64(row.get::<_, i64>(7)?);
                let cached_input_tokens = u64_from_i64(row.get::<_, i64>(8)?);
                let output_tokens = u64_from_i64(row.get::<_, i64>(9)?);
                let reasoning_output_tokens = u64_from_i64(row.get::<_, i64>(10)?);
                Ok(UsageEvent {
                    request_id: row.get(0)?,
                    fingerprint: row.get(1)?,
                    session_id: row.get(2)?,
                    profile: row.get(3)?,
                    account_label: row.get(4)?,
                    official_source: row.get(5)?,
                    model: row.get(6)?,
                    delta: UsageCounts {
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens,
                    },
                    cost: CalculatedCost {
                        breakdown: OpenAiUsageCostBreakdown {
                            input_cost_micros: u64_from_i64(row.get::<_, i64>(11)?),
                            cached_input_cost_micros: u64_from_i64(row.get::<_, i64>(12)?),
                            output_cost_micros: u64_from_i64(row.get::<_, i64>(13)?),
                            total_cost_micros: u64_from_i64(row.get::<_, i64>(14)?),
                            unpriced_tokens: u64_from_i64(row.get::<_, i64>(15)?),
                        },
                        priced: row.get::<_, i64>(16)? != 0,
                        pricing_note: row.get(17)?,
                    },
                    status_code: u16::try_from(row.get::<_, i64>(18)?).unwrap_or(0),
                    created_at_ms: u64_from_i64(row.get::<_, i64>(19)?),
                    data_source: row.get(20)?,
                })
            },
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_LEDGER_QUERY_FAILED",
                format!("Failed to query usage ledger: {error}"),
            )
        })?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_LEDGER_ROW_FAILED",
                format!("Failed to read usage ledger row: {error}"),
            )
        })?);
    }
    Ok(events)
}

fn i64_from_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn u64_from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn event_matches_query(event: &UsageEvent, query: &OpenAiUsageQuery) -> bool {
    if let Some(start) = query.start_date.and_then(seconds_to_ms) {
        if event.created_at_ms < start {
            return false;
        }
    }
    if let Some(end) = query.end_date.and_then(end_seconds_to_ms) {
        if event.created_at_ms > end {
            return false;
        }
    }
    if let Some(profile) = query
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if event.profile != profile {
            return false;
        }
    }
    if let Some(account_label) = query
        .account_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if event.account_label.as_deref() != Some(account_label) {
            return false;
        }
    }
    if let Some(model) = query
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if event.model != normalize_model(model) {
            return false;
        }
    }
    if let Some(status_code) = query.status_code {
        if event.status_code != status_code {
            return false;
        }
    }
    true
}

fn build_trends(events: &[UsageEvent], query: &OpenAiUsageQuery) -> Vec<OpenAiUsageTrendPoint> {
    let now_sec = i64::try_from(current_time_ms() / 1000).unwrap_or_default();
    let end_sec = query.end_date.unwrap_or(now_sec);
    let start_sec = query
        .start_date
        .unwrap_or(end_sec.saturating_sub(24 * 60 * 60));
    let Some(start_ms) = seconds_to_ms(start_sec) else {
        return Vec::new();
    };
    let Some(end_ms) = end_seconds_to_ms(end_sec) else {
        return Vec::new();
    };
    if start_ms > end_ms {
        return Vec::new();
    }
    let hourly = end_sec.saturating_sub(start_sec) <= 24 * 60 * 60;
    let mut buckets = BTreeMap::<u64, Aggregate>::new();
    for bucket_start in trend_bucket_starts(start_ms, end_ms, hourly) {
        buckets.entry(bucket_start).or_default();
    }
    for event in events
        .iter()
        .filter(|event| event_matches_query(event, query))
    {
        let bucket = if hourly {
            local_hour_bucket_ms(event.created_at_ms)
        } else {
            local_day_bucket_ms(event.created_at_ms)
        };
        add_to_aggregate(
            buckets.entry(bucket).or_default(),
            event.delta,
            &event.cost.breakdown,
        );
    }
    buckets
        .into_iter()
        .map(|(bucket_start_ms, aggregate)| OpenAiUsageTrendPoint {
            date: format_bucket_date(bucket_start_ms, hourly),
            bucket_start_ms,
            request_count: aggregate.requests,
            tokens: aggregate.tokens,
            cost: aggregate.cost,
        })
        .collect()
}

fn trend_bucket_starts(start_ms: u64, end_ms: u64, hourly: bool) -> Vec<u64> {
    let mut buckets = Vec::new();
    let mut current = if hourly {
        local_hour_bucket_ms(start_ms)
    } else {
        local_day_bucket_ms(start_ms)
    };
    while current <= end_ms {
        buckets.push(current);
        let Some(next) = next_bucket_ms(current, hourly) else {
            break;
        };
        if next <= current {
            break;
        }
        current = next;
    }
    buckets
}

fn next_bucket_ms(bucket_ms: u64, hourly: bool) -> Option<u64> {
    let timestamp_ms = i64::try_from(bucket_ms).ok()?;
    let current = Local.timestamp_millis_opt(timestamp_ms).single()?;
    let next = if hourly {
        current + chrono::Duration::hours(1)
    } else {
        current + chrono::Duration::days(1)
    };
    u64::try_from(next.timestamp_millis()).ok()
}

fn local_hour_bucket_ms(timestamp_ms: u64) -> u64 {
    let timestamp_ms = i64::try_from(timestamp_ms).unwrap_or_default();
    let Some(local) = Local.timestamp_millis_opt(timestamp_ms).single() else {
        return 0;
    };
    local
        .with_minute(0)
        .and_then(|value| value.with_second(0))
        .and_then(|value| value.with_nanosecond(0))
        .and_then(|value| u64::try_from(value.timestamp_millis()).ok())
        .unwrap_or_default()
}

fn local_day_bucket_ms(timestamp_ms: u64) -> u64 {
    let timestamp_ms = i64::try_from(timestamp_ms).unwrap_or_default();
    let Some(local) = Local.timestamp_millis_opt(timestamp_ms).single() else {
        return 0;
    };
    let date = local.date_naive();
    let Some(naive) = date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(value) => u64::try_from(value.timestamp_millis()).unwrap_or(0),
        chrono::LocalResult::Ambiguous(first, _) => {
            u64::try_from(first.timestamp_millis()).unwrap_or(0)
        }
        chrono::LocalResult::None => 0,
    }
}

fn format_bucket_date(bucket_ms: u64, hourly: bool) -> String {
    let timestamp_ms = i64::try_from(bucket_ms).unwrap_or_default();
    let Some(local) = Local.timestamp_millis_opt(timestamp_ms).single() else {
        return String::new();
    };
    if hourly {
        format!(
            "{:04}-{:02}-{:02}T{:02}:00:00{}",
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.format("%:z")
        )
    } else {
        format!(
            "{:04}-{:02}-{:02}",
            local.year(),
            local.month(),
            local.day()
        )
    }
}

fn event_to_log(event: &UsageEvent) -> OpenAiUsageRequestLog {
    OpenAiUsageRequestLog {
        request_id: event.request_id.clone(),
        session_id: event.session_id.clone(),
        profile: event.profile.clone(),
        account_label: event.account_label.clone(),
        official_source: event.official_source.clone(),
        model: event.model.clone(),
        tokens: tokens_from_delta(event.delta),
        cost: event.cost.breakdown.clone(),
        priced: event.cost.priced,
        pricing_note: event.cost.pricing_note.clone(),
        status_code: event.status_code,
        created_at_ms: event.created_at_ms,
        data_source: event.data_source.clone(),
    }
}

fn tokens_from_delta(delta: UsageCounts) -> OpenAiUsageTokenTotals {
    OpenAiUsageTokenTotals {
        input_tokens: delta.input_tokens,
        cached_input_tokens: delta.cached_input_tokens,
        output_tokens: delta.output_tokens,
        reasoning_output_tokens: delta.reasoning_output_tokens,
        real_total_tokens: delta.input_tokens.saturating_add(delta.output_tokens),
    }
}

fn seconds_to_ms(value: i64) -> Option<u64> {
    u64::try_from(value).ok()?.checked_mul(1000)
}

fn end_seconds_to_ms(value: i64) -> Option<u64> {
    seconds_to_ms(value).map(|millis| millis.saturating_add(999))
}

pub fn record_current_openai_usage_attribution(codex_home: Option<&Path>) -> AppResult<()> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let record = build_current_attribution_record(&codex_home)?;
    append_attribution_record(&codex_home, &record)
}

fn build_current_attribution_record(codex_home: &Path) -> AppResult<AttributionRecord> {
    let index = load_profiles_index(Some(codex_home))?;
    let observed_at_ms = current_time_ms();
    let Some(profile_name) = index.current_profile.clone() else {
        return Ok(AttributionRecord {
            observed_at_ms,
            profile: None,
            account_label: None,
            openai_base_url: None,
            official_source: None,
            is_official_openai: false,
            exclusion_reason: Some("no active profile".to_string()),
        });
    };
    let Some(profile) = index
        .profiles
        .iter()
        .find(|entry| entry.folder_name == profile_name)
    else {
        return Ok(AttributionRecord {
            observed_at_ms,
            profile: Some(profile_name),
            account_label: None,
            openai_base_url: None,
            official_source: None,
            is_official_openai: false,
            exclusion_reason: Some("active profile not found".to_string()),
        });
    };
    let official_source = match official_source_for_profile(profile) {
        OfficialSource::Official(value) => Some(value),
        OfficialSource::NonOfficial => None,
    };
    let backup_root = get_backup_root(Some(codex_home));
    let profile_dir = backup_root.join(&profile_name);
    let root_identity = load_account_identity_from_path(&codex_home.join("auth.json"));
    let profile_identity = load_account_identity_from_path(&profile_dir.join("auth.json"));
    let identity_matches = matches!(
        (root_identity, profile_identity),
        (Some(root), Some(slot)) if root.same_account(&slot)
    );
    let is_official_openai = official_source.is_some() && identity_matches;
    let exclusion_reason = if is_official_openai {
        None
    } else if official_source.is_none() {
        Some("non-official base_url".to_string())
    } else {
        Some("account identity does not match active profile".to_string())
    };
    Ok(AttributionRecord {
        observed_at_ms,
        profile: Some(profile_name),
        account_label: profile.account_label.clone(),
        openai_base_url: profile.openai_base_url.clone(),
        official_source,
        is_official_openai,
        exclusion_reason,
    })
}

fn append_attribution_record(codex_home: &Path, record: &AttributionRecord) -> AppResult<()> {
    let path = get_openai_usage_attribution_path(Some(codex_home));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_ATTRIBUTION_DIR_FAILED",
                format!(
                    "Failed to create usage attribution dir {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    let serialized = serde_json::to_string(record).map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_ATTRIBUTION_SERIALIZE_FAILED",
            format!("Failed to serialize usage attribution record: {error}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_ATTRIBUTION_OPEN_FAILED",
                format!(
                    "Failed to open usage attribution log {}: {error}",
                    path.display()
                ),
            )
        })?;
    writeln!(file, "{serialized}").map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_ATTRIBUTION_WRITE_FAILED",
            format!(
                "Failed to write usage attribution log {}: {error}",
                path.display()
            ),
        )
    })
}

fn load_attribution_records(codex_home: &Path) -> Vec<AttributionRecord> {
    let path = get_openai_usage_attribution_path(Some(codex_home));
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut records = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<AttributionRecord>(line).ok())
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.observed_at_ms);
    records
}

fn attribution_for_event<'a>(
    event_time_ms: Option<u64>,
    fallback: Option<&'a AttributionContext>,
    records: &'a [AttributionRecord],
    exclusions: &mut OpenAiUsageExclusions,
) -> Option<AttributionContext> {
    let Some(event_time_ms) = event_time_ms else {
        exclusions.unproven_history = exclusions.unproven_history.saturating_add(1);
        return None;
    };
    if let Some(record) = records
        .iter()
        .rev()
        .find(|record| record.observed_at_ms <= event_time_ms)
    {
        if !record.is_official_openai {
            exclusions.non_official_base_url = exclusions.non_official_base_url.saturating_add(1);
            return None;
        }
        return record.context();
    }
    if let Some(fallback) = fallback {
        if event_time_ms >= fallback.attribution_since_ms {
            return Some(AttributionContext {
                profile: fallback.profile.clone(),
                account_label: fallback.account_label.clone(),
                official_source: fallback.official_source.clone(),
                attribution_since_ms: fallback.attribution_since_ms,
            });
        }
    }
    exclusions.unproven_history = exclusions.unproven_history.saturating_add(1);
    None
}

fn official_pricing_note(pricing: ModelPricing) -> String {
    format!(
        "{} ({}, checked {})",
        PRICING_SOURCE, pricing.source_url, pricing.checked_at
    )
}

fn official_data_source() -> &'static str {
    "codex_session"
}

fn fingerprint_for_event(
    attribution: &AttributionContext,
    model: &str,
    delta: UsageCounts,
    event_time_ms: Option<u64>,
) -> String {
    let bucket = event_time_ms.unwrap_or_default() / 60_000;
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        attribution.profile,
        attribution.official_source,
        model,
        delta.input_tokens,
        delta.cached_input_tokens,
        delta.output_tokens,
        delta.reasoning_output_tokens,
        bucket,
        official_data_source()
    )
}

fn build_attribution_context(
    codex_home: &Path,
    exclusions: &mut OpenAiUsageExclusions,
) -> AppResult<Option<AttributionContext>> {
    let index = load_profiles_index(Some(codex_home))?;
    let Some(profile_name) = index.current_profile.clone() else {
        return Ok(None);
    };
    let Some(profile) = index
        .profiles
        .iter()
        .find(|entry| entry.folder_name == profile_name)
    else {
        return Ok(None);
    };

    let official_source = match official_source_for_profile(profile) {
        OfficialSource::Official(value) => value,
        OfficialSource::NonOfficial => {
            exclusions.non_official_base_url = exclusions.non_official_base_url.saturating_add(1);
            return Ok(None);
        }
    };

    let backup_root = get_backup_root(Some(codex_home));
    let profile_dir = backup_root.join(&profile_name);
    let root_identity = load_account_identity_from_path(&codex_home.join("auth.json"));
    let profile_identity = load_account_identity_from_path(&profile_dir.join("auth.json"));
    if !matches!(
        (root_identity, profile_identity),
        (Some(root), Some(slot)) if root.same_account(&slot)
    ) {
        exclusions.unattributed = exclusions.unattributed.saturating_add(1);
        return Ok(None);
    }

    let active_marker = profile_dir.join(super::paths::ACTIVE_MARKER_FILE);
    let current_marker = get_current_profile_file(Some(codex_home));
    let since = [
        file_modified_ms(&active_marker),
        file_modified_ms(&current_marker),
    ]
    .into_iter()
    .flatten()
    .max();
    let Some(attribution_since_ms) = since else {
        exclusions.unproven_history = exclusions.unproven_history.saturating_add(1);
        return Ok(None);
    };

    Ok(Some(AttributionContext {
        profile: profile_name,
        account_label: profile.account_label.clone(),
        official_source,
        attribution_since_ms,
    }))
}

enum OfficialSource {
    Official(String),
    NonOfficial,
}

fn official_source_for_profile(profile: &ProfileIndexEntry) -> OfficialSource {
    let Some(base_url) = profile.openai_base_url.as_deref() else {
        return OfficialSource::Official("OpenAI default".to_string());
    };
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return OfficialSource::Official("OpenAI default".to_string());
    }
    if official_host(trimmed).as_deref() == Some("api.openai.com") {
        return OfficialSource::Official(trimmed.to_string());
    }
    OfficialSource::NonOfficial
}

fn official_host(base_url: &str) -> Option<String> {
    let lower = base_url.trim().to_ascii_lowercase();
    let without_scheme = lower.strip_prefix("https://")?;
    without_scheme
        .split(['/', ':', '?', '#'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn collect_session_file_events(
    file: &Path,
    fallback_attribution: Option<&AttributionContext>,
    attribution_records: &[AttributionRecord],
    request_ids: &mut HashSet<String>,
    fingerprints: &mut HashSet<String>,
    events: &mut Vec<UsageEvent>,
    summary: &mut OpenAiUsageSummary,
) {
    let Ok(raw) = fs::read_to_string(file) else {
        return;
    };
    let mut session_id = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown-session")
        .to_string();
    let mut current_model: Option<String> = None;
    let mut previous_usage: Option<UsageCounts> = None;
    let mut event_index = 0_u64;

    for line in raw.lines() {
        let parsed = match serde_json::from_str::<SessionLine>(line) {
            Ok(value) => value,
            Err(_) => {
                summary.exclusions.malformed_lines =
                    summary.exclusions.malformed_lines.saturating_add(1);
                continue;
            }
        };
        let payload = parsed.payload.as_ref();
        match parsed.line_type.as_str() {
            "session_meta" => {
                if let Some(id) = payload
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    session_id = id.to_string();
                }
            }
            "turn_context" => {
                current_model = payload
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
                    .map(normalize_model)
                    .filter(|value| !value.is_empty());
            }
            "event_msg" => {
                if payload
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    != Some("token_count")
                {
                    continue;
                }
                event_index = event_index.saturating_add(1);
                let event_time_ms = parsed.timestamp.as_deref().and_then(parse_timestamp_ms);
                let Some(current_usage) = payload.and_then(parse_usage_counts) else {
                    summary.exclusions.malformed_lines =
                        summary.exclusions.malformed_lines.saturating_add(1);
                    continue;
                };
                let Some(attribution) = attribution_for_event(
                    event_time_ms,
                    fallback_attribution,
                    attribution_records,
                    &mut summary.exclusions,
                ) else {
                    previous_usage = Some(current_usage);
                    continue;
                };
                let Some(model) = current_model.clone() else {
                    previous_usage = Some(current_usage);
                    summary.exclusions.missing_model =
                        summary.exclusions.missing_model.saturating_add(1);
                    continue;
                };
                let Some(delta) = usage_delta(previous_usage, current_usage) else {
                    previous_usage = Some(current_usage);
                    summary.exclusions.invalid_delta =
                        summary.exclusions.invalid_delta.saturating_add(1);
                    continue;
                };
                previous_usage = Some(current_usage);
                if delta.input_tokens == 0 && delta.output_tokens == 0 {
                    continue;
                }

                let request_id = format!("codex_session:{session_id}:{event_index}");
                if !request_ids.insert(request_id.clone()) {
                    summary.exclusions.duplicate_events =
                        summary.exclusions.duplicate_events.saturating_add(1);
                    continue;
                }
                let fingerprint = fingerprint_for_event(&attribution, &model, delta, event_time_ms);
                if !fingerprints.insert(fingerprint.clone()) {
                    summary.exclusions.duplicate_events =
                        summary.exclusions.duplicate_events.saturating_add(1);
                    continue;
                }
                let Some(created_at_ms) = event_time_ms else {
                    summary.exclusions.unproven_history =
                        summary.exclusions.unproven_history.saturating_add(1);
                    continue;
                };
                let cost = calculate_cost(&model, delta);
                events.push(UsageEvent {
                    request_id,
                    fingerprint,
                    session_id: session_id.clone(),
                    profile: attribution.profile,
                    account_label: attribution.account_label,
                    official_source: attribution.official_source,
                    model,
                    delta,
                    cost,
                    status_code: 200,
                    created_at_ms,
                    data_source: official_data_source().to_string(),
                });
            }
            _ => {}
        }
    }
}

fn parse_usage_counts(payload: &Value) -> Option<UsageCounts> {
    let info = payload.get("info")?;
    let usage = info
        .get("total_token_usage")
        .or_else(|| info.get("last_token_usage"))?;
    let input_tokens = usage.get("input_tokens")?.as_u64()?;
    let cached_input_tokens = usage
        .get("cached_input_tokens")
        .or_else(|| usage.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(input_tokens);
    Some(UsageCounts {
        input_tokens,
        cached_input_tokens,
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        reasoning_output_tokens: usage
            .get("reasoning_output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn usage_delta(previous: Option<UsageCounts>, current: UsageCounts) -> Option<UsageCounts> {
    let Some(previous) = previous else {
        return Some(current);
    };
    if current.input_tokens < previous.input_tokens
        || current.cached_input_tokens < previous.cached_input_tokens
        || current.output_tokens < previous.output_tokens
        || current.reasoning_output_tokens < previous.reasoning_output_tokens
    {
        return None;
    }
    Some(UsageCounts {
        input_tokens: current.input_tokens - previous.input_tokens,
        cached_input_tokens: current.cached_input_tokens - previous.cached_input_tokens,
        output_tokens: current.output_tokens - previous.output_tokens,
        reasoning_output_tokens: current.reasoning_output_tokens - previous.reasoning_output_tokens,
    })
}

fn add_event_usage(
    accounts: &mut BTreeMap<String, AccountAggregate>,
    global_models: &mut BTreeMap<String, ModelAggregate>,
    event: &UsageEvent,
) {
    let account = accounts
        .entry(event.profile.clone())
        .or_insert_with(|| AccountAggregate {
            account_label: event.account_label.clone(),
            official_source: event.official_source.clone(),
            ..AccountAggregate::default()
        });
    add_to_aggregate(&mut account.aggregate, event.delta, &event.cost.breakdown);
    add_to_model(
        account.models.entry(event.model.clone()).or_default(),
        event.delta,
        &event.cost,
    );
    add_to_model(
        global_models.entry(event.model.clone()).or_default(),
        event.delta,
        &event.cost,
    );
}

fn total_account_aggregate(accounts: &BTreeMap<String, AccountAggregate>) -> Aggregate {
    let mut total = Aggregate::default();
    for account in accounts.values() {
        total.requests = total.requests.saturating_add(account.aggregate.requests);
        total.tokens.input_tokens = total
            .tokens
            .input_tokens
            .saturating_add(account.aggregate.tokens.input_tokens);
        total.tokens.cached_input_tokens = total
            .tokens
            .cached_input_tokens
            .saturating_add(account.aggregate.tokens.cached_input_tokens);
        total.tokens.output_tokens = total
            .tokens
            .output_tokens
            .saturating_add(account.aggregate.tokens.output_tokens);
        total.tokens.reasoning_output_tokens = total
            .tokens
            .reasoning_output_tokens
            .saturating_add(account.aggregate.tokens.reasoning_output_tokens);
        total.tokens.real_total_tokens = total
            .tokens
            .real_total_tokens
            .saturating_add(account.aggregate.tokens.real_total_tokens);
        total.cost.input_cost_micros = total
            .cost
            .input_cost_micros
            .saturating_add(account.aggregate.cost.input_cost_micros);
        total.cost.cached_input_cost_micros = total
            .cost
            .cached_input_cost_micros
            .saturating_add(account.aggregate.cost.cached_input_cost_micros);
        total.cost.output_cost_micros = total
            .cost
            .output_cost_micros
            .saturating_add(account.aggregate.cost.output_cost_micros);
        total.cost.total_cost_micros = total
            .cost
            .total_cost_micros
            .saturating_add(account.aggregate.cost.total_cost_micros);
        total.cost.unpriced_tokens = total
            .cost
            .unpriced_tokens
            .saturating_add(account.aggregate.cost.unpriced_tokens);
    }
    total
}

fn add_to_model(model: &mut ModelAggregate, delta: UsageCounts, cost: &CalculatedCost) {
    add_to_aggregate(&mut model.aggregate, delta, &cost.breakdown);
    model.priced |= cost.priced;
    if let Some(note) = &cost.pricing_note {
        model.pricing_note = Some(note.clone());
    }
}

fn add_to_aggregate(
    aggregate: &mut Aggregate,
    delta: UsageCounts,
    cost: &OpenAiUsageCostBreakdown,
) {
    aggregate.requests = aggregate.requests.saturating_add(1);
    aggregate.tokens.input_tokens = aggregate
        .tokens
        .input_tokens
        .saturating_add(delta.input_tokens);
    aggregate.tokens.cached_input_tokens = aggregate
        .tokens
        .cached_input_tokens
        .saturating_add(delta.cached_input_tokens);
    aggregate.tokens.output_tokens = aggregate
        .tokens
        .output_tokens
        .saturating_add(delta.output_tokens);
    aggregate.tokens.reasoning_output_tokens = aggregate
        .tokens
        .reasoning_output_tokens
        .saturating_add(delta.reasoning_output_tokens);
    aggregate.tokens.real_total_tokens = aggregate
        .tokens
        .real_total_tokens
        .saturating_add(delta.input_tokens.saturating_add(delta.output_tokens));
    aggregate.cost.input_cost_micros = aggregate
        .cost
        .input_cost_micros
        .saturating_add(cost.input_cost_micros);
    aggregate.cost.cached_input_cost_micros = aggregate
        .cost
        .cached_input_cost_micros
        .saturating_add(cost.cached_input_cost_micros);
    aggregate.cost.output_cost_micros = aggregate
        .cost
        .output_cost_micros
        .saturating_add(cost.output_cost_micros);
    aggregate.cost.total_cost_micros = aggregate
        .cost
        .total_cost_micros
        .saturating_add(cost.total_cost_micros);
    aggregate.cost.unpriced_tokens = aggregate
        .cost
        .unpriced_tokens
        .saturating_add(cost.unpriced_tokens);
}

#[derive(Clone)]
struct CalculatedCost {
    breakdown: OpenAiUsageCostBreakdown,
    priced: bool,
    pricing_note: Option<String>,
}

fn calculate_cost(model: &str, delta: UsageCounts) -> CalculatedCost {
    let fresh_input = delta.input_tokens.saturating_sub(delta.cached_input_tokens);
    let Some(pricing) = pricing_for_model(model) else {
        return CalculatedCost {
            breakdown: OpenAiUsageCostBreakdown {
                unpriced_tokens: delta.input_tokens.saturating_add(delta.output_tokens),
                ..OpenAiUsageCostBreakdown::default()
            },
            priced: false,
            pricing_note: Some("No OpenAI price configured for this model".to_string()),
        };
    };
    let input_cost = cost_micros(fresh_input, pricing.input_micros_per_million);
    let cached_input_cost = cost_micros(
        delta.cached_input_tokens,
        pricing.cached_input_micros_per_million,
    );
    let output_cost = cost_micros(delta.output_tokens, pricing.output_micros_per_million);
    CalculatedCost {
        breakdown: OpenAiUsageCostBreakdown {
            input_cost_micros: input_cost,
            cached_input_cost_micros: cached_input_cost,
            output_cost_micros: output_cost,
            total_cost_micros: input_cost
                .saturating_add(cached_input_cost)
                .saturating_add(output_cost),
            unpriced_tokens: 0,
        },
        priced: true,
        pricing_note: Some(official_pricing_note(pricing)),
    }
}

fn cost_micros(tokens: u64, micros_per_million: u64) -> u64 {
    let raw = u128::from(tokens) * u128::from(micros_per_million);
    let rounded = (raw + 500_000) / 1_000_000;
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    let normalized = normalize_model(model);
    MODEL_PRICES
        .iter()
        .copied()
        .find(|entry| entry.model == normalized)
}

fn normalize_model(model: &str) -> String {
    let mut value = model
        .trim()
        .to_ascii_lowercase()
        .replace('@', "-")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    value = strip_date_suffix(&value);
    value
}

fn strip_date_suffix(value: &str) -> String {
    if value.len() > 11 {
        let suffix = &value[value.len() - 11..];
        if suffix.as_bytes()[0] == b'-'
            && suffix[1..].chars().enumerate().all(|(idx, ch)| {
                matches!(idx, 4 | 7) && ch == '-' || !matches!(idx, 4 | 7) && ch.is_ascii_digit()
            })
        {
            return value[..value.len() - 11].to_string();
        }
    }
    if value.len() > 9 {
        let suffix = &value[value.len() - 9..];
        if suffix.as_bytes()[0] == b'-' && suffix[1..].chars().all(|ch| ch.is_ascii_digit()) {
            return value[..value.len() - 9].to_string();
        }
    }
    value.to_string()
}

fn account_to_summary(profile: String, account: AccountAggregate) -> OpenAiUsageAccountSummary {
    OpenAiUsageAccountSummary {
        profile,
        account_label: account.account_label,
        official_source: account.official_source,
        request_count: account.aggregate.requests,
        tokens: account.aggregate.tokens,
        cost: account.aggregate.cost,
        models: models_to_summary(account.models),
    }
}

fn models_to_summary(models: BTreeMap<String, ModelAggregate>) -> Vec<OpenAiUsageModelSummary> {
    models
        .into_iter()
        .map(|(model, aggregate)| OpenAiUsageModelSummary {
            model,
            request_count: aggregate.aggregate.requests,
            tokens: aggregate.aggregate.tokens,
            cost: aggregate.aggregate.cost,
            priced: aggregate.priced,
            pricing_note: aggregate.pricing_note,
        })
        .collect()
}

fn cache_hit_rate(tokens: &OpenAiUsageTokenTotals) -> Option<f64> {
    (tokens.input_tokens > 0)
        .then(|| tokens.cached_input_tokens as f64 / tokens.input_tokens as f64)
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|datetime| u64::try_from(datetime.timestamp_millis()).ok())
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-openai-usage-{name}-{unique}"))
    }

    fn setup_profile(codex_home: &Path, base_url: Option<&str>) {
        let profile_dir = codex_home.join("account_backup").join("a");
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(
            codex_home.join("auth.json"),
            r#"{"tokens":{"account_id":"acct_A"}}"#,
        )
        .unwrap();
        fs::write(
            profile_dir.join("auth.json"),
            r#"{"tokens":{"account_id":"acct_A"}}"#,
        )
        .unwrap();
        let base_url_field = base_url
            .map(|value| format!(r#""openai_base_url":"{value}","#))
            .unwrap_or_default();
        fs::write(
            profile_dir.join("profile.json"),
            format!(r#"{{"account_label":"acct_A",{base_url_field}"plan_name":"Pro"}}"#),
        )
        .unwrap();
        fs::write(
            codex_home.join("account_backup").join(".current_profile"),
            "a\n",
        )
        .unwrap();
        fs::write(
            profile_dir.join(super::super::paths::ACTIVE_MARKER_FILE),
            "",
        )
        .unwrap();
    }

    fn write_session(codex_home: &Path, body: &str) {
        let session_dir = codex_home
            .join("sessions")
            .join("2999")
            .join("01")
            .join("01");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("rollout-test.jsonl"), body).unwrap();
    }

    fn ts(value: &str) -> u64 {
        parse_timestamp_ms(value).unwrap()
    }

    #[test]
    fn calculates_cached_input_and_output_cost_without_reasoning_double_count() {
        let delta = UsageCounts {
            input_tokens: 1_000_000,
            cached_input_tokens: 200_000,
            output_tokens: 100_000,
            reasoning_output_tokens: 50_000,
        };
        let cost = calculate_cost("gpt-5.3-codex", delta);
        assert!(cost.priced);
        assert_eq!(cost.breakdown.input_cost_micros, 1_400_000);
        assert_eq!(cost.breakdown.cached_input_cost_micros, 35_000);
        assert_eq!(cost.breakdown.output_cost_micros, 1_400_000);
        assert_eq!(cost.breakdown.total_cost_micros, 2_835_000);
    }

    #[test]
    fn unknown_model_counts_tokens_as_unpriced() {
        let cost = calculate_cost(
            "unknown-model",
            UsageCounts {
                input_tokens: 10,
                cached_input_tokens: 2,
                output_tokens: 5,
                reasoning_output_tokens: 0,
            },
        );
        assert!(!cost.priced);
        assert_eq!(cost.breakdown.total_cost_micros, 0);
        assert_eq!(cost.breakdown.unpriced_tokens, 15);
    }

    #[test]
    fn official_default_profile_counts_only_post_attribution_deltas() {
        let codex_home = temp_codex_home("official");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            r#"{"timestamp":"2000-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s1"}}"#
                .to_string()
                .as_str(),
        );
        let session_path = codex_home
            .join("sessions")
            .join("2999")
            .join("01")
            .join("01")
            .join("rollout-test.jsonl");
        fs::write(
            session_path,
            [
                r#"{"timestamp":"2000-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s1"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"reasoning_output_tokens":50}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":500,"output_tokens":200,"reasoning_output_tokens":75}}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.tokens.input_tokens, 1500);
        assert_eq!(summary.tokens.cached_input_tokens, 500);
        assert_eq!(summary.tokens.output_tokens, 200);
        assert!(summary.cost.total_cost_micros > 0);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn custom_non_official_base_url_is_excluded() {
        let codex_home = temp_codex_home("custom-base");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, Some("https://example.com/v1"));
        write_session(
            &codex_home,
            r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.exclusions.non_official_base_url, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn http_api_openai_base_url_is_excluded() {
        let codex_home = temp_codex_home("http-openai-base");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, Some("http://api.openai.com/v1"));
        write_session(
            &codex_home,
            r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.exclusions.non_official_base_url, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn attribution_ledger_splits_usage_across_profiles() {
        let codex_home = temp_codex_home("ledger-split");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("a".to_string()),
                account_label: Some("acct_A".to_string()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("acct_B".to_string()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-ledger"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.accounts.len(), 2);
        assert_eq!(summary.accounts[0].profile, "a");
        assert_eq!(summary.accounts[0].tokens.input_tokens, 1000);
        assert_eq!(summary.accounts[1].profile, "b");
        assert_eq!(summary.accounts[1].tokens.input_tokens, 500);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_facets_keep_time_scope_without_self_filter_collapse() {
        let codex_home = temp_codex_home("dashboard-facets");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("a".to_string()),
                account_label: Some("acct_A".to_string()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("acct_B".to_string()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-facets"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"account-a-only-model"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.500Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:05.000Z","type":"turn_context","payload":{"model":"unknown-model"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:06.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1900,"cached_input_tokens":260,"output_tokens":250,"reasoning_output_tokens":50}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2500,"cached_input_tokens":300,"output_tokens":300,"reasoning_output_tokens":60}}}}"#,
            ]
            .join("\n"),
        );

        let account_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:07.000Z") / 1000) as i64),
            profile: Some("b".to_string()),
            ..OpenAiUsageQuery::default()
        };
        let account_dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(account_query), 1, 20).unwrap();

        assert_eq!(account_dashboard.summary.accounts.len(), 1);
        assert_eq!(account_dashboard.summary.accounts[0].profile, "b");
        assert_eq!(
            account_dashboard
                .account_summary
                .iter()
                .map(|account| account.profile.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(account_dashboard.account_summary[1].request_count, 2);

        let model_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:07.000Z") / 1000) as i64),
            profile: Some("b".to_string()),
            model: Some("unknown-model".to_string()),
            ..OpenAiUsageQuery::default()
        };
        let model_dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(model_query), 1, 20).unwrap();

        assert_eq!(model_dashboard.summary.models.len(), 1);
        assert_eq!(model_dashboard.summary.models[0].model, "unknown-model");
        assert_eq!(
            model_dashboard
                .account_summary
                .iter()
                .map(|account| account.profile.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
        assert_eq!(
            model_dashboard
                .model_options
                .iter()
                .map(|model| model.model.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.3-codex", "unknown-model"]
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn sqlite_ledger_sync_is_idempotent() {
        let codex_home = temp_codex_home("ledger-idempotent");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-idempotent"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":15,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let first = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        let second = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        let logs = load_openai_usage_request_logs(Some(&codex_home), None, 1, 20).unwrap();
        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(first.total_requests, 2);
        assert_eq!(second.total_requests, 2);
        assert_eq!(logs.total, 2);
        assert_eq!(dashboard.summary.total_requests, 2);
        assert_eq!(dashboard.logs.total, 2);
        assert_eq!(dashboard.data_sources[0].request_count, 2);
        assert_eq!(
            dashboard
                .trends
                .iter()
                .map(|point| point.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(second.tokens.input_tokens, 150);
        assert_eq!(second.tokens.output_tokens, 15);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn time_filter_counts_only_delta_events_inside_window() {
        let codex_home = temp_codex_home("time-filter-delta");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-window"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":10}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.500Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":175,"reasoning_output_tokens":25}}}}"#,
            ]
            .join("\n"),
        );

        let query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:03.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:04.000Z") / 1000) as i64),
            ..OpenAiUsageQuery::default()
        };
        let summary = load_openai_usage_summary(Some(&codex_home), Some(query)).unwrap();

        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.tokens.input_tokens, 500);
        assert_eq!(summary.tokens.cached_input_tokens, 50);
        assert_eq!(summary.tokens.output_tokens, 75);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn negative_delta_is_invalid() {
        assert!(usage_delta(
            Some(UsageCounts {
                input_tokens: 10,
                cached_input_tokens: 5,
                output_tokens: 2,
                reasoning_output_tokens: 1,
            }),
            UsageCounts {
                input_tokens: 9,
                cached_input_tokens: 5,
                output_tokens: 3,
                reasoning_output_tokens: 1,
            },
        )
        .is_none());
    }
}

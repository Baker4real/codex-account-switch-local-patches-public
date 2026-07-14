//! Bounded SQL reads for the Usage dashboard.
//!
//! Keep canonical-account resolution in the parent module. These helpers are
//! deliberately limited to queries that do not require canonical identity
//! joins; callers must fall back to the resolver path for account-group filters.

use rusqlite::{named_params, Connection, OptionalExtension};

use crate::errors::{AppError, AppResult};
use crate::models::{
    OpenAiUsageCostBreakdown, OpenAiUsageQuery, OpenAiUsageRequestLog, OpenAiUsageRequestLogPage,
    OpenAiUsageTokenTotals,
};

use super::{end_seconds_to_ms, i64_from_u64, normalize_model, seconds_to_ms, u64_from_i64};

#[derive(Clone, Debug, Default)]
pub(super) struct SqlAggregate {
    pub requests: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
}

#[derive(Clone, Debug)]
pub(super) struct SqlGroupAggregate {
    pub key: String,
    pub aggregate: SqlAggregate,
}

#[derive(Clone, Debug)]
pub(super) struct SqlCanonicalGroup {
    pub canonical_key: String,
    pub preferred_label: Option<String>,
    pub profile: String,
    pub account_identity_key: Option<String>,
    pub legacy_account_key: Option<String>,
    pub official_source: String,
    pub ledger_layer: String,
    pub attribution_kind: String,
    pub attribution_window_id: Option<String>,
    pub model: String,
    pub requested_model: Option<String>,
    pub pricing_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub verbosity: Option<String>,
    pub pricing_tier: Option<String>,
    pub pricing_source_url: Option<String>,
    pub priced: bool,
    pub pricing_note: Option<String>,
    pub aggregate: SqlAggregate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SqlLogKeyPage {
    pub evidence_ids: Vec<String>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    /// Deterministic upper bound for rows materialized into Rust. This is used
    /// by regression fixtures instead of flaky wall-clock assertions.
    pub materialized_rows: u32,
}

#[derive(Clone, Copy)]
enum FactTable {
    Facts,
    Candidate,
    Excluded,
}

impl FactTable {
    fn for_query(query: &OpenAiUsageQuery) -> Self {
        match query.scope.as_deref() {
            Some("candidate") => Self::Candidate,
            Some("excluded") => Self::Excluded,
            _ => Self::Facts,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Facts => "openai_usage_facts",
            Self::Candidate | Self::Excluded => "openai_usage_evidence",
        }
    }

    fn time(self) -> &'static str {
        match self {
            Self::Facts => "created_at_ms",
            Self::Candidate | Self::Excluded => {
                "COALESCE(event_completed_at_ms, observed_at_ms, created_at_ms)"
            }
        }
    }

    fn scope_predicate(self, query: &OpenAiUsageQuery) -> &'static str {
        match self {
            Self::Candidate => " AND decision = 'candidate'",
            Self::Excluded => " AND decision IN ('rejected', 'superseded')",
            Self::Facts if query.scope.as_deref() == Some("legacy") => {
                " AND ledger_layer = 'legacy_profile_attributed'"
            }
            Self::Facts if query.scope.as_deref() == Some("combined_attributed") => {
                " AND ledger_layer IN ('strict_proven', 'legacy_profile_attributed', 'mixed_attributed')"
            }
            Self::Facts => " AND ledger_layer = 'strict_proven'",
        }
    }
}

struct SqlFilter {
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    profile: Option<String>,
    account_label: Option<String>,
    model: Option<String>,
    status_code: Option<i64>,
}

impl SqlFilter {
    fn new(query: &OpenAiUsageQuery) -> Self {
        Self {
            start_ms: query.start_date.and_then(seconds_to_ms).map(i64_from_u64),
            end_ms: query.end_date.and_then(end_seconds_to_ms).map(i64_from_u64),
            profile: clean(query.profile.as_deref()),
            account_label: clean(query.account_label.as_deref()),
            model: clean(query.model.as_deref()).map(|value| normalize_model(&value)),
            status_code: query.status_code.map(i64::from),
        }
    }
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn filtered_from(query: &OpenAiUsageQuery) -> (FactTable, String, SqlFilter) {
    let table = FactTable::for_query(query);
    let time = table.time();
    let sql = format!(
        " FROM {} WHERE (:start IS NULL OR {time} >= :start)\
         AND (:end IS NULL OR {time} <= :end)\
         AND (:profile IS NULL OR profile = :profile)\
         AND (:account_label IS NULL OR account_label = :account_label)\
         AND (:model IS NULL OR COALESCE(model, request_model) = :model)\
         AND (:status IS NULL OR status_code = :status){}",
        table.name(),
        table.scope_predicate(query),
    );
    (table, sql, SqlFilter::new(query))
}

macro_rules! filter_params {
    ($filter:expr) => {
        named_params! {
            ":start": $filter.start_ms,
            ":end": $filter.end_ms,
            ":profile": $filter.profile,
            ":account_label": $filter.account_label,
            ":model": $filter.model,
            ":status": $filter.status_code,
        }
    };
}

pub(super) fn aggregate(conn: &Connection, query: &OpenAiUsageQuery) -> AppResult<SqlAggregate> {
    let (table, from, filter) = filtered_from(query);
    let (cost_columns, unpriced_column) = match table {
        FactTable::Facts => (
            "COALESCE(SUM(input_cost_micros), 0),\
             COALESCE(SUM(cached_input_cost_micros), 0),\
             COALESCE(SUM(output_cost_micros), 0), COALESCE(SUM(total_cost_micros), 0)",
            "COALESCE(SUM(unpriced_tokens), 0)",
        ),
        FactTable::Candidate | FactTable::Excluded => (
            "0, 0, 0, 0",
            "COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0)), 0)",
        ),
    };
    let sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0),\
         COALESCE(SUM(cached_input_tokens), 0), COALESCE(SUM(output_tokens), 0),\
         COALESCE(SUM(reasoning_output_tokens), 0),\
         COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0)), 0),\
         {cost_columns}, {unpriced_column}{from}"
    );
    conn.query_row(&sql, filter_params!(filter), aggregate_from_row)
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SQL_AGGREGATE_FAILED",
                format!("Failed to aggregate usage facts: {error}"),
            )
        })
}

pub(super) fn group_by_data_source(
    conn: &Connection,
    query: &OpenAiUsageQuery,
) -> AppResult<Vec<SqlGroupAggregate>> {
    let (_, from, filter) = filtered_from(query);
    let sql = format!(
        "SELECT data_source, COUNT(*), COALESCE(SUM(input_tokens), 0),\
         COALESCE(SUM(cached_input_tokens), 0), COALESCE(SUM(output_tokens), 0),\
         COALESCE(SUM(reasoning_output_tokens), 0),\
         COALESCE(SUM(input_tokens + output_tokens), 0),\
         COALESCE(SUM(input_cost_micros), 0),\
         COALESCE(SUM(cached_input_cost_micros), 0),\
         COALESCE(SUM(output_cost_micros), 0), COALESCE(SUM(total_cost_micros), 0),\
         COALESCE(SUM(unpriced_tokens), 0){from} GROUP BY data_source ORDER BY data_source"
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_prepare_error)?;
    let rows = stmt
        .query_map(filter_params!(filter), |row| {
            Ok(SqlGroupAggregate {
                key: row.get(0)?,
                aggregate: aggregate_from_row_at(row, 1)?,
            })
        })
        .map_err(sql_query_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_row_error)
}

pub(super) fn group_by_ledger_layer(
    conn: &Connection,
    query: &OpenAiUsageQuery,
) -> AppResult<Vec<SqlGroupAggregate>> {
    let (table, from, filter) = filtered_from(query);
    let (cost_columns, unpriced_column) = match table {
        FactTable::Facts => (
            "COALESCE(SUM(input_cost_micros),0), COALESCE(SUM(cached_input_cost_micros),0), COALESCE(SUM(output_cost_micros),0), COALESCE(SUM(total_cost_micros),0)",
            "COALESCE(SUM(unpriced_tokens),0)",
        ),
        FactTable::Candidate | FactTable::Excluded => (
            "0, 0, 0, 0",
            "COALESCE(SUM(COALESCE(input_tokens,0)+COALESCE(output_tokens,0)),0)",
        ),
    };
    let sql = format!(
        "SELECT ledger_layer, COUNT(*), COALESCE(SUM(input_tokens),0),
         COALESCE(SUM(cached_input_tokens),0), COALESCE(SUM(output_tokens),0),
         COALESCE(SUM(reasoning_output_tokens),0),
         COALESCE(SUM(COALESCE(input_tokens,0)+COALESCE(output_tokens,0)),0),
         {cost_columns}, {unpriced_column}{from} GROUP BY ledger_layer ORDER BY ledger_layer"
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_prepare_error)?;
    let rows = stmt
        .query_map(filter_params!(filter), |row| {
            Ok(SqlGroupAggregate {
                key: row.get(0)?,
                aggregate: aggregate_from_row_at(row, 1)?,
            })
        })
        .map_err(sql_query_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_row_error)
}

/// Returns already-aggregated canonical account/model rows. The grouping keeps
/// every DTO metadata dimension, so the parent can perform only the small
/// `single_or_mixed` fold without materializing request history.
pub(super) fn canonical_groups(
    conn: &Connection,
    query: &OpenAiUsageQuery,
) -> AppResult<Option<Vec<SqlCanonicalGroup>>> {
    if matches!(query.scope.as_deref(), Some("candidate" | "excluded"))
        || query
            .account_group_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Ok(None);
    }
    let filter = SqlFilter::new(query);
    let account_group = clean(query.account_group_key.as_deref());
    let scope = FactTable::Facts.scope_predicate(query);
    let sql = format!(
        "SELECT m.canonical_key, MAX(m.preferred_label), f.profile,
         f.account_identity_key, f.legacy_account_key, f.official_source,
         f.ledger_layer, f.attribution_kind, f.attribution_window_id, f.model,
         f.request_model, f.pricing_model, f.model_reasoning_effort,
         f.model_verbosity, f.pricing_tier, f.pricing_source_url, f.priced,
         f.pricing_note, COUNT(*), COALESCE(SUM(f.input_tokens), 0),
         COALESCE(SUM(f.cached_input_tokens), 0), COALESCE(SUM(f.output_tokens), 0),
         COALESCE(SUM(f.reasoning_output_tokens), 0),
         COALESCE(SUM(f.input_tokens + f.output_tokens), 0),
         COALESCE(SUM(f.input_cost_micros), 0),
         COALESCE(SUM(f.cached_input_cost_micros), 0),
         COALESCE(SUM(f.output_cost_micros), 0), COALESCE(SUM(f.total_cost_micros), 0),
         COALESCE(SUM(f.unpriced_tokens), 0)
         FROM openai_usage_facts f JOIN openai_usage_canonical_map m USING(evidence_id)
         WHERE (:start IS NULL OR f.created_at_ms >= :start)
         AND (:end IS NULL OR f.created_at_ms <= :end)
         AND (:profile IS NULL OR f.profile = :profile)
         AND (:account_label IS NULL OR m.preferred_label = :account_label)
         AND (:account_group IS NULL OR m.canonical_key = :account_group)
         AND (:model IS NULL OR COALESCE(f.model, f.request_model) = :model)
         AND (:status IS NULL OR f.status_code = :status){scope}
         GROUP BY m.canonical_key, f.profile, f.account_identity_key,
         f.legacy_account_key, f.official_source, f.ledger_layer,
         f.attribution_kind, f.attribution_window_id, f.model, f.request_model,
         f.pricing_model, f.model_reasoning_effort, f.model_verbosity,
         f.pricing_tier, f.pricing_source_url, f.priced, f.pricing_note
         ORDER BY m.canonical_key, f.model"
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_prepare_error)?;
    let rows = stmt
        .query_map(
            named_params! {
                ":start": filter.start_ms, ":end": filter.end_ms, ":profile": filter.profile,
                ":account_label": filter.account_label, ":account_group": account_group,
                ":model": filter.model, ":status": filter.status_code,
            },
            |row| {
                Ok(SqlCanonicalGroup {
                    canonical_key: row.get(0)?,
                    preferred_label: row.get(1)?,
                    profile: row.get(2)?,
                    account_identity_key: row.get(3)?,
                    legacy_account_key: row.get(4)?,
                    official_source: row.get(5)?,
                    ledger_layer: row.get(6)?,
                    attribution_kind: row.get(7)?,
                    attribution_window_id: row.get(8)?,
                    model: row.get(9)?,
                    requested_model: row.get(10)?,
                    pricing_model: row.get(11)?,
                    reasoning_effort: row.get(12)?,
                    verbosity: row.get(13)?,
                    pricing_tier: row.get(14)?,
                    pricing_source_url: row.get(15)?,
                    priced: row.get::<_, i64>(16)? != 0,
                    pricing_note: row.get(17)?,
                    aggregate: aggregate_from_row_at(row, 18)?,
                })
            },
        )
        .map_err(sql_query_error)?;
    let groups = rows.collect::<Result<Vec<_>, _>>().map_err(sql_row_error)?;
    let grouped_requests = groups
        .iter()
        .map(|group| group.aggregate.requests)
        .sum::<u64>();
    // A stale or incomplete derived map must never turn valid usage into zero
    // or partial totals. Its state is rebuilt transactionally, but this guard
    // also protects additive migrations and interrupted older builds.
    if grouped_requests != aggregate(conn, query)?.requests {
        return Ok(None);
    }
    Ok(Some(groups))
}

pub(super) fn aggregate_between(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    start_ms: u64,
    end_ms: u64,
) -> AppResult<Option<SqlAggregate>> {
    if matches!(query.scope.as_deref(), Some("candidate" | "excluded")) {
        return Ok(None);
    }
    let mut bounded = query.clone();
    bounded.start_date = i64::try_from(start_ms / 1000).ok();
    bounded.end_date = i64::try_from(end_ms / 1000).ok();
    if bounded
        .account_group_key
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        return Ok(None);
    }
    aggregate(conn, &bounded).map(Some)
}

pub(super) fn log_key_page(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    page: u32,
    page_size: u32,
) -> AppResult<Option<SqlLogKeyPage>> {
    // Canonical account keys are derived from multiple evidence rows and cannot
    // be represented by this local SQL predicate without changing semantics.
    if query
        .account_group_key
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        return Ok(None);
    }
    let (table, from, filter) = filtered_from(query);
    let total_i64: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*){from}"),
            filter_params!(filter),
            |row| row.get(0),
        )
        .map_err(sql_query_error)?;
    let total = u32::try_from(total_i64).unwrap_or(u32::MAX);
    let page_size = page_size.clamp(1, 200);
    let pages = total.saturating_add(page_size - 1) / page_size;
    let page = page.max(1).min(pages.max(1));
    let offset = i64::from((page - 1).saturating_mul(page_size));
    let sql = format!(
        "SELECT evidence_id{from} ORDER BY {} DESC, COALESCE(request_id, evidence_id) DESC LIMIT :limit OFFSET :offset",
        table.time()
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_prepare_error)?;
    let rows = stmt
        .query_map(
            named_params! {
                ":start": filter.start_ms, ":end": filter.end_ms, ":profile": filter.profile,
                ":account_label": filter.account_label, ":model": filter.model,
                ":status": filter.status_code, ":limit": i64::from(page_size), ":offset": offset,
            },
            |row| row.get(0),
        )
        .map_err(sql_query_error)?;
    let evidence_ids = rows.collect::<Result<Vec<_>, _>>().map_err(sql_row_error)?;
    Ok(Some(SqlLogKeyPage {
        materialized_rows: u32::try_from(evidence_ids.len()).unwrap_or(u32::MAX),
        evidence_ids,
        total,
        page,
        page_size,
    }))
}

/// Reads one bounded, already-canonicalized request-log page. Unlike the
/// legacy resolver path, the amount of Rust work is limited by `page_size`.
pub(super) fn canonical_log_page(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    page: u32,
    page_size: u32,
) -> AppResult<Option<OpenAiUsageRequestLogPage>> {
    if matches!(query.scope.as_deref(), Some("candidate" | "excluded"))
        || query
            .account_group_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Ok(None);
    }
    let Some(keys) = log_key_page(conn, query, page, page_size)? else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare(
            "SELECT f.request_id, f.session_id, f.profile,
                    COALESCE(m.preferred_label, f.account_label), f.official_source,
                    f.ledger_layer, f.attribution_kind, f.attribution_window_id,
                    f.model, f.request_model, f.pricing_model,
                    f.model_reasoning_effort, f.model_verbosity, f.pricing_tier,
                    f.pricing_source_url, f.input_tokens, f.cached_input_tokens,
                    f.output_tokens, f.reasoning_output_tokens, f.input_cost_micros,
                    f.cached_input_cost_micros, f.output_cost_micros,
                    f.total_cost_micros, f.unpriced_tokens, f.priced, f.pricing_note,
                    f.status_code, f.created_at_ms, f.data_source, f.usage_confidence
             FROM openai_usage_facts f
             JOIN openai_usage_canonical_map m USING(evidence_id)
             WHERE f.evidence_id = ?1",
        )
        .map_err(sql_prepare_error)?;
    let mut data = Vec::with_capacity(keys.evidence_ids.len());
    for evidence_id in &keys.evidence_ids {
        let Some(log) = stmt
            .query_row([evidence_id], |row| {
                let input_tokens = u64_from_i64(row.get(15)?);
                let cached_input_tokens = u64_from_i64(row.get(16)?);
                let output_tokens = u64_from_i64(row.get(17)?);
                Ok(OpenAiUsageRequestLog {
                    request_id: row.get(0)?,
                    session_id: row.get(1)?,
                    profile: row.get(2)?,
                    account_label: row.get(3)?,
                    official_source: row.get(4)?,
                    ledger_layer: row.get(5)?,
                    attribution_kind: row.get(6)?,
                    attribution_window_id: row.get(7)?,
                    model: row.get(8)?,
                    requested_model: row.get(9)?,
                    pricing_model: row.get(10)?,
                    model_reasoning_effort: row.get(11)?,
                    model_verbosity: row.get(12)?,
                    pricing_tier: row.get(13)?,
                    pricing_source_url: row.get(14)?,
                    tokens: OpenAiUsageTokenTotals {
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens: u64_from_i64(row.get(18)?),
                        real_total_tokens: input_tokens.saturating_add(output_tokens),
                    },
                    cost: OpenAiUsageCostBreakdown {
                        input_cost_micros: u64_from_i64(row.get(19)?),
                        cached_input_cost_micros: u64_from_i64(row.get(20)?),
                        output_cost_micros: u64_from_i64(row.get(21)?),
                        total_cost_micros: u64_from_i64(row.get(22)?),
                        unpriced_tokens: u64_from_i64(row.get(23)?),
                    },
                    priced: row.get::<_, i64>(24)? != 0,
                    pricing_note: row.get(25)?,
                    status_code: u16::try_from(row.get::<_, i64>(26)?).unwrap_or(0),
                    created_at_ms: u64_from_i64(row.get(27)?),
                    data_source: row.get(28)?,
                    usage_confidence: row.get(29)?,
                })
            })
            .optional()
            .map_err(sql_query_error)?
        else {
            return Ok(None);
        };
        data.push(log);
    }
    Ok(Some(OpenAiUsageRequestLogPage {
        data,
        total: keys.total,
        page: keys.page,
        page_size: keys.page_size,
    }))
}

fn aggregate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SqlAggregate> {
    aggregate_from_row_at(row, 0)
}

fn aggregate_from_row_at(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<SqlAggregate> {
    Ok(SqlAggregate {
        requests: u64_from_i64(row.get(offset)?),
        tokens: OpenAiUsageTokenTotals {
            input_tokens: u64_from_i64(row.get(offset + 1)?),
            cached_input_tokens: u64_from_i64(row.get(offset + 2)?),
            output_tokens: u64_from_i64(row.get(offset + 3)?),
            reasoning_output_tokens: u64_from_i64(row.get(offset + 4)?),
            real_total_tokens: u64_from_i64(row.get(offset + 5)?),
        },
        cost: OpenAiUsageCostBreakdown {
            input_cost_micros: u64_from_i64(row.get(offset + 6)?),
            cached_input_cost_micros: u64_from_i64(row.get(offset + 7)?),
            output_cost_micros: u64_from_i64(row.get(offset + 8)?),
            total_cost_micros: u64_from_i64(row.get(offset + 9)?),
            unpriced_tokens: u64_from_i64(row.get(offset + 10)?),
        },
    })
}

fn sql_prepare_error(error: rusqlite::Error) -> AppError {
    AppError::new(
        "OPENAI_USAGE_SQL_PREPARE_FAILED",
        format!("Failed to prepare bounded usage query: {error}"),
    )
}
fn sql_query_error(error: rusqlite::Error) -> AppError {
    AppError::new(
        "OPENAI_USAGE_SQL_QUERY_FAILED",
        format!("Failed to run bounded usage query: {error}"),
    )
}
fn sql_row_error(error: rusqlite::Error) -> AppError {
    AppError::new(
        "OPENAI_USAGE_SQL_ROW_FAILED",
        format!("Failed to read bounded usage row: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rows: u32) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE openai_usage_facts (evidence_id TEXT PRIMARY KEY, request_id TEXT,\
             profile TEXT, account_label TEXT, model TEXT, request_model TEXT, status_code INTEGER,\
             ledger_layer TEXT, created_at_ms INTEGER, data_source TEXT, input_tokens INTEGER,\
             cached_input_tokens INTEGER, output_tokens INTEGER, reasoning_output_tokens INTEGER,\
             input_cost_micros INTEGER, cached_input_cost_micros INTEGER, output_cost_micros INTEGER,\
             total_cost_micros INTEGER, unpriced_tokens INTEGER);\
             CREATE TABLE openai_usage_evidence (evidence_id TEXT PRIMARY KEY, request_id TEXT,\
             profile TEXT, account_label TEXT, model TEXT, request_model TEXT, status_code INTEGER,\
             decision TEXT, event_completed_at_ms INTEGER, observed_at_ms INTEGER, created_at_ms INTEGER,\
             data_source TEXT, input_tokens INTEGER, cached_input_tokens INTEGER, output_tokens INTEGER,\
             reasoning_output_tokens INTEGER);\
             CREATE TABLE openai_usage_canonical_map (evidence_id TEXT PRIMARY KEY,\
             canonical_key TEXT NOT NULL, preferred_label TEXT);",
        )
        .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for i in 0..rows {
            tx.execute(
                "INSERT INTO openai_usage_facts VALUES (?1, ?2, 'a', 'A', 'gpt-5', NULL, 200,\
                 'strict_proven', ?3, ?4, 10, 2, 4, 1, 10, 2, 4, 16, 0)",
                rusqlite::params![
                    format!("e-{i:05}"),
                    format!("r-{i:05}"),
                    i64::from(i) + 1,
                    if i % 2 == 0 { "session" } else { "proxy" }
                ],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO openai_usage_canonical_map VALUES (?1, ?2, 'A')",
                rusqlite::params![format!("e-{i:05}"), format!("account:{}", i % 2)],
            )
            .unwrap();
            if i < 100 {
                tx.execute(
                    "INSERT INTO openai_usage_evidence VALUES (?1, ?2, 'candidate', NULL, 'gpt-5',\
                     NULL, 200, 'candidate', ?3, NULL, ?3, 'ccswitch', 10, 2, 4, 1)",
                    rusqlite::params![
                        format!("c-{i:05}"),
                        format!("candidate-{i:05}"),
                        i64::from(i) + 1,
                    ],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
        conn
    }

    #[test]
    fn aggregates_and_groups_in_sql_without_materializing_history() {
        let conn = fixture(10_000);
        let total = aggregate(&conn, &OpenAiUsageQuery::default()).unwrap();
        assert_eq!(total.requests, 10_000);
        assert_eq!(total.tokens.real_total_tokens, 140_000);
        let sources = group_by_data_source(&conn, &OpenAiUsageQuery::default()).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(
            sources.iter().map(|s| s.aggregate.requests).sum::<u64>(),
            10_000
        );
    }

    #[test]
    fn first_and_no_change_pages_have_history_independent_materialization() {
        for history in [31, 30_000] {
            let conn = fixture(history);
            let first = log_key_page(&conn, &OpenAiUsageQuery::default(), 1, 20)
                .unwrap()
                .unwrap();
            let unchanged = log_key_page(&conn, &OpenAiUsageQuery::default(), 1, 20)
                .unwrap()
                .unwrap();
            assert_eq!(first.materialized_rows, 20);
            assert_eq!(unchanged.materialized_rows, 20);
            assert_eq!(first.evidence_ids, unchanged.evidence_ids);
        }
    }

    #[test]
    fn page_is_clamped_by_count_before_limit_offset() {
        let conn = fixture(21);
        let page = log_key_page(&conn, &OpenAiUsageQuery::default(), 99, 20)
            .unwrap()
            .unwrap();
        assert_eq!(page.page, 2);
        assert_eq!(page.total, 21);
        assert_eq!(page.materialized_rows, 1);
        assert_eq!(page.evidence_ids, vec!["e-00000"]);
    }

    #[test]
    fn canonical_account_filter_requires_resolver_fallback() {
        let conn = fixture(1);
        let query = OpenAiUsageQuery {
            account_group_key: Some("account:v1:x".into()),
            ..Default::default()
        };
        assert!(log_key_page(&conn, &query, 1, 20).unwrap().is_none());
    }

    #[test]
    fn benchmark_matrix_bounds_strict_candidate_and_combined_reads() {
        let conn = fixture(30_000);
        let strict_1d = OpenAiUsageQuery {
            start_date: Some(29),
            end_date: Some(30),
            ..Default::default()
        };
        let candidate_1d = OpenAiUsageQuery {
            scope: Some("candidate".into()),
            start_date: Some(0),
            end_date: Some(86_400),
            ..Default::default()
        };
        let combined_30d = OpenAiUsageQuery {
            scope: Some("combined_attributed".into()),
            start_date: Some(0),
            end_date: Some(30 * 86_400),
            ..Default::default()
        };
        for query in [&strict_1d, &candidate_1d, &combined_30d] {
            let page = log_key_page(&conn, query, 1, 20).unwrap().unwrap();
            assert!(page.materialized_rows <= 20);
        }
        assert_eq!(
            log_key_page(&conn, &candidate_1d, 1, 20)
                .unwrap()
                .unwrap()
                .total,
            100
        );
        assert_eq!(aggregate(&conn, &combined_30d).unwrap().requests, 30_000);
    }
}

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{AppError, AppResult};
use crate::models::{
    OpenAiUsageAccountSummary, OpenAiUsageCostBreakdown, OpenAiUsageExclusions,
    OpenAiUsageModelSummary, OpenAiUsageSummary, OpenAiUsageTokenTotals, ProfileIndexEntry,
};

use super::metadata::load_account_identity_from_path;
use super::paths::{
    get_backup_root, get_codex_home, get_current_profile_file, get_openai_usage_attribution_path,
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

pub fn load_openai_usage_summary(codex_home: Option<&Path>) -> AppResult<OpenAiUsageSummary> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let generated_at_ms = current_time_ms();
    let mut summary = OpenAiUsageSummary {
        generated_at_ms,
        pricing_source: PRICING_SOURCE.to_string(),
        pricing_checked_at: PRICING_CHECKED_AT.to_string(),
        ..OpenAiUsageSummary::default()
    };

    let fallback_attribution = build_attribution_context(&codex_home, &mut summary.exclusions)?;
    let attribution_records = load_attribution_records(&codex_home);
    let earliest_attribution = attribution_records
        .iter()
        .filter(|record| record.is_official_openai)
        .map(|record| record.observed_at_ms)
        .min()
        .or_else(|| {
            fallback_attribution
                .as_ref()
                .map(|record| record.attribution_since_ms)
        });
    summary.attribution_since_ms = earliest_attribution;
    if fallback_attribution.is_none() && attribution_records.is_empty() {
        return Ok(summary);
    }

    let mut accounts: BTreeMap<String, AccountAggregate> = BTreeMap::new();
    let mut global_models: BTreeMap<String, ModelAggregate> = BTreeMap::new();
    let mut request_ids = HashSet::new();
    let mut fingerprints = HashSet::new();

    for sessions_root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        let mut files = Vec::new();
        collect_jsonl_files(&sessions_root, &mut files);
        files.sort();
        for file in files {
            process_session_file(
                &file,
                fallback_attribution.as_ref(),
                &attribution_records,
                &mut request_ids,
                &mut fingerprints,
                &mut accounts,
                &mut global_models,
                &mut summary,
            );
        }
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
    Ok(summary)
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
    if official_host(trimmed).is_some_and(|host| host == "api.openai.com") {
        return OfficialSource::Official(trimmed.to_string());
    }
    OfficialSource::NonOfficial
}

fn official_host(base_url: &str) -> Option<&str> {
    let without_scheme = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))?;
    without_scheme
        .split(['/', ':', '?', '#'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn process_session_file(
    file: &Path,
    fallback_attribution: Option<&AttributionContext>,
    attribution_records: &[AttributionRecord],
    request_ids: &mut HashSet<String>,
    fingerprints: &mut HashSet<String>,
    accounts: &mut BTreeMap<String, AccountAggregate>,
    global_models: &mut BTreeMap<String, ModelAggregate>,
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
                if !request_ids.insert(request_id) {
                    summary.exclusions.duplicate_events =
                        summary.exclusions.duplicate_events.saturating_add(1);
                    continue;
                }
                let fingerprint = fingerprint_for_event(&attribution, &model, delta, event_time_ms);
                if !fingerprints.insert(fingerprint) {
                    summary.exclusions.duplicate_events =
                        summary.exclusions.duplicate_events.saturating_add(1);
                    continue;
                }
                add_usage(accounts, global_models, &attribution, &model, delta);
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

fn add_usage(
    accounts: &mut BTreeMap<String, AccountAggregate>,
    global_models: &mut BTreeMap<String, ModelAggregate>,
    attribution: &AttributionContext,
    model: &str,
    delta: UsageCounts,
) {
    let cost = calculate_cost(model, delta);
    let account = accounts
        .entry(attribution.profile.clone())
        .or_insert_with(|| AccountAggregate {
            account_label: attribution.account_label.clone(),
            official_source: attribution.official_source.clone(),
            ..AccountAggregate::default()
        });
    add_to_aggregate(&mut account.aggregate, delta, &cost.breakdown);
    add_to_model(
        account.models.entry(model.to_string()).or_default(),
        delta,
        &cost,
    );
    add_to_model(
        global_models.entry(model.to_string()).or_default(),
        delta,
        &cost,
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

        let summary = load_openai_usage_summary(Some(&codex_home)).unwrap();
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

        let summary = load_openai_usage_summary(Some(&codex_home)).unwrap();
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

        let summary = load_openai_usage_summary(Some(&codex_home)).unwrap();
        assert_eq!(summary.accounts.len(), 2);
        assert_eq!(summary.accounts[0].profile, "a");
        assert_eq!(summary.accounts[0].tokens.input_tokens, 1000);
        assert_eq!(summary.accounts[1].profile, "b");
        assert_eq!(summary.accounts[1].tokens.input_tokens, 500);
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

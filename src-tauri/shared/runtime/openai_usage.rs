use std::collections::{hash_map::DefaultHasher, BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{AppError, AppResult};
use crate::models::{
    OpenAiUsageAccountSummary, OpenAiUsageCostBreakdown, OpenAiUsageDashboard,
    OpenAiUsageDataSourceSummary, OpenAiUsageExclusions, OpenAiUsageLayerSummary,
    OpenAiUsageModelSummary, OpenAiUsageQuery, OpenAiUsageRequestLog, OpenAiUsageRequestLogPage,
    OpenAiUsageSummary, OpenAiUsageTokenTotals, OpenAiUsageTrendPoint, ProfileIndexEntry,
};

use super::metadata::{load_account_identity_from_path, AccountIdentity};
use super::paths::{
    get_backup_root, get_codex_home, get_current_profile_file, get_openai_usage_attribution_path,
    get_openai_usage_ledger_path,
};
use super::profiles_index::load_profiles_index;
use super::session_files::{collect_jsonl_files, file_modified_ms};

mod identity;
mod ingestion;

const PRICING_SOURCE: &str = "OpenAI API pricing";
const PRICING_SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing";
const PRICING_CHECKED_AT: &str = "2026-07-08";
const PRICING_TIER_STANDARD_SHORT_CONTEXT: &str = "standard_short_context";
const EVIDENCE_PARSER_VERSION: &str = "openai-usage-evidence-v2-parser-2";
const EVIDENCE_CLASSIFIER_VERSION: &str = "openai-usage-evidence-v2-classifier-3";
const EVIDENCE_DEDUPE_VERSION: &str = "openai-usage-evidence-v2-dedupe-3";
const LEDGER_SCHEMA_VERSION: &str = "openai-usage-ledger-schema-4";
const LAYER_CLASSIFIER_VERSION: &str = "openai-usage-layer-classifier-1";
const ATTRIBUTION_BACKFILL_VERSION: &str = "openai-usage-attribution-backfill-1";
const PRICING_VERSION: &str = PRICING_CHECKED_AT;
const SOURCE_OPENAI_RESPONSE_USAGE: &str = "openai_response_usage";
const SOURCE_CCSWITCH_PROXY: &str = "ccswitch_proxy";
const SOURCE_CODEX_SESSION_JSONL: &str = "codex_session_jsonl";
const SOURCE_CCSWITCH_CODEX_SESSION: &str = "ccswitch_codex_session";
const SOURCE_CCSWITCH_ROLLUP: &str = "ccswitch_rollup";
const DECISION_ACCEPTED: &str = "accepted";
const DECISION_CANDIDATE: &str = "candidate";
const DECISION_REJECTED: &str = "rejected";
const DECISION_SUPERSEDED: &str = "superseded";
const LEDGER_LAYER_STRICT_PROVEN: &str = "strict_proven";
const LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED: &str = "legacy_profile_attributed";
const LEDGER_LAYER_CANDIDATE_UNATTRIBUTED: &str = "candidate_unattributed";
const LEDGER_LAYER_REJECTED_INVALID: &str = "rejected_invalid";
const LEDGER_LAYER_SUPERSEDED: &str = "superseded";
const LEDGER_LAYER_MIXED_ATTRIBUTED: &str = "mixed_attributed";
const ATTRIBUTION_KIND_STRICT_ACCOUNT: &str = "strict_account";
const ATTRIBUTION_KIND_LEGACY_PROFILE: &str = "legacy_profile";
const ATTRIBUTION_KIND_MIXED_ACCOUNT: &str = "mixed_account";
const ATTRIBUTION_KIND_UNATTRIBUTED: &str = "unattributed";
const SCOPE_STRICT: &str = "strict";
const SCOPE_LEGACY: &str = "legacy";
const SCOPE_COMBINED_ATTRIBUTED: &str = "combined_attributed";
const SCOPE_CANDIDATE: &str = "candidate";
const SCOPE_EXCLUDED: &str = "excluded";
const CONFIDENCE_PROVEN: &str = "proven";
const CONFIDENCE_HIGH: &str = "high";
const CONFIDENCE_LOW: &str = "low";
const CONFIDENCE_UNPROVEN: &str = "unproven";
const USAGE_SHAPE_CUMULATIVE_TOTAL_DELTA: &str = "cumulative_total_delta";
const USAGE_SHAPE_LAST_TOKEN_USAGE: &str = "last_token_usage";
const USAGE_SHAPE_PROXY_REQUEST: &str = "proxy_request";
const USAGE_SHAPE_DAILY_ROLLUP: &str = "daily_rollup";

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
    requested_models: BTreeSet<String>,
    pricing_models: BTreeSet<String>,
    reasoning_efforts: BTreeSet<String>,
    verbosities: BTreeSet<String>,
    pricing_tiers: BTreeSet<String>,
    pricing_source_urls: BTreeSet<String>,
    priced: bool,
    pricing_note: Option<String>,
}

#[derive(Default)]
struct AccountAggregate {
    profile: String,
    profiles: BTreeSet<String>,
    account_group_key: String,
    account_identity_key: Option<String>,
    legacy_account_key: Option<String>,
    account_label: Option<String>,
    official_source: String,
    ledger_layer: String,
    attribution_kind: String,
    attribution_window_id: Option<String>,
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

#[derive(Debug, Clone, Copy)]
struct ParsedUsage {
    counts: UsageCounts,
    usage_shape: &'static str,
    usage_confidence: &'static str,
}

struct AttributionContext {
    profile: String,
    account_label: Option<String>,
    official_source: String,
    attribution_since_ms: u64,
    account_identity_key: Option<String>,
    account_id_alias: Option<String>,
    email_alias: Option<String>,
    legacy_account_key: Option<String>,
    identity_source: Option<String>,
    identity_observed_at_ms: Option<u64>,
    ledger_layer: String,
    layer_reason: Option<String>,
    attribution_kind: String,
    attribution_window_id: Option<String>,
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
}

#[derive(Clone)]
struct UsageEvent {
    evidence_id: String,
    request_id: String,
    fingerprint: String,
    session_id: String,
    profile: String,
    account_label: Option<String>,
    account_identity_key: String,
    official_source: String,
    model: String,
    requested_model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
    delta: UsageCounts,
    cost: CalculatedCost,
    status_code: u16,
    created_at_ms: u64,
    data_source: String,
    source_type: String,
    source_priority: i64,
    usage_confidence: String,
    pricing_model: String,
    pricing_tier: String,
    pricing_source_url: String,
}

#[derive(Clone)]
struct UsageFact {
    evidence_id: String,
    request_id: String,
    fingerprint: String,
    session_id: String,
    profile: String,
    account_label: Option<String>,
    account_identity_key: Option<String>,
    account_id_alias: Option<String>,
    email_alias: Option<String>,
    legacy_account_key: Option<String>,
    official_source: String,
    ledger_layer: String,
    layer_reason: Option<String>,
    attribution_kind: String,
    attribution_window_id: Option<String>,
    model: String,
    requested_model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
    delta: UsageCounts,
    cost: CalculatedCost,
    status_code: u16,
    created_at_ms: u64,
    data_source: String,
    source_type: String,
    source_priority: i64,
    usage_confidence: String,
    pricing_model: String,
    pricing_tier: String,
    pricing_source_url: String,
}

#[derive(Clone)]
struct UsageEvidence {
    evidence_id: String,
    request_id: Option<String>,
    fingerprint: Option<String>,
    session_id: Option<String>,
    source_type: String,
    source_priority: i64,
    source_locator: String,
    source_hash: String,
    parser_version: String,
    event_started_at_ms: Option<u64>,
    event_completed_at_ms: Option<u64>,
    observed_at_ms: Option<u64>,
    turn_index: Option<u64>,
    response_id: Option<String>,
    previous_response_id: Option<String>,
    model: Option<String>,
    request_model: Option<String>,
    pricing_model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
    pricing_tier: Option<String>,
    pricing_source_url: Option<String>,
    usage_shape: String,
    usage_confidence: String,
    delta: Option<UsageCounts>,
    raw_usage_json: Option<String>,
    profile: Option<String>,
    account_identity_key: Option<String>,
    account_id_alias: Option<String>,
    email_alias: Option<String>,
    account_label: Option<String>,
    identity_source: Option<String>,
    identity_observed_at_ms: Option<u64>,
    account_confidence: String,
    effective_base_url: Option<String>,
    official_source: Option<String>,
    official_confidence: String,
    status_code: Option<u16>,
    decision: String,
    decision_reason: Option<String>,
    superseded_by: Option<String>,
    ledger_layer: String,
    layer_reason: Option<String>,
    attribution_kind: String,
    attribution_window_id: Option<String>,
    legacy_account_key: Option<String>,
    created_at_ms: u64,
}

struct EvidenceDecision {
    evidence: UsageEvidence,
    accepted: Option<UsageEvent>,
    fact: Option<UsageFact>,
}

struct EvidenceClassifier;

impl EvidenceClassifier {
    fn accept_session_evidence(evidence: UsageEvidence, cost: CalculatedCost) -> EvidenceDecision {
        let fact = if evidence.decision == DECISION_ACCEPTED
            && matches!(
                evidence.ledger_layer.as_str(),
                LEDGER_LAYER_STRICT_PROVEN | LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED
            ) {
            usage_fact_from_evidence(&evidence, cost.clone())
        } else {
            None
        };
        let accepted = if evidence.decision == DECISION_ACCEPTED {
            match (
                evidence.request_id.clone(),
                evidence.fingerprint.clone(),
                evidence.session_id.clone(),
                evidence.profile.clone(),
                evidence.account_identity_key.clone(),
                evidence.official_source.clone(),
                evidence.model.clone(),
                evidence.delta,
                evidence.status_code,
                evidence.event_completed_at_ms,
            ) {
                (
                    Some(request_id),
                    Some(fingerprint),
                    Some(session_id),
                    Some(profile),
                    Some(account_identity_key),
                    Some(official_source),
                    Some(model),
                    Some(delta),
                    Some(status_code),
                    Some(created_at_ms),
                ) => Some(UsageEvent {
                    evidence_id: evidence.evidence_id.clone(),
                    request_id,
                    fingerprint,
                    session_id,
                    profile,
                    account_label: evidence.account_label.clone(),
                    account_identity_key,
                    official_source,
                    model: model.clone(),
                    requested_model: evidence.request_model.clone(),
                    model_reasoning_effort: evidence.model_reasoning_effort.clone(),
                    model_verbosity: evidence.model_verbosity.clone(),
                    delta,
                    cost,
                    status_code,
                    created_at_ms,
                    data_source: official_data_source().to_string(),
                    source_type: evidence.source_type.clone(),
                    source_priority: evidence.source_priority,
                    usage_confidence: evidence.usage_confidence.clone(),
                    pricing_model: evidence.pricing_model.clone().unwrap_or(model),
                    pricing_tier: evidence
                        .pricing_tier
                        .clone()
                        .unwrap_or_else(|| PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
                    pricing_source_url: evidence
                        .pricing_source_url
                        .clone()
                        .unwrap_or_else(|| PRICING_SOURCE_URL.to_string()),
                }),
                _ => None,
            }
        } else {
            None
        };
        EvidenceDecision {
            evidence,
            accepted,
            fact,
        }
    }
}

fn usage_fact_from_evidence(evidence: &UsageEvidence, cost: CalculatedCost) -> Option<UsageFact> {
    let request_id = evidence.request_id.clone()?;
    let fingerprint = evidence.fingerprint.clone()?;
    let session_id = evidence.session_id.clone()?;
    let profile = evidence.profile.clone()?;
    let official_source = evidence.official_source.clone()?;
    let model = evidence.model.clone()?;
    let delta = evidence.delta?;
    let status_code = evidence.status_code?;
    let created_at_ms = evidence.event_completed_at_ms?;
    Some(UsageFact {
        evidence_id: evidence.evidence_id.clone(),
        request_id,
        fingerprint,
        session_id,
        profile,
        account_label: evidence.account_label.clone(),
        account_identity_key: evidence.account_identity_key.clone(),
        account_id_alias: identity::validated_account_id_alias(
            evidence.account_id_alias.as_deref(),
        ),
        email_alias: identity::validated_email_alias(evidence.email_alias.as_deref()),
        legacy_account_key: evidence.legacy_account_key.clone(),
        official_source,
        ledger_layer: evidence.ledger_layer.clone(),
        layer_reason: evidence.layer_reason.clone(),
        attribution_kind: evidence.attribution_kind.clone(),
        attribution_window_id: evidence.attribution_window_id.clone(),
        model: model.clone(),
        requested_model: evidence.request_model.clone(),
        model_reasoning_effort: evidence.model_reasoning_effort.clone(),
        model_verbosity: evidence.model_verbosity.clone(),
        delta,
        cost,
        status_code,
        created_at_ms,
        data_source: official_data_source().to_string(),
        source_type: evidence.source_type.clone(),
        source_priority: evidence.source_priority,
        usage_confidence: evidence.usage_confidence.clone(),
        pricing_model: evidence.pricing_model.clone().unwrap_or(model),
        pricing_tier: evidence
            .pricing_tier
            .clone()
            .unwrap_or_else(|| PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
        pricing_source_url: evidence
            .pricing_source_url
            .clone()
            .unwrap_or_else(|| PRICING_SOURCE_URL.to_string()),
    })
}

#[derive(Default)]
struct EvidenceDecisionCounts {
    accepted: u64,
    candidate: u64,
    rejected: u64,
    superseded: u64,
    strict_proven: u64,
    legacy_profile_attributed: u64,
    candidate_unattributed: u64,
    rejected_invalid: u64,
}

struct UsageDedupeEngine {
    request_ids: HashMap<String, String>,
    fingerprints: HashMap<String, String>,
}

impl UsageDedupeEngine {
    fn accept_or_reason(
        &mut self,
        request_id: &str,
        fingerprint: &str,
        evidence_id: &str,
    ) -> Option<(&'static str, String)> {
        if let Some(existing) = self.request_ids.get(request_id) {
            return Some(("duplicate request_id", existing.clone()));
        }
        if let Some(existing) = self.fingerprints.get(fingerprint) {
            return Some(("duplicate fingerprint", existing.clone()));
        }
        self.request_ids
            .insert(request_id.to_string(), evidence_id.to_string());
        self.fingerprints
            .insert(fingerprint.to_string(), evidence_id.to_string());
        None
    }
}

impl Default for UsageDedupeEngine {
    fn default() -> Self {
        Self {
            request_ids: HashMap::new(),
            fingerprints: HashMap::new(),
        }
    }
}

struct UsageLoadResult {
    source_signature: String,
    cache_hit: bool,
    events: Vec<UsageEvent>,
    facts: Vec<UsageFact>,
    evidence: Vec<UsageEvidence>,
    decisions: EvidenceDecisionCounts,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AttributionRecord {
    observed_at_ms: u64,
    profile: Option<String>,
    account_label: Option<String>,
    account_identity_key: Option<String>,
    #[serde(default)]
    account_id_alias: Option<String>,
    #[serde(default)]
    email_alias: Option<String>,
    openai_base_url: Option<String>,
    official_source: Option<String>,
    is_official_openai: bool,
    exclusion_reason: Option<String>,
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
}

impl AttributionRecord {
    fn context(&self) -> Option<AttributionContext> {
        let account_identity_key = self.account_identity_key.clone()?;
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
            account_identity_key: Some(account_identity_key),
            account_id_alias: identity::validated_account_id_alias(
                self.account_id_alias.as_deref(),
            ),
            email_alias: identity::validated_email_alias(self.email_alias.as_deref()),
            legacy_account_key: None,
            identity_source: Some("attribution_record".to_string()),
            identity_observed_at_ms: Some(self.observed_at_ms),
            ledger_layer: LEDGER_LAYER_STRICT_PROVEN.to_string(),
            layer_reason: Some("account identity key recorded with official source".to_string()),
            attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string(),
            attribution_window_id: Some(self.observed_at_ms.to_string()),
            model_reasoning_effort: self.model_reasoning_effort.clone(),
            model_verbosity: self.model_verbosity.clone(),
        })
    }

    fn legacy_context(&self) -> Option<AttributionContext> {
        if !self.is_official_openai || self.account_identity_key.is_some() {
            return None;
        }
        let profile = self.profile.as_deref()?.trim();
        if profile.is_empty() {
            return None;
        }
        let attribution_window_id = self.observed_at_ms.to_string();
        Some(AttributionContext {
            profile: profile.to_string(),
            account_label: self.account_label.clone(),
            official_source: self
                .official_source
                .clone()
                .unwrap_or_else(|| "OpenAI default".to_string()),
            attribution_since_ms: self.observed_at_ms,
            account_identity_key: None,
            account_id_alias: None,
            email_alias: None,
            legacy_account_key: Some(legacy_account_key(profile, &attribution_window_id)),
            identity_source: Some("legacy_attribution_record".to_string()),
            identity_observed_at_ms: Some(self.observed_at_ms),
            ledger_layer: LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED.to_string(),
            layer_reason: Some(
                "official source and profile were recorded before account identity keys existed"
                    .to_string(),
            ),
            attribution_kind: ATTRIBUTION_KIND_LEGACY_PROFILE.to_string(),
            attribution_window_id: Some(attribution_window_id),
            model_reasoning_effort: self.model_reasoning_effort.clone(),
            model_verbosity: self.model_verbosity.clone(),
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
    let resolved = load_resolved_usage_facts(&conn, &codex_home, &query)?;
    let summary = build_summary_from_facts_with_resolver(
        summary,
        &resolved.facts,
        &query,
        Some(&resolved.resolver),
    );
    attach_layer_summaries(&conn, &query, summary, &resolved.resolver)
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
    let resolved = load_resolved_usage_facts(&conn, &codex_home, &query)?;
    let facts = &resolved.facts;
    let account_query = account_visibility_query(&query);
    let account_facts = load_usage_facts_with_resolver(&conn, &account_query, &resolved.resolver)?;
    let account_summary = build_summary_from_facts_with_resolver(
        new_usage_summary(&account_query),
        &account_facts,
        &account_query,
        Some(&resolved.resolver),
    )
    .accounts;
    let model_query = without_model_filter(&query);
    let model_facts = load_usage_facts_with_resolver(&conn, &model_query, &resolved.resolver)?;
    let model_options = build_summary_from_facts_with_resolver(
        new_usage_summary(&model_query),
        &model_facts,
        &model_query,
        Some(&resolved.resolver),
    )
    .models;
    let summary = attach_layer_summaries(
        &conn,
        &query,
        build_summary_from_facts_with_resolver(summary, facts, &query, Some(&resolved.resolver)),
        &resolved.resolver,
    )?;
    let trends = build_trends(facts, &query);
    let logs = build_request_log_page(facts, page, page_size);
    let data_sources = build_data_source_summaries(facts);
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
        pricing_source_url: PRICING_SOURCE_URL.to_string(),
        pricing_checked_at: PRICING_CHECKED_AT.to_string(),
        scope: normalized_usage_scope(query).to_string(),
        range_start_ms: query.start_date.and_then(seconds_to_ms),
        range_end_ms: query.end_date.and_then(end_seconds_to_ms),
        ..OpenAiUsageSummary::default()
    }
}

fn normalized_usage_scope(query: &OpenAiUsageQuery) -> &'static str {
    match query
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(SCOPE_LEGACY) => SCOPE_LEGACY,
        Some(SCOPE_COMBINED_ATTRIBUTED) => SCOPE_COMBINED_ATTRIBUTED,
        Some(SCOPE_CANDIDATE) => SCOPE_CANDIDATE,
        Some(SCOPE_EXCLUDED) => SCOPE_EXCLUDED,
        _ => SCOPE_STRICT,
    }
}

fn query_with_scope(query: &OpenAiUsageQuery, scope: &str) -> OpenAiUsageQuery {
    let mut next = query.clone();
    next.scope = Some(scope.to_string());
    next
}

fn without_account_filter(query: &OpenAiUsageQuery) -> OpenAiUsageQuery {
    let mut next = query.clone();
    next.profile = None;
    next.account_group_key = None;
    next.account_label = None;
    next
}

fn account_visibility_query(query: &OpenAiUsageQuery) -> OpenAiUsageQuery {
    let mut next = without_account_filter(query);
    if normalized_usage_scope(query) == SCOPE_STRICT {
        next.scope = Some(SCOPE_COMBINED_ATTRIBUTED.to_string());
    }
    next
}

fn without_model_filter(query: &OpenAiUsageQuery) -> OpenAiUsageQuery {
    let mut next = query.clone();
    next.model = None;
    next
}

struct ResolvedUsageFacts {
    facts: Vec<UsageFact>,
    resolver: identity::CanonicalAccountResolver,
}

fn load_resolved_usage_facts(
    conn: &Connection,
    codex_home: &Path,
    query: &OpenAiUsageQuery,
) -> AppResult<ResolvedUsageFacts> {
    let resolver_query = without_model_filter(&account_visibility_query(query));
    let mut attributed_query = resolver_query.clone();
    attributed_query.scope = Some(SCOPE_COMBINED_ATTRIBUTED.to_string());
    let mut universe = load_usage_facts(conn, &attributed_query)?;
    if matches!(
        normalized_usage_scope(query),
        SCOPE_CANDIDATE | SCOPE_EXCLUDED
    ) {
        universe.extend(load_usage_facts(conn, &resolver_query)?);
    }
    let label_identities = strict_label_identities(&universe);
    let fallback_keys = universe
        .iter()
        .map(|fact| account_group_key(fact, &label_identities))
        .collect::<Vec<_>>();
    let verified_profile_aliases = load_verified_profile_aliases(codex_home)?;
    let resolver = identity::CanonicalAccountResolver::from_facts(
        &universe,
        &fallback_keys,
        &verified_profile_aliases,
    );
    let facts = load_usage_facts_with_resolver(conn, query, &resolver)?;
    Ok(ResolvedUsageFacts { facts, resolver })
}

fn load_usage_facts_with_resolver(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    resolver: &identity::CanonicalAccountResolver,
) -> AppResult<Vec<UsageFact>> {
    let mut sql_query = query.clone();
    sql_query.account_group_key = None;
    sql_query.account_label = None;
    let facts = load_usage_facts(conn, &sql_query)?
        .into_iter()
        .filter(|fact| fact_matches_query(fact, query))
        .collect();
    Ok(filter_facts_by_account_group(facts, query, resolver))
}

fn load_verified_profile_aliases(codex_home: &Path) -> AppResult<identity::VerifiedProfileAliases> {
    let index = load_profiles_index(Some(codex_home))?;
    let backup_root = get_backup_root(Some(codex_home));
    Ok(index
        .profiles
        .into_iter()
        .filter_map(|profile| {
            let auth_path = backup_root.join(&profile.folder_name).join("auth.json");
            let identity = load_account_identity_from_path(&auth_path)?;
            let (v1_key, aliases) = identity::verified_identity_aliases(&identity)?;
            Some(((profile.folder_name, v1_key), aliases))
        })
        .collect())
}

fn build_summary_from_facts_with_resolver(
    mut summary: OpenAiUsageSummary,
    facts: &[UsageFact],
    query: &OpenAiUsageQuery,
    account_resolver: Option<&identity::CanonicalAccountResolver>,
) -> OpenAiUsageSummary {
    let mut accounts: BTreeMap<String, AccountAggregate> = BTreeMap::new();
    let mut global_models: BTreeMap<String, ModelAggregate> = BTreeMap::new();
    let label_identities = strict_label_identities(facts);

    for fact in facts.iter().filter(|fact| fact_matches_query(fact, query)) {
        add_fact_usage(
            &mut accounts,
            &mut global_models,
            &label_identities,
            account_resolver,
            fact,
        );
    }

    let total = total_account_aggregate(&accounts);
    summary.total_requests = total.requests;
    summary.tokens = total.tokens;
    summary.cost = total.cost;
    summary.cache_hit_rate = cache_hit_rate(&summary.tokens);
    summary.accounts = accounts
        .into_iter()
        .map(|(_, account)| account_to_summary(account))
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
    let resolved = load_resolved_usage_facts(&conn, &codex_home, &query)?;
    Ok(build_trends(&resolved.facts, &query))
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
    let resolved = load_resolved_usage_facts(&conn, &codex_home, &query)?;
    Ok(build_request_log_page(&resolved.facts, page, page_size))
}

pub fn load_openai_usage_request_detail(
    codex_home: Option<&Path>,
    request_id: &str,
) -> AppResult<Option<OpenAiUsageRequestLog>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let mut summary = new_usage_summary(&OpenAiUsageQuery::default());
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    Ok(load_usage_facts(
        &conn,
        &query_with_scope(&OpenAiUsageQuery::default(), SCOPE_COMBINED_ATTRIBUTED),
    )?
    .iter()
    .find(|fact| fact.request_id == request_id)
    .map(fact_to_log))
}

pub fn load_openai_usage_data_sources(
    codex_home: Option<&Path>,
    query: Option<OpenAiUsageQuery>,
) -> AppResult<Vec<OpenAiUsageDataSourceSummary>> {
    let codex_home = codex_home.map(PathBuf::from).unwrap_or_else(get_codex_home);
    let query = query.unwrap_or_default();
    let mut summary = new_usage_summary(&query);
    let conn = sync_openai_usage_ledger(&codex_home, &mut summary)?;
    let resolved = load_resolved_usage_facts(&conn, &codex_home, &query)?;
    Ok(build_data_source_summaries(&resolved.facts))
}

fn build_request_log_page(
    facts: &[UsageFact],
    page: u32,
    page_size: u32,
) -> OpenAiUsageRequestLogPage {
    let mut logs = facts.iter().map(fact_to_log).collect::<Vec<_>>();
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

fn build_data_source_summaries(facts: &[UsageFact]) -> Vec<OpenAiUsageDataSourceSummary> {
    let mut sources: BTreeMap<String, Aggregate> = BTreeMap::new();
    for fact in facts {
        add_to_aggregate(
            sources.entry(fact.data_source.clone()).or_default(),
            fact.delta,
            &fact.cost.breakdown,
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

fn finalize_usage_load(
    _events: Vec<UsageEvent>,
    _facts: Vec<UsageFact>,
    mut evidence: Vec<UsageEvidence>,
    mut decisions: EvidenceDecisionCounts,
    summary: &mut OpenAiUsageSummary,
) -> UsageLoadResult {
    resolve_accepted_evidence_by_priority(&mut evidence);

    let mut events = Vec::new();
    let mut facts = Vec::new();
    for entry in evidence
        .iter()
        .filter(|entry| entry.decision == DECISION_ACCEPTED)
    {
        let Some(model) = entry.model.as_deref() else {
            continue;
        };
        let Some(delta) = entry.delta else {
            continue;
        };
        let cost = calculate_cost(model, delta);
        let decision = EvidenceClassifier::accept_session_evidence(entry.clone(), cost);
        if let Some(event) = decision.accepted {
            events.push(event);
        }
        if let Some(fact) = decision.fact {
            facts.push(fact);
        }
    }

    decisions.accepted = evidence
        .iter()
        .filter(|entry| entry.decision == DECISION_ACCEPTED)
        .count() as u64;
    decisions.candidate = evidence
        .iter()
        .filter(|entry| entry.decision == DECISION_CANDIDATE)
        .count() as u64;
    decisions.rejected = evidence
        .iter()
        .filter(|entry| entry.decision == DECISION_REJECTED)
        .count() as u64;
    decisions.superseded = evidence
        .iter()
        .filter(|entry| entry.decision == DECISION_SUPERSEDED)
        .count() as u64;
    decisions.strict_proven = facts
        .iter()
        .filter(|entry| entry.ledger_layer == LEDGER_LAYER_STRICT_PROVEN)
        .count() as u64;
    decisions.legacy_profile_attributed = facts
        .iter()
        .filter(|entry| entry.ledger_layer == LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED)
        .count() as u64;
    decisions.candidate_unattributed = evidence
        .iter()
        .filter(|entry| entry.ledger_layer == LEDGER_LAYER_CANDIDATE_UNATTRIBUTED)
        .count() as u64;
    decisions.rejected_invalid = evidence
        .iter()
        .filter(|entry| entry.ledger_layer == LEDGER_LAYER_REJECTED_INVALID)
        .count() as u64;
    summary.exclusions.accepted_events = decisions.accepted;
    summary.exclusions.candidate_events = decisions.candidate;
    summary.exclusions.rejected_events = decisions.rejected;
    summary.exclusions.superseded_events = decisions.superseded;
    summary.exclusions.strict_proven_events = decisions.strict_proven;
    summary.exclusions.legacy_profile_attributed_events = decisions.legacy_profile_attributed;
    summary.exclusions.candidate_unattributed_events = decisions.candidate_unattributed;
    summary.exclusions.rejected_invalid_events = decisions.rejected_invalid;
    UsageLoadResult {
        source_signature: String::new(),
        cache_hit: false,
        events,
        facts,
        evidence,
        decisions,
    }
}

fn resolve_accepted_evidence_by_priority(evidence: &mut [UsageEvidence]) {
    let mut request_winners: HashMap<String, usize> = HashMap::new();
    let mut fingerprint_winners: HashMap<String, usize> = HashMap::new();

    for index in 0..evidence.len() {
        if evidence[index].decision != DECISION_ACCEPTED {
            continue;
        }

        let duplicate = evidence[index]
            .request_id
            .as_ref()
            .and_then(|request_id| request_winners.get(request_id).copied())
            .or_else(|| {
                evidence[index]
                    .fingerprint
                    .as_ref()
                    .and_then(|fingerprint| fingerprint_winners.get(fingerprint).copied())
            });

        let Some(existing_index) = duplicate else {
            remember_evidence_winner(
                evidence,
                index,
                &mut request_winners,
                &mut fingerprint_winners,
            );
            continue;
        };

        if evidence[index].source_priority > evidence[existing_index].source_priority {
            let winner_id = evidence[index].evidence_id.clone();
            if let Some(request_id) = evidence[existing_index].request_id.as_ref() {
                request_winners.insert(request_id.clone(), index);
            }
            if let Some(fingerprint) = evidence[existing_index].fingerprint.as_ref() {
                fingerprint_winners.insert(fingerprint.clone(), index);
            }
            mark_evidence_superseded(
                &mut evidence[existing_index],
                &winner_id,
                "superseded by higher-priority evidence source",
            );
            remember_evidence_winner(
                evidence,
                index,
                &mut request_winners,
                &mut fingerprint_winners,
            );
        } else {
            let winner_id = evidence[existing_index].evidence_id.clone();
            let reason =
                if evidence[index].source_priority == evidence[existing_index].source_priority {
                    "duplicate evidence from same-priority source"
                } else {
                    "duplicate evidence from lower-priority source"
                };
            mark_evidence_superseded(&mut evidence[index], &winner_id, reason);
        }
    }
}

fn remember_evidence_winner(
    evidence: &[UsageEvidence],
    index: usize,
    request_winners: &mut HashMap<String, usize>,
    fingerprint_winners: &mut HashMap<String, usize>,
) {
    if let Some(request_id) = evidence[index].request_id.as_ref() {
        request_winners.insert(request_id.clone(), index);
    }
    if let Some(fingerprint) = evidence[index].fingerprint.as_ref() {
        fingerprint_winners.insert(fingerprint.clone(), index);
    }
}

fn mark_evidence_superseded(entry: &mut UsageEvidence, winner_id: &str, reason: &str) {
    entry.decision = DECISION_SUPERSEDED.to_string();
    entry.decision_reason = Some(reason.to_string());
    entry.superseded_by = Some(winner_id.to_string());
    entry.ledger_layer = LEDGER_LAYER_SUPERSEDED.to_string();
    entry.layer_reason = Some(reason.to_string());
}

fn sync_openai_usage_ledger(
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
) -> AppResult<Connection> {
    let mut conn = open_usage_ledger(codex_home)?;
    ensure_usage_ledger_schema(&conn)?;
    let usage = ingestion::ingest_usage(codex_home, summary, |source_signature, summary| {
        apply_cached_usage_sync_state(&conn, source_signature, summary)
    })?;
    if usage.cache_hit {
        return Ok(conn);
    }
    rebuild_usage_events(&mut conn, &usage, &usage.source_signature, summary)?;
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
    drop_legacy_usage_events_table_if_needed(conn)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS openai_usage_events (
            request_id TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL UNIQUE,
            evidence_id TEXT,
            session_id TEXT NOT NULL,
            profile TEXT NOT NULL,
            account_label TEXT,
            account_identity_key TEXT NOT NULL,
            official_source TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT,
            model_reasoning_effort TEXT,
            model_verbosity TEXT,
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
            pricing_model TEXT,
            pricing_tier TEXT,
            pricing_source_url TEXT,
            pricing_version TEXT,
            status_code INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            data_source TEXT NOT NULL,
            source_type TEXT,
            source_priority INTEGER NOT NULL DEFAULT 0,
            usage_confidence TEXT,
            inserted_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_created_at
            ON openai_usage_events(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_profile
            ON openai_usage_events(profile);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_model
            ON openai_usage_events(model);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_account_identity
            ON openai_usage_events(account_identity_key);
        CREATE TABLE IF NOT EXISTS openai_usage_evidence (
            evidence_id TEXT PRIMARY KEY,
            request_id TEXT,
            fingerprint TEXT,
            session_id TEXT,
            source_type TEXT NOT NULL,
            source_priority INTEGER NOT NULL,
            source_locator TEXT NOT NULL,
            source_hash TEXT NOT NULL,
            parser_version TEXT NOT NULL,
            event_started_at_ms INTEGER,
            event_completed_at_ms INTEGER,
            observed_at_ms INTEGER,
            turn_index INTEGER,
            response_id TEXT,
            previous_response_id TEXT,
            model TEXT,
            request_model TEXT,
            pricing_model TEXT,
            model_reasoning_effort TEXT,
            model_verbosity TEXT,
            pricing_tier TEXT,
            pricing_source_url TEXT,
            usage_shape TEXT NOT NULL,
            usage_confidence TEXT NOT NULL,
            input_tokens INTEGER,
            cached_input_tokens INTEGER,
            output_tokens INTEGER,
            reasoning_output_tokens INTEGER,
            raw_usage_json TEXT,
            profile TEXT,
            account_identity_key TEXT,
            account_id_alias TEXT,
            email_alias TEXT,
            account_label TEXT,
            identity_source TEXT,
            identity_observed_at_ms INTEGER,
            account_confidence TEXT NOT NULL,
            effective_base_url TEXT,
            official_source TEXT,
            official_confidence TEXT NOT NULL,
            status_code INTEGER,
            decision TEXT NOT NULL,
            decision_reason TEXT,
            superseded_by TEXT,
            ledger_layer TEXT NOT NULL DEFAULT 'rejected_invalid',
            layer_reason TEXT,
            attribution_kind TEXT NOT NULL DEFAULT 'unattributed',
            attribution_window_id TEXT,
            legacy_account_key TEXT,
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_openai_usage_evidence_decision
            ON openai_usage_evidence(decision);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_evidence_source
            ON openai_usage_evidence(source_type);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_evidence_created_at
            ON openai_usage_evidence(created_at_ms);
        CREATE TABLE IF NOT EXISTS openai_usage_facts (
            evidence_id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL,
            fingerprint TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            profile TEXT NOT NULL,
            account_label TEXT,
            account_identity_key TEXT,
            account_id_alias TEXT,
            email_alias TEXT,
            legacy_account_key TEXT,
            official_source TEXT NOT NULL,
            ledger_layer TEXT NOT NULL,
            layer_reason TEXT,
            attribution_kind TEXT NOT NULL,
            attribution_window_id TEXT,
            model TEXT NOT NULL,
            request_model TEXT,
            model_reasoning_effort TEXT,
            model_verbosity TEXT,
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
            pricing_model TEXT,
            pricing_tier TEXT,
            pricing_source_url TEXT,
            pricing_version TEXT,
            status_code INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            data_source TEXT NOT NULL,
            source_type TEXT,
            source_priority INTEGER NOT NULL DEFAULT 0,
            usage_confidence TEXT,
            inserted_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS openai_usage_sync_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            source_signature TEXT NOT NULL,
            evidence_source_signature TEXT,
            parser_version TEXT,
            classifier_version TEXT,
            dedupe_version TEXT,
            ledger_schema_version TEXT,
            layer_classifier_version TEXT,
            attribution_backfill_version TEXT,
            pricing_version TEXT,
            pricing_checked_at TEXT NOT NULL,
            synced_at_ms INTEGER NOT NULL,
            attribution_since_ms INTEGER,
            accepted_events INTEGER NOT NULL DEFAULT 0,
            candidate_events INTEGER NOT NULL DEFAULT 0,
            rejected_events INTEGER NOT NULL DEFAULT 0,
            superseded_events INTEGER NOT NULL DEFAULT 0,
            strict_proven_events INTEGER NOT NULL DEFAULT 0,
            legacy_profile_attributed_events INTEGER NOT NULL DEFAULT 0,
            candidate_unattributed_events INTEGER NOT NULL DEFAULT 0,
            rejected_invalid_events INTEGER NOT NULL DEFAULT 0,
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
    ensure_usage_events_columns(conn)?;
    ensure_usage_events_account_identity_not_null(conn)?;
    ensure_usage_evidence_columns(conn)?;
    ensure_usage_facts_columns(conn)?;
    ensure_usage_projection_indexes(conn)?;
    ensure_usage_sync_state_columns(conn)
}

fn drop_legacy_usage_events_table_if_needed(conn: &Connection) -> AppResult<()> {
    if !table_exists(conn, "openai_usage_events")? {
        return Ok(());
    }
    let columns = table_columns(conn, "openai_usage_events")?;
    let required_columns = [
        "request_id",
        "fingerprint",
        "created_at_ms",
        "profile",
        "model",
        "account_identity_key",
    ];
    let missing_required_column = required_columns
        .iter()
        .any(|column| !columns.contains(*column));
    let nullable_account_key =
        !table_column_not_null(conn, "openai_usage_events", "account_identity_key")?;
    if !missing_required_column && !nullable_account_key {
        return Ok(());
    }
    conn.execute("DROP TABLE IF EXISTS openai_usage_events", [])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVENTS_SCHEMA_DROP_FAILED",
                format!("Failed to drop legacy usage events table: {error}"),
            )
        })?;
    if table_exists(conn, "openai_usage_sync_state")? {
        conn.execute("DELETE FROM openai_usage_sync_state WHERE id = 1", [])
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_SYNC_STATE_RESET_FAILED",
                    format!("Failed to reset usage sync state after schema rebuild: {error}"),
                )
            })?;
    }
    Ok(())
}

fn ensure_usage_events_columns(conn: &Connection) -> AppResult<()> {
    let columns = table_columns(conn, "openai_usage_events")?;
    for (name, definition) in [
        ("evidence_id", "evidence_id TEXT"),
        ("account_identity_key", "account_identity_key TEXT"),
        ("request_model", "request_model TEXT"),
        ("model_reasoning_effort", "model_reasoning_effort TEXT"),
        ("model_verbosity", "model_verbosity TEXT"),
        ("pricing_model", "pricing_model TEXT"),
        ("pricing_tier", "pricing_tier TEXT"),
        ("pricing_source_url", "pricing_source_url TEXT"),
        ("pricing_version", "pricing_version TEXT"),
        ("source_type", "source_type TEXT"),
        (
            "source_priority",
            "source_priority INTEGER NOT NULL DEFAULT 0",
        ),
        ("usage_confidence", "usage_confidence TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE openai_usage_events ADD COLUMN {definition}"),
                [],
            )
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_EVENTS_SCHEMA_ALTER_FAILED",
                    format!("Failed to add usage event column {name}: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

fn ensure_usage_events_account_identity_not_null(conn: &Connection) -> AppResult<()> {
    if table_column_not_null(conn, "openai_usage_events", "account_identity_key")? {
        return Ok(());
    }
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS openai_usage_events;
        CREATE TABLE openai_usage_events (
            request_id TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL UNIQUE,
            evidence_id TEXT,
            session_id TEXT NOT NULL,
            profile TEXT NOT NULL,
            account_label TEXT,
            account_identity_key TEXT NOT NULL,
            official_source TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT,
            model_reasoning_effort TEXT,
            model_verbosity TEXT,
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
            pricing_model TEXT,
            pricing_tier TEXT,
            pricing_source_url TEXT,
            pricing_version TEXT,
            status_code INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            data_source TEXT NOT NULL,
            source_type TEXT,
            source_priority INTEGER NOT NULL DEFAULT 0,
            usage_confidence TEXT,
            inserted_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_created_at
            ON openai_usage_events(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_profile
            ON openai_usage_events(profile);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_model
            ON openai_usage_events(model);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_events_account_identity
            ON openai_usage_events(account_identity_key);
        DELETE FROM openai_usage_sync_state WHERE id = 1;
        "#,
    )
    .map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_EVENTS_SCHEMA_REBUILD_FAILED",
            format!(
                "Failed to rebuild usage events table with account identity constraint: {error}"
            ),
        )
    })?;
    Ok(())
}

fn ensure_usage_evidence_columns(conn: &Connection) -> AppResult<()> {
    let columns = table_columns(conn, "openai_usage_evidence")?;
    for (name, definition) in [
        (
            "ledger_layer",
            "ledger_layer TEXT NOT NULL DEFAULT 'rejected_invalid'",
        ),
        ("layer_reason", "layer_reason TEXT"),
        (
            "attribution_kind",
            "attribution_kind TEXT NOT NULL DEFAULT 'unattributed'",
        ),
        ("attribution_window_id", "attribution_window_id TEXT"),
        ("legacy_account_key", "legacy_account_key TEXT"),
        ("account_id_alias", "account_id_alias TEXT"),
        ("email_alias", "email_alias TEXT"),
        ("model_reasoning_effort", "model_reasoning_effort TEXT"),
        ("model_verbosity", "model_verbosity TEXT"),
        ("pricing_tier", "pricing_tier TEXT"),
        ("pricing_source_url", "pricing_source_url TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE openai_usage_evidence ADD COLUMN {definition}"),
                [],
            )
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_EVIDENCE_SCHEMA_ALTER_FAILED",
                    format!("Failed to add usage evidence column {name}: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

fn ensure_usage_facts_columns(conn: &Connection) -> AppResult<()> {
    let columns = table_columns(conn, "openai_usage_facts")?;
    for (name, definition) in [
        ("legacy_account_key", "legacy_account_key TEXT"),
        ("account_id_alias", "account_id_alias TEXT"),
        ("email_alias", "email_alias TEXT"),
        (
            "ledger_layer",
            "ledger_layer TEXT NOT NULL DEFAULT 'strict_proven'",
        ),
        ("layer_reason", "layer_reason TEXT"),
        (
            "attribution_kind",
            "attribution_kind TEXT NOT NULL DEFAULT 'strict_account'",
        ),
        ("attribution_window_id", "attribution_window_id TEXT"),
        ("request_model", "request_model TEXT"),
        ("model_reasoning_effort", "model_reasoning_effort TEXT"),
        ("model_verbosity", "model_verbosity TEXT"),
        ("pricing_model", "pricing_model TEXT"),
        ("pricing_tier", "pricing_tier TEXT"),
        ("pricing_source_url", "pricing_source_url TEXT"),
        ("pricing_version", "pricing_version TEXT"),
        ("source_type", "source_type TEXT"),
        (
            "source_priority",
            "source_priority INTEGER NOT NULL DEFAULT 0",
        ),
        ("usage_confidence", "usage_confidence TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE openai_usage_facts ADD COLUMN {definition}"),
                [],
            )
            .map_err(|error| {
                AppError::new(
                    "OPENAI_USAGE_FACTS_SCHEMA_ALTER_FAILED",
                    format!("Failed to add usage fact column {name}: {error}"),
                )
            })?;
        }
    }
    Ok(())
}

fn ensure_usage_projection_indexes(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_openai_usage_evidence_layer
            ON openai_usage_evidence(ledger_layer);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_created_at
            ON openai_usage_facts(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_profile
            ON openai_usage_facts(profile);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_model
            ON openai_usage_facts(model);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_layer
            ON openai_usage_facts(ledger_layer);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_account_identity
            ON openai_usage_facts(account_identity_key);
        CREATE INDEX IF NOT EXISTS idx_openai_usage_facts_legacy_account
            ON openai_usage_facts(legacy_account_key);
        "#,
    )
    .map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_PROJECTION_INDEX_FAILED",
            format!("Failed to initialize usage projection indexes: {error}"),
        )
    })?;
    Ok(())
}

fn ensure_usage_sync_state_columns(conn: &Connection) -> AppResult<()> {
    let columns = table_columns(conn, "openai_usage_sync_state")?;

    for (name, definition) in [
        (
            "evidence_source_signature",
            "evidence_source_signature TEXT",
        ),
        ("parser_version", "parser_version TEXT"),
        ("classifier_version", "classifier_version TEXT"),
        ("dedupe_version", "dedupe_version TEXT"),
        ("pricing_version", "pricing_version TEXT"),
        ("ledger_schema_version", "ledger_schema_version TEXT"),
        ("layer_classifier_version", "layer_classifier_version TEXT"),
        (
            "attribution_backfill_version",
            "attribution_backfill_version TEXT",
        ),
        (
            "accepted_events",
            "accepted_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "candidate_events",
            "candidate_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "rejected_events",
            "rejected_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "superseded_events",
            "superseded_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "strict_proven_events",
            "strict_proven_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "legacy_profile_attributed_events",
            "legacy_profile_attributed_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "candidate_unattributed_events",
            "candidate_unattributed_events INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "rejected_invalid_events",
            "rejected_invalid_events INTEGER NOT NULL DEFAULT 0",
        ),
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

fn table_columns(conn: &Connection, table: &str) -> AppResult<HashSet<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_SCHEMA_PREPARE_FAILED",
                format!("Failed to inspect {table} schema: {error}"),
            )
        })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SYNC_STATE_SCHEMA_QUERY_FAILED",
                format!("Failed to query {table} schema: {error}"),
            )
        })?;
    let columns = rows.collect::<Result<HashSet<_>, _>>().map_err(|error| {
        AppError::new(
            "OPENAI_USAGE_SYNC_STATE_SCHEMA_ROW_FAILED",
            format!("Failed to read {table} schema: {error}"),
        )
    })?;
    Ok(columns)
}

fn table_exists(conn: &Connection, table: &str) -> AppResult<bool> {
    let count = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SCHEMA_TABLE_LOOKUP_FAILED",
                format!("Failed to inspect table {table}: {error}"),
            )
        })?;
    Ok(count > 0)
}

fn table_column_not_null(conn: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SCHEMA_PREPARE_FAILED",
                format!("Failed to inspect {table} schema: {error}"),
            )
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)? != 0))
        })
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SCHEMA_QUERY_FAILED",
                format!("Failed to query {table} schema: {error}"),
            )
        })?;
    for row in rows {
        let (name, not_null) = row.map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_SCHEMA_ROW_FAILED",
                format!("Failed to read {table} schema: {error}"),
            )
        })?;
        if name == column {
            return Ok(not_null);
        }
    }
    Ok(false)
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
                   unattributed, missing_model, invalid_delta, malformed_lines,
                   parser_version, classifier_version, dedupe_version, pricing_version,
                   accepted_events, candidate_events, rejected_events, superseded_events,
                   ledger_schema_version, layer_classifier_version, attribution_backfill_version,
                   strict_proven_events, legacy_profile_attributed_events,
                   candidate_unattributed_events, rejected_invalid_events
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
                accepted_events: u64_from_i64(row.get::<_, i64>(14)?),
                candidate_events: u64_from_i64(row.get::<_, i64>(15)?),
                rejected_events: u64_from_i64(row.get::<_, i64>(16)?),
                superseded_events: u64_from_i64(row.get::<_, i64>(17)?),
                strict_proven_events: u64_from_i64(row.get::<_, i64>(21)?),
                legacy_profile_attributed_events: u64_from_i64(row.get::<_, i64>(22)?),
                candidate_unattributed_events: u64_from_i64(row.get::<_, i64>(23)?),
                rejected_invalid_events: u64_from_i64(row.get::<_, i64>(24)?),
            },
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(18)?,
            row.get::<_, Option<String>>(19)?,
            row.get::<_, Option<String>>(20)?,
        ))
    });
    match result {
        Ok((
            stored_signature,
            stored_pricing_checked_at,
            attribution_since_ms,
            exclusions,
            parser_version,
            classifier_version,
            dedupe_version,
            pricing_version,
            ledger_schema_version,
            layer_classifier_version,
            attribution_backfill_version,
        )) => {
            let current = stored_signature == source_signature
                && stored_pricing_checked_at == PRICING_CHECKED_AT
                && parser_version.as_deref() == Some(EVIDENCE_PARSER_VERSION)
                && classifier_version.as_deref() == Some(EVIDENCE_CLASSIFIER_VERSION)
                && dedupe_version.as_deref() == Some(EVIDENCE_DEDUPE_VERSION)
                && ledger_schema_version.as_deref() == Some(LEDGER_SCHEMA_VERSION)
                && layer_classifier_version.as_deref() == Some(LAYER_CLASSIFIER_VERSION)
                && attribution_backfill_version.as_deref() == Some(ATTRIBUTION_BACKFILL_VERSION)
                && pricing_version.as_deref() == Some(PRICING_VERSION);
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

fn rebuild_usage_events(
    conn: &mut Connection,
    usage: &UsageLoadResult,
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
    tx.execute("DELETE FROM openai_usage_evidence", [])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_CLEAR_FAILED",
                format!("Failed to clear usage evidence ledger: {error}"),
            )
        })?;
    tx.execute("DELETE FROM openai_usage_facts", [])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_CLEAR_FAILED",
                format!("Failed to clear usage facts projection: {error}"),
            )
        })?;
    insert_usage_evidence(&tx, &usage.evidence)?;
    insert_usage_facts(&tx, &usage.facts)?;
    {
        let mut stmt = tx
            .prepare(
                r#"
            INSERT INTO openai_usage_events (
                request_id, fingerprint, evidence_id, session_id, profile, account_label,
                account_identity_key, official_source, model, request_model,
                model_reasoning_effort, model_verbosity, input_tokens, cached_input_tokens,
                output_tokens, reasoning_output_tokens, real_total_tokens,
                input_cost_micros, cached_input_cost_micros, output_cost_micros,
                total_cost_micros, unpriced_tokens, priced, pricing_note,
                pricing_source, pricing_checked_at, pricing_model, pricing_tier,
                pricing_source_url, pricing_version,
                status_code, created_at_ms, data_source, source_type, source_priority,
                usage_confidence, inserted_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35,
                ?36, ?37
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
        for event in &usage.events {
            stmt.execute(params![
                &event.request_id,
                &event.fingerprint,
                &event.evidence_id,
                &event.session_id,
                &event.profile,
                event.account_label.as_deref(),
                &event.account_identity_key,
                &event.official_source,
                &event.model,
                event.requested_model.as_deref(),
                event.model_reasoning_effort.as_deref(),
                event.model_verbosity.as_deref(),
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
                &event.pricing_model,
                &event.pricing_tier,
                &event.pricing_source_url,
                PRICING_VERSION,
                i64::from(event.status_code),
                i64_from_u64(event.created_at_ms),
                &event.data_source,
                &event.source_type,
                event.source_priority,
                &event.usage_confidence,
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
            id, source_signature, evidence_source_signature, parser_version,
            classifier_version, dedupe_version, ledger_schema_version,
            layer_classifier_version, attribution_backfill_version,
            pricing_version, pricing_checked_at,
            synced_at_ms, attribution_since_ms, accepted_events, candidate_events,
            rejected_events, superseded_events, strict_proven_events,
            legacy_profile_attributed_events, candidate_unattributed_events,
            rejected_invalid_events, duplicate_events, non_official_base_url,
            unproven_history, unattributed, missing_model, invalid_delta, malformed_lines
        ) VALUES (1, ?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)
        ON CONFLICT(id) DO UPDATE SET
            source_signature = excluded.source_signature,
            evidence_source_signature = excluded.evidence_source_signature,
            parser_version = excluded.parser_version,
            classifier_version = excluded.classifier_version,
            dedupe_version = excluded.dedupe_version,
            ledger_schema_version = excluded.ledger_schema_version,
            layer_classifier_version = excluded.layer_classifier_version,
            attribution_backfill_version = excluded.attribution_backfill_version,
            pricing_version = excluded.pricing_version,
            pricing_checked_at = excluded.pricing_checked_at,
            synced_at_ms = excluded.synced_at_ms,
            attribution_since_ms = excluded.attribution_since_ms,
            accepted_events = excluded.accepted_events,
            candidate_events = excluded.candidate_events,
            rejected_events = excluded.rejected_events,
            superseded_events = excluded.superseded_events,
            strict_proven_events = excluded.strict_proven_events,
            legacy_profile_attributed_events = excluded.legacy_profile_attributed_events,
            candidate_unattributed_events = excluded.candidate_unattributed_events,
            rejected_invalid_events = excluded.rejected_invalid_events,
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
            EVIDENCE_PARSER_VERSION,
            EVIDENCE_CLASSIFIER_VERSION,
            EVIDENCE_DEDUPE_VERSION,
            LEDGER_SCHEMA_VERSION,
            LAYER_CLASSIFIER_VERSION,
            ATTRIBUTION_BACKFILL_VERSION,
            PRICING_VERSION,
            PRICING_CHECKED_AT,
            i64_from_u64(current_time_ms()),
            summary.attribution_since_ms.map(i64_from_u64),
            i64_from_u64(usage.decisions.accepted),
            i64_from_u64(usage.decisions.candidate),
            i64_from_u64(usage.decisions.rejected),
            i64_from_u64(usage.decisions.superseded),
            i64_from_u64(usage.decisions.strict_proven),
            i64_from_u64(usage.decisions.legacy_profile_attributed),
            i64_from_u64(usage.decisions.candidate_unattributed),
            i64_from_u64(usage.decisions.rejected_invalid),
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

fn insert_usage_evidence(tx: &Transaction<'_>, evidence: &[UsageEvidence]) -> AppResult<()> {
    let mut stmt = tx
        .prepare(
            r#"
            INSERT INTO openai_usage_evidence (
                evidence_id, request_id, fingerprint, session_id, source_type,
                source_priority, source_locator, source_hash, parser_version,
                event_started_at_ms, event_completed_at_ms, observed_at_ms,
                turn_index, response_id, previous_response_id, model, request_model,
                pricing_model, model_reasoning_effort, model_verbosity, pricing_tier,
                pricing_source_url, usage_shape, usage_confidence, input_tokens,
                cached_input_tokens, output_tokens, reasoning_output_tokens,
                raw_usage_json, profile, account_identity_key, account_label,
                identity_source, identity_observed_at_ms, account_confidence,
                effective_base_url, official_source, official_confidence,
                status_code, decision, decision_reason, superseded_by, ledger_layer,
                layer_reason, attribution_kind, attribution_window_id, legacy_account_key,
                account_id_alias, email_alias, created_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35,
                ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46,
                ?47, ?48, ?49, ?50
            )
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_PREPARE_FAILED",
                format!("Failed to prepare usage evidence insert: {error}"),
            )
        })?;
    for entry in evidence {
        let delta = entry.delta.unwrap_or_default();
        let account_id_alias =
            identity::validated_account_id_alias(entry.account_id_alias.as_deref());
        let email_alias = identity::validated_email_alias(entry.email_alias.as_deref());
        stmt.execute(params![
            &entry.evidence_id,
            entry.request_id.as_deref(),
            entry.fingerprint.as_deref(),
            entry.session_id.as_deref(),
            &entry.source_type,
            entry.source_priority,
            &entry.source_locator,
            &entry.source_hash,
            &entry.parser_version,
            entry.event_started_at_ms.map(i64_from_u64),
            entry.event_completed_at_ms.map(i64_from_u64),
            entry.observed_at_ms.map(i64_from_u64),
            entry.turn_index.map(i64_from_u64),
            entry.response_id.as_deref(),
            entry.previous_response_id.as_deref(),
            entry.model.as_deref(),
            entry.request_model.as_deref(),
            entry.pricing_model.as_deref(),
            entry.model_reasoning_effort.as_deref(),
            entry.model_verbosity.as_deref(),
            entry.pricing_tier.as_deref(),
            entry.pricing_source_url.as_deref(),
            &entry.usage_shape,
            &entry.usage_confidence,
            entry.delta.map(|_| i64_from_u64(delta.input_tokens)),
            entry.delta.map(|_| i64_from_u64(delta.cached_input_tokens)),
            entry.delta.map(|_| i64_from_u64(delta.output_tokens)),
            entry
                .delta
                .map(|_| i64_from_u64(delta.reasoning_output_tokens)),
            entry.raw_usage_json.as_deref(),
            entry.profile.as_deref(),
            entry.account_identity_key.as_deref(),
            entry.account_label.as_deref(),
            entry.identity_source.as_deref(),
            entry.identity_observed_at_ms.map(i64_from_u64),
            &entry.account_confidence,
            entry.effective_base_url.as_deref(),
            entry.official_source.as_deref(),
            &entry.official_confidence,
            entry.status_code.map(i64::from),
            &entry.decision,
            entry.decision_reason.as_deref(),
            entry.superseded_by.as_deref(),
            &entry.ledger_layer,
            entry.layer_reason.as_deref(),
            &entry.attribution_kind,
            entry.attribution_window_id.as_deref(),
            entry.legacy_account_key.as_deref(),
            account_id_alias.as_deref(),
            email_alias.as_deref(),
            i64_from_u64(entry.created_at_ms),
        ])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_INSERT_FAILED",
                format!(
                    "Failed to insert usage evidence {}: {error}",
                    entry.evidence_id
                ),
            )
        })?;
    }
    Ok(())
}

fn insert_usage_facts(tx: &Transaction<'_>, facts: &[UsageFact]) -> AppResult<()> {
    let mut stmt = tx
        .prepare(
            r#"
            INSERT INTO openai_usage_facts (
                evidence_id, request_id, fingerprint, session_id, profile, account_label,
                account_identity_key, legacy_account_key, official_source, ledger_layer,
                layer_reason, attribution_kind, attribution_window_id, model, request_model,
                model_reasoning_effort, model_verbosity, input_tokens,
                cached_input_tokens, output_tokens, reasoning_output_tokens, real_total_tokens,
                input_cost_micros, cached_input_cost_micros, output_cost_micros,
                total_cost_micros, unpriced_tokens, priced, pricing_note, pricing_source,
                pricing_checked_at, pricing_model, pricing_tier, pricing_source_url,
                pricing_version, status_code, created_at_ms,
                data_source, source_type, source_priority, usage_confidence,
                account_id_alias, email_alias, inserted_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35,
                ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44
            )
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_PREPARE_FAILED",
                format!("Failed to prepare usage facts insert: {error}"),
            )
        })?;
    let inserted_at_ms = i64_from_u64(current_time_ms());
    for fact in facts {
        let account_id_alias =
            identity::validated_account_id_alias(fact.account_id_alias.as_deref());
        let email_alias = identity::validated_email_alias(fact.email_alias.as_deref());
        stmt.execute(params![
            &fact.evidence_id,
            &fact.request_id,
            &fact.fingerprint,
            &fact.session_id,
            &fact.profile,
            fact.account_label.as_deref(),
            fact.account_identity_key.as_deref(),
            fact.legacy_account_key.as_deref(),
            &fact.official_source,
            &fact.ledger_layer,
            fact.layer_reason.as_deref(),
            &fact.attribution_kind,
            fact.attribution_window_id.as_deref(),
            &fact.model,
            fact.requested_model.as_deref(),
            fact.model_reasoning_effort.as_deref(),
            fact.model_verbosity.as_deref(),
            i64_from_u64(fact.delta.input_tokens),
            i64_from_u64(fact.delta.cached_input_tokens),
            i64_from_u64(fact.delta.output_tokens),
            i64_from_u64(fact.delta.reasoning_output_tokens),
            i64_from_u64(
                fact.delta
                    .input_tokens
                    .saturating_add(fact.delta.output_tokens)
            ),
            i64_from_u64(fact.cost.breakdown.input_cost_micros),
            i64_from_u64(fact.cost.breakdown.cached_input_cost_micros),
            i64_from_u64(fact.cost.breakdown.output_cost_micros),
            i64_from_u64(fact.cost.breakdown.total_cost_micros),
            i64_from_u64(fact.cost.breakdown.unpriced_tokens),
            if fact.cost.priced { 1_i64 } else { 0_i64 },
            fact.cost.pricing_note.as_deref(),
            PRICING_SOURCE,
            PRICING_CHECKED_AT,
            &fact.pricing_model,
            &fact.pricing_tier,
            &fact.pricing_source_url,
            PRICING_VERSION,
            i64::from(fact.status_code),
            i64_from_u64(fact.created_at_ms),
            &fact.data_source,
            &fact.source_type,
            fact.source_priority,
            &fact.usage_confidence,
            account_id_alias.as_deref(),
            email_alias.as_deref(),
            inserted_at_ms,
        ])
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_INSERT_FAILED",
                format!(
                    "Failed to insert usage fact {} / {}: {error}",
                    fact.ledger_layer, fact.request_id
                ),
            )
        })?;
    }
    Ok(())
}

#[allow(dead_code)]
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
            SELECT request_id, fingerprint, evidence_id, session_id, profile, account_label,
                   account_identity_key, official_source, model, request_model,
                   model_reasoning_effort, model_verbosity, input_tokens, cached_input_tokens,
                   output_tokens, reasoning_output_tokens, input_cost_micros,
                   cached_input_cost_micros, output_cost_micros, total_cost_micros,
                   unpriced_tokens, priced, pricing_note, status_code, created_at_ms,
                   data_source, source_type, source_priority, usage_confidence, pricing_model,
                   pricing_tier, pricing_source_url
            FROM openai_usage_events
            WHERE (?1 IS NULL OR created_at_ms >= ?1)
              AND (?2 IS NULL OR created_at_ms <= ?2)
              AND (?3 IS NULL OR profile = ?3)
              AND (?4 IS NULL OR account_label = ?4)
              AND (?5 IS NULL OR COALESCE(model, request_model) = ?5)
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
                let input_tokens = u64_from_i64(row.get::<_, i64>(12)?);
                let cached_input_tokens = u64_from_i64(row.get::<_, i64>(13)?);
                let output_tokens = u64_from_i64(row.get::<_, i64>(14)?);
                let reasoning_output_tokens = u64_from_i64(row.get::<_, i64>(15)?);
                Ok(UsageEvent {
                    evidence_id: row
                        .get::<_, Option<String>>(2)?
                        .unwrap_or_else(|| row.get::<_, String>(0).unwrap_or_default()),
                    request_id: row.get(0)?,
                    fingerprint: row.get(1)?,
                    session_id: row.get(3)?,
                    profile: row.get(4)?,
                    account_label: row.get(5)?,
                    account_identity_key: row.get::<_, String>(6)?,
                    official_source: row.get(7)?,
                    model: row.get(8)?,
                    requested_model: row.get(9)?,
                    model_reasoning_effort: row.get(10)?,
                    model_verbosity: row.get(11)?,
                    delta: UsageCounts {
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens,
                    },
                    cost: CalculatedCost {
                        breakdown: OpenAiUsageCostBreakdown {
                            input_cost_micros: u64_from_i64(row.get::<_, i64>(16)?),
                            cached_input_cost_micros: u64_from_i64(row.get::<_, i64>(17)?),
                            output_cost_micros: u64_from_i64(row.get::<_, i64>(18)?),
                            total_cost_micros: u64_from_i64(row.get::<_, i64>(19)?),
                            unpriced_tokens: u64_from_i64(row.get::<_, i64>(20)?),
                        },
                        priced: row.get::<_, i64>(21)? != 0,
                        pricing_note: row.get(22)?,
                    },
                    status_code: u16::try_from(row.get::<_, i64>(23)?).unwrap_or(0),
                    created_at_ms: u64_from_i64(row.get::<_, i64>(24)?),
                    data_source: row.get(25)?,
                    source_type: row
                        .get::<_, Option<String>>(26)?
                        .unwrap_or_else(|| SOURCE_CODEX_SESSION_JSONL.to_string()),
                    source_priority: row.get::<_, Option<i64>>(27)?.unwrap_or_default(),
                    usage_confidence: row
                        .get::<_, Option<String>>(28)?
                        .unwrap_or_else(|| CONFIDENCE_HIGH.to_string()),
                    pricing_model: row
                        .get::<_, Option<String>>(29)?
                        .unwrap_or_else(|| row.get::<_, String>(8).unwrap_or_default()),
                    pricing_tier: row
                        .get::<_, Option<String>>(30)?
                        .unwrap_or_else(|| PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
                    pricing_source_url: row
                        .get::<_, Option<String>>(31)?
                        .unwrap_or_else(|| PRICING_SOURCE_URL.to_string()),
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

fn load_usage_facts(conn: &Connection, query: &OpenAiUsageQuery) -> AppResult<Vec<UsageFact>> {
    let scope = normalized_usage_scope(query);
    if matches!(scope, SCOPE_CANDIDATE | SCOPE_EXCLUDED) {
        return load_evidence_usage_facts(conn, query, scope);
    }
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
            SELECT evidence_id, request_id, fingerprint, session_id, profile, account_label,
                   account_identity_key, legacy_account_key, official_source, ledger_layer,
                   layer_reason, attribution_kind, attribution_window_id, model, request_model,
                   model_reasoning_effort, model_verbosity, input_tokens,
                   cached_input_tokens, output_tokens, reasoning_output_tokens,
                   input_cost_micros, cached_input_cost_micros, output_cost_micros,
                   total_cost_micros, unpriced_tokens, priced, pricing_note, status_code,
                   created_at_ms, data_source, source_type, source_priority, usage_confidence,
                   pricing_model, pricing_tier, pricing_source_url,
                   account_id_alias, email_alias
            FROM openai_usage_facts
            WHERE (?1 IS NULL OR created_at_ms >= ?1)
              AND (?2 IS NULL OR created_at_ms <= ?2)
              AND (?3 IS NULL OR profile = ?3)
              AND (?4 IS NULL OR account_label = ?4)
              AND (?5 IS NULL OR COALESCE(model, request_model) = ?5)
              AND (?6 IS NULL OR status_code = ?6)
              AND (
                (?7 = 'strict' AND ledger_layer = 'strict_proven')
                OR (?7 = 'legacy' AND ledger_layer = 'legacy_profile_attributed')
                OR (?7 = 'combined_attributed' AND ledger_layer IN ('strict_proven', 'legacy_profile_attributed'))
              )
            ORDER BY created_at_ms ASC, request_id ASC
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_QUERY_PREPARE_FAILED",
                format!("Failed to prepare usage facts query: {error}"),
            )
        })?;

    let rows = stmt
        .query_map(
            params![
                start_ms,
                end_ms,
                profile,
                account_label,
                model,
                status_code,
                scope
            ],
            |row| {
                let persisted_account_id_alias = row.get::<_, Option<String>>(37)?;
                let persisted_email_alias = row.get::<_, Option<String>>(38)?;
                let input_tokens = u64_from_i64(row.get::<_, i64>(17)?);
                let cached_input_tokens = u64_from_i64(row.get::<_, i64>(18)?);
                let output_tokens = u64_from_i64(row.get::<_, i64>(19)?);
                let reasoning_output_tokens = u64_from_i64(row.get::<_, i64>(20)?);
                Ok(UsageFact {
                    evidence_id: row.get(0)?,
                    request_id: row.get(1)?,
                    fingerprint: row.get(2)?,
                    session_id: row.get(3)?,
                    profile: row.get(4)?,
                    account_label: row.get(5)?,
                    account_identity_key: row.get(6)?,
                    account_id_alias: identity::validated_account_id_alias(
                        persisted_account_id_alias.as_deref(),
                    ),
                    email_alias: identity::validated_email_alias(persisted_email_alias.as_deref()),
                    legacy_account_key: row.get(7)?,
                    official_source: row.get(8)?,
                    ledger_layer: row.get(9)?,
                    layer_reason: row.get(10)?,
                    attribution_kind: row.get(11)?,
                    attribution_window_id: row.get(12)?,
                    model: row.get(13)?,
                    requested_model: row.get(14)?,
                    model_reasoning_effort: row.get(15)?,
                    model_verbosity: row.get(16)?,
                    delta: UsageCounts {
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens,
                    },
                    cost: CalculatedCost {
                        breakdown: OpenAiUsageCostBreakdown {
                            input_cost_micros: u64_from_i64(row.get::<_, i64>(21)?),
                            cached_input_cost_micros: u64_from_i64(row.get::<_, i64>(22)?),
                            output_cost_micros: u64_from_i64(row.get::<_, i64>(23)?),
                            total_cost_micros: u64_from_i64(row.get::<_, i64>(24)?),
                            unpriced_tokens: u64_from_i64(row.get::<_, i64>(25)?),
                        },
                        priced: row.get::<_, i64>(26)? != 0,
                        pricing_note: row.get(27)?,
                    },
                    status_code: u16::try_from(row.get::<_, i64>(28)?).unwrap_or(0),
                    created_at_ms: u64_from_i64(row.get::<_, i64>(29)?),
                    data_source: row.get(30)?,
                    source_type: row
                        .get::<_, Option<String>>(31)?
                        .unwrap_or_else(|| SOURCE_CODEX_SESSION_JSONL.to_string()),
                    source_priority: row.get::<_, Option<i64>>(32)?.unwrap_or_default(),
                    usage_confidence: row
                        .get::<_, Option<String>>(33)?
                        .unwrap_or_else(|| CONFIDENCE_HIGH.to_string()),
                    pricing_model: row
                        .get::<_, Option<String>>(34)?
                        .unwrap_or_else(|| row.get::<_, String>(13).unwrap_or_default()),
                    pricing_tier: row
                        .get::<_, Option<String>>(35)?
                        .unwrap_or_else(|| PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
                    pricing_source_url: row
                        .get::<_, Option<String>>(36)?
                        .unwrap_or_else(|| PRICING_SOURCE_URL.to_string()),
                })
            },
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_QUERY_FAILED",
                format!("Failed to query usage facts: {error}"),
            )
        })?;

    let mut facts = Vec::new();
    for row in rows {
        facts.push(row.map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_FACTS_ROW_FAILED",
                format!("Failed to read usage facts row: {error}"),
            )
        })?);
    }
    Ok(facts)
}

fn load_evidence_usage_facts(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    scope: &str,
) -> AppResult<Vec<UsageFact>> {
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
            SELECT evidence_id, request_id, fingerprint, session_id, source_type,
                   source_priority, source_hash, event_completed_at_ms, observed_at_ms,
                   model, request_model, pricing_model, model_reasoning_effort,
                   model_verbosity, pricing_tier, pricing_source_url,
                   usage_confidence, input_tokens, cached_input_tokens, output_tokens,
                   reasoning_output_tokens, profile, account_identity_key, account_label,
                   official_source, status_code, decision, decision_reason, ledger_layer,
                   layer_reason, attribution_kind, attribution_window_id,
                   legacy_account_key, created_at_ms, account_id_alias, email_alias
            FROM openai_usage_evidence
            WHERE (?1 IS NULL OR COALESCE(event_completed_at_ms, observed_at_ms, created_at_ms) >= ?1)
              AND (?2 IS NULL OR COALESCE(event_completed_at_ms, observed_at_ms, created_at_ms) <= ?2)
              AND (?3 IS NULL OR profile = ?3)
              AND (?4 IS NULL OR account_label = ?4)
              AND (?5 IS NULL OR COALESCE(model, request_model) = ?5)
              AND (?6 IS NULL OR status_code = ?6)
              AND (
                (?7 = 'candidate' AND decision = 'candidate')
                OR (?7 = 'excluded' AND decision IN ('rejected', 'superseded'))
              )
            ORDER BY COALESCE(event_completed_at_ms, observed_at_ms, created_at_ms) ASC,
                     COALESCE(request_id, evidence_id) ASC
            "#,
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_QUERY_PREPARE_FAILED",
                format!("Failed to prepare usage evidence query: {error}"),
            )
        })?;

    let rows = stmt
        .query_map(
            params![
                start_ms,
                end_ms,
                profile,
                account_label,
                model,
                status_code,
                scope
            ],
            |row| {
                let evidence_id: String = row.get(0)?;
                let persisted_account_id_alias = row.get::<_, Option<String>>(34)?;
                let persisted_email_alias = row.get::<_, Option<String>>(35)?;
                let request_id = row
                    .get::<_, Option<String>>(1)?
                    .unwrap_or_else(|| evidence_id.clone());
                let source_type: String = row.get(4)?;
                let source_hash: String = row.get(6)?;
                let model = row
                    .get::<_, Option<String>>(9)?
                    .or(row.get::<_, Option<String>>(10)?)
                    .unwrap_or_else(|| "unknown".to_string());
                let delta = UsageCounts {
                    input_tokens: row
                        .get::<_, Option<i64>>(17)?
                        .map(u64_from_i64)
                        .unwrap_or(0),
                    cached_input_tokens: row
                        .get::<_, Option<i64>>(18)?
                        .map(u64_from_i64)
                        .unwrap_or(0),
                    output_tokens: row
                        .get::<_, Option<i64>>(19)?
                        .map(u64_from_i64)
                        .unwrap_or(0),
                    reasoning_output_tokens: row
                        .get::<_, Option<i64>>(20)?
                        .map(u64_from_i64)
                        .unwrap_or(0),
                };
                let cost = if model == "unknown" || delta == UsageCounts::default() {
                    CalculatedCost {
                        breakdown: OpenAiUsageCostBreakdown::default(),
                        priced: false,
                        pricing_note: Some("evidence row has no priced usage delta".to_string()),
                    }
                } else {
                    calculate_cost(&model, delta)
                };
                let created_at_ms = row
                    .get::<_, Option<i64>>(7)?
                    .or(row.get::<_, Option<i64>>(8)?)
                    .unwrap_or_else(|| row.get::<_, i64>(33).unwrap_or_default());
                let decision: String = row.get(26)?;
                Ok(UsageFact {
                    evidence_id,
                    request_id,
                    fingerprint: row.get::<_, Option<String>>(2)?.unwrap_or(source_hash),
                    session_id: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    profile: row
                        .get::<_, Option<String>>(21)?
                        .unwrap_or_else(|| "evidence".to_string()),
                    account_label: row.get(23)?,
                    account_identity_key: row.get(22)?,
                    account_id_alias: identity::validated_account_id_alias(
                        persisted_account_id_alias.as_deref(),
                    ),
                    email_alias: identity::validated_email_alias(persisted_email_alias.as_deref()),
                    legacy_account_key: row.get(32)?,
                    official_source: row
                        .get::<_, Option<String>>(24)?
                        .unwrap_or_else(|| source_type.clone()),
                    ledger_layer: row.get(28)?,
                    layer_reason: row.get(29)?,
                    attribution_kind: row
                        .get::<_, Option<String>>(30)?
                        .unwrap_or_else(|| decision.clone()),
                    attribution_window_id: row.get(31)?,
                    model: model.clone(),
                    requested_model: row.get(10)?,
                    model_reasoning_effort: row.get(12)?,
                    model_verbosity: row.get(13)?,
                    delta,
                    cost,
                    status_code: row
                        .get::<_, Option<i64>>(25)?
                        .and_then(|value| u16::try_from(value).ok())
                        .unwrap_or(0),
                    created_at_ms: u64_from_i64(created_at_ms),
                    data_source: source_type.clone(),
                    source_type,
                    source_priority: row.get::<_, Option<i64>>(5)?.unwrap_or_default(),
                    usage_confidence: row
                        .get::<_, Option<String>>(16)?
                        .unwrap_or_else(|| CONFIDENCE_UNPROVEN.to_string()),
                    pricing_model: row
                        .get::<_, Option<String>>(11)?
                        .unwrap_or_else(|| model.clone()),
                    pricing_tier: row
                        .get::<_, Option<String>>(14)?
                        .unwrap_or_else(|| PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
                    pricing_source_url: row
                        .get::<_, Option<String>>(15)?
                        .unwrap_or_else(|| PRICING_SOURCE_URL.to_string()),
                })
            },
        )
        .map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_QUERY_FAILED",
                format!("Failed to query usage evidence: {error}"),
            )
        })?;

    let mut facts = Vec::new();
    for row in rows {
        facts.push(row.map_err(|error| {
            AppError::new(
                "OPENAI_USAGE_EVIDENCE_ROW_FAILED",
                format!("Failed to read usage evidence row: {error}"),
            )
        })?);
    }
    Ok(facts)
}

fn filter_facts_by_account_group(
    facts: Vec<UsageFact>,
    query: &OpenAiUsageQuery,
    resolver: &identity::CanonicalAccountResolver,
) -> Vec<UsageFact> {
    let Some(selected_account_group_key) = query
        .account_group_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return facts;
    };
    let Some(selected_canonical) = resolver.resolve_group_filter(selected_account_group_key) else {
        return Vec::new();
    };
    let label_identities = strict_label_identities(&facts);
    facts
        .into_iter()
        .filter(|fact| {
            let fallback = account_group_key(fact, &label_identities);
            resolver.matches_group_filter(fact, fallback, &selected_canonical)
        })
        .collect()
}

fn attach_layer_summaries(
    conn: &Connection,
    query: &OpenAiUsageQuery,
    mut summary: OpenAiUsageSummary,
    resolver: &identity::CanonicalAccountResolver,
) -> AppResult<OpenAiUsageSummary> {
    let layer_query = query_with_scope(query, SCOPE_COMBINED_ATTRIBUTED);
    let mut facts = load_usage_facts_with_resolver(conn, &layer_query, resolver)?;
    let candidate_query = query_with_scope(query, SCOPE_CANDIDATE);
    facts.extend(load_usage_facts_with_resolver(
        conn,
        &candidate_query,
        resolver,
    )?);
    summary.layer_summaries = build_layer_summaries(&facts, query);
    Ok(summary)
}

fn build_layer_summaries(
    facts: &[UsageFact],
    query: &OpenAiUsageQuery,
) -> Vec<OpenAiUsageLayerSummary> {
    let mut layers = BTreeMap::<String, Aggregate>::new();
    layers
        .entry(LEDGER_LAYER_STRICT_PROVEN.to_string())
        .or_default();
    layers
        .entry(LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED.to_string())
        .or_default();
    layers
        .entry(LEDGER_LAYER_CANDIDATE_UNATTRIBUTED.to_string())
        .or_default();
    for fact in facts.iter().filter(|fact| fact_matches_query(fact, query)) {
        add_to_aggregate(
            layers.entry(fact.ledger_layer.clone()).or_default(),
            fact.delta,
            &fact.cost.breakdown,
        );
    }
    layers
        .into_iter()
        .map(|(ledger_layer, aggregate)| OpenAiUsageLayerSummary {
            ledger_layer,
            request_count: aggregate.requests,
            tokens: aggregate.tokens,
            cost: aggregate.cost,
        })
        .collect()
}

fn i64_from_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn u64_from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

#[allow(dead_code)]
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

fn fact_matches_query(fact: &UsageFact, query: &OpenAiUsageQuery) -> bool {
    if let Some(start) = query.start_date.and_then(seconds_to_ms) {
        if fact.created_at_ms < start {
            return false;
        }
    }
    if let Some(end) = query.end_date.and_then(end_seconds_to_ms) {
        if fact.created_at_ms > end {
            return false;
        }
    }
    if let Some(profile) = query
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if fact.profile != profile {
            return false;
        }
    }
    if let Some(account_label) = query
        .account_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if fact.account_label.as_deref() != Some(account_label) {
            return false;
        }
    }
    if let Some(model) = query
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if fact.model != normalize_model(model) {
            return false;
        }
    }
    if let Some(status_code) = query.status_code {
        if fact.status_code != status_code {
            return false;
        }
    }
    true
}

fn build_trends(facts: &[UsageFact], query: &OpenAiUsageQuery) -> Vec<OpenAiUsageTrendPoint> {
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
    for fact in facts.iter().filter(|fact| fact_matches_query(fact, query)) {
        let bucket = if hourly {
            local_hour_bucket_ms(fact.created_at_ms)
        } else {
            local_day_bucket_ms(fact.created_at_ms)
        };
        add_to_aggregate(
            buckets.entry(bucket).or_default(),
            fact.delta,
            &fact.cost.breakdown,
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

#[allow(dead_code)]
fn event_to_log(event: &UsageEvent) -> OpenAiUsageRequestLog {
    OpenAiUsageRequestLog {
        request_id: event.request_id.clone(),
        session_id: event.session_id.clone(),
        profile: event.profile.clone(),
        account_label: event.account_label.clone(),
        official_source: event.official_source.clone(),
        ledger_layer: LEDGER_LAYER_STRICT_PROVEN.to_string(),
        attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string(),
        attribution_window_id: None,
        model: event.model.clone(),
        requested_model: event.requested_model.clone(),
        pricing_model: Some(event.pricing_model.clone()),
        model_reasoning_effort: event.model_reasoning_effort.clone(),
        model_verbosity: event.model_verbosity.clone(),
        pricing_tier: Some(event.pricing_tier.clone()),
        pricing_source_url: Some(event.pricing_source_url.clone()),
        tokens: tokens_from_delta(event.delta),
        cost: event.cost.breakdown.clone(),
        priced: event.cost.priced,
        pricing_note: event.cost.pricing_note.clone(),
        status_code: event.status_code,
        created_at_ms: event.created_at_ms,
        data_source: event.data_source.clone(),
        usage_confidence: event.usage_confidence.clone(),
    }
}

fn fact_to_log(fact: &UsageFact) -> OpenAiUsageRequestLog {
    OpenAiUsageRequestLog {
        request_id: fact.request_id.clone(),
        session_id: fact.session_id.clone(),
        profile: fact.profile.clone(),
        account_label: fact.account_label.clone(),
        official_source: fact.official_source.clone(),
        ledger_layer: fact.ledger_layer.clone(),
        attribution_kind: fact.attribution_kind.clone(),
        attribution_window_id: fact.attribution_window_id.clone(),
        model: fact.model.clone(),
        requested_model: fact.requested_model.clone(),
        pricing_model: Some(fact.pricing_model.clone()),
        model_reasoning_effort: fact.model_reasoning_effort.clone(),
        model_verbosity: fact.model_verbosity.clone(),
        pricing_tier: Some(fact.pricing_tier.clone()),
        pricing_source_url: Some(fact.pricing_source_url.clone()),
        tokens: tokens_from_delta(fact.delta),
        cost: fact.cost.breakdown.clone(),
        priced: fact.cost.priced,
        pricing_note: fact.cost.pricing_note.clone(),
        status_code: fact.status_code,
        created_at_ms: fact.created_at_ms,
        data_source: fact.data_source.clone(),
        usage_confidence: fact.usage_confidence.clone(),
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

#[derive(Default)]
struct UsageConfigSnapshot {
    model_reasoning_effort: Option<String>,
    model_verbosity: Option<String>,
}

fn load_usage_config_snapshot(path: &Path) -> UsageConfigSnapshot {
    let Ok(raw) = fs::read_to_string(path) else {
        return UsageConfigSnapshot::default();
    };
    let mut snapshot = UsageConfigSnapshot::default();
    for line in raw.lines() {
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_simple_toml_string(raw_value) else {
            continue;
        };
        match key.trim() {
            "model_reasoning_effort" => {
                snapshot.model_reasoning_effort = normalize_model_reasoning_effort(value);
            }
            "model_verbosity" => {
                snapshot.model_verbosity = normalize_model_verbosity(value);
            }
            _ => {}
        }
    }
    snapshot
}

fn parse_simple_toml_string(raw_value: &str) -> Option<String> {
    let value = raw_value
        .split('#')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn build_current_attribution_record(codex_home: &Path) -> AppResult<AttributionRecord> {
    let index = load_profiles_index(Some(codex_home))?;
    let observed_at_ms = current_time_ms();
    let Some(profile_name) = index.current_profile.clone() else {
        return Ok(AttributionRecord {
            observed_at_ms,
            profile: None,
            account_label: None,
            account_identity_key: None,
            account_id_alias: None,
            email_alias: None,
            openai_base_url: None,
            official_source: None,
            is_official_openai: false,
            exclusion_reason: Some("no active profile".to_string()),
            model_reasoning_effort: None,
            model_verbosity: None,
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
            account_identity_key: None,
            account_id_alias: None,
            email_alias: None,
            openai_base_url: None,
            official_source: None,
            is_official_openai: false,
            exclusion_reason: Some("active profile not found".to_string()),
            model_reasoning_effort: None,
            model_verbosity: None,
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
        (&root_identity, &profile_identity),
        (Some(root), Some(slot)) if root.same_account(slot)
    );
    let confirmed_identity = root_identity.as_ref().filter(|_| identity_matches);
    let account_identity_key = confirmed_identity.map(account_identity_key);
    let account_id_alias = confirmed_identity
        .and_then(|identity| identity::account_id_alias(identity.account_id.as_deref()));
    let email_alias =
        confirmed_identity.and_then(|identity| identity::email_alias(identity.email.as_deref()));
    let is_official_openai = official_source.is_some() && identity_matches;
    let exclusion_reason = if is_official_openai {
        None
    } else if official_source.is_none() {
        Some("non-official base_url".to_string())
    } else {
        Some("account identity does not match active profile".to_string())
    };
    let config = load_usage_config_snapshot(&codex_home.join("config.toml"));
    Ok(AttributionRecord {
        observed_at_ms,
        profile: Some(profile_name),
        account_label: profile.account_label.clone(),
        account_identity_key,
        account_id_alias,
        email_alias,
        openai_base_url: profile.openai_base_url.clone(),
        official_source,
        is_official_openai,
        exclusion_reason,
        model_reasoning_effort: config.model_reasoning_effort,
        model_verbosity: config.model_verbosity,
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

enum AttributionLookup {
    Attributed(AttributionContext),
    CandidateUnattributed(&'static str),
    RejectedInvalid(&'static str),
}

fn attribution_for_event(
    event_time_ms: u64,
    fallback: Option<&AttributionContext>,
    records: &[AttributionRecord],
    exclusions: &mut OpenAiUsageExclusions,
) -> AttributionLookup {
    if let Some(record) = records
        .iter()
        .rev()
        .find(|record| record.observed_at_ms <= event_time_ms)
    {
        if !record.is_official_openai {
            if record
                .exclusion_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("base_url"))
            {
                exclusions.non_official_base_url =
                    exclusions.non_official_base_url.saturating_add(1);
            } else {
                exclusions.unattributed = exclusions.unattributed.saturating_add(1);
            }
            return AttributionLookup::RejectedInvalid("non official OpenAI source");
        }
        if let Some(context) = record.context() {
            return AttributionLookup::Attributed(context);
        }
        if let Some(context) = record.legacy_context() {
            return AttributionLookup::Attributed(context);
        }
        exclusions.unattributed = exclusions.unattributed.saturating_add(1);
        return AttributionLookup::CandidateUnattributed(
            "official source was recorded but account/profile attribution is incomplete",
        );
    }
    if let Some(fallback) = fallback {
        if event_time_ms >= fallback.attribution_since_ms {
            return AttributionLookup::Attributed(AttributionContext {
                profile: fallback.profile.clone(),
                account_label: fallback.account_label.clone(),
                official_source: fallback.official_source.clone(),
                attribution_since_ms: fallback.attribution_since_ms,
                account_identity_key: fallback.account_identity_key.clone(),
                account_id_alias: fallback.account_id_alias.clone(),
                email_alias: fallback.email_alias.clone(),
                legacy_account_key: fallback.legacy_account_key.clone(),
                identity_source: fallback.identity_source.clone(),
                identity_observed_at_ms: fallback.identity_observed_at_ms,
                ledger_layer: fallback.ledger_layer.clone(),
                layer_reason: fallback.layer_reason.clone(),
                attribution_kind: fallback.attribution_kind.clone(),
                attribution_window_id: fallback.attribution_window_id.clone(),
                model_reasoning_effort: fallback.model_reasoning_effort.clone(),
                model_verbosity: fallback.model_verbosity.clone(),
            });
        }
    }
    exclusions.unproven_history = exclusions.unproven_history.saturating_add(1);
    AttributionLookup::CandidateUnattributed("no attribution record covered this event")
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

/// Persisted v1 identity key. Golden vectors lock the historical
/// `DefaultHasher` output so a toolchain/hash drift fails before release.
fn account_identity_key(identity: &AccountIdentity) -> String {
    let mut hasher = DefaultHasher::new();
    "openai-account-identity-v1".hash(&mut hasher);
    identity
        .account_id
        .as_deref()
        .unwrap_or_default()
        .hash(&mut hasher);
    identity
        .email
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .hash(&mut hasher);
    format!("acct_{:016x}", hasher.finish())
}

fn legacy_account_key(profile: &str, attribution_window_id: &str) -> String {
    format!("legacy_profile:{profile}:{attribution_window_id}")
}

fn stable_hash(parts: &[&str]) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn fingerprint_for_event(
    attribution: &AttributionContext,
    model: &str,
    delta: UsageCounts,
    event_time_ms: Option<u64>,
) -> String {
    let bucket = event_time_ms.unwrap_or_default() / 60_000;
    let account_key = attribution
        .account_identity_key
        .as_deref()
        .or(attribution.legacy_account_key.as_deref())
        .unwrap_or(&attribution.profile);
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        account_key,
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

fn ledger_layer_for_decision(decision: &str, attribution: Option<&AttributionContext>) -> String {
    match decision {
        DECISION_ACCEPTED => attribution
            .map(|value| value.ledger_layer.clone())
            .unwrap_or_else(|| LEDGER_LAYER_CANDIDATE_UNATTRIBUTED.to_string()),
        DECISION_CANDIDATE => LEDGER_LAYER_CANDIDATE_UNATTRIBUTED.to_string(),
        DECISION_SUPERSEDED => LEDGER_LAYER_SUPERSEDED.to_string(),
        DECISION_REJECTED => LEDGER_LAYER_REJECTED_INVALID.to_string(),
        _ => LEDGER_LAYER_REJECTED_INVALID.to_string(),
    }
}

fn accepted_decision_reason(attribution: &AttributionContext) -> &'static str {
    if attribution.ledger_layer == LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED {
        "official source, legacy profile window, token delta, and dedupe proven"
    } else {
        "account, official source, token delta, and dedupe proven"
    }
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
    let identity_matches = matches!(
        (&root_identity, &profile_identity),
        (Some(root), Some(slot)) if root.same_account(slot)
    );
    if !identity_matches {
        exclusions.unattributed = exclusions.unattributed.saturating_add(1);
        return Ok(None);
    }
    let account_identity_key = root_identity.as_ref().map(account_identity_key);
    let account_id_alias = root_identity
        .as_ref()
        .and_then(|identity| identity::account_id_alias(identity.account_id.as_deref()));
    let email_alias = root_identity
        .as_ref()
        .and_then(|identity| identity::email_alias(identity.email.as_deref()));

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
        account_identity_key,
        account_id_alias,
        email_alias,
        legacy_account_key: None,
        identity_source: Some("live_root_profile_auth_match".to_string()),
        identity_observed_at_ms: Some(attribution_since_ms),
        ledger_layer: LEDGER_LAYER_STRICT_PROVEN.to_string(),
        layer_reason: Some(
            "live root/profile auth identity match with official source".to_string(),
        ),
        attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string(),
        attribution_window_id: Some(attribution_since_ms.to_string()),
        model_reasoning_effort: None,
        model_verbosity: None,
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

fn string_field(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_model_reasoning_effort(value: String) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
    )
    .then_some(normalized)
}

fn normalize_model_verbosity(value: String) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "low" | "medium" | "high").then_some(normalized)
}

#[allow(dead_code)]
fn add_event_usage(
    accounts: &mut BTreeMap<String, AccountAggregate>,
    global_models: &mut BTreeMap<String, ModelAggregate>,
    event: &UsageEvent,
) {
    let account = accounts
        .entry(event.account_identity_key.clone())
        .or_insert_with(|| AccountAggregate {
            profile: event.profile.clone(),
            account_identity_key: Some(event.account_identity_key.clone()),
            legacy_account_key: None,
            account_label: event.account_label.clone(),
            official_source: event.official_source.clone(),
            ledger_layer: LEDGER_LAYER_STRICT_PROVEN.to_string(),
            attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string(),
            attribution_window_id: None,
            ..AccountAggregate::default()
        });
    add_to_aggregate(&mut account.aggregate, event.delta, &event.cost.breakdown);
    add_to_model(
        account.models.entry(event.model.clone()).or_default(),
        event.delta,
        &event.cost,
        event.requested_model.as_ref(),
        &event.pricing_model,
        event.model_reasoning_effort.as_ref(),
        event.model_verbosity.as_ref(),
        &event.pricing_tier,
        &event.pricing_source_url,
    );
    add_to_model(
        global_models.entry(event.model.clone()).or_default(),
        event.delta,
        &event.cost,
        event.requested_model.as_ref(),
        &event.pricing_model,
        event.model_reasoning_effort.as_ref(),
        event.model_verbosity.as_ref(),
        &event.pricing_tier,
        &event.pricing_source_url,
    );
}

fn add_fact_usage(
    accounts: &mut BTreeMap<String, AccountAggregate>,
    global_models: &mut BTreeMap<String, ModelAggregate>,
    label_identities: &BTreeMap<String, BTreeSet<String>>,
    account_resolver: Option<&identity::CanonicalAccountResolver>,
    fact: &UsageFact,
) {
    let fallback_key = account_group_key(fact, label_identities);
    let key = account_resolver
        .map(|resolver| resolver.canonical_key(fact, fallback_key.clone()))
        .unwrap_or(fallback_key);
    let account = accounts
        .entry(key.clone())
        .or_insert_with(|| new_account_aggregate(key, fact));
    account.profiles.insert(fact.profile.clone());
    if account.account_identity_key.is_none() {
        account.account_identity_key = fact.account_identity_key.clone();
    }
    if account.legacy_account_key.is_none() {
        account.legacy_account_key = fact.legacy_account_key.clone();
    }
    if account.account_label.is_none() {
        account.account_label = fact.account_label.clone();
    }
    if account.official_source != fact.official_source {
        account.official_source = "Multiple OpenAI sources".to_string();
    }
    if account.ledger_layer != fact.ledger_layer {
        account.ledger_layer = LEDGER_LAYER_MIXED_ATTRIBUTED.to_string();
    }
    if account.attribution_kind != fact.attribution_kind {
        account.attribution_kind = ATTRIBUTION_KIND_MIXED_ACCOUNT.to_string();
    }
    if account.attribution_window_id != fact.attribution_window_id {
        account.attribution_window_id = None;
    }
    add_to_aggregate(&mut account.aggregate, fact.delta, &fact.cost.breakdown);
    add_to_model(
        account.models.entry(fact.model.clone()).or_default(),
        fact.delta,
        &fact.cost,
        fact.requested_model.as_ref(),
        &fact.pricing_model,
        fact.model_reasoning_effort.as_ref(),
        fact.model_verbosity.as_ref(),
        &fact.pricing_tier,
        &fact.pricing_source_url,
    );
    add_to_model(
        global_models.entry(fact.model.clone()).or_default(),
        fact.delta,
        &fact.cost,
        fact.requested_model.as_ref(),
        &fact.pricing_model,
        fact.model_reasoning_effort.as_ref(),
        fact.model_verbosity.as_ref(),
        &fact.pricing_tier,
        &fact.pricing_source_url,
    );
}

fn new_account_aggregate(account_group_key: String, fact: &UsageFact) -> AccountAggregate {
    let mut profiles = BTreeSet::new();
    profiles.insert(fact.profile.clone());
    AccountAggregate {
        profile: fact.profile.clone(),
        profiles,
        account_group_key,
        account_identity_key: fact.account_identity_key.clone(),
        legacy_account_key: fact.legacy_account_key.clone(),
        account_label: fact.account_label.clone(),
        official_source: fact.official_source.clone(),
        ledger_layer: fact.ledger_layer.clone(),
        attribution_kind: fact.attribution_kind.clone(),
        attribution_window_id: fact.attribution_window_id.clone(),
        ..AccountAggregate::default()
    }
}

fn account_group_key(
    fact: &UsageFact,
    label_identities: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    if let Some(value) = fact.account_identity_key.as_deref() {
        return format!("identity:{value}");
    }
    normalized_account_label_key(fact.account_label.as_deref())
        .map(|label_key| {
            let identities = label_identities.get(&label_key);
            match identities.and_then(single_btree_value) {
                Some(identity) => format!("identity:{identity}"),
                None => label_key,
            }
        })
        .or_else(|| {
            fact.legacy_account_key
                .as_deref()
                .map(|value| format!("legacy:{value}"))
        })
        .unwrap_or_else(|| format!("profile:{}", fact.profile))
}

fn strict_label_identities(facts: &[UsageFact]) -> BTreeMap<String, BTreeSet<String>> {
    let mut labels = BTreeMap::<String, BTreeSet<String>>::new();
    for fact in facts {
        let (Some(label_key), Some(identity)) = (
            normalized_account_label_key(fact.account_label.as_deref()),
            fact.account_identity_key.as_deref(),
        ) else {
            continue;
        };
        labels
            .entry(label_key)
            .or_default()
            .insert(identity.to_string());
    }
    labels
}

fn single_btree_value(values: &BTreeSet<String>) -> Option<&str> {
    (values.len() == 1)
        .then(|| values.iter().next())
        .flatten()
        .map(String::as_str)
}

fn normalized_account_label_key(label: Option<&str>) -> Option<String> {
    let value = label?.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let looks_like_account = value.contains('@') || value.starts_with("acct_");
    looks_like_account.then(|| format!("label:{value}"))
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

fn add_to_model(
    model: &mut ModelAggregate,
    delta: UsageCounts,
    cost: &CalculatedCost,
    requested_model: Option<&String>,
    pricing_model: &str,
    model_reasoning_effort: Option<&String>,
    model_verbosity: Option<&String>,
    pricing_tier: &str,
    pricing_source_url: &str,
) {
    add_to_aggregate(&mut model.aggregate, delta, &cost.breakdown);
    insert_optional_value(&mut model.requested_models, requested_model);
    model.pricing_models.insert(pricing_model.to_string());
    insert_optional_value(&mut model.reasoning_efforts, model_reasoning_effort);
    insert_optional_value(&mut model.verbosities, model_verbosity);
    model.pricing_tiers.insert(pricing_tier.to_string());
    model
        .pricing_source_urls
        .insert(pricing_source_url.to_string());
    model.priced |= cost.priced;
    if let Some(note) = &cost.pricing_note {
        model.pricing_note = Some(note.clone());
    }
}

fn insert_optional_value(values: &mut BTreeSet<String>, value: Option<&String>) {
    if let Some(value) = value
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        values.insert(value.to_string());
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

fn account_to_summary(account: AccountAggregate) -> OpenAiUsageAccountSummary {
    let profiles = account.profiles.into_iter().collect::<Vec<_>>();
    let profile = if profiles.is_empty() {
        account.profile
    } else {
        profiles.join(", ")
    };
    OpenAiUsageAccountSummary {
        profile,
        profiles,
        account_group_key: Some(account.account_group_key),
        account_identity_key: account.account_identity_key,
        legacy_account_key: account.legacy_account_key,
        account_label: account.account_label,
        official_source: account.official_source,
        ledger_layer: account.ledger_layer,
        attribution_kind: account.attribution_kind,
        attribution_window_id: account.attribution_window_id,
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
            requested_model: single_or_mixed(aggregate.requested_models),
            pricing_model: single_or_mixed(aggregate.pricing_models),
            model_reasoning_effort: single_or_mixed(aggregate.reasoning_efforts),
            model_verbosity: single_or_mixed(aggregate.verbosities),
            pricing_tier: single_or_mixed(aggregate.pricing_tiers),
            pricing_source_url: single_or_mixed(aggregate.pricing_source_urls),
            request_count: aggregate.aggregate.requests,
            tokens: aggregate.aggregate.tokens,
            cost: aggregate.aggregate.cost,
            priced: aggregate.priced,
            pricing_note: aggregate.pricing_note,
        })
        .collect()
}

fn single_or_mixed(values: BTreeSet<String>) -> Option<String> {
    match values.len() {
        0 => None,
        1 => values.into_iter().next(),
        _ => Some("mixed".to_string()),
    }
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

    fn write_profile_snapshot(codex_home: &Path, profile: &str, auth: &str) {
        let profile_dir = get_backup_root(Some(codex_home)).join(profile);
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join("auth.json"), auth).unwrap();
        fs::write(
            profile_dir.join("profile.json"),
            r#"{"account_label":"snapshot","plan_name":"Pro"}"#,
        )
        .unwrap();
    }

    struct IdentityFactFixture<'a> {
        evidence_id: &'a str,
        request_id: &'a str,
        fingerprint: &'a str,
        profile: &'a str,
        account_identity_key: Option<&'a str>,
        account_id_alias: Option<&'a str>,
        legacy_account_key: Option<&'a str>,
        ledger_layer: &'a str,
        attribution_kind: &'a str,
        attribution_window_id: Option<&'a str>,
        created_at_ms: i64,
    }

    fn confirmed_identity_fact<'a>(
        evidence_id: &'a str,
        request_id: &'a str,
        profile: &'a str,
        account_identity_key: &'a str,
        account_id_alias: &'a str,
        attribution_window_id: &'a str,
        created_at_ms: i64,
    ) -> IdentityFactFixture<'a> {
        IdentityFactFixture {
            evidence_id,
            request_id,
            fingerprint: evidence_id,
            profile,
            account_identity_key: Some(account_identity_key),
            account_id_alias: Some(account_id_alias),
            legacy_account_key: None,
            ledger_layer: LEDGER_LAYER_STRICT_PROVEN,
            attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT,
            attribution_window_id: Some(attribution_window_id),
            created_at_ms,
        }
    }

    fn legacy_identity_fact<'a>(
        evidence_id: &'a str,
        request_id: &'a str,
        profile: &'a str,
        legacy_account_key: &'a str,
        attribution_window_id: &'a str,
        created_at_ms: i64,
    ) -> IdentityFactFixture<'a> {
        IdentityFactFixture {
            evidence_id,
            request_id,
            fingerprint: evidence_id,
            profile,
            account_identity_key: None,
            account_id_alias: None,
            legacy_account_key: Some(legacy_account_key),
            ledger_layer: LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED,
            attribution_kind: ATTRIBUTION_KIND_LEGACY_PROFILE,
            attribution_window_id: Some(attribution_window_id),
            created_at_ms,
        }
    }

    fn seed_usage_fact_fixture(
        codex_home: &Path,
        name: &str,
        cumulative_totals: &[(u64, u64)],
    ) -> Vec<String> {
        fs::create_dir_all(codex_home).unwrap();
        setup_profile(codex_home, None);
        append_attribution_record(
            codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("seed".to_string()),
                account_label: Some("seed".to_string()),
                account_identity_key: None,
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        let mut lines = vec![
            format!(r#"{{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{{"id":"s-{name}"}}}}"#),
            r#"{"timestamp":"2999-01-01T00:00:00.500Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#.to_string(),
        ];
        for (index, (input_tokens, output_tokens)) in cumulative_totals.iter().enumerate() {
            lines.push(format!(
                r#"{{"timestamp":"2999-01-01T00:00:{:02}.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input_tokens},"cached_input_tokens":0,"output_tokens":{output_tokens},"reasoning_output_tokens":0}}}}}}}}"#,
                index + 1,
            ));
        }
        write_session(codex_home, &lines.join("\n"));
        let seeded = load_openai_usage_dashboard(Some(codex_home), None, 1, 20).unwrap();
        assert_eq!(
            seeded
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            cumulative_totals.len() as u64
        );
        let conn = Connection::open(get_openai_usage_ledger_path(Some(codex_home))).unwrap();
        let mut stmt = conn
            .prepare("SELECT evidence_id FROM openai_usage_facts ORDER BY created_at_ms")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn rewrite_identity_fact(
        conn: &Connection,
        old_evidence_id: &str,
        fixture: &IdentityFactFixture<'_>,
    ) {
        conn.execute(
            r#"
            UPDATE openai_usage_facts
            SET evidence_id = ?1, request_id = ?2, fingerprint = ?3,
                profile = ?4, account_label = NULL, account_identity_key = ?5,
                account_id_alias = ?6, email_alias = NULL, legacy_account_key = ?7,
                ledger_layer = ?8, attribution_kind = ?9,
                attribution_window_id = ?10, created_at_ms = ?11
            WHERE evidence_id = ?12
            "#,
            rusqlite::params![
                fixture.evidence_id,
                fixture.request_id,
                fixture.fingerprint,
                fixture.profile,
                fixture.account_identity_key,
                fixture.account_id_alias,
                fixture.legacy_account_key,
                fixture.ledger_layer,
                fixture.attribution_kind,
                fixture.attribution_window_id,
                fixture.created_at_ms,
                old_evidence_id,
            ],
        )
        .unwrap();
    }

    fn assert_profile_snapshot_does_not_recover(
        name: &str,
        auth: Option<&str>,
        legacy_identity_key: Option<&str>,
    ) {
        let codex_home = temp_codex_home(name);
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let profile_dir = get_backup_root(Some(&codex_home)).join("legacy-profile");
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(
            profile_dir.join("profile.json"),
            r#"{"account_label":"legacy","plan_name":"Pro"}"#,
        )
        .unwrap();
        if let Some(auth) = auth {
            fs::write(profile_dir.join("auth.json"), auth).unwrap();
        }
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        let account_alias = identity::account_id_alias(Some("acct_A")).unwrap();
        let email_alias = identity::email_alias(Some("User@Example.com")).unwrap();
        fs::write(
            attribution_path,
            [
                serde_json::json!({
                    "observed_at_ms": 32472144000000_u64,
                    "profile": "legacy-profile",
                    "account_label": "historical-label",
                    "account_identity_key": legacy_identity_key,
                    "account_id_alias": null,
                    "email_alias": null,
                    "openai_base_url": null,
                    "official_source": "OpenAI default",
                    "is_official_openai": true,
                    "exclusion_reason": null,
                    "model_reasoning_effort": null,
                    "model_verbosity": null,
                })
                .to_string(),
                serde_json::json!({
                    "observed_at_ms": 32472144003000_u64,
                    "profile": "new-alias",
                    "account_label": "current-label",
                    "account_identity_key": "acct_key_new_alias",
                    "account_id_alias": account_alias,
                    "email_alias": email_alias,
                    "openai_base_url": null,
                    "official_source": "OpenAI default",
                    "is_official_openai": true,
                    "exclusion_reason": null,
                    "model_reasoning_effort": null,
                    "model_verbosity": null,
                })
                .to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                format!(r#"{{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{{"id":"s-{name}"}}}}"#),
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#.to_string(),
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#.to_string(),
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#.to_string(),
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2, "{name}");
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2,
            "{name}"
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700,
            "{name}"
        );
        let _ = fs::remove_dir_all(&codex_home);
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

    fn write_archived_session(codex_home: &Path, body: &str) {
        let session_dir = codex_home
            .join("archived_sessions")
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
    fn account_identity_key_v1_matches_published_golden_vectors() {
        let identities = [
            AccountIdentity {
                account_id: None,
                email: Some("User@Example.com".to_string()),
            },
            AccountIdentity {
                account_id: Some("acct_A".to_string()),
                email: None,
            },
            AccountIdentity {
                account_id: Some("acct_A".to_string()),
                email: Some("User@Example.com".to_string()),
            },
        ];

        assert_eq!(
            identities.map(|identity| account_identity_key(&identity)),
            [
                "acct_681356aab274b3b0",
                "acct_4e434b75e0952ae5",
                "acct_b12448b5e4c359a3",
            ]
        );
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
                account_identity_key: Some("acct_key_a".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("acct_B".to_string()),
                account_identity_key: Some("acct_key_b".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
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
    fn dashboard_merges_email_alias_with_dual_identity() {
        let codex_home = temp_codex_home("dashboard-email-alias-merge");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"email-only","account_label":"User@Example.com","account_identity_key":"acct_key_email_only","account_id_alias":null,"email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"dual","account_label":"user@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-email-alias"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        assert_eq!(dashboard.account_summary[0].request_count, 2);
        assert_eq!(dashboard.account_summary[0].tokens.input_tokens, 1500);
        assert_eq!(dashboard.account_summary[0].tokens.output_tokens, 200);
        assert_eq!(dashboard.account_summary[0].tokens.real_total_tokens, 1700);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_merges_account_id_alias_with_dual_identity() {
        let codex_home = temp_codex_home("dashboard-account-id-alias-merge");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"account-only","account_label":"acct_shared","account_identity_key":"acct_key_account_only","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"dual","account_label":"dual@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-account-id-alias"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        assert_eq!(dashboard.account_summary[0].request_count, 2);
        assert_eq!(dashboard.account_summary[0].tokens.input_tokens, 1500);
        assert_eq!(dashboard.account_summary[0].tokens.output_tokens, 200);
        assert_eq!(dashboard.account_summary[0].tokens.real_total_tokens, 1700);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_merges_identity_aliases_transitively() {
        let codex_home = temp_codex_home("dashboard-transitive-identity-aliases");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"email-only","account_label":"User@Example.com","account_identity_key":"acct_key_email_only","account_id_alias":null,"email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"dual","account_label":"user@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144005000,"profile":"account-only","account_label":"acct_A","account_identity_key":"acct_key_account_only","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-transitive-identity-aliases"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:06.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1800,"cached_input_tokens":280,"output_tokens":250,"reasoning_output_tokens":50}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        let account = &dashboard.account_summary[0];
        assert_eq!(account.request_count, 3);
        assert_eq!(
            account.profiles,
            ["account-only", "dual", "email-only"].map(str::to_string)
        );
        assert_eq!(account.tokens.input_tokens, 1800);
        assert_eq!(account.tokens.output_tokens, 250);
        assert_eq!(account.tokens.real_total_tokens, 2050);
        assert_eq!(
            account.account_group_key.as_deref(),
            Some("acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_recovers_legacy_aliases_from_exact_profile_snapshot() {
        let codex_home = temp_codex_home("dashboard-exact-profile-snapshot");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_profile_snapshot(
            &codex_home,
            "legacy-dual",
            r#"{"tokens":{"account_id":"acct_A","id_token":"x.eyJlbWFpbCI6IlVzZXJARXhhbXBsZS5jb20ifQ.x"}}"#,
        );
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"legacy-dual","account_label":"historical-label","account_identity_key":"acct_b12448b5e4c359a3","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"new-alias","account_label":"current-label","account_identity_key":"acct_key_new_alias","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-exact-profile-snapshot"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        let account = &dashboard.account_summary[0];
        assert_eq!(account.request_count, 2);
        assert_eq!(account.tokens.real_total_tokens, 1700);
        assert_eq!(
            account.account_group_key.as_deref(),
            Some("acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_uses_latest_prior_identity_without_future_backfill() {
        let codex_home = temp_codex_home("dashboard-latest-prior-identity");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("seed".to_string()),
                account_label: Some("seed".to_string()),
                account_identity_key: None,
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-latest-prior"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:00.500Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":1,"output_tokens":1,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30,"cached_input_tokens":3,"output_tokens":3,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":60,"cached_input_tokens":6,"output_tokens":6,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":10,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );
        let seeded = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(seeded.account_summary.len(), 1);

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let evidence_ids = {
            let mut stmt = conn
                .prepare("SELECT evidence_id FROM openai_usage_facts ORDER BY created_at_ms")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(evidence_ids.len(), 4);
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let alias_b = identity::account_id_alias(Some("acct_timeline_b")).unwrap();
        let rows = [
            (
                "e-a-100",
                "r-a-100",
                "f-a-100",
                Some("identity-a"),
                Some(alias_a.as_str()),
                None,
                LEDGER_LAYER_STRICT_PROVEN,
                ATTRIBUTION_KIND_STRICT_ACCOUNT,
                "0100",
                100_i64,
            ),
            (
                "e-legacy-150",
                "r-legacy-150",
                "f-legacy-150",
                None,
                None,
                Some("legacy-p-150"),
                LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED,
                ATTRIBUTION_KIND_LEGACY_PROFILE,
                "0150",
                150_i64,
            ),
            (
                "e-b-200",
                "r-b-200",
                "f-b-200",
                Some("identity-b"),
                Some(alias_b.as_str()),
                None,
                LEDGER_LAYER_STRICT_PROVEN,
                ATTRIBUTION_KIND_STRICT_ACCOUNT,
                "0200",
                200_i64,
            ),
            (
                "e-legacy-250",
                "r-legacy-250",
                "f-legacy-250",
                None,
                None,
                Some("legacy-p-250"),
                LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED,
                ATTRIBUTION_KIND_LEGACY_PROFILE,
                "0250",
                250_i64,
            ),
        ];
        for (old_evidence_id, row) in evidence_ids.iter().zip(rows) {
            conn.execute(
                r#"
                UPDATE openai_usage_facts
                SET evidence_id = ?1, request_id = ?2, fingerprint = ?3,
                    profile = 'p', account_label = NULL, account_identity_key = ?4,
                    account_id_alias = ?5, email_alias = NULL, legacy_account_key = ?6,
                    ledger_layer = ?7, attribution_kind = ?8,
                    attribution_window_id = ?9, created_at_ms = ?10
                WHERE evidence_id = ?11
                "#,
                rusqlite::params![
                    row.0,
                    row.1,
                    row.2,
                    row.3,
                    row.4,
                    row.5,
                    row.6,
                    row.7,
                    row.8,
                    row.9,
                    old_evidence_id,
                ],
            )
            .unwrap();
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(dashboard.account_summary.len(), 2);
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.as_deref().unwrap(),
                    (
                        account.request_count,
                        account.tokens.input_tokens,
                        account.tokens.output_tokens,
                        account.tokens.real_total_tokens,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&(2, 30, 3, 33)));
        assert_eq!(accounts.get(alias_b.as_str()), Some(&(2, 70, 7, 77)));
        assert_eq!(accounts.values().map(|value| value.0).sum::<u64>(), 4);
        assert_eq!(accounts.values().map(|value| value.1).sum::<u64>(), 100);
        assert_eq!(accounts.values().map(|value| value.2).sum::<u64>(), 10);
        assert_eq!(accounts.values().map(|value| value.3).sum::<u64>(), 110);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_canonical_account_filter_includes_latest_prior_usage() {
        let codex_home = temp_codex_home("canonical-filter-latest-prior");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "canonical-filter-latest-prior",
            &[(100, 10), (160, 16)],
        );
        let account_alias = identity::account_id_alias(Some("acct_canonical_filter")).unwrap();
        let strict_created_at = i64::try_from(ts("2999-01-01T00:00:01.000Z")).unwrap();
        let legacy_created_at = i64::try_from(ts("2999-01-01T00:00:02.000Z")).unwrap();
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        rewrite_identity_fact(
            &conn,
            &evidence_ids[0],
            &confirmed_identity_fact(
                "e-canonical-strict",
                "r-canonical-strict",
                "p",
                "identity-canonical-old",
                &account_alias,
                "0100",
                strict_created_at,
            ),
        );
        rewrite_identity_fact(
            &conn,
            &evidence_ids[1],
            &legacy_identity_fact(
                "e-canonical-legacy",
                "r-canonical-legacy",
                "p",
                "legacy-canonical-old",
                "0200",
                legacy_created_at,
            ),
        );
        drop(conn);

        let range_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:03.000Z") / 1000) as i64),
            scope: Some(SCOPE_COMBINED_ATTRIBUTED.to_string()),
            ..OpenAiUsageQuery::default()
        };
        let unfiltered =
            load_openai_usage_dashboard(Some(&codex_home), Some(range_query.clone()), 1, 20)
                .unwrap();
        assert_eq!(unfiltered.account_summary.len(), 1);
        let canonical_key = unfiltered.account_summary[0]
            .account_group_key
            .clone()
            .unwrap();
        let filtered_query = OpenAiUsageQuery {
            account_group_key: Some(canonical_key),
            ..range_query
        };

        let dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(filtered_query.clone()), 1, 1)
                .unwrap();
        let summary =
            load_openai_usage_summary(Some(&codex_home), Some(filtered_query.clone())).unwrap();
        let trends =
            load_openai_usage_trends(Some(&codex_home), Some(filtered_query.clone())).unwrap();
        let logs =
            load_openai_usage_request_logs(Some(&codex_home), Some(filtered_query), 1, 20).unwrap();

        let expected_request_ids = BTreeSet::from([
            "r-canonical-legacy".to_string(),
            "r-canonical-strict".to_string(),
        ]);
        assert_eq!(dashboard.summary.total_requests, 2);
        assert_eq!(dashboard.summary.tokens.real_total_tokens, 176);
        assert_eq!(dashboard.logs.total, 2);
        assert_eq!(dashboard.logs.data.len(), 1);
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.tokens.real_total_tokens, 176);
        assert_eq!(
            trends.iter().map(|point| point.request_count).sum::<u64>(),
            2
        );
        assert_eq!(logs.total, 2);
        assert_eq!(
            logs.data
                .iter()
                .map(|entry| entry.request_id.clone())
                .collect::<BTreeSet<_>>(),
            expected_request_ids
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn canonical_account_filter_accepts_previous_aliases_across_views() {
        let codex_home = temp_codex_home("canonical-filter-aliases");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "canonical-filter-aliases",
            &[(100, 10), (160, 16)],
        );
        let account_alias = identity::account_id_alias(Some("acct_alias_filter")).unwrap();
        let email_alias = identity::email_alias(Some("alias-filter@example.com")).unwrap();
        let raw_old_identity_key = "identity-alias-filter-old";
        let old_identity_key = format!("identity:{raw_old_identity_key}");
        let raw_legacy_key = "legacy-alias-filter-old";
        let legacy_key = format!("legacy:{raw_legacy_key}");
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        rewrite_identity_fact(
            &conn,
            &evidence_ids[0],
            &confirmed_identity_fact(
                "e-alias-strict",
                "r-alias-strict",
                "p",
                "identity-alias-filter-old",
                &account_alias,
                "0100",
                i64::try_from(ts("2999-01-01T00:00:01.000Z")).unwrap(),
            ),
        );
        conn.execute(
            "UPDATE openai_usage_facts SET email_alias = ?1 WHERE evidence_id = 'e-alias-strict'",
            [&email_alias],
        )
        .unwrap();
        rewrite_identity_fact(
            &conn,
            &evidence_ids[1],
            &legacy_identity_fact(
                "e-alias-legacy",
                "r-alias-legacy",
                "p",
                "legacy-alias-filter-old",
                "0200",
                i64::try_from(ts("2999-01-01T00:00:02.000Z")).unwrap(),
            ),
        );
        drop(conn);

        let base_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:03.000Z") / 1000) as i64),
            scope: Some(SCOPE_COMBINED_ATTRIBUTED.to_string()),
            ..OpenAiUsageQuery::default()
        };
        let dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(base_query.clone()), 1, 20)
                .unwrap();
        let canonical_key = dashboard.account_summary[0]
            .account_group_key
            .clone()
            .unwrap();
        let expected_request_ids =
            BTreeSet::from(["r-alias-legacy".to_string(), "r-alias-strict".to_string()]);

        for query_key in [
            canonical_key.clone(),
            old_identity_key,
            raw_old_identity_key.to_string(),
            account_alias,
            email_alias,
            legacy_key,
            raw_legacy_key.to_string(),
        ] {
            let query = OpenAiUsageQuery {
                account_group_key: Some(query_key.clone()),
                ..base_query.clone()
            };
            let dashboard =
                load_openai_usage_dashboard(Some(&codex_home), Some(query.clone()), 1, 20).unwrap();
            let trends = load_openai_usage_trends(Some(&codex_home), Some(query.clone())).unwrap();
            let logs =
                load_openai_usage_request_logs(Some(&codex_home), Some(query), 1, 20).unwrap();
            assert_eq!(dashboard.summary.total_requests, 2, "{query_key}");
            assert_eq!(
                dashboard.summary.tokens.real_total_tokens, 176,
                "{query_key}"
            );
            assert_eq!(
                trends.iter().map(|point| point.request_count).sum::<u64>(),
                2,
                "{query_key}"
            );
            assert_eq!(logs.total, 2, "{query_key}");
            assert_eq!(
                logs.data
                    .iter()
                    .map(|entry| entry.request_id.clone())
                    .collect::<BTreeSet<_>>(),
                expected_request_ids,
                "{query_key}"
            );
        }
        let legacy_dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                account_group_key: Some(canonical_key),
                scope: Some(SCOPE_LEGACY.to_string()),
                ..base_query
            }),
            1,
            20,
        )
        .unwrap();
        assert_eq!(legacy_dashboard.summary.total_requests, 1);
        assert_eq!(legacy_dashboard.logs.total, 1);
        assert_eq!(legacy_dashboard.logs.data[0].request_id, "r-alias-legacy");
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn canonical_account_key_is_stable_across_model_filters() {
        let codex_home = temp_codex_home("canonical-key-model-stability");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let account_alias = identity::account_id_alias(Some("acct_model_stability")).unwrap();
        let email_alias = identity::email_alias(Some("model-stability@example.com")).unwrap();
        for record in [
            AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("email-profile".to_string()),
                account_label: Some("model-stability@example.com".to_string()),
                account_identity_key: Some("identity-model-email".to_string()),
                account_id_alias: None,
                email_alias: Some(email_alias.clone()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
            AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("dual-profile".to_string()),
                account_label: Some("dual".to_string()),
                account_identity_key: Some("identity-model-dual".to_string()),
                account_id_alias: Some(account_alias.clone()),
                email_alias: Some(email_alias),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
            AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:05.000Z"),
                profile: Some("account-profile".to_string()),
                account_label: Some("account".to_string()),
                account_identity_key: Some("identity-model-account".to_string()),
                account_id_alias: Some(account_alias),
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        ] {
            append_attribution_record(&codex_home, &record).unwrap();
        }
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-canonical-model-stability"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"model-email"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.500Z","type":"turn_context","payload":{"model":"model-dual"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":0,"output_tokens":16,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:05.500Z","type":"turn_context","payload":{"model":"model-account"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:06.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":220,"cached_input_tokens":0,"output_tokens":22,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let base_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:07.000Z") / 1000) as i64),
            ..OpenAiUsageQuery::default()
        };
        let unfiltered =
            load_openai_usage_dashboard(Some(&codex_home), Some(base_query.clone()), 1, 20)
                .unwrap();
        assert_eq!(unfiltered.account_summary.len(), 1);
        let canonical_key = unfiltered.account_summary[0]
            .account_group_key
            .clone()
            .unwrap();

        let filtered = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                account_group_key: Some(canonical_key.clone()),
                model: Some("model-email".to_string()),
                ..base_query
            }),
            1,
            20,
        )
        .unwrap();

        assert_eq!(filtered.summary.total_requests, 1);
        assert_eq!(filtered.logs.total, 1);
        assert_eq!(filtered.logs.data[0].model, "model-email");
        assert_eq!(filtered.account_summary.len(), 1);
        assert_eq!(
            filtered.account_summary[0].account_group_key.as_deref(),
            Some(canonical_key.as_str())
        );
        assert_eq!(
            filtered
                .model_options
                .iter()
                .map(|model| model.model.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["model-account", "model-dual", "model-email"])
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn canonical_account_filter_fails_closed_and_respects_profile_scope() {
        let codex_home = temp_codex_home("canonical-filter-profile-scope");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let account_alias = identity::account_id_alias(Some("acct_profile_scope")).unwrap();
        let email_alias = identity::email_alias(Some("profile-scope@example.com")).unwrap();
        for record in [
            AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("email-profile".to_string()),
                account_label: Some("profile-scope@example.com".to_string()),
                account_identity_key: Some("identity-profile-email".to_string()),
                account_id_alias: None,
                email_alias: Some(email_alias.clone()),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
            AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("dual-profile".to_string()),
                account_label: Some("dual".to_string()),
                account_identity_key: Some("identity-profile-dual".to_string()),
                account_id_alias: Some(account_alias),
                email_alias: Some(email_alias),
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        ] {
            append_attribution_record(&codex_home, &record).unwrap();
        }
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-canonical-profile-scope"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":0,"output_tokens":16,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );
        let base_query = OpenAiUsageQuery {
            start_date: Some((ts("2999-01-01T00:00:00.000Z") / 1000) as i64),
            end_date: Some((ts("2999-01-01T00:00:05.000Z") / 1000) as i64),
            ..OpenAiUsageQuery::default()
        };
        let unfiltered =
            load_openai_usage_dashboard(Some(&codex_home), Some(base_query.clone()), 1, 20)
                .unwrap();
        let canonical_key = unfiltered.account_summary[0]
            .account_group_key
            .clone()
            .unwrap();
        let profile_query = OpenAiUsageQuery {
            profile: Some("email-profile".to_string()),
            account_group_key: Some(canonical_key),
            ..base_query.clone()
        };
        let dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(profile_query.clone()), 1, 20)
                .unwrap();
        let trends =
            load_openai_usage_trends(Some(&codex_home), Some(profile_query.clone())).unwrap();
        let logs =
            load_openai_usage_request_logs(Some(&codex_home), Some(profile_query), 1, 20).unwrap();
        assert_eq!(dashboard.summary.total_requests, 1);
        assert_eq!(dashboard.summary.accounts[0].profile, "email-profile");
        assert_eq!(
            trends.iter().map(|point| point.request_count).sum::<u64>(),
            1
        );
        assert_eq!(logs.total, 1);
        assert_eq!(logs.data[0].profile, "email-profile");

        let unknown_query = OpenAiUsageQuery {
            account_group_key: Some("identity:unknown-canonical-key".to_string()),
            ..base_query
        };
        let unknown_dashboard =
            load_openai_usage_dashboard(Some(&codex_home), Some(unknown_query.clone()), 1, 20)
                .unwrap();
        let unknown_trends =
            load_openai_usage_trends(Some(&codex_home), Some(unknown_query.clone())).unwrap();
        let unknown_logs =
            load_openai_usage_request_logs(Some(&codex_home), Some(unknown_query), 1, 20).unwrap();
        assert_eq!(unknown_dashboard.summary.total_requests, 0);
        assert_eq!(unknown_dashboard.logs.total, 0);
        assert_eq!(
            unknown_trends
                .iter()
                .map(|point| point.request_count)
                .sum::<u64>(),
            0
        );
        assert_eq!(unknown_logs.total, 0);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_keeps_pre_identity_usage_unattributed() {
        let codex_home = temp_codex_home("dashboard-pre-identity-unknown");
        let evidence_ids =
            seed_usage_fact_fixture(&codex_home, "pre-identity-unknown", &[(10, 1), (30, 3)]);
        assert_eq!(evidence_ids.len(), 2);
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        rewrite_identity_fact(
            &conn,
            &evidence_ids[0],
            &legacy_identity_fact(
                "e-legacy-050",
                "r-legacy-050",
                "p",
                "legacy-p-050",
                "0050",
                50,
            ),
        );
        rewrite_identity_fact(
            &conn,
            &evidence_ids[1],
            &confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "0100",
                100,
            ),
        );
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(dashboard.account_summary.len(), 2);
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.as_deref().unwrap(),
                    (
                        account.request_count,
                        account.tokens.input_tokens,
                        account.tokens.output_tokens,
                        account.tokens.real_total_tokens,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get("legacy:legacy-p-050"), Some(&(1, 10, 1, 11)));
        assert_eq!(accounts.get(alias_a.as_str()), Some(&(1, 20, 2, 22)));
        assert_eq!(accounts.values().map(|value| value.0).sum::<u64>(), 2);
        assert_eq!(accounts.values().map(|value| value.3).sum::<u64>(), 33);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_latest_prior_is_profile_scoped_and_order_independent() {
        let codex_home = temp_codex_home("dashboard-profile-scoped-latest-prior");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "profile-scoped-latest-prior",
            &[
                (10, 1),
                (30, 3),
                (60, 6),
                (100, 10),
                (150, 15),
                (210, 21),
                (280, 28),
                (360, 36),
            ],
        );
        assert_eq!(evidence_ids.len(), 8);
        let alias_x = identity::account_id_alias(Some("acct_timeline_x")).unwrap();
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let alias_b = identity::account_id_alias(Some("acct_timeline_b")).unwrap();
        let alias_c = identity::account_id_alias(Some("acct_timeline_c")).unwrap();
        let fixtures = [
            confirmed_identity_fact(
                "e-x-040",
                "r-x-040",
                "q",
                "identity-x",
                &alias_x,
                "0040",
                40,
            ),
            legacy_identity_fact(
                "e-legacy-050",
                "r-legacy-050",
                "p",
                "legacy-p-050",
                "0050",
                50,
            ),
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "0100",
                100,
            ),
            legacy_identity_fact(
                "e-legacy-150",
                "r-legacy-150",
                "p",
                "legacy-p-150",
                "0150",
                150,
            ),
            confirmed_identity_fact(
                "e-b-200",
                "z-confirmed-b-200",
                "p",
                "identity-b",
                &alias_b,
                "200",
                200,
            ),
            legacy_identity_fact(
                "e-legacy-after-b-200",
                "a-legacy-after-b-200",
                "p",
                "legacy-p-after-b-200",
                "201",
                200,
            ),
            legacy_identity_fact(
                "e-legacy-before-c-300",
                "z-legacy-before-c-300",
                "p",
                "legacy-p-before-c-300",
                "300",
                300,
            ),
            confirmed_identity_fact(
                "e-c-300",
                "a-confirmed-c-300",
                "p",
                "identity-c",
                &alias_c,
                "301",
                300,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        assert_eq!(first_counts.get(alias_x.as_str()), Some(&1));
        assert_eq!(first_counts.get("legacy:legacy-p-050"), Some(&1));
        assert_eq!(first_counts.get(alias_a.as_str()), Some(&2));
        assert_eq!(first_counts.get(alias_b.as_str()), Some(&3));
        assert_eq!(first_counts.get(alias_c.as_str()), Some(&1));
        assert_eq!(first_counts.values().sum::<u64>(), 8);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            396
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'a-confirmed-b-200' WHERE evidence_id = 'e-b-200'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'z-legacy-after-b-200' WHERE evidence_id = 'e-legacy-after-b-200'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'a-legacy-before-c-300' WHERE evidence_id = 'e-legacy-before-c-300'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'z-confirmed-c-300' WHERE evidence_id = 'e-c-300'",
            [],
        )
        .unwrap();
        drop(conn);

        let reversed = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&reversed), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_latest_prior_rejects_non_strict_typed_aliases() {
        let codex_home = temp_codex_home("dashboard-latest-prior-strict-confirmation");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "latest-prior-strict-confirmation",
            &[(10, 1), (30, 3), (60, 6), (100, 10), (150, 15)],
        );
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let alias_b = identity::account_id_alias(Some("acct_timeline_b")).unwrap();
        let alias_c = identity::account_id_alias(Some("acct_timeline_c")).unwrap();
        let mut non_strict_layer = confirmed_identity_fact(
            "e-b-200",
            "r-b-200",
            "p",
            "identity-b",
            &alias_b,
            "200",
            200,
        );
        non_strict_layer.ledger_layer = LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED;
        let mut non_strict_kind = confirmed_identity_fact(
            "e-c-300",
            "r-c-300",
            "p",
            "identity-c",
            &alias_c,
            "300",
            300,
        );
        non_strict_kind.attribution_kind = ATTRIBUTION_KIND_LEGACY_PROFILE;
        let fixtures = [
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "100",
                100,
            ),
            non_strict_layer,
            legacy_identity_fact(
                "e-legacy-250",
                "r-legacy-250",
                "p",
                "legacy-p-250",
                "250",
                250,
            ),
            non_strict_kind,
            legacy_identity_fact(
                "e-legacy-350",
                "r-legacy-350",
                "p",
                "legacy-p-350",
                "350",
                350,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&3));
        assert_eq!(accounts.get(alias_b.as_str()), Some(&1));
        assert_eq!(accounts.get(alias_c.as_str()), Some(&1));
        assert_eq!(accounts.values().sum::<u64>(), 5);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            165
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_timeline_uses_profile_scoped_trusted_canonical() {
        let codex_home = temp_codex_home("dashboard-profile-scoped-trusted-canonical");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "profile-scoped-trusted-canonical",
            &[(10, 1), (30, 3), (60, 6)],
        );
        let alias_one = identity::account_id_alias(Some("acct_trusted_one")).unwrap();
        let alias_two = identity::account_id_alias(Some("acct_trusted_two")).unwrap();
        let (alias_a, alias_b) = if alias_one > alias_two {
            (alias_one, alias_two)
        } else {
            (alias_two, alias_one)
        };
        assert!(alias_b < alias_a);
        let mut non_strict_b = confirmed_identity_fact(
            "e-b-110",
            "r-b-110",
            "q",
            "shared-fallback",
            &alias_b,
            "110",
            110,
        );
        non_strict_b.ledger_layer = LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED;
        non_strict_b.attribution_kind = ATTRIBUTION_KIND_LEGACY_PROFILE;
        let fixtures = [
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "shared-fallback",
                &alias_a,
                "100",
                100,
            ),
            non_strict_b,
            legacy_identity_fact(
                "e-legacy-p-150",
                "r-legacy-p-150",
                "p",
                "legacy-p-150",
                "150",
                150,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&2));
        assert_eq!(accounts.get(alias_b.as_str()), Some(&1));
        assert_eq!(accounts.values().sum::<u64>(), 3);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            66
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_eligible_legacy_ignores_untrusted_fallback_component() {
        let codex_home = temp_codex_home("dashboard-untrusted-fallback-bypass");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "untrusted-fallback-bypass",
            &[(10, 1), (30, 3), (60, 6), (100, 10)],
        );
        let alias_one = identity::account_id_alias(Some("acct_prior_one")).unwrap();
        let alias_two = identity::account_id_alias(Some("acct_future_two")).unwrap();
        let (alias_a, alias_b) = if alias_one > alias_two {
            (alias_one, alias_two)
        } else {
            (alias_two, alias_one)
        };
        let mut strict_a =
            confirmed_identity_fact("e-a-100", "r-a-100", "p", "unused-a", &alias_a, "100", 100);
        strict_a.account_identity_key = None;
        let mut future_non_strict_b =
            confirmed_identity_fact("e-b-200", "r-b-200", "q", "unused-b", &alias_b, "200", 200);
        future_non_strict_b.account_identity_key = None;
        future_non_strict_b.ledger_layer = LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED;
        future_non_strict_b.attribution_kind = ATTRIBUTION_KIND_LEGACY_PROFILE;
        let fixtures = [
            strict_a,
            legacy_identity_fact(
                "e-legacy-150-a",
                "a-legacy-150",
                "p",
                "legacy-p-150-a",
                "150",
                150,
            ),
            legacy_identity_fact(
                "e-legacy-150-z",
                "z-legacy-150",
                "p",
                "legacy-p-150-z",
                "150",
                150,
            ),
            future_non_strict_b,
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        conn.execute(
            "UPDATE openai_usage_facts SET account_label = 'acct_shared' WHERE evidence_id IN ('e-a-100', 'e-legacy-150-a', 'e-legacy-150-z', 'e-b-200')",
            [],
        )
        .unwrap();
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        assert_eq!(first_counts.get(alias_a.as_str()), Some(&3));
        assert_eq!(first_counts.get(alias_b.as_str()), Some(&1));
        assert_eq!(first_counts.values().sum::<u64>(), 4);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            110
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'z-legacy-150' WHERE evidence_id = 'e-legacy-150-a'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'a-legacy-150' WHERE evidence_id = 'e-legacy-150-z'",
            [],
        )
        .unwrap();
        drop(conn);

        let permuted = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&permuted), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_timeline_reuses_safe_global_trusted_alias_canonical() {
        let codex_home = temp_codex_home("dashboard-safe-global-trusted-canonical");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "safe-global-trusted-canonical",
            &[(10, 1), (30, 3), (60, 6)],
        );
        let account_alias = identity::account_id_alias(Some("acct_global_trusted")).unwrap();
        let email_alias = identity::email_alias(Some("trusted@example.com")).unwrap();
        let fixtures = [
            confirmed_identity_fact(
                "e-p-email-100",
                "z-p-email-100",
                "p",
                "identity-p",
                &account_alias,
                "100",
                100,
            ),
            confirmed_identity_fact(
                "e-q-dual-100",
                "a-q-dual-100",
                "q",
                "identity-q",
                &account_alias,
                "100",
                100,
            ),
            legacy_identity_fact(
                "e-p-legacy-150",
                "r-p-legacy-150",
                "p",
                "legacy-p-150",
                "150",
                150,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        conn.execute(
            "UPDATE openai_usage_facts SET account_id_alias = NULL, email_alias = ?1 WHERE evidence_id = 'e-p-email-100'",
            [&email_alias],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET email_alias = ?1 WHERE evidence_id = 'e-q-dual-100'",
            [&email_alias],
        )
        .unwrap();
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        assert_eq!(first_counts.get(account_alias.as_str()), Some(&3));
        assert_eq!(first_counts.len(), 1);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            66
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'a-p-email-100' WHERE evidence_id = 'e-p-email-100'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'z-q-dual-100' WHERE evidence_id = 'e-q-dual-100'",
            [],
        )
        .unwrap();
        drop(conn);

        let permuted = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&permuted), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_trusted_global_canonical_survives_untrusted_component_pollution() {
        let codex_home = temp_codex_home("dashboard-trusted-global-with-pollution");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "trusted-global-with-pollution",
            &[(10, 1), (30, 3), (60, 6), (100, 10)],
        );
        let alias_one = identity::account_id_alias(Some("acct_trusted_account")).unwrap();
        let alias_two = identity::account_id_alias(Some("acct_untrusted_smaller")).unwrap();
        let (alias_a, alias_b) = if alias_one > alias_two {
            (alias_one, alias_two)
        } else {
            (alias_two, alias_one)
        };
        let email_alias = identity::email_alias(Some("bridge@example.com")).unwrap();
        let mut non_strict_b = confirmed_identity_fact(
            "e-b-200",
            "r-b-200",
            "r",
            "shared-fallback",
            &alias_b,
            "200",
            200,
        );
        non_strict_b.ledger_layer = LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED;
        non_strict_b.attribution_kind = ATTRIBUTION_KIND_LEGACY_PROFILE;
        let fixtures = [
            confirmed_identity_fact(
                "e-p-email-100",
                "z-p-email-100",
                "p",
                "identity-p",
                &alias_a,
                "100",
                100,
            ),
            confirmed_identity_fact(
                "e-q-dual-100",
                "a-q-dual-100",
                "q",
                "shared-fallback",
                &alias_a,
                "100",
                100,
            ),
            legacy_identity_fact(
                "e-p-legacy-150",
                "r-p-legacy-150",
                "p",
                "legacy-p-150",
                "150",
                150,
            ),
            non_strict_b,
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        conn.execute(
            "UPDATE openai_usage_facts SET account_id_alias = NULL, email_alias = ?1 WHERE evidence_id = 'e-p-email-100'",
            [&email_alias],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET email_alias = ?1 WHERE evidence_id = 'e-q-dual-100'",
            [&email_alias],
        )
        .unwrap();
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        assert_eq!(first_counts.get(alias_a.as_str()), Some(&3));
        assert_eq!(first_counts.get(alias_b.as_str()), Some(&1));
        assert_eq!(first_counts.values().sum::<u64>(), 4);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            110
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'a-p-email-100' WHERE evidence_id = 'e-p-email-100'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET request_id = 'z-q-dual-100' WHERE evidence_id = 'e-q-dual-100'",
            [],
        )
        .unwrap();
        drop(conn);

        let permuted = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&permuted), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_latest_prior_rejects_malformed_strict_typed_alias() {
        let codex_home = temp_codex_home("dashboard-latest-prior-malformed-alias");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "latest-prior-malformed-alias",
            &[(10, 1), (30, 3), (60, 6)],
        );
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let malformed = confirmed_identity_fact(
            "e-b-200",
            "r-b-200",
            "p",
            "identity-b",
            "acct:not-a-valid-digest",
            "200",
            200,
        );
        let fixtures = [
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "100",
                100,
            ),
            malformed,
            legacy_identity_fact(
                "e-legacy-250",
                "r-legacy-250",
                "p",
                "legacy-p-250",
                "250",
                250,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&2));
        assert_eq!(accounts.get("identity:identity-b"), Some(&1));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts.values().sum::<u64>(), 3);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            66
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_latest_prior_does_not_override_legacy_fact_with_identity_key() {
        let codex_home = temp_codex_home("dashboard-latest-prior-legacy-identity-key");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "latest-prior-legacy-identity-key",
            &[(10, 1), (30, 3)],
        );
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let mut legacy_with_identity = legacy_identity_fact(
            "e-legacy-b-150",
            "r-legacy-b-150",
            "p",
            "legacy-p-b-150",
            "150",
            150,
        );
        legacy_with_identity.account_identity_key = Some("identity-b");
        let fixtures = [
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "100",
                100,
            ),
            legacy_with_identity,
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&1));
        assert_eq!(accounts.get("identity:identity-b"), Some(&1));
        assert_eq!(accounts.values().sum::<u64>(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            33
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_snapshot_verified_fact_can_establish_latest_prior() {
        let codex_home = temp_codex_home("dashboard-snapshot-verified-latest-prior");
        write_profile_snapshot(
            &codex_home,
            "p",
            r#"{"tokens":{"account_id":"acct_A","id_token":"x.eyJlbWFpbCI6IlVzZXJARXhhbXBsZS5jb20ifQ.x"}}"#,
        );
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "snapshot-verified-latest-prior",
            &[(10, 1), (30, 3)],
        );
        let alias_a = identity::account_id_alias(Some("acct_A")).unwrap();
        let mut snapshot_verified = legacy_identity_fact(
            "e-snapshot-100",
            "r-snapshot-100",
            "p",
            "legacy-p-snapshot",
            "100",
            100,
        );
        snapshot_verified.account_identity_key = Some("acct_b12448b5e4c359a3");
        let fixtures = [
            snapshot_verified,
            legacy_identity_fact(
                "e-legacy-150",
                "r-legacy-150",
                "p",
                "legacy-p-150",
                "150",
                150,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(dashboard.account_summary.len(), 1);
        let account = &dashboard.account_summary[0];
        assert_eq!(account.account_group_key.as_deref(), Some(alias_a.as_str()));
        assert_eq!(account.request_count, 2);
        assert_eq!(account.tokens.real_total_tokens, 33);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_ambiguous_confirmed_cohort_keeps_previous_prior_or_unknown() {
        let codex_home = temp_codex_home("dashboard-ambiguous-confirmed-cohort");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "ambiguous-confirmed-cohort",
            &[(10, 1), (30, 3), (60, 6), (100, 10), (150, 15), (210, 21)],
        );
        let alias_a = identity::account_id_alias(Some("acct_timeline_a")).unwrap();
        let alias_b = identity::account_id_alias(Some("acct_timeline_b")).unwrap();
        let alias_c = identity::account_id_alias(Some("acct_timeline_c")).unwrap();
        let alias_d = identity::account_id_alias(Some("acct_timeline_d")).unwrap();
        let mut missing_window = confirmed_identity_fact(
            "e-c-200",
            "r-c-200",
            "p",
            "identity-c",
            &alias_c,
            "unused",
            200,
        );
        missing_window.attribution_window_id = None;
        let fixtures = [
            confirmed_identity_fact("a-b-050", "r-b-050", "p", "identity-b", &alias_b, "50", 50),
            legacy_identity_fact(
                "z-legacy-050",
                "r-legacy-050",
                "p",
                "legacy-p-050",
                "50",
                50,
            ),
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &alias_a,
                "100",
                100,
            ),
            missing_window,
            confirmed_identity_fact(
                "e-d-200",
                "r-d-200",
                "p",
                "identity-d",
                &alias_d,
                "not-a-number",
                200,
            ),
            legacy_identity_fact(
                "e-legacy-250",
                "r-legacy-250",
                "p",
                "legacy-p-250",
                "250",
                250,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounts.get(alias_a.as_str()), Some(&2));
        assert_eq!(accounts.get(alias_b.as_str()), Some(&1));
        assert_eq!(accounts.get(alias_c.as_str()), Some(&1));
        assert_eq!(accounts.get(alias_d.as_str()), Some(&1));
        assert_eq!(accounts.get("legacy:legacy-p-050"), Some(&1));
        assert_eq!(accounts.values().sum::<u64>(), 6);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            231
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_same_millisecond_unknown_competitor_blocks_prior_cohort() {
        let codex_home = temp_codex_home("dashboard-same-ms-three-state-cohort");
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "same-ms-three-state-cohort",
            &[
                (10, 1),
                (30, 3),
                (60, 6),
                (100, 10),
                (150, 15),
                (210, 21),
                (280, 28),
                (360, 36),
            ],
        );
        let alias_previous = identity::account_id_alias(Some("acct_previous")).unwrap();
        let alias_a = identity::account_id_alias(Some("acct_candidate_a")).unwrap();
        let alias_b = identity::account_id_alias(Some("acct_unknown_b")).unwrap();
        let alias_after = identity::account_id_alias(Some("acct_after")).unwrap();
        let mut missing_window = confirmed_identity_fact(
            "e-b-missing",
            "r-3",
            "p",
            "identity-b-missing",
            &alias_b,
            "unused",
            200,
        );
        missing_window.attribution_window_id = None;
        let fixtures = [
            confirmed_identity_fact(
                "e-previous-100",
                "r-0",
                "p",
                "identity-previous",
                &alias_previous,
                "100",
                100,
            ),
            confirmed_identity_fact("e-a-prior", "r-1", "p", "identity-a", &alias_a, "100", 200),
            missing_window,
            confirmed_identity_fact(
                "e-b-nonnumeric",
                "r-4",
                "p",
                "identity-b-nonnumeric",
                &alias_b,
                "not-a-number",
                200,
            ),
            confirmed_identity_fact(
                "e-b-overflow",
                "r-5",
                "p",
                "identity-b-overflow",
                &alias_b,
                "18446744073709551616",
                200,
            ),
            confirmed_identity_fact(
                "e-b-equal",
                "r-6",
                "p",
                "identity-b-equal",
                &alias_b,
                "150",
                200,
            ),
            confirmed_identity_fact(
                "e-after",
                "r-7",
                "p",
                "identity-after",
                &alias_after,
                "200",
                200,
            ),
            legacy_identity_fact("e-event", "r-8", "p", "legacy-p-event", "150", 200),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        assert_eq!(first_counts.get(alias_previous.as_str()), Some(&2));
        assert_eq!(first_counts.get(alias_a.as_str()), Some(&1));
        assert_eq!(first_counts.get(alias_b.as_str()), Some(&4));
        assert_eq!(first_counts.get(alias_after.as_str()), Some(&1));
        assert_eq!(first_counts.values().sum::<u64>(), 8);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            396
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (index, fixture) in fixtures.iter().enumerate() {
            conn.execute(
                "UPDATE openai_usage_facts SET request_id = ?1 WHERE evidence_id = ?2",
                rusqlite::params![
                    format!("permuted-{}", fixtures.len() - index),
                    fixture.evidence_id
                ],
            )
            .unwrap();
        }
        drop(conn);

        let permuted = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&permuted), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_latest_prior_sweep_is_linear_for_pre_identity_history() {
        const LEGACY_COUNT: usize = 25;
        const STRICT_COUNT: usize = 25;
        let codex_home = temp_codex_home("dashboard-latest-prior-linear-sweep");
        let cumulative_totals = (1_u64..=(LEGACY_COUNT + STRICT_COUNT) as u64)
            .map(|value| {
                let triangular = value * (value + 1) / 2;
                (triangular * 10, triangular)
            })
            .collect::<Vec<_>>();
        let evidence_ids =
            seed_usage_fact_fixture(&codex_home, "latest-prior-linear-sweep", &cumulative_totals);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (index, old_evidence_id) in evidence_ids.iter().enumerate() {
            if index < LEGACY_COUNT {
                let evidence_id = format!("e-legacy-{index:03}");
                let legacy_key = format!("legacy-p-{index:03}");
                conn.execute(
                    r#"
                    UPDATE openai_usage_facts
                    SET evidence_id = ?1, request_id = ?2, fingerprint = ?1,
                        profile = 'p', account_label = NULL, account_identity_key = NULL,
                        account_id_alias = NULL, email_alias = NULL, legacy_account_key = ?3,
                        ledger_layer = ?4, attribution_kind = ?5,
                        attribution_window_id = ?6, created_at_ms = ?7
                    WHERE evidence_id = ?8
                    "#,
                    rusqlite::params![
                        evidence_id,
                        format!("r-legacy-{index:03}"),
                        legacy_key,
                        LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED,
                        ATTRIBUTION_KIND_LEGACY_PROFILE,
                        (index + 1).to_string(),
                        (index + 1) as i64,
                        old_evidence_id,
                    ],
                )
                .unwrap();
            } else {
                let strict_index = index - LEGACY_COUNT;
                let evidence_id = format!("e-strict-{strict_index:03}");
                let account_alias =
                    identity::account_id_alias(Some(&format!("acct-strict-{strict_index:03}")))
                        .unwrap();
                conn.execute(
                    r#"
                    UPDATE openai_usage_facts
                    SET evidence_id = ?1, request_id = ?2, fingerprint = ?1,
                        profile = 'p', account_label = NULL, account_identity_key = ?3,
                        account_id_alias = ?4, email_alias = NULL, legacy_account_key = NULL,
                        ledger_layer = ?5, attribution_kind = ?6,
                        attribution_window_id = ?7, created_at_ms = ?8
                    WHERE evidence_id = ?9
                    "#,
                    rusqlite::params![
                        evidence_id,
                        format!("r-strict-{strict_index:03}"),
                        format!("identity-strict-{strict_index:03}"),
                        account_alias,
                        LEDGER_LAYER_STRICT_PROVEN,
                        ATTRIBUTION_KIND_STRICT_ACCOUNT,
                        (100 + strict_index).to_string(),
                        (100 + strict_index) as i64,
                        old_evidence_id,
                    ],
                )
                .unwrap();
            }
        }
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 100).unwrap();
        assert_eq!(dashboard.account_summary.len(), LEGACY_COUNT + STRICT_COUNT);
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.as_deref().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        for index in 0..LEGACY_COUNT {
            assert_eq!(
                accounts.get(format!("legacy:legacy-p-{index:03}").as_str()),
                Some(&1)
            );
        }
        for index in 0..STRICT_COUNT {
            let account_alias =
                identity::account_id_alias(Some(&format!("acct-strict-{index:03}"))).unwrap();
            assert_eq!(accounts.get(account_alias.as_str()), Some(&1));
        }
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            (LEGACY_COUNT + STRICT_COUNT) as u64
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            14_025
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_same_millisecond_prior_requires_earlier_numeric_window() {
        let codex_home = temp_codex_home("dashboard-latest-prior-numeric-window");
        let cumulative_totals = (1_u64..=13)
            .map(|value| {
                let triangular = value * (value + 1) / 2;
                (triangular * 10, triangular)
            })
            .collect::<Vec<_>>();
        let evidence_ids = seed_usage_fact_fixture(
            &codex_home,
            "latest-prior-numeric-window",
            &cumulative_totals,
        );
        let aliases = (b'a'..=b'g')
            .map(|suffix| {
                identity::account_id_alias(Some(&format!("acct_timeline_{}", char::from(suffix))))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut missing_confirmed_window = confirmed_identity_fact(
            "e-c-300",
            "r-c-300",
            "p",
            "identity-c",
            &aliases[2],
            "unused",
            300,
        );
        missing_confirmed_window.attribution_window_id = None;
        let mut missing_event_window = legacy_identity_fact(
            "e-legacy-400",
            "r-legacy-400",
            "p",
            "legacy-p-400",
            "unused",
            400,
        );
        missing_event_window.attribution_window_id = None;
        let fixtures = [
            confirmed_identity_fact(
                "e-a-100",
                "r-a-100",
                "p",
                "identity-a",
                &aliases[0],
                "100",
                100,
            ),
            confirmed_identity_fact(
                "a-b-200",
                "r-b-200",
                "p",
                "identity-b",
                &aliases[1],
                "200",
                200,
            ),
            legacy_identity_fact(
                "z-legacy-200",
                "r-legacy-200",
                "p",
                "legacy-p-200",
                "200",
                200,
            ),
            missing_confirmed_window,
            legacy_identity_fact(
                "e-legacy-300",
                "r-legacy-300",
                "p",
                "legacy-p-300",
                "301",
                300,
            ),
            confirmed_identity_fact(
                "e-d-400",
                "r-d-400",
                "p",
                "identity-d",
                &aliases[3],
                "400",
                400,
            ),
            missing_event_window,
            confirmed_identity_fact(
                "e-e-500",
                "r-e-500",
                "p",
                "identity-e",
                &aliases[4],
                "500-bad",
                500,
            ),
            legacy_identity_fact(
                "e-legacy-500",
                "r-legacy-500",
                "p",
                "legacy-p-500",
                "501",
                500,
            ),
            confirmed_identity_fact(
                "e-f-600",
                "r-f-600",
                "p",
                "identity-f",
                &aliases[5],
                "600",
                600,
            ),
            legacy_identity_fact(
                "e-legacy-600",
                "r-legacy-600",
                "p",
                "legacy-p-600",
                "not-a-number",
                600,
            ),
            confirmed_identity_fact(
                "z-g-700",
                "r-g-700",
                "p",
                "identity-g",
                &aliases[6],
                "700",
                700,
            ),
            legacy_identity_fact(
                "a-legacy-700",
                "r-legacy-700",
                "p",
                "legacy-p-700",
                "701",
                700,
            ),
        ];
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for (old_evidence_id, fixture) in evidence_ids.iter().zip(fixtures.iter()) {
            rewrite_identity_fact(&conn, old_evidence_id, fixture);
        }
        drop(conn);

        let account_counts = |dashboard: &OpenAiUsageDashboard| {
            dashboard
                .account_summary
                .iter()
                .map(|account| {
                    (
                        account.account_group_key.clone().unwrap(),
                        account.request_count,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let first_counts = account_counts(&first);
        for (alias, expected) in aliases.iter().zip([2, 2, 2, 2, 2, 1, 2]) {
            assert_eq!(first_counts.get(alias), Some(&expected));
        }
        assert_eq!(first_counts.values().sum::<u64>(), 13);
        assert_eq!(
            first
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1_001
        );

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET evidence_id = 'swap-temp' WHERE evidence_id = 'a-b-200'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET evidence_id = 'a-b-200' WHERE evidence_id = 'z-legacy-200'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET evidence_id = 'z-legacy-200' WHERE evidence_id = 'swap-temp'",
            [],
        )
        .unwrap();
        drop(conn);

        let swapped = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(account_counts(&swapped), first_counts);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_rejects_mismatched_profile_snapshot_aliases() {
        let codex_home = temp_codex_home("dashboard-mismatched-profile-snapshot");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_profile_snapshot(
            &codex_home,
            "same-profile",
            r#"{"tokens":{"account_id":"acct_B","id_token":"x.eyJlbWFpbCI6Ik90aGVyQEV4YW1wbGUuY29tIn0.x"}}"#,
        );
        let account_alias = identity::account_id_alias(Some("acct_B")).unwrap();
        let email_alias = identity::email_alias(Some("Other@Example.com")).unwrap();
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"same-profile","account_label":"same-label","account_identity_key":"acct_b12448b5e4c359a3","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#.to_string(),
                format!(r#"{{"observed_at_ms":32472144003000,"profile":"same-profile","account_label":"same-label","account_identity_key":"acct_key_current","account_id_alias":"{account_alias}","email_alias":"{email_alias}","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}}"#),
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-mismatched-profile-snapshot"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_rejects_unverified_profile_snapshot_aliases() {
        assert_profile_snapshot_does_not_recover(
            "dashboard-missing-profile-auth",
            None,
            Some("acct_b12448b5e4c359a3"),
        );
        assert_profile_snapshot_does_not_recover(
            "dashboard-corrupt-profile-auth",
            Some("not-json"),
            Some("acct_b12448b5e4c359a3"),
        );
        assert_profile_snapshot_does_not_recover(
            "dashboard-incomplete-profile-identity",
            Some(r#"{"tokens":{"account_id":"acct_A"}}"#),
            Some("acct_b12448b5e4c359a3"),
        );
        assert_profile_snapshot_does_not_recover(
            "dashboard-missing-legacy-identity-key",
            Some(
                r#"{"tokens":{"account_id":"acct_A","id_token":"x.eyJlbWFpbCI6IlVzZXJARXhhbXBsZS5jb20ifQ.x"}}"#,
            ),
            None,
        );
    }

    #[test]
    fn dashboard_keeps_eligible_aliasless_history_out_of_untrusted_fallback_component() {
        let codex_home = temp_codex_home("dashboard-aliasless-history-merge");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"old-strict","account_label":"same@example.com","account_identity_key":"acct_key_same","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"new-strict","account_label":"same@example.com","account_identity_key":"acct_key_same","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144005000,"profile":"legacy","account_label":"same@example.com","account_identity_key":null,"account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-aliasless-history"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:06.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1800,"cached_input_tokens":280,"output_tokens":250,"reasoning_output_tokens":50}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        let accounts = dashboard
            .account_summary
            .iter()
            .map(|account| {
                (
                    account.account_group_key.clone().unwrap(),
                    account.request_count,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            accounts.get("acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829"),
            Some(&2)
        );
        assert_eq!(
            accounts.get("legacy:legacy_profile:legacy:32472144005000"),
            Some(&1)
        );
        assert_eq!(accounts.values().sum::<u64>(), 3);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            2050
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_bridges_email_label_history_to_matching_known_alias() {
        let codex_home = temp_codex_home("dashboard-email-label-alias-bridge");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"email-only-history","account_label":" User@Example.com ","account_identity_key":"acct_681356aab274b3b0","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"dual-current","account_label":"user@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-email-label-alias-bridge"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        let account = &dashboard.account_summary[0];
        assert_eq!(account.request_count, 2);
        assert_eq!(account.tokens.real_total_tokens, 1700);
        assert_eq!(
            account.account_group_key.as_deref(),
            Some("acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_bridges_account_id_label_history_to_matching_known_alias() {
        let codex_home = temp_codex_home("dashboard-account-label-alias-bridge");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"account-only-history","account_label":" acct_A ","account_identity_key":"acct_4e434b75e0952ae5","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"dual-current","account_label":"user@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-account-label-alias-bridge"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        let account = &dashboard.account_summary[0];
        assert_eq!(account.request_count, 2);
        assert_eq!(account.tokens.real_total_tokens, 1700);
        assert_eq!(
            account.account_group_key.as_deref(),
            Some("acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_rejects_email_label_bridge_with_conflicting_identity_key() {
        let codex_home = temp_codex_home("dashboard-conflicting-email-label-key");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"conflicting-history","account_label":"user@example.com","account_identity_key":"acct_4e434b75e0952ae5","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"other-account","account_label":"user@example.com","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-conflicting-email-label-key"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_rejects_account_id_label_bridge_with_conflicting_identity_key() {
        let codex_home = temp_codex_home("dashboard-conflicting-account-label-key");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"conflicting-history","account_label":"acct_A","account_identity_key":"acct_681356aab274b3b0","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"other-account","account_label":"acct_A","account_identity_key":"acct_key_dual","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-conflicting-account-label-key"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_keeps_matching_labels_isolated_when_candidate_hash_misses() {
        let codex_home = temp_codex_home("dashboard-label-hash-miss-isolated");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"label-history","account_label":"same@example.com","account_identity_key":"acct_key_history","account_id_alias":null,"email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"different-account","account_label":"same@example.com","account_identity_key":"acct_key_different","account_id_alias":"acct:05e6a0f2f34cad8c8aa359b9e96915d4ec025bfabae0a1af07c5e385edfcbf65","email_alias":"email:5b71ed5f946240dc76f3b7c24bdcbbc3528284ec5f4519249fb702686f0df5b8","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-label-hash-miss"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_rejects_invalid_persisted_aliases() {
        let codex_home = temp_codex_home("dashboard-invalid-aliases");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"raw-id","account_label":"raw-id","account_identity_key":"acct_key_raw","account_id_alias":"acct_A","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"wrong-prefix","account_label":"wrong-prefix","account_identity_key":"acct_key_wrong_prefix","account_id_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144005000,"profile":"short","account_label":"short","account_identity_key":"acct_key_short","account_id_alias":null,"email_alias":"email:abc","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144007000,"profile":"non-hex","account_label":"non-hex","account_identity_key":"acct_key_non_hex","account_id_alias":"acct:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-invalid-aliases"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:06.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"cached_input_tokens":40,"output_tokens":30,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":400,"cached_input_tokens":50,"output_tokens":40,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 4);
        assert!(dashboard.account_summary.iter().all(|account| account
            .account_group_key
            .as_deref()
            .is_some_and(|key| key.starts_with("identity:"))));
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        for table in ["openai_usage_evidence", "openai_usage_facts"] {
            let persisted_aliases = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE account_id_alias IS NOT NULL OR email_alias IS NOT NULL"
                    ),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(persisted_aliases, 0, "invalid alias persisted in {table}");
        }
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_does_not_union_cross_typed_alias_fields() {
        let codex_home = temp_codex_home("dashboard-cross-typed-aliases");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"wrong-field","account_label":"not-an-email","account_identity_key":"acct_key_wrong_field","account_id_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","email_alias":null,"openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"real-email","account_label":"user@example.com","account_identity_key":"acct_key_real_email","account_id_alias":null,"email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-cross-typed-aliases"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            1700
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_prefers_lexicographically_smallest_account_alias() {
        let codex_home = temp_codex_home("dashboard-canonical-alias-order");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let attribution_path = get_openai_usage_attribution_path(Some(&codex_home));
        fs::create_dir_all(attribution_path.parent().unwrap()).unwrap();
        fs::write(
            attribution_path,
            [
                r#"{"observed_at_ms":32472144000000,"profile":"account-a","account_label":"user@example.com","account_identity_key":"acct_key_a","account_id_alias":"acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
                r#"{"observed_at_ms":32472144003000,"profile":"account-b","account_label":"user@example.com","account_identity_key":"acct_key_b","account_id_alias":"acct:05e6a0f2f34cad8c8aa359b9e96915d4ec025bfabae0a1af07c5e385edfcbf65","email_alias":"email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514","openai_base_url":null,"official_source":"OpenAI default","is_official_openai":true,"exclusion_reason":null,"model_reasoning_effort":null,"model_verbosity":null}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-canonical-alias-order"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":250,"output_tokens":200,"reasoning_output_tokens":40}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        assert_eq!(
            dashboard.account_summary[0].account_group_key.as_deref(),
            Some("acct:05e6a0f2f34cad8c8aa359b9e96915d4ec025bfabae0a1af07c5e385edfcbf65")
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_ignores_invalid_aliases_loaded_from_existing_ledger() {
        let codex_home = temp_codex_home("dashboard-invalid-ledger-aliases");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-invalid-ledger-aliases"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(first.account_summary.len(), 1);

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        conn.execute(
            "UPDATE openai_usage_facts SET account_id_alias = ?1, email_alias = ?2",
            [
                "email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514",
                "email:abc",
            ],
        )
        .unwrap();
        drop(conn);

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        assert!(dashboard.account_summary[0]
            .account_group_key
            .as_deref()
            .is_some_and(|key| key.starts_with("identity:")));
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
                account_identity_key: Some("acct_key_a".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("acct_B".to_string()),
                account_identity_key: Some("acct_key_b".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
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
    fn sqlite_alias_columns_round_trip_and_sync_is_idempotent() {
        let codex_home = temp_codex_home("sqlite-alias-round-trip");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let auth = r#"{"tokens":{"account_id":"acct_A","id_token":"x.eyJlbWFpbCI6IlVzZXJARXhhbXBsZS5jb20ifQ.x"}}"#;
        fs::write(codex_home.join("auth.json"), auth).unwrap();
        fs::write(
            codex_home
                .join("account_backup")
                .join("a")
                .join("auth.json"),
            auth,
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-alias-round-trip"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":2}}}}"#,
            ]
            .join("\n"),
        );

        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let second = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let expected_account_alias =
            "acct:1933afe14abca7ece6bb7c820736c6c25f0960d2815743451ae9cd2a2cbbe829";
        let expected_email_alias =
            "email:b4c9a289323b21a01c3e940f150eb9b8c542587f1abfd8f0e1cc1ffc5e475514";
        let fact_aliases = conn
            .query_row(
                "SELECT account_id_alias, email_alias FROM openai_usage_facts",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        let evidence_aliases = conn
            .query_row(
                "SELECT account_id_alias, email_alias FROM openai_usage_evidence WHERE decision = 'accepted'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();

        assert_eq!(fact_aliases.0, expected_account_alias);
        assert_eq!(fact_aliases.1, expected_email_alias);
        assert_eq!(evidence_aliases, fact_aliases);
        assert_eq!(first.summary.total_requests, 1);
        assert_eq!(second.summary.total_requests, 1);
        assert_eq!(second.summary.tokens.real_total_tokens, 110);
        assert_eq!(second.logs.total, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn sqlite_alias_columns_migrate_additively_with_legacy_rows_defaulting_to_none() {
        let codex_home = temp_codex_home("sqlite-alias-additive-migration");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-alias-migration"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );
        let first = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(first.summary.total_requests, 1);

        let ledger_path = get_openai_usage_ledger_path(Some(&codex_home));
        let legacy_conn = Connection::open(&ledger_path).unwrap();
        legacy_conn
            .execute_batch(
                r#"
                ALTER TABLE openai_usage_evidence DROP COLUMN account_id_alias;
                ALTER TABLE openai_usage_evidence DROP COLUMN email_alias;
                ALTER TABLE openai_usage_facts DROP COLUMN account_id_alias;
                ALTER TABLE openai_usage_facts DROP COLUMN email_alias;
                "#,
            )
            .unwrap();
        drop(legacy_conn);

        let migrated = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let repeated = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let conn = Connection::open(&ledger_path).unwrap();
        for table in ["openai_usage_evidence", "openai_usage_facts"] {
            let columns = table_columns(&conn, table).unwrap();
            assert!(columns.contains("account_id_alias"));
            assert!(columns.contains("email_alias"));
            let aliases = conn
                .query_row(
                    &format!("SELECT account_id_alias, email_alias FROM {table} LIMIT 1"),
                    [],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(aliases, (None, None));
        }
        assert_eq!(migrated.summary.total_requests, 1);
        assert_eq!(repeated.summary.total_requests, 1);
        assert_eq!(repeated.logs.total, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn evidence_ledger_records_accepted_decisions_and_versions() {
        let codex_home = temp_codex_home("evidence-ledger");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-evidence"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":15,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.exclusions.accepted_events, 2);
        assert_eq!(summary.exclusions.candidate_events, 0);
        assert_eq!(summary.exclusions.superseded_events, 0);

        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let accepted_evidence = conn
            .query_row(
                "SELECT COUNT(*) FROM openai_usage_evidence WHERE decision = 'accepted'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(accepted_evidence, 2);
        let accepted_events_with_evidence = conn
            .query_row(
                "SELECT COUNT(*) FROM openai_usage_events WHERE evidence_id IS NOT NULL AND source_type = ?1",
                [SOURCE_CODEX_SESSION_JSONL],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(accepted_events_with_evidence, 2);
        let accepted_events_with_account_key = conn
            .query_row(
                "SELECT COUNT(*) FROM openai_usage_events WHERE account_identity_key IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(accepted_events_with_account_key, 2);
        let versions = conn
            .query_row(
                "SELECT parser_version, classifier_version, dedupe_version, pricing_version, ledger_schema_version, layer_classifier_version, attribution_backfill_version FROM openai_usage_sync_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(versions.0, EVIDENCE_PARSER_VERSION);
        assert_eq!(versions.1, EVIDENCE_CLASSIFIER_VERSION);
        assert_eq!(versions.2, EVIDENCE_DEDUPE_VERSION);
        assert_eq!(versions.3, PRICING_VERSION);
        assert_eq!(versions.4, LEDGER_SCHEMA_VERSION);
        assert_eq!(versions.5, LAYER_CLASSIFIER_VERSION);
        assert_eq!(versions.6, ATTRIBUTION_BACKFILL_VERSION);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn auth_identity_change_rebuilds_and_removes_strict_accepted_totals() {
        let codex_home = temp_codex_home("auth-signature-rebuild");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-auth"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let first = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(first.total_requests, 1);

        fs::write(
            codex_home.join("auth.json"),
            r#"{"tokens":{"account_id":"acct_B_changed"}}"#,
        )
        .unwrap();
        let second = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(second.total_requests, 0);
        assert_eq!(second.exclusions.accepted_events, 0);
        assert!(second.exclusions.candidate_events > 0);
        assert!(second.exclusions.candidate_unattributed_events > 0);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let accepted_events = conn
            .query_row("SELECT COUNT(*) FROM openai_usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(accepted_events, 0);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn official_attribution_without_account_key_becomes_legacy_fact_not_strict() {
        let codex_home = temp_codex_home("legacy-attribution-no-key");
        fs::create_dir_all(&codex_home).unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("legacy".to_string()),
                account_label: Some("legacy".to_string()),
                account_identity_key: None,
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-legacy"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.exclusions.accepted_events, 1);
        assert_eq!(summary.exclusions.strict_proven_events, 0);
        assert_eq!(summary.exclusions.legacy_profile_attributed_events, 1);
        assert_eq!(summary.exclusions.unattributed, 0);
        assert_eq!(
            summary
                .layer_summaries
                .iter()
                .find(|layer| layer.ledger_layer == LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED)
                .map(|layer| layer.request_count),
            Some(1)
        );
        let legacy = load_openai_usage_summary(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_LEGACY.to_string()),
                ..OpenAiUsageQuery::default()
            }),
        )
        .unwrap();
        assert_eq!(legacy.total_requests, 1);
        let combined = load_openai_usage_summary(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_COMBINED_ATTRIBUTED.to_string()),
                ..OpenAiUsageQuery::default()
            }),
        )
        .unwrap();
        assert_eq!(combined.total_requests, 1);
        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(dashboard.summary.total_requests, 0);
        assert_eq!(dashboard.logs.total, 0);
        assert_eq!(dashboard.account_summary.len(), 1);
        assert_eq!(dashboard.account_summary[0].profile, "legacy");
        assert_eq!(
            dashboard.account_summary[0].ledger_layer,
            LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED
        );
        assert_eq!(dashboard.account_summary[0].request_count, 1);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let strict_events = conn
            .query_row("SELECT COUNT(*) FROM openai_usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(strict_events, 0);
        let legacy_facts = conn
            .query_row(
                "SELECT COUNT(*) FROM openai_usage_facts WHERE ledger_layer = 'legacy_profile_attributed' AND account_identity_key IS NULL AND legacy_account_key LIKE 'legacy_profile:legacy:%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(legacy_facts, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_does_not_backfill_eligible_legacy_from_future_strict_label() {
        let codex_home = temp_codex_home("account-label-merge");
        fs::create_dir_all(&codex_home).unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("same@example.com".to_string()),
                account_identity_key: None,
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("c".to_string()),
                account_label: Some("same@example.com".to_string()),
                account_identity_key: Some("acct_same".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-merge"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":30,"output_tokens":16,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.summary.total_requests, 1);
        assert_eq!(dashboard.logs.total, 1);
        assert_eq!(dashboard.account_summary.len(), 2);
        let legacy = dashboard
            .account_summary
            .iter()
            .find(|account| account.profiles == ["b"])
            .unwrap();
        let strict = dashboard
            .account_summary
            .iter()
            .find(|account| account.profiles == ["c"])
            .unwrap();
        assert_ne!(legacy.account_group_key, strict.account_group_key);
        assert!(legacy
            .account_group_key
            .as_deref()
            .is_some_and(|key| key.starts_with("legacy:")));
        assert_eq!(
            strict.account_group_key.as_deref(),
            Some("identity:acct_same")
        );
        assert_eq!(legacy.account_label.as_deref(), Some("same@example.com"));
        assert_eq!(strict.account_label.as_deref(), Some("same@example.com"));
        assert_eq!(legacy.ledger_layer, LEDGER_LAYER_LEGACY_PROFILE_ATTRIBUTED);
        assert_eq!(legacy.attribution_kind, ATTRIBUTION_KIND_LEGACY_PROFILE);
        assert_eq!(strict.ledger_layer, LEDGER_LAYER_STRICT_PROVEN);
        assert_eq!(strict.attribution_kind, ATTRIBUTION_KIND_STRICT_ACCOUNT);
        assert_eq!(legacy.request_count, 1);
        assert_eq!(legacy.tokens.input_tokens, 100);
        assert_eq!(strict.request_count, 1);
        assert_eq!(strict.tokens.input_tokens, 60);
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.request_count)
                .sum::<u64>(),
            2
        );
        assert_eq!(
            dashboard
                .account_summary
                .iter()
                .map(|account| account.tokens.real_total_tokens)
                .sum::<u64>(),
            176
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_account_summary_does_not_merge_distinct_strict_identities_with_same_label() {
        let codex_home = temp_codex_home("same-label-distinct-identities");
        fs::create_dir_all(&codex_home).unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("a".to_string()),
                account_label: Some("shared@example.com".to_string()),
                account_identity_key: Some("acct_one".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("shared@example.com".to_string()),
                account_identity_key: Some("acct_two".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-distinct"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":30,"output_tokens":16,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_COMBINED_ATTRIBUTED.to_string()),
                ..OpenAiUsageQuery::default()
            }),
            1,
            20,
        )
        .unwrap();

        assert_eq!(dashboard.account_summary.len(), 2);
        assert!(dashboard
            .account_summary
            .iter()
            .all(|account| account.ledger_layer == LEDGER_LAYER_STRICT_PROVEN));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn dashboard_account_summary_merges_same_strict_identity_across_label_changes() {
        let codex_home = temp_codex_home("same-identity-label-change");
        fs::create_dir_all(&codex_home).unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:00.000Z"),
                profile: Some("a".to_string()),
                account_label: Some("old@example.com".to_string()),
                account_identity_key: Some("acct_same".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        append_attribution_record(
            &codex_home,
            &AttributionRecord {
                observed_at_ms: ts("2999-01-01T00:00:03.000Z"),
                profile: Some("b".to_string()),
                account_label: Some("new@example.com".to_string()),
                account_identity_key: Some("acct_same".to_string()),
                account_id_alias: None,
                email_alias: None,
                openai_base_url: None,
                official_source: Some("OpenAI default".to_string()),
                is_official_openai: true,
                exclusion_reason: None,
                model_reasoning_effort: None,
                model_verbosity: None,
            },
        )
        .unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-same-id"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
                r#"{"timestamp":"2999-01-01T00:00:04.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":30,"output_tokens":16,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_COMBINED_ATTRIBUTED.to_string()),
                ..OpenAiUsageQuery::default()
            }),
            1,
            20,
        )
        .unwrap();

        assert_eq!(dashboard.account_summary.len(), 1);
        assert_eq!(dashboard.account_summary[0].request_count, 2);
        assert_eq!(
            dashboard.account_summary[0].profiles,
            vec!["a".to_string(), "b".to_string()]
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn sessions_without_attribution_are_recorded_as_candidate_evidence() {
        let codex_home = temp_codex_home("no-attribution-evidence");
        fs::create_dir_all(&codex_home).unwrap();
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-none"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.exclusions.accepted_events, 0);
        assert_eq!(summary.exclusions.candidate_events, 1);
        assert_eq!(summary.exclusions.candidate_unattributed_events, 1);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let candidate = conn
            .query_row(
                "SELECT decision, ledger_layer, decision_reason FROM openai_usage_evidence",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(candidate.0, DECISION_CANDIDATE);
        assert_eq!(candidate.1, LEDGER_LAYER_CANDIDATE_UNATTRIBUTED);
        assert_eq!(candidate.2, "no attribution record covered this event");
        let candidate_dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_CANDIDATE.to_string()),
                ..OpenAiUsageQuery::default()
            }),
            1,
            20,
        )
        .unwrap();
        assert_eq!(candidate_dashboard.logs.total, 1);
        assert_eq!(
            candidate_dashboard.logs.data[0].ledger_layer,
            LEDGER_LAYER_CANDIDATE_UNATTRIBUTED
        );
        assert_eq!(candidate_dashboard.summary.total_requests, 1);
        conn.execute(
            "UPDATE openai_usage_evidence SET model = NULL WHERE decision = 'candidate'",
            [],
        )
        .unwrap();
        let candidate_request_model_dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_CANDIDATE.to_string()),
                model: Some("gpt-5.3-codex".to_string()),
                ..OpenAiUsageQuery::default()
            }),
            1,
            20,
        )
        .unwrap();
        assert_eq!(candidate_request_model_dashboard.logs.total, 1);
        assert_eq!(
            candidate_request_model_dashboard.logs.data[0].model,
            "gpt-5.3-codex"
        );
        let strict_dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(strict_dashboard.summary.total_requests, 0);
        assert_eq!(strict_dashboard.logs.total, 0);
        assert_eq!(
            strict_dashboard
                .summary
                .layer_summaries
                .iter()
                .find(|layer| layer.ledger_layer == LEDGER_LAYER_CANDIDATE_UNATTRIBUTED)
                .map(|layer| layer.request_count),
            Some(1)
        );
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn duplicate_archived_session_writes_superseded_evidence_without_key_collision() {
        let codex_home = temp_codex_home("archived-session-dedupe");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let body = [
            r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-dup"}}"#,
            r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
            r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            r#"{"timestamp":"2999-01-01T00:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":15,"reasoning_output_tokens":0}}}}"#,
        ]
        .join("\n");
        write_session(&codex_home, &body);
        write_archived_session(&codex_home, &body);

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.exclusions.accepted_events, 2);
        assert_eq!(summary.exclusions.superseded_events, 2);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let superseded_rows = conn
            .query_row(
                "SELECT COUNT(*) FROM openai_usage_evidence WHERE decision = 'superseded' AND superseded_by IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(superseded_rows, 2);
        let excluded_dashboard = load_openai_usage_dashboard(
            Some(&codex_home),
            Some(OpenAiUsageQuery {
                scope: Some(SCOPE_EXCLUDED.to_string()),
                ..OpenAiUsageQuery::default()
            }),
            1,
            20,
        )
        .unwrap();
        assert_eq!(excluded_dashboard.logs.total, 2);
        assert!(excluded_dashboard
            .logs
            .data
            .iter()
            .all(|entry| entry.ledger_layer == LEDGER_LAYER_SUPERSEDED));
        assert_eq!(excluded_dashboard.summary.total_requests, 2);
        let strict_dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        assert_eq!(strict_dashboard.summary.total_requests, 2);
        assert_eq!(strict_dashboard.logs.total, 2);
        let total_evidence = conn
            .query_row("SELECT COUNT(*) FROM openai_usage_evidence", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(total_evidence, 4);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn legacy_nullable_account_key_events_table_is_rebuilt() {
        let codex_home = temp_codex_home("legacy-nullable-account-key");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let ledger_path = get_openai_usage_ledger_path(Some(&codex_home));
        fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        let legacy_conn = Connection::open(&ledger_path).unwrap();
        legacy_conn
            .execute_batch(
                r#"
                CREATE TABLE openai_usage_events (
                    request_id TEXT PRIMARY KEY,
                    fingerprint TEXT NOT NULL UNIQUE,
                    account_identity_key TEXT
                );
                "#,
            )
            .unwrap();
        drop(legacy_conn);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-schema"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(summary.total_requests, 1);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        assert!(
            table_column_not_null(&conn, "openai_usage_events", "account_identity_key").unwrap()
        );
        let account_key = conn
            .query_row(
                "SELECT account_identity_key FROM openai_usage_events",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert!(!account_key.is_empty());
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn legacy_evidence_table_without_layer_columns_is_migrated_before_indexing() {
        let codex_home = temp_codex_home("legacy-evidence-layer-migration");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        let ledger_path = get_openai_usage_ledger_path(Some(&codex_home));
        fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        let legacy_conn = Connection::open(&ledger_path).unwrap();
        legacy_conn
            .execute_batch(
                r#"
                CREATE TABLE openai_usage_evidence (
                    evidence_id TEXT PRIMARY KEY,
                    request_id TEXT,
                    fingerprint TEXT,
                    session_id TEXT,
                    source_type TEXT NOT NULL,
                    source_priority INTEGER NOT NULL,
                    source_locator TEXT NOT NULL,
                    source_hash TEXT NOT NULL,
                    parser_version TEXT NOT NULL,
                    event_started_at_ms INTEGER,
                    event_completed_at_ms INTEGER,
                    observed_at_ms INTEGER,
                    turn_index INTEGER,
                    response_id TEXT,
                    previous_response_id TEXT,
                    model TEXT,
                    request_model TEXT,
                    pricing_model TEXT,
                    usage_shape TEXT NOT NULL,
                    usage_confidence TEXT NOT NULL,
                    input_tokens INTEGER,
                    cached_input_tokens INTEGER,
                    output_tokens INTEGER,
                    reasoning_output_tokens INTEGER,
                    raw_usage_json TEXT,
                    profile TEXT,
                    account_identity_key TEXT,
                    account_label TEXT,
                    identity_source TEXT,
                    identity_observed_at_ms INTEGER,
                    account_confidence TEXT NOT NULL,
                    effective_base_url TEXT,
                    official_source TEXT,
                    official_confidence TEXT NOT NULL,
                    status_code INTEGER,
                    decision TEXT NOT NULL,
                    decision_reason TEXT,
                    superseded_by TEXT,
                    created_at_ms INTEGER NOT NULL
                );
                "#,
            )
            .unwrap();
        drop(legacy_conn);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-evidence-schema"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();

        assert_eq!(summary.total_requests, 1);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let columns = table_columns(&conn, "openai_usage_evidence").unwrap();
        assert!(columns.contains("ledger_layer"));
        assert!(columns.contains("legacy_account_key"));
        let index_count = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_openai_usage_evidence_layer'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn last_token_usage_is_accepted_with_low_usage_confidence() {
        let codex_home = temp_codex_home("last-token-usage");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-last"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"cached_input_tokens":5,"output_tokens":3,"reasoning_output_tokens":0}}}}"#,
            ]
            .join("\n"),
        );

        let summary = load_openai_usage_summary(Some(&codex_home), None).unwrap();
        assert_eq!(summary.total_requests, 1);
        let conn = Connection::open(get_openai_usage_ledger_path(Some(&codex_home))).unwrap();
        let row = conn
            .query_row(
                "SELECT usage_shape, usage_confidence FROM openai_usage_evidence WHERE decision = 'accepted'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, USAGE_SHAPE_LAST_TOKEN_USAGE);
        assert_eq!(row.1, CONFIDENCE_LOW);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn turn_context_effort_and_pricing_metadata_are_explicit_usage_fields() {
        let codex_home = temp_codex_home("turn-context-effort");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-effort"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"openai/gpt-5.3-codex-2026-07-08","model_reasoning_effort":"xhigh","model_verbosity":"high"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":2}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();

        assert_eq!(dashboard.summary.pricing_source_url, PRICING_SOURCE_URL);
        let model = &dashboard.summary.models[0];
        assert_eq!(model.model, "gpt-5.3-codex");
        assert_eq!(
            model.requested_model.as_deref(),
            Some("openai/gpt-5.3-codex-2026-07-08")
        );
        assert_eq!(model.pricing_model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(model.model_reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(model.model_verbosity.as_deref(), Some("high"));
        assert_eq!(
            model.pricing_tier.as_deref(),
            Some(PRICING_TIER_STANDARD_SHORT_CONTEXT)
        );
        assert_eq!(
            model.pricing_source_url.as_deref(),
            Some(PRICING_SOURCE_URL)
        );

        let log = &dashboard.logs.data[0];
        assert_eq!(log.model, "gpt-5.3-codex");
        assert_eq!(
            log.requested_model.as_deref(),
            Some("openai/gpt-5.3-codex-2026-07-08")
        );
        assert_eq!(log.pricing_model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(log.model_reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(log.model_verbosity.as_deref(), Some("high"));
        assert_eq!(log.usage_confidence, CONFIDENCE_HIGH);
        assert_eq!(
            log.pricing_tier.as_deref(),
            Some(PRICING_TIER_STANDARD_SHORT_CONTEXT)
        );
        assert_eq!(log.pricing_source_url.as_deref(), Some(PRICING_SOURCE_URL));
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn missing_effort_stays_unknown_even_when_usage_confidence_is_high() {
        let codex_home = temp_codex_home("no-effort-no-inference");
        fs::create_dir_all(&codex_home).unwrap();
        setup_profile(&codex_home, None);
        write_session(
            &codex_home,
            &[
                r#"{"timestamp":"2999-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"s-no-effort"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
                r#"{"timestamp":"2999-01-01T00:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":2}}}}"#,
            ]
            .join("\n"),
        );

        let dashboard = load_openai_usage_dashboard(Some(&codex_home), None, 1, 20).unwrap();
        let model = &dashboard.summary.models[0];
        let log = &dashboard.logs.data[0];

        assert_eq!(log.usage_confidence, CONFIDENCE_HIGH);
        assert_eq!(model.model_reasoning_effort, None);
        assert_eq!(model.model_verbosity, None);
        assert_eq!(log.model_reasoning_effort, None);
        assert_eq!(log.model_verbosity, None);
        assert_eq!(log.pricing_model.as_deref(), Some("gpt-5.3-codex"));
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
}

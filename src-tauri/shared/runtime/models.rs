use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QuotaWindow {
    pub remaining_percent: Option<u8>,
    pub refresh_at: Option<String>,
    pub reset_at_timestamp: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    Unknown,
    ChatGptApi,
    CodexAppServer,
    SessionJsonl,
}

impl Default for QuotaSource {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaConfidence {
    Unknown,
    Official,
    Fallback,
    LocalObservation,
}

impl Default for QuotaConfidence {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QuotaProvenance {
    pub source: QuotaSource,
    pub confidence: QuotaConfidence,
    pub observed_at_ms: Option<u64>,
    pub source_mtime_ms: Option<u64>,
    pub note: Option<String>,
}

impl QuotaProvenance {
    pub fn chatgpt_api(observed_at_ms: Option<u64>) -> Self {
        Self {
            source: QuotaSource::ChatGptApi,
            confidence: QuotaConfidence::Official,
            observed_at_ms,
            source_mtime_ms: None,
            note: None,
        }
    }

    pub fn codex_app_server(observed_at_ms: Option<u64>) -> Self {
        Self {
            source: QuotaSource::CodexAppServer,
            confidence: QuotaConfidence::Fallback,
            observed_at_ms,
            source_mtime_ms: None,
            note: Some("codex app-server fallback".to_string()),
        }
    }

    pub fn session_jsonl(source_mtime_ms: Option<u64>) -> Self {
        Self {
            source: QuotaSource::SessionJsonl,
            confidence: QuotaConfidence::LocalObservation,
            observed_at_ms: None,
            source_mtime_ms,
            note: Some("local Codex session token_count event".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QuotaSummary {
    pub five_hour: QuotaWindow,
    pub weekly: QuotaWindow,
    pub provenance: QuotaProvenance,
}

impl QuotaSummary {
    pub fn with_provenance(mut self, provenance: QuotaProvenance) -> Self {
        self.provenance = provenance;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileMetadata {
    pub folder_name: Option<String>,
    pub account_label: Option<String>,
    pub plan_name: Option<String>,
    pub subscription_expires_at: Option<String>,
    pub openai_base_url: Option<String>,
    pub login_browser: Option<String>,
    pub quota: QuotaSummary,
    pub quota_updated_at_ms: Option<u64>,
    /// Wall-clock millis at the most recent confirmed plan check (any
    /// path that proved plan info was current: API plan_type override,
    /// fresh id_token claim, login). Independent of
    /// `quota_updated_at_ms` because plan changes much less frequently
    /// than usage and the UI surfaces plan freshness on its own. `None`
    /// for legacy profile.json that predates the field.
    pub last_plan_check_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshTokenHealth {
    Healthy,
    RefreshDue,
    NeedsLogin,
    UnknownExpiry,
    NotRefreshable,
    MissingAuth,
}

impl Default for RefreshTokenHealth {
    fn default() -> Self {
        Self::UnknownExpiry
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RefreshTokenStatus {
    pub health: RefreshTokenHealth,
    pub can_refresh: bool,
    pub refresh_token_present: bool,
    /// OAuth providers do not always expose refresh-token expiry. Keep
    /// this nullable and let the UI say "unknown" instead of inventing a
    /// date. If OpenAI/Codex starts writing one, the derivation layer can
    /// fill it without changing the front-end contract.
    pub refresh_token_expires_at_ms: Option<u64>,
    pub access_token_expires_at_ms: Option<u64>,
    pub id_token_expires_at_ms: Option<u64>,
    pub auth_updated_at_ms: Option<u64>,
    pub last_refresh_at_ms: Option<u64>,
}

impl ProfileMetadata {
    pub fn with_folder_name(folder_name: &str) -> Self {
        Self {
            folder_name: Some(folder_name.to_string()),
            ..Self::default()
        }
    }

    pub fn validate(self) -> Option<Self> {
        let five_hour_ok = self
            .quota
            .five_hour
            .remaining_percent
            .map_or(true, |value| value <= 100);
        let weekly_ok = self
            .quota
            .weekly
            .remaining_percent
            .map_or(true, |value| value <= 100);

        if five_hour_ok && weekly_ok {
            Some(self)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCard {
    pub folder_name: String,
    pub display_title: String,
    pub account_label: Option<String>,
    pub status: String,
    pub auth_present: bool,
    pub has_account_identity: bool,
    pub plan_name: Option<String>,
    pub subscription_days_left: Option<i64>,
    pub openai_base_url: Option<String>,
    pub quota: QuotaSummary,
    /// Surfaces plan freshness to the front-end so the dashboard can
    /// render a hover-time tooltip and a stale indicator without
    /// re-fetching per-profile metadata. `None` for legacy entries
    /// that predate the field.
    pub last_plan_check_ms: Option<u64>,
    pub refresh_token_status: RefreshTokenStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentCard {
    pub folder_name: String,
    pub display_title: String,
    pub account_label: Option<String>,
    pub has_account_identity: bool,
    pub plan_name: Option<String>,
    pub subscription_days_left: Option<i64>,
    pub profile_folder_path: String,
    pub last_plan_check_ms: Option<u64>,
    pub refresh_token_status: RefreshTokenStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileIndexEntry {
    pub folder_name: String,
    pub account_label: Option<String>,
    pub has_account_identity: bool,
    pub plan_name: Option<String>,
    pub subscription_expires_at: Option<String>,
    pub openai_base_url: Option<String>,
    pub auth_present: bool,
    pub stored_quota: QuotaSummary,
    pub stored_quota_updated_at_ms: Option<u64>,
    /// Mirrors `ProfileMetadata::last_plan_check_ms` after the index
    /// rolls up profile.json. Lets the dashboard show plan freshness
    /// without re-reading per-profile metadata.
    pub last_plan_check_ms: Option<u64>,
    pub refresh_token_status: RefreshTokenStatus,
    pub auth_mtime_ms: Option<u64>,
    pub auth_size: Option<u64>,
    pub auth_fingerprint: Option<u64>,
    pub profile_mtime_ms: Option<u64>,
    pub profile_size: Option<u64>,
    pub profile_fingerprint: Option<u64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfilesIndex {
    pub schema_version: u32,
    pub updated_at: String,
    pub current_profile: Option<String>,
    pub profiles: Vec<ProfileIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesSnapshotResponse {
    pub page_size: u32,
    pub profiles: Vec<ProfileCard>,
    pub current_card: Option<CurrentCard>,
    pub current_quota_card: Option<QuotaSummary>,
    pub unmanaged_live_account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentQuotaResponse {
    pub profile: Option<String>,
    pub quota: Option<QuotaSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageTokenTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub real_total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageCostBreakdown {
    pub input_cost_micros: u64,
    pub cached_input_cost_micros: u64,
    pub output_cost_micros: u64,
    pub total_cost_micros: u64,
    pub unpriced_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageModelSummary {
    pub model: String,
    pub requested_model: Option<String>,
    pub pricing_model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub model_verbosity: Option<String>,
    pub pricing_tier: Option<String>,
    pub pricing_source_url: Option<String>,
    pub request_count: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
    pub priced: bool,
    pub pricing_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageAccountSummary {
    pub profile: String,
    pub profiles: Vec<String>,
    pub account_group_key: Option<String>,
    pub account_identity_key: Option<String>,
    pub legacy_account_key: Option<String>,
    pub account_label: Option<String>,
    pub official_source: String,
    pub ledger_layer: String,
    pub attribution_kind: String,
    pub attribution_window_id: Option<String>,
    pub request_count: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
    pub models: Vec<OpenAiUsageModelSummary>,
    #[serde(default)]
    pub attribution_breakdown: Vec<OpenAiUsageLayerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageExclusions {
    pub duplicate_events: u64,
    pub non_official_base_url: u64,
    pub unproven_history: u64,
    pub unattributed: u64,
    pub missing_model: u64,
    pub invalid_delta: u64,
    pub malformed_lines: u64,
    pub accepted_events: u64,
    pub candidate_events: u64,
    pub rejected_events: u64,
    pub superseded_events: u64,
    pub strict_proven_events: u64,
    pub legacy_profile_attributed_events: u64,
    pub candidate_unattributed_events: u64,
    pub rejected_invalid_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageLayerSummary {
    pub ledger_layer: String,
    pub request_count: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageSummary {
    pub generated_at_ms: u64,
    pub attribution_since_ms: Option<u64>,
    pub range_start_ms: Option<u64>,
    pub range_end_ms: Option<u64>,
    pub total_requests: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
    pub cache_hit_rate: Option<f64>,
    pub accounts: Vec<OpenAiUsageAccountSummary>,
    pub models: Vec<OpenAiUsageModelSummary>,
    pub exclusions: OpenAiUsageExclusions,
    pub scope: String,
    pub layer_summaries: Vec<OpenAiUsageLayerSummary>,
    pub pricing_source: String,
    pub pricing_source_url: String,
    pub pricing_checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageQuery {
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
    pub profile: Option<String>,
    pub account_group_key: Option<String>,
    pub account_label: Option<String>,
    pub model: Option<String>,
    pub status_code: Option<u16>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageTrendPoint {
    pub date: String,
    pub bucket_start_ms: u64,
    pub request_count: u64,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageRequestLog {
    pub request_id: String,
    pub session_id: String,
    pub profile: String,
    pub account_label: Option<String>,
    pub official_source: String,
    pub ledger_layer: String,
    pub attribution_kind: String,
    pub attribution_window_id: Option<String>,
    pub model: String,
    pub requested_model: Option<String>,
    pub pricing_model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub model_verbosity: Option<String>,
    pub pricing_tier: Option<String>,
    pub pricing_source_url: Option<String>,
    pub tokens: OpenAiUsageTokenTotals,
    pub cost: OpenAiUsageCostBreakdown,
    pub priced: bool,
    pub pricing_note: Option<String>,
    pub status_code: u16,
    pub created_at_ms: u64,
    pub data_source: String,
    pub usage_confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageRequestLogPage {
    pub data: Vec<OpenAiUsageRequestLog>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageDataSourceSummary {
    pub data_source: String,
    pub request_count: u64,
    pub cost: OpenAiUsageCostBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenAiUsageDashboard {
    pub summary: OpenAiUsageSummary,
    pub account_summary: Vec<OpenAiUsageAccountSummary>,
    pub model_options: Vec<OpenAiUsageModelSummary>,
    pub trends: Vec<OpenAiUsageTrendPoint>,
    pub logs: OpenAiUsageRequestLogPage,
    pub data_sources: Vec<OpenAiUsageDataSourceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePayload {
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddProfilePayload {
    pub folder_name: String,
    pub openai_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameProfilePayload {
    pub profile: String,
    pub new_folder_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProfileBaseUrlPayload {
    pub profile: String,
    pub openai_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckPayload {
    pub update_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenUrlPayload {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckResponse {
    pub ok: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub local_patch_required: bool,
    pub release_url: Option<String>,
    pub notes: Option<String>,
    pub checked_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchResponse {
    pub ok: bool,
    pub profile: String,
    pub message: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCliStatus {
    /// Currently resolved real-codex path, or None when nothing was
    /// found. Front-end uses this to decide whether the "已定位"
    /// indicator shows up and what to prefill the input with.
    pub resolved_path: Option<String>,
    /// `"user_override" | "install_state" | "discovery" | "none"`
    /// — frontend i18n maps this to a label so the user can tell
    /// whether they're looking at their manual override or the
    /// auto-discovered path.
    pub source: String,
    /// Common platform-specific install locations that exist on disk
    /// right now. Frontend renders these as click-to-fill chips.
    pub suggested_paths: Vec<String>,
}

/// A codex CLI candidate confirmed runnable by the re-detection scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexCliCandidate {
    /// Absolute path to the verified-runnable codex binary.
    pub path: String,
    /// Version line from `codex --version` (e.g. "codex-cli 0.133.0"), or
    /// None if the binary ran successfully but printed nothing parseable.
    /// Shown next to the path so the user can tell several installs apart.
    pub version: Option<String>,
}

/// Result of a forced re-detection scan triggered by the Settings
/// "auto-detect" button. Unlike `get_codex_cli_status` (which honours
/// the cached/override path), this rescans from scratch across every
/// known source and keeps only the candidates that pass a
/// `codex --version` runnable probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCliRedetectResult {
    /// Verified-runnable candidates, deduped and best-first. The
    /// front-end auto-applies a lone hit and lets the user pick (with
    /// versions shown) when there are several.
    pub candidates: Vec<CodexCliCandidate>,
    /// Refreshed status snapshot so the Settings row and the dialog can
    /// update in lock-step after the scan.
    pub status: CodexCliStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetCodexCliPathPayload {
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_quota_summary_without_provenance_defaults_to_unknown() {
        let raw = r#"{
            "five_hour": { "remaining_percent": 42, "refresh_at": "later", "reset_at_timestamp": null },
            "weekly": { "remaining_percent": 84, "refresh_at": null, "reset_at_timestamp": null }
        }"#;

        let quota: QuotaSummary = serde_json::from_str(raw).expect("legacy quota summary");

        assert_eq!(quota.five_hour.remaining_percent, Some(42));
        assert_eq!(quota.provenance.source, QuotaSource::Unknown);
        assert_eq!(quota.provenance.confidence, QuotaConfidence::Unknown);
        assert_eq!(quota.provenance.observed_at_ms, None);
        assert_eq!(quota.provenance.source_mtime_ms, None);
    }

    #[test]
    fn quota_provenance_serializes_as_frontend_snake_case_contract() {
        let quota = QuotaSummary::default()
            .with_provenance(QuotaProvenance::chatgpt_api(Some(1_800_000_000_000)));

        let serialized = serde_json::to_value(&quota).expect("quota summary json");

        assert_eq!(serialized["provenance"]["source"], "chat_gpt_api");
        assert_eq!(serialized["provenance"]["confidence"], "official");
        assert_eq!(
            serialized["provenance"]["observed_at_ms"],
            1_800_000_000_000_u64
        );
    }
}

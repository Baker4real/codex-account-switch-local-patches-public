export interface QuotaWindow {
  remaining_percent: number | null;
  refresh_at: string | null;
  reset_at_timestamp: number | null;
}

export type QuotaSource = "unknown" | "chat_gpt_api" | "codex_app_server" | "session_jsonl";

export type QuotaConfidence = "unknown" | "official" | "fallback" | "local_observation";

export interface QuotaProvenance {
  source: QuotaSource;
  confidence: QuotaConfidence;
  observed_at_ms: number | null;
  source_mtime_ms: number | null;
  note: string | null;
}

export interface QuotaSummary {
  five_hour: QuotaWindow;
  weekly: QuotaWindow;
  provenance?: QuotaProvenance;
}

export type RefreshTokenHealth =
  | "healthy"
  | "refresh_due"
  | "needs_login"
  | "unknown_expiry"
  | "not_refreshable"
  | "missing_auth";

export interface RefreshTokenStatus {
  health: RefreshTokenHealth;
  can_refresh: boolean;
  refresh_token_present: boolean;
  refresh_token_expires_at_ms: number | null;
  access_token_expires_at_ms: number | null;
  id_token_expires_at_ms: number | null;
  auth_updated_at_ms: number | null;
  last_refresh_at_ms: number | null;
}

export interface ProfileCard {
  folder_name: string;
  display_title: string;
  account_label: string | null;
  status: "current" | "available" | "missing_auth";
  auth_present: boolean;
  has_account_identity: boolean;
  plan_name: string | null;
  subscription_days_left: number | null;
  openai_base_url: string | null;
  quota: QuotaSummary;
  last_plan_check_ms: number | null;
  refresh_token_status: RefreshTokenStatus;
}

export interface CurrentCard {
  folder_name: string;
  display_title: string;
  account_label: string | null;
  has_account_identity: boolean;
  plan_name: string | null;
  subscription_days_left: number | null;
  profile_folder_path: string;
  last_plan_check_ms: number | null;
  refresh_token_status: RefreshTokenStatus;
}

export interface PagingInfo {
  page: number;
  page_size: number;
  total_profiles: number;
  total_pages: number;
  has_previous: boolean;
  has_next: boolean;
}

export interface DashboardViewModel {
  paging: PagingInfo;
  profiles: ProfileCard[];
  current_card: CurrentCard | null;
  current_quota_card: QuotaSummary | null;
}

export interface ProfilesSnapshotResponse {
  page_size: number;
  profiles: ProfileCard[];
  current_card: CurrentCard | null;
  current_quota_card: QuotaSummary | null;
  unmanaged_live_account: string | null;
}

export interface CurrentQuotaResponse {
  profile: string | null;
  quota: QuotaSummary | null;
}

export interface OpenAiUsageTokenTotals {
  input_tokens: number;
  cached_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  real_total_tokens: number;
}

export interface OpenAiUsageCostBreakdown {
  input_cost_micros: number;
  cached_input_cost_micros: number;
  output_cost_micros: number;
  total_cost_micros: number;
  unpriced_tokens: number;
}

export interface OpenAiUsageModelSummary {
  model: string;
  request_count: number;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
  priced: boolean;
  pricing_note: string | null;
}

export interface OpenAiUsageAccountSummary {
  profile: string;
  profiles?: string[];
  account_group_key?: string | null;
  account_identity_key?: string | null;
  legacy_account_key?: string | null;
  account_label: string | null;
  official_source: string;
  ledger_layer: string;
  attribution_kind: string;
  attribution_window_id?: string | null;
  request_count: number;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
  models: OpenAiUsageModelSummary[];
}

export interface OpenAiUsageExclusions {
  duplicate_events: number;
  non_official_base_url: number;
  unproven_history: number;
  unattributed: number;
  missing_model: number;
  invalid_delta: number;
  malformed_lines: number;
  accepted_events: number;
  candidate_events: number;
  rejected_events: number;
  superseded_events: number;
  strict_proven_events: number;
  legacy_profile_attributed_events: number;
  candidate_unattributed_events: number;
  rejected_invalid_events: number;
}

export interface OpenAiUsageLayerSummary {
  ledger_layer: string;
  request_count: number;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
}

export interface OpenAiUsageSummary {
  generated_at_ms: number;
  attribution_since_ms: number | null;
  range_start_ms: number | null;
  range_end_ms: number | null;
  total_requests: number;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
  cache_hit_rate: number | null;
  accounts: OpenAiUsageAccountSummary[];
  models: OpenAiUsageModelSummary[];
  exclusions: OpenAiUsageExclusions;
  scope: OpenAiUsageScope;
  layer_summaries: OpenAiUsageLayerSummary[];
  pricing_source: string;
  pricing_checked_at: string;
}

export interface OpenAiUsageQuery {
  start_date?: number | null;
  end_date?: number | null;
  profile?: string | null;
  account_group_key?: string | null;
  account_label?: string | null;
  model?: string | null;
  status_code?: number | null;
  scope?: OpenAiUsageScope | null;
}

export type OpenAiUsageRangePreset = "today" | "1d" | "7d" | "14d" | "30d" | "custom";
export type OpenAiUsageScope = "strict" | "legacy" | "combined_attributed" | "candidate" | "excluded";

export interface OpenAiUsageTrendPoint {
  date: string;
  bucket_start_ms: number;
  request_count: number;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
}

export interface OpenAiUsageRequestLog {
  request_id: string;
  session_id: string;
  profile: string;
  account_label: string | null;
  official_source: string;
  ledger_layer: string;
  attribution_kind: string;
  attribution_window_id?: string | null;
  model: string;
  tokens: OpenAiUsageTokenTotals;
  cost: OpenAiUsageCostBreakdown;
  priced: boolean;
  pricing_note: string | null;
  status_code: number;
  created_at_ms: number;
  data_source: string;
  usage_confidence: string;
}

export interface OpenAiUsageRequestLogPage {
  data: OpenAiUsageRequestLog[];
  total: number;
  page: number;
  page_size: number;
}

export interface OpenAiUsageDataSourceSummary {
  data_source: string;
  request_count: number;
  cost: OpenAiUsageCostBreakdown;
}

export interface OpenAiUsageDashboard {
  summary: OpenAiUsageSummary;
  account_summary: OpenAiUsageAccountSummary[];
  model_options: OpenAiUsageModelSummary[];
  trends: OpenAiUsageTrendPoint[];
  logs: OpenAiUsageRequestLogPage;
  data_sources: OpenAiUsageDataSourceSummary[];
}

export interface SwitchResponse {
  ok: boolean;
  profile: string;
  message: string;
  warnings: string[];
}

export interface ActionResponse {
  ok: boolean;
  message: string;
  path: string | null;
}

export interface UpdateCheckResponse {
  ok: boolean;
  current_version: string;
  latest_version: string | null;
  has_update: boolean;
  local_patch_required: boolean;
  release_url: string | null;
  notes: string | null;
  checked_url: string;
}

export interface CommandError {
  error_code?: string;
  message?: string;
}

export type CodexCliSource = "user_override" | "install_state" | "discovery" | "none";

export interface CodexCliStatus {
  resolved_path: string | null;
  source: CodexCliSource;
  suggested_paths: string[];
}

export interface CodexCliCandidate {
  /** Absolute path to the verified-runnable codex binary. */
  path: string;
  /** `codex --version` line (e.g. "codex-cli 0.133.0"), or null if it ran but printed nothing parseable. */
  version: string | null;
}

export interface CodexCliRedetectResult {
  /** Candidates verified runnable by the forced scan, deduped, best-first. */
  candidates: CodexCliCandidate[];
  /** Refreshed status snapshot so the Settings row can update in step. */
  status: CodexCliStatus;
}

export type ShellRoute = "dashboard" | "profiles" | "usage" | "settings" | "guide";

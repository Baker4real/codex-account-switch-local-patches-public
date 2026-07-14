import { invoke } from "@tauri-apps/api/core";

import type {
  ActionResponse,
  CodexCliRedetectResult,
  CodexCliStatus,
  CommandError,
  CurrentCard,
  CurrentQuotaResponse,
  OpenAiUsageDashboard,
  OpenAiUsageDataSourceSummary,
  OpenAiUsageQuery,
  OpenAiUsageRequestLog,
  OpenAiUsageRequestLogPage,
  OpenAiUsageSummary,
  OpenAiUsageTrendPoint,
  ProfileCard,
  ProfilesSnapshotResponse,
  QuotaConfidence,
  QuotaSource,
  QuotaSummary,
  RefreshTokenStatus,
  SwitchResponse,
  UpdateCheckResponse,
} from "@front-shared/types";

type NativeCommandError = Error & {
  code?: string;
};

type RuntimeWindow = typeof globalThis & {
  __TAURI_INTERNALS__?: unknown;
  __TAURI__?: unknown;
};

const hasTauriRuntime = Boolean(
  (globalThis as RuntimeWindow).__TAURI_INTERNALS__ || (globalThis as RuntimeWindow).__TAURI__,
);

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function quota(
  fiveHourPercent: number | null,
  fiveHourRefresh: string | null,
  weeklyPercent: number | null,
  weeklyRefresh: string | null,
  source: QuotaSource = "chat_gpt_api",
  confidence: QuotaConfidence = "official",
  ageMs: number | null = 8 * 60 * 1000,
  availableResetCredits: number | null = null,
): QuotaSummary {
  const observedAtMs = ageMs == null ? null : Date.now() - ageMs;
  return {
    five_hour: {
      remaining_percent: fiveHourPercent,
      refresh_at: fiveHourRefresh,
      reset_at_timestamp: null,
    },
    weekly: {
      remaining_percent: weeklyPercent,
      refresh_at: weeklyRefresh,
      reset_at_timestamp: null,
    },
    additional: null,
    rate_limit_reset_credits: availableResetCredits == null
      ? null
      : {
          available_count: availableResetCredits,
          credits: [{
            granted_at: Math.floor((Date.now() - 2 * 24 * 60 * 60 * 1000) / 1000),
            expires_at: Math.floor((Date.now() + 28 * 24 * 60 * 60 * 1000) / 1000),
          }],
        },
    provenance: {
      source,
      confidence,
      observed_at_ms: observedAtMs,
      source_mtime_ms: source === "session_jsonl" ? observedAtMs : null,
      note: null,
    },
  };
}

function refreshStatus(
  overrides: Partial<RefreshTokenStatus> = {},
): RefreshTokenStatus {
  return {
    health: "unknown_expiry",
    can_refresh: true,
    refresh_token_present: true,
    refresh_token_expires_at_ms: null,
    access_token_expires_at_ms: Date.now() + 2 * 60 * 60 * 1000,
    id_token_expires_at_ms: Date.now() + 2 * 60 * 60 * 1000,
    auth_updated_at_ms: Date.now() - 3 * 24 * 60 * 60 * 1000,
    last_refresh_at_ms: null,
    ...overrides,
  };
}

const previewProfiles: ProfileCard[] = [
  {
    folder_name: "workspace-alpha",
    display_title: "Workspace Alpha",
    account_label: "Workspace Alpha",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "Pro plan",
    subscription_days_left: 18,
    openai_base_url: null,
    quota: quota(84, "3小时后刷新", 61, "2天4小时后刷新", "chat_gpt_api", "official", 30 * 60 * 1000, 1),
    last_plan_check_ms: Date.now() - 30 * 60 * 1000,
    refresh_token_status: refreshStatus(),
  },
  {
    folder_name: "workspace-beta",
    display_title: "Workspace Beta",
    account_label: "Workspace Beta",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "Plus plan",
    subscription_days_left: 12,
    openai_base_url: null,
    quota: quota(18, "1小时后刷新", 42, "4天6小时后刷新", "codex_app_server", "fallback", 3 * 60 * 60 * 1000),
    last_plan_check_ms: Date.now() - 4 * 60 * 60 * 1000,
    refresh_token_status: refreshStatus({
      health: "refresh_due",
      access_token_expires_at_ms: Date.now() + 45 * 60 * 1000,
    }),
  },
  {
    folder_name: "workspace-gamma",
    display_title: "Metadata Not Configured",
    account_label: null,
    status: "missing_auth",
    auth_present: false,
    has_account_identity: false,
    plan_name: null,
    subscription_days_left: null,
    openai_base_url: null,
    quota: quota(null, "6小时后刷新", null, "7天2小时后刷新", "unknown", "unknown", null),
    last_plan_check_ms: null,
    refresh_token_status: refreshStatus({
      health: "missing_auth",
      can_refresh: false,
      refresh_token_present: false,
      access_token_expires_at_ms: null,
      id_token_expires_at_ms: null,
      auth_updated_at_ms: null,
    }),
  },
  {
    folder_name: "workspace-delta",
    display_title: "Workspace Delta",
    account_label: "Workspace Delta",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "Custom endpoint",
    subscription_days_left: 31,
    openai_base_url: "https://example.com/v1",
    quota: quota(73, "2小时后刷新", 88, "1天9小时后刷新", "session_jsonl", "local_observation", 26 * 60 * 60 * 1000),
    last_plan_check_ms: Date.now() - 8 * 24 * 60 * 60 * 1000,
    refresh_token_status: refreshStatus({
      health: "not_refreshable",
      can_refresh: false,
      refresh_token_present: false,
    }),
  },
  {
    folder_name: "workspace-epsilon",
    display_title: "Workspace Epsilon",
    account_label: "Workspace Epsilon",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "Pro plan",
    subscription_days_left: 18,
    openai_base_url: null,
    quota: quota(84, "3小时后刷新", 61, "2天4小时后刷新", "chat_gpt_api", "official", 2 * 24 * 60 * 60 * 1000),
    last_plan_check_ms: Date.now() - 2 * 24 * 60 * 60 * 1000,
    refresh_token_status: refreshStatus({
      health: "healthy",
      refresh_token_expires_at_ms: Date.now() + 14 * 24 * 60 * 60 * 1000,
    }),
  },
  {
    folder_name: "workspace-zeta",
    display_title: "Workspace Zeta",
    account_label: "Workspace Zeta",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "Custom endpoint",
    subscription_days_left: 31,
    openai_base_url: "https://example.com/v1",
    quota: quota(73, "2小时后刷新", 88, "1天9小时后刷新", "codex_app_server", "fallback", 8 * 24 * 60 * 60 * 1000),
    last_plan_check_ms: null,
    refresh_token_status: refreshStatus({
      health: "refresh_due",
      auth_updated_at_ms: Date.now() - 36 * 24 * 60 * 60 * 1000,
    }),
  },
];

let previewCurrentCard: CurrentCard = {
  folder_name: "workspace-alpha",
  display_title: "Workspace Alpha",
  account_label: "Workspace Alpha",
  has_account_identity: true,
  plan_name: "Pro plan",
  subscription_days_left: 18,
  profile_folder_path: "/mock/workspace-alpha",
  last_plan_check_ms: Date.now() - 30 * 60 * 1000,
  refresh_token_status: refreshStatus(),
};

let previewCurrentQuota: QuotaSummary = quota(
  84,
  "3小时后刷新",
  61,
  "2天4小时后刷新",
  "chat_gpt_api",
  "official",
  30 * 60 * 1000,
);

let previewSnapshot: ProfilesSnapshotResponse = {
  page_size: 8,
  profiles: clone(previewProfiles),
  current_card: clone(previewCurrentCard),
  current_quota_card: clone(previewCurrentQuota),
  unmanaged_live_account: null,
};

function previewOpenAiUsageSummary(): OpenAiUsageSummary {
  const primaryTokens = {
    input_tokens: 1_800_000,
    cached_input_tokens: 1_120_000,
    output_tokens: 115_000,
    reasoning_output_tokens: 42_000,
    real_total_tokens: 1_915_000,
  };
  const secondaryTokens = {
    input_tokens: 900_000,
    cached_input_tokens: 410_000,
    output_tokens: 78_000,
    reasoning_output_tokens: 28_000,
    real_total_tokens: 978_000,
  };
  const primaryCost = {
    input_cost_micros: 1_190_000,
    cached_input_cost_micros: 196_000,
    output_cost_micros: 1_379_000,
    total_cost_micros: 2_765_000,
    unpriced_tokens: 0,
  };
  const secondaryCost = {
    input_cost_micros: 700_000,
    cached_input_cost_micros: 71_800,
    output_cost_micros: 1_092_000,
    total_cost_micros: 1_863_800,
    unpriced_tokens: 0,
  };
  const generatedAt = Date.now();
  return {
    generated_at_ms: generatedAt,
    attribution_since_ms: generatedAt - 6 * 60 * 60 * 1000,
    range_start_ms: generatedAt - 24 * 60 * 60 * 1000,
    range_end_ms: generatedAt,
    total_requests: 18,
    tokens: primaryTokens,
    cost: primaryCost,
    cache_hit_rate: primaryTokens.cached_input_tokens / primaryTokens.input_tokens,
    accounts: [{
      profile: previewCurrentCard.folder_name,
      profiles: [previewCurrentCard.folder_name],
      account_group_key: "label:preview@example.com",
      account_identity_key: "acct_preview",
      legacy_account_key: null,
      account_label: previewCurrentCard.account_label,
      official_source: "OpenAI default",
      ledger_layer: "strict_proven",
      attribution_kind: "strict_account",
      attribution_window_id: null,
      request_count: 18,
      tokens: primaryTokens,
      cost: primaryCost,
      attribution_breakdown: [{
        ledger_layer: "strict_proven",
        request_count: 18,
        tokens: primaryTokens,
        cost: primaryCost,
      }],
      models: [{
        model: "gpt-5.3-codex",
        requested_model: "gpt-5.3-codex",
        pricing_model: "gpt-5.3-codex",
        model_reasoning_effort: "xhigh",
        model_verbosity: "high",
        pricing_tier: "standard_short_context",
        pricing_source_url: "https://developers.openai.com/api/docs/pricing",
        request_count: 18,
        tokens: primaryTokens,
        cost: primaryCost,
        priced: true,
        pricing_note: null,
      }],
    }],
    models: [{
      model: "gpt-5.3-codex",
      requested_model: "gpt-5.3-codex",
      pricing_model: "gpt-5.3-codex",
      model_reasoning_effort: "xhigh",
      model_verbosity: "high",
      pricing_tier: "standard_short_context",
      pricing_source_url: "https://developers.openai.com/api/docs/pricing",
      request_count: 18,
      tokens: primaryTokens,
      cost: primaryCost,
      priced: true,
      pricing_note: null,
    }],
    exclusions: {
      duplicate_events: 2,
      non_official_base_url: 0,
      unproven_history: 7,
      unattributed: 0,
      missing_model: 0,
      invalid_delta: 0,
      malformed_lines: 0,
      accepted_events: 65,
      candidate_events: 0,
      rejected_events: 9,
      superseded_events: 2,
      strict_proven_events: 18,
      legacy_profile_attributed_events: 12,
      candidate_unattributed_events: 0,
      rejected_invalid_events: 9,
    },
    scope: "strict",
    layer_summaries: [{
      ledger_layer: "strict_proven",
      request_count: 18,
      tokens: primaryTokens,
      cost: primaryCost,
    }, {
      ledger_layer: "legacy_profile_attributed",
      request_count: 12,
      tokens: secondaryTokens,
      cost: secondaryCost,
    }],
    pricing_source: "OpenAI API pricing",
    pricing_source_url: "https://developers.openai.com/api/docs/pricing",
    pricing_checked_at: "2026-07-08",
  };
}

function previewOpenAiUsageTrends(): OpenAiUsageTrendPoint[] {
  const now = Date.now();
  return Array.from({ length: 12 }, (_, index) => {
    const input = 40_000 + index * 8_000;
    const cached = Math.floor(input * 0.55);
    const output = 4_000 + index * 600;
    const cost = 60_000 + index * 8_000;
    const bucket = now - (11 - index) * 60 * 60 * 1000;
    return {
      date: new Date(bucket).toISOString(),
      bucket_start_ms: bucket,
      request_count: index % 3 === 0 ? 2 : 1,
      tokens: {
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        reasoning_output_tokens: Math.floor(output * 0.3),
        real_total_tokens: input + output,
      },
      cost: {
        input_cost_micros: Math.floor(cost * 0.45),
        cached_input_cost_micros: Math.floor(cost * 0.08),
        output_cost_micros: Math.floor(cost * 0.47),
        total_cost_micros: cost,
        unpriced_tokens: 0,
      },
    };
  });
}

function previewOpenAiUsageLogs(page = 1, pageSize = 20): OpenAiUsageRequestLogPage {
  const summary = previewOpenAiUsageSummary();
  const total = 1960;
  const safePageSize = Math.max(1, Math.min(200, pageSize));
  const safePage = Math.max(1, page);
  const start = (safePage - 1) * safePageSize;
  const rows: OpenAiUsageRequestLog[] = Array.from({ length: safePageSize }, (_, index) => {
    const rowNumber = start + index + 1;
    return {
      request_id: `preview-${rowNumber}`,
      session_id: "preview-session",
      profile: previewCurrentCard.folder_name,
      account_label: previewCurrentCard.account_label,
      official_source: "OpenAI default",
      ledger_layer: "strict_proven",
      attribution_kind: "strict_account",
      attribution_window_id: null,
      model: "gpt-5.3-codex",
      requested_model: "gpt-5.3-codex",
      pricing_model: "gpt-5.3-codex",
      model_reasoning_effort: "xhigh",
      model_verbosity: "high",
      pricing_tier: "standard_short_context",
      pricing_source_url: "https://developers.openai.com/api/docs/pricing",
      tokens: {
        input_tokens: 80_000 + rowNumber * 1_000,
        cached_input_tokens: 44_000 + rowNumber * 400,
        output_tokens: 6_000 + rowNumber * 120,
        reasoning_output_tokens: 2_000,
        real_total_tokens: 86_000 + rowNumber * 1_120,
      },
      cost: summary.cost,
      priced: true,
      pricing_note: "OpenAI API pricing",
      status_code: 200,
      created_at_ms: Date.now() - rowNumber * 60_000,
      data_source: "codex_session",
      usage_confidence: "high",
    };
  });
  return {
    data: rows,
    total,
    page: safePage,
    page_size: safePageSize,
  };
}

function previewOpenAiUsageDataSources(): OpenAiUsageDataSourceSummary[] {
  return [{
    data_source: "codex_session",
    request_count: 18,
    cost: previewOpenAiUsageSummary().cost,
  }];
}

function zeroUsageTokens() {
  return {
    input_tokens: 0,
    cached_input_tokens: 0,
    output_tokens: 0,
    reasoning_output_tokens: 0,
    real_total_tokens: 0,
  };
}

function previewOpenAiUsageDashboard(page = 1, pageSize = 20): OpenAiUsageDashboard {
  const summary = previewOpenAiUsageSummary();
  return {
    summary,
    account_summary: summary.accounts,
    model_options: summary.models,
    trends: previewOpenAiUsageTrends(),
    logs: previewOpenAiUsageLogs(page, pageSize),
    data_sources: previewOpenAiUsageDataSources(),
    reconciliation: {
      cutoff_ms: Date.now(),
      source_signature: "preview-ccswitch-source",
      source_tokens: summary.tokens,
      source_rows: 0,
      classified_rows: 0,
      accepted_tokens: summary.tokens,
      candidate_tokens: zeroUsageTokens(),
      superseded_tokens: zeroUsageTokens(),
      status: "unavailable",
      rejected_tokens: zeroUsageTokens(),
      unknown_decision_tokens: zeroUsageTokens(),
      ledger_minus_source: { input_tokens: 0, cached_input_tokens: 0, output_tokens: 0, reasoning_output_tokens: 0, real_total_tokens: 0 },
      mismatch_reason: null,
      candidate_reasons: [],
    },
  };
}

function mockAction(message: string, path: string | null = null): Promise<ActionResponse> {
  return Promise.resolve({
    ok: true,
    message,
    path,
  });
}

function refreshPreviewSnapshot(): void {
  previewSnapshot = {
    page_size: 8,
    profiles: clone(previewSnapshot.profiles),
    current_card: clone(previewCurrentCard),
    current_quota_card: clone(previewCurrentQuota),
    unmanaged_live_account: null,
  };
}

function toError(error: unknown): Error {
  if (typeof error === "string") {
    return new Error(error);
  }

  if (error && typeof error === "object") {
    const payload = error as CommandError;
    if (payload.message || payload.error_code) {
      const nextError = new Error(payload.message || "Unknown native command error.") as NativeCommandError;
      nextError.code = payload.error_code;
      return nextError;
    }
  }

  return new Error("Unknown native command error.");
}

async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasTauriRuntime) {
    switch (command) {
      case "get_profiles_snapshot":
        return clone(previewSnapshot) as T;
      case "get_current_live_quota":
      case "refresh_active_profile_quota_silent":
        return {
          profile: previewCurrentCard.folder_name,
          quota: clone(previewCurrentQuota),
        } as T;
      case "refresh_all_oauth_profile_plans_silent":
        return 0 as T;
      case "get_openai_usage_dashboard":
        return clone(previewOpenAiUsageDashboard(
          Number(args?.page) || 1,
          Number(args?.pageSize) || 20,
        )) as T;
      case "get_openai_usage_summary":
        return clone(previewOpenAiUsageSummary()) as T;
      case "get_openai_usage_trends":
        return clone(previewOpenAiUsageTrends()) as T;
      case "get_openai_usage_request_logs":
        return clone(previewOpenAiUsageLogs(
          Number(args?.page) || 1,
          Number(args?.pageSize) || 20,
        )) as T;
      case "get_openai_usage_request_detail": {
        const requestId = args?.requestId as string | undefined;
        return (previewOpenAiUsageLogs().data.find((row) => row.request_id === requestId) ?? null) as T;
      }
      case "get_openai_usage_data_sources":
        return clone(previewOpenAiUsageDataSources()) as T;
      case "switch_profile": {
        const profile = (args?.payload as { profile?: string } | undefined)?.profile ?? previewCurrentCard.folder_name;
        const next = previewSnapshot.profiles.find((entry) => entry.folder_name === profile);
        if (next) {
          previewCurrentCard = {
            folder_name: next.folder_name,
            display_title: next.display_title,
            account_label: next.account_label,
            has_account_identity: next.has_account_identity,
            plan_name: next.plan_name,
            subscription_days_left: next.subscription_days_left,
            profile_folder_path: `C:/mock/${next.folder_name}`,
            last_plan_check_ms: next.last_plan_check_ms,
            refresh_token_status: clone(next.refresh_token_status),
          };
          previewCurrentQuota = clone(next.quota);
          refreshPreviewSnapshot();
        }
        return {
          ok: true,
          profile,
          message: "Switched in preview mode",
          warnings: [],
        } as T;
      }
      case "rename_profile": {
        const payload = args?.payload as { profile?: string; new_folder_name?: string } | undefined;
        if (payload?.profile && payload.new_folder_name) {
          const sourceProfile = payload.profile;
          const nextFolderName = payload.new_folder_name;
          previewSnapshot.profiles = previewSnapshot.profiles.map((entry) =>
            entry.folder_name === sourceProfile
              ? {
                  ...entry,
                  folder_name: nextFolderName,
                  display_title: nextFolderName,
                  account_label: nextFolderName,
                }
              : entry,
          );
          if (previewCurrentCard.folder_name === sourceProfile) {
            previewCurrentCard = {
              ...previewCurrentCard,
              folder_name: nextFolderName,
              display_title: nextFolderName,
              account_label: nextFolderName,
            };
          }
          refreshPreviewSnapshot();
        }
        return mockAction("Renamed in preview mode") as Promise<T>;
      }
      case "add_profile": {
        const payload = args?.payload as { folder_name?: string; openai_base_url?: string | null } | undefined;
        if (payload?.folder_name) {
          previewSnapshot.profiles.push({
            folder_name: payload.folder_name,
            display_title: payload.folder_name,
            account_label: payload.folder_name,
            status: "available",
            auth_present: true,
            has_account_identity: true,
            plan_name: "Pro plan",
            subscription_days_left: 30,
            openai_base_url: payload.openai_base_url ?? null,
            quota: quota(52, "5小时后刷新", 67, "3天后刷新"),
            last_plan_check_ms: Date.now(),
            refresh_token_status: refreshStatus({
              health: "unknown_expiry",
              auth_updated_at_ms: Date.now(),
            }),
          });
          refreshPreviewSnapshot();
        }
        return mockAction("Added in preview mode") as Promise<T>;
      }
      case "update_profile_base_url": {
        const payload = args?.payload as { profile?: string; openai_base_url?: string } | undefined;
        if (payload?.profile) {
          previewSnapshot.profiles = previewSnapshot.profiles.map((entry) =>
            entry.folder_name === payload.profile
              ? { ...entry, openai_base_url: payload.openai_base_url ?? null }
              : entry,
          );
          refreshPreviewSnapshot();
        }
        return mockAction("Base URL updated in preview mode") as Promise<T>;
      }
      case "delete_profile": {
        const payload = args?.payload as { profile?: string } | undefined;
        if (payload?.profile) {
          previewSnapshot.profiles = previewSnapshot.profiles.filter(
            (entry) => entry.folder_name !== payload.profile,
          );
          refreshPreviewSnapshot();
        }
        return mockAction("Deleted in preview mode") as Promise<T>;
      }
      case "clear_profile_account": {
        const payload = args?.payload as { profile?: string } | undefined;
        if (payload?.profile) {
          previewSnapshot.profiles = previewSnapshot.profiles.map((entry) =>
            entry.folder_name === payload.profile
              ? {
                  ...entry,
                  account_label: null,
                  display_title: entry.folder_name,
                  status: "missing_auth",
                  auth_present: false,
                  has_account_identity: false,
                  plan_name: null,
                  subscription_days_left: null,
                  openai_base_url: null,
                  quota: quota(null, null, null, null),
                }
              : entry,
          );
          refreshPreviewSnapshot();
        }
        return mockAction("Cleared in preview mode") as Promise<T>;
      }
      case "check_update":
        return Promise.resolve({
          ok: true,
          current_version: "1.5.0",
          latest_version: "1.5.0",
          has_update: false,
          local_patch_required: false,
          release_url: "https://github.com/Baker4real/codex-account-switch-local-patches/releases",
          notes: null,
          checked_url: "preview",
        }) as Promise<T>;
      case "open_url":
        return mockAction("Opened URL in preview mode", "preview:url") as Promise<T>;
      case "get_codex_cli_status":
      case "set_codex_cli_path":
      case "clear_codex_cli_path":
        return Promise.resolve({
          resolved_path: "/preview/codex",
          source: command === "set_codex_cli_path" ? "user_override" : "discovery",
          suggested_paths: ["/preview/codex", "/preview/usr/local/bin/codex"],
        }) as Promise<T>;
      case "redetect_codex_cli_path":
        return Promise.resolve({
          candidates: [{ path: "/preview/codex", version: "codex-cli 0.133.0" }],
          status: {
            resolved_path: "/preview/codex",
            source: "user_override",
            suggested_paths: ["/preview/codex", "/preview/usr/local/bin/codex"],
          },
        }) as Promise<T>;
      case "cancel_codex_login":
        return Promise.resolve(true) as Promise<T>;
      case "open_profile_folder":
      case "open_codex":
      case "restart_codex":
      case "login_current_profile":
      case "login_profile":
      case "refresh_profile":
      case "open_releases":
      case "open_contact":
      case "open_xiaohongshu":
        return mockAction(`${command} completed in preview mode`) as Promise<T>;
      default:
        return Promise.reject(new Error(`Unsupported preview command: ${command}`));
    }
  }

  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toError(error);
  }
}

export function getProfilesSnapshot(): Promise<ProfilesSnapshotResponse> {
  return invokeCommand<ProfilesSnapshotResponse>("get_profiles_snapshot");
}

export function getCurrentLiveQuota(): Promise<CurrentQuotaResponse> {
  return invokeCommand<CurrentQuotaResponse>("get_current_live_quota");
}

export function refreshActiveProfileQuotaSilent(): Promise<CurrentQuotaResponse> {
  return invokeCommand<CurrentQuotaResponse>("refresh_active_profile_quota_silent");
}

export function refreshAllOauthProfilePlansSilent(): Promise<number> {
  return invokeCommand<number>("refresh_all_oauth_profile_plans_silent");
}

export function getOpenAiUsageDashboard(
  query: OpenAiUsageQuery | undefined,
  page: number,
  pageSize: number,
): Promise<OpenAiUsageDashboard> {
  return invokeCommand<OpenAiUsageDashboard>("get_openai_usage_dashboard", {
    query: query ?? null,
    page,
    pageSize,
  });
}

export function getOpenAiUsageSummary(query?: OpenAiUsageQuery): Promise<OpenAiUsageSummary> {
  return invokeCommand<OpenAiUsageSummary>("get_openai_usage_summary", { query: query ?? null });
}

export function getOpenAiUsageTrends(query?: OpenAiUsageQuery): Promise<OpenAiUsageTrendPoint[]> {
  return invokeCommand<OpenAiUsageTrendPoint[]>("get_openai_usage_trends", { query: query ?? null });
}

export function getOpenAiUsageRequestLogs(
  query: OpenAiUsageQuery | undefined,
  page: number,
  pageSize: number,
): Promise<OpenAiUsageRequestLogPage> {
  return invokeCommand<OpenAiUsageRequestLogPage>("get_openai_usage_request_logs", {
    query: query ?? null,
    page,
    pageSize,
  });
}

export function getOpenAiUsageRequestDetail(
  requestId: string,
): Promise<OpenAiUsageRequestLog | null> {
  return invokeCommand<OpenAiUsageRequestLog | null>("get_openai_usage_request_detail", {
    requestId,
  });
}

export function getOpenAiUsageDataSources(
  query?: OpenAiUsageQuery,
): Promise<OpenAiUsageDataSourceSummary[]> {
  return invokeCommand<OpenAiUsageDataSourceSummary[]>("get_openai_usage_data_sources", {
    query: query ?? null,
  });
}

export function switchProfile(profile: string): Promise<SwitchResponse> {
  return invokeCommand<SwitchResponse>("switch_profile", { payload: { profile } });
}

export function openProfileFolder(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_profile_folder", { payload: { profile } });
}

export function addProfile(folderName: string, openaiBaseUrl: string | null): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("add_profile", {
    payload: {
      folder_name: folderName,
      openai_base_url: openaiBaseUrl,
    },
  });
}

export function openCodex(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_codex");
}

export function restartCodex(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("restart_codex");
}

export function loginCurrentProfile(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("login_current_profile");
}

export function loginProfile(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("login_profile", { payload: { profile } });
}

export function refreshProfile(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("refresh_profile", { payload: { profile } });
}

export function renameProfile(profile: string, newFolderName: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("rename_profile", {
    payload: { profile, new_folder_name: newFolderName },
  });
}

export function updateProfileBaseUrl(profile: string, openaiBaseUrl: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("update_profile_base_url", {
    payload: { profile, openai_base_url: openaiBaseUrl },
  });
}

export function deleteProfile(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("delete_profile", { payload: { profile } });
}

export function clearProfileAccount(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("clear_profile_account", { payload: { profile } });
}

export function openContact(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_contact");
}

export function openReleases(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_releases");
}

export function openUrl(url: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_url", {
    payload: { url },
  });
}

export function checkUpdate(updateUrl: string): Promise<UpdateCheckResponse> {
  return invokeCommand<UpdateCheckResponse>("check_update", {
    payload: { update_url: updateUrl },
  });
}

export function openXiaohongshu(): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_xiaohongshu");
}

export function getCodexCliStatus(): Promise<CodexCliStatus> {
  return invokeCommand<CodexCliStatus>("get_codex_cli_status");
}

export function setCodexCliPath(path: string): Promise<CodexCliStatus> {
  return invokeCommand<CodexCliStatus>("set_codex_cli_path", {
    payload: { path },
  });
}

export function clearCodexCliPath(): Promise<CodexCliStatus> {
  return invokeCommand<CodexCliStatus>("clear_codex_cli_path");
}

export function redetectCodexCliPath(): Promise<CodexCliRedetectResult> {
  return invokeCommand<CodexCliRedetectResult>("redetect_codex_cli_path");
}

export function cancelCodexLogin(): Promise<boolean> {
  return invokeCommand<boolean>("cancel_codex_login");
}

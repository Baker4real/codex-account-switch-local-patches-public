import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { DeviceCodePromptPayload } from "@front-shared/device-code-login";
import type {
  ActionResponse,
  AuthorizationMethod,
  CodexCliRedetectResult,
  CodexCliStatus,
  CommandError,
  CurrentCard,
  CurrentQuotaResponse,
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

function previewArrangementName(index: number): string {
  let value = index + 1;
  let result = "";
  while (value > 0) {
    const digit = (value - 1) % 26;
    result = String.fromCharCode(97 + digit) + result;
    value = Math.floor((value - 1) / 26);
  }
  return result;
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
    last_authorization_method: "browser",
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
    last_authorization_method: "device_code",
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
    last_authorization_method: null,
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
    last_authorization_method: null,
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
    last_authorization_method: "browser",
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
    last_authorization_method: null,
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

function setPreviewAuthorizationMethod(
  profileName: string,
  authorizationMethod: AuthorizationMethod,
): void {
  previewSnapshot.profiles = previewSnapshot.profiles.map((profile) =>
    profile.folder_name === profileName
      ? { ...profile, last_authorization_method: authorizationMethod }
      : profile,
  );
  refreshPreviewSnapshot();
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
        const payload = args?.payload as { profile?: string; new_nickname?: string } | undefined;
        if (payload?.profile && payload.new_nickname !== undefined) {
          const sourceProfile = payload.profile;
          const nextNickname = payload.new_nickname.trim() || undefined;
          previewSnapshot.profiles = previewSnapshot.profiles.map((entry) =>
            entry.folder_name === sourceProfile
              ? {
                  ...entry,
                  nickname: nextNickname,
                  display_title: nextNickname
                    ? `${entry.folder_name} · ${nextNickname}`
                    : entry.folder_name,
                }
              : entry,
          );
          if (previewCurrentCard.folder_name === sourceProfile) {
            previewCurrentCard = {
              ...previewCurrentCard,
              nickname: nextNickname,
              display_title: nextNickname || previewCurrentCard.folder_name,
            };
          }
          refreshPreviewSnapshot();
        }
        return mockAction("Renamed in preview mode") as Promise<T>;
      }
      case "add_profile": {
        const payload = args?.payload as { openai_base_url?: string | null } | undefined;
        const folderName = previewArrangementName(previewSnapshot.profiles.length);
        previewSnapshot.profiles.push({
            folder_name: folderName,
            display_title: folderName,
            account_label: null,
            status: "available",
            auth_present: false,
            has_account_identity: false,
            plan_name: null,
            subscription_days_left: null,
            openai_base_url: payload?.openai_base_url ?? null,
            last_authorization_method: null,
            quota: quota(52, "5小时后刷新", 67, "3天后刷新"),
            last_plan_check_ms: null,
            refresh_token_status: refreshStatus({
              health: "missing_auth",
              can_refresh: false,
              refresh_token_present: false,
              access_token_expires_at_ms: null,
              id_token_expires_at_ms: null,
              auth_updated_at_ms: null,
            }),
          });
        refreshPreviewSnapshot();
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
                  last_authorization_method: null,
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
      case "login_current_profile":
        setPreviewAuthorizationMethod(previewCurrentCard.folder_name, "browser");
        return mockAction(`${command} completed in preview mode`) as Promise<T>;
      case "login_profile":
      case "login_profile_device_code": {
        const payload = args?.payload as { profile?: string } | undefined;
        if (payload?.profile) {
          setPreviewAuthorizationMethod(
            payload.profile,
            command === "login_profile" ? "browser" : "device_code",
          );
        }
        return mockAction(`${command} completed in preview mode`) as Promise<T>;
      }
      case "open_profile_folder":
      case "open_codex":
      case "restart_codex":
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

export function switchProfile(profile: string): Promise<SwitchResponse> {
  return invokeCommand<SwitchResponse>("switch_profile", { payload: { profile } });
}

export function openProfileFolder(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("open_profile_folder", { payload: { profile } });
}

export function addProfile(openaiBaseUrl: string | null): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("add_profile", {
    payload: {
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

export function loginProfileDeviceCode(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("login_profile_device_code", { payload: { profile } });
}

export async function listenForDeviceCodePrompts(
  handler: (payload: DeviceCodePromptPayload) => void,
): Promise<() => void> {
  if (!hasTauriRuntime) {
    return () => {};
  }
  return listen<DeviceCodePromptPayload>("codex-device-code-prompt", (event) => {
    handler(event.payload);
  });
}

export function refreshProfile(profile: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("refresh_profile", { payload: { profile } });
}

export function renameProfile(profile: string, newNickname: string): Promise<ActionResponse> {
  return invokeCommand<ActionResponse>("rename_profile", {
    payload: { profile, new_nickname: newNickname },
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

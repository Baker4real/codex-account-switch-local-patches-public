use crate::errors::CommandError;
use crate::models::{CurrentQuotaResponse, ProfilesSnapshotResponse};

#[cfg(target_os = "macos")]
use crate::macos as platform_runtime;

#[cfg(not(target_os = "macos"))]
use crate::windows as platform_runtime;

#[tauri::command]
pub fn get_profiles_snapshot() -> Result<ProfilesSnapshotResponse, CommandError> {
    platform_runtime::profiles_index::load_profiles_snapshot(None).map_err(Into::into)
}

#[tauri::command]
pub fn get_current_live_quota() -> Result<CurrentQuotaResponse, CommandError> {
    platform_runtime::profiles_index::load_current_live_quota(None).map_err(Into::into)
}

/// Silent background refresh of the active OAuth profile's quota via the
/// ChatGPT-API path. Skipped (returns the existing snapshot) when:
///   * No active profile is selected.
///   * The active profile is API-key (not OAuth).
///   * The cached quota was updated less than `SILENT_REFRESH_MIN_AGE_MS`
///     ago, so we don't HTTP-spam during fast-tab churn.
///   * The HTTP path itself fails (network, 401 we couldn't recover from,
///     parse error). Failure is treated as "no update", never as an error
///     surfaced to the user — the legacy local-JSONL polling continues to
///     drive the visible quota.
///
/// Returns `CurrentQuotaResponse` so the front-end can apply the snapshot
/// without round-tripping through `get_current_live_quota`.
#[tauri::command]
pub async fn refresh_active_profile_quota_silent() -> Result<CurrentQuotaResponse, CommandError> {
    tauri::async_runtime::spawn_blocking(refresh_active_profile_quota_silent_inner)
        .await
        .map_err(|error| {
            CommandError::new(
                "QUOTA_AUTO_REFRESH_TASK_FAILED",
                format!("Quota auto-refresh task failed: {error}"),
            )
        })?
}

fn refresh_active_profile_quota_silent_inner() -> Result<CurrentQuotaResponse, CommandError> {
    crate::shared::dashboard_refresh::refresh_active_profile_quota_silent(
        crate::platform::current_hooks(),
        crate::shared::chatgpt_api::refresh_profile_via_api_detailed,
        |codex_home| {
            platform_runtime::profiles_index::load_current_live_quota(Some(codex_home))
                .map_err(Into::into)
        },
    )
}

/// Startup / local-day rollover background pass: serially refresh every OAuth
/// profile's plan and quota through the usage API. Each profile is skipped
/// while its plan check is still fresh (6h TTL). `chatgpt_api` rotates OAuth
/// tokens only when the access_token is expired or the usage endpoint returns
/// 401; a stale plan cache alone must not spend a refresh_token.
///
/// Failures per-profile are silent — this is best-effort, never blocks
/// the user, and the dashboard's other refresh paths remain the
/// authoritative source if any individual refresh fails.
///
/// The frontend invokes this twice: once on app startup, then on every
/// local-day rollover (detected by a setInterval comparing the cached
/// last-run date with `new Date().toDateString()`).
#[tauri::command]
pub async fn refresh_all_oauth_profile_plans_silent() -> Result<u32, CommandError> {
    tauri::async_runtime::spawn_blocking(refresh_all_oauth_profile_plans_silent_inner)
        .await
        .map_err(|error| {
            CommandError::new(
                "PLAN_BULK_REFRESH_TASK_FAILED",
                format!("Bulk plan refresh task failed: {error}"),
            )
        })?
}

fn refresh_all_oauth_profile_plans_silent_inner() -> Result<u32, CommandError> {
    crate::shared::dashboard_refresh::refresh_all_oauth_profile_plans_silent(
        crate::platform::current_hooks(),
        crate::shared::chatgpt_api::refresh_profile_via_api_with_options_detailed,
    )
}

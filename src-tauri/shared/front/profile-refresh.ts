import type { ProfileCard, RefreshTokenStatus } from "@front-shared/types";

export function canRefreshProfile(profile: Pick<ProfileCard, "auth_present" | "refresh_token_status">): boolean {
  return profile.auth_present && profile.refresh_token_status.can_refresh;
}

export function refreshableAuthObservedAt(status: RefreshTokenStatus): number | null {
  if (!status.can_refresh) {
    return null;
  }

  return status.last_refresh_at_ms ?? status.auth_updated_at_ms;
}

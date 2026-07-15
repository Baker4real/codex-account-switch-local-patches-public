import type { ProfileCard } from "@front-shared/types";

export function canRefreshProfile(profile: Pick<ProfileCard, "auth_present" | "refresh_token_status">): boolean {
  return profile.auth_present && profile.refresh_token_status.can_refresh;
}

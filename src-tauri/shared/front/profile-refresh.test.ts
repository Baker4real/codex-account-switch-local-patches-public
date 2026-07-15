import { describe, expect, it } from "vitest";
import type { ProfileCard } from "@front-shared/types";
import { canRefreshProfile } from "@front-shared/profile-refresh";

function profile(canRefresh: boolean): Pick<ProfileCard, "auth_present" | "refresh_token_status"> {
  return {
    auth_present: true,
    refresh_token_status: {
      health: canRefresh ? "healthy" : "not_refreshable",
      can_refresh: canRefresh,
      refresh_token_present: canRefresh,
      refresh_token_expires_at_ms: null,
      access_token_expires_at_ms: null,
      id_token_expires_at_ms: null,
      auth_updated_at_ms: null,
      last_refresh_at_ms: null,
    },
  };
}

describe("profile refresh eligibility", () => {
  it("does not offer refresh for an auth file without a refreshable OAuth session", () => {
    expect(canRefreshProfile(profile(false))).toBe(false);
  });

  it("allows refresh only when the profile has a refreshable OAuth session", () => {
    expect(canRefreshProfile(profile(true))).toBe(true);
  });
});

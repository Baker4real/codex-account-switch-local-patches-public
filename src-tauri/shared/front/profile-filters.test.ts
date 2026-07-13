import { describe, expect, it } from "vitest";
import type { ProfileCard } from "@front-shared/types";
import {
  buildProfileCounts,
  countActiveProfileFilters,
  filterProfiles,
  listProfilePlanOptions,
  normalizeProfileFilterText,
  type ProfileFilters,
} from "@front-shared/profile-filters";

function profile(
  folder_name: string,
  display_title: string,
  account_label: string | null,
  plan_name: string | null,
): ProfileCard {
  return {
    folder_name,
    display_title,
    account_label,
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name,
    subscription_days_left: null,
    openai_base_url: null,
    quota: {
      five_hour: { remaining_percent: null, refresh_at: null, reset_at_timestamp: null },
      weekly: { remaining_percent: null, refresh_at: null, reset_at_timestamp: null },
      additional: null,
    },
    last_plan_check_ms: null,
    refresh_token_status: {
      health: "healthy",
      can_refresh: true,
      refresh_token_present: true,
      refresh_token_expires_at_ms: null,
      access_token_expires_at_ms: null,
      id_token_expires_at_ms: null,
      auth_updated_at_ms: null,
      last_refresh_at_ms: null,
    },
  };
}

const profiles = [
  profile("work", "Work / alice@example.com", "alice@example.com", "Plus"),
  profile("personal", "Personal / bob@example.com", "bob@example.com", "Free"),
  profile("team", "Team / carol@example.com", "carol@example.com", null),
];

describe("profile filters", () => {
  it("normalizes query text without changing the source profile", () => {
    expect(normalizeProfileFilterText("  ALICE@example.com ")).toBe("alice@example.com");
    expect(filterProfiles(profiles, { query: " ALICE ", plan: null })).toHaveLength(1);
    expect(profiles[0]?.folder_name).toBe("work");
  });

  it("searches only the visible account fields", () => {
    expect(filterProfiles(profiles, { query: "personal", plan: null })[0]?.folder_name).toBe(
      "personal",
    );
    expect(filterProfiles(profiles, { query: "Free", plan: null })[0]?.folder_name).toBe(
      "personal",
    );
    expect(filterProfiles(profiles, { query: "https://secret.example", plan: null })).toEqual(
      [],
    );
  });

  it("builds dynamic plan options from the complete snapshot", () => {
    expect(listProfilePlanOptions(profiles)).toEqual(["Free", "Plus"]);
  });

  it("filters by plan and reports visible versus total counts", () => {
    const filters: ProfileFilters = { query: "", plan: " plus " };
    expect(filterProfiles(profiles, filters).map((item) => item.folder_name)).toEqual(["work"]);
    expect(buildProfileCounts(profiles, filters)).toEqual({ visible: 1, total: 3 });
    expect(countActiveProfileFilters(filters)).toBe(1);
  });

  it("clearing filters restores the original order and count", () => {
    const filtered = filterProfiles(profiles, { query: "team", plan: "Free" });
    expect(filtered).toEqual([]);
    expect(filterProfiles(profiles, { query: "", plan: null })).toEqual(profiles);
    expect(countActiveProfileFilters({ query: "", plan: null })).toBe(0);
  });
});

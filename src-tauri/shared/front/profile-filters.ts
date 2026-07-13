import type { ProfileCard } from "@front-shared/types";

export interface ProfileFilters {
  query: string;
  plan: string | null;
}

export interface ProfileCounts {
  visible: number;
  total: number;
}

export function normalizeProfileFilterText(value: string | null | undefined): string {
  return (value ?? "").normalize("NFKC").trim().toLowerCase();
}

export function listProfilePlanOptions(profiles: readonly ProfileCard[]): string[] {
  const options = new Map<string, string>();
  for (const profile of profiles) {
    const raw = profile.plan_name?.trim() ?? "";
    const normalized = normalizeProfileFilterText(raw);
    if (normalized && !options.has(normalized)) {
      options.set(normalized, raw);
    }
  }

  return [...options.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([, value]) => value);
}

function profileSearchText(profile: ProfileCard): string {
  return [
    profile.folder_name,
    profile.display_title,
    profile.account_label ?? "",
    profile.plan_name ?? "",
  ]
    .map(normalizeProfileFilterText)
    .filter(Boolean)
    .join(" ");
}

export function filterProfiles(
  profiles: readonly ProfileCard[],
  filters: ProfileFilters,
): ProfileCard[] {
  const query = normalizeProfileFilterText(filters.query);
  const plan = normalizeProfileFilterText(filters.plan);

  return profiles.filter((profile) => {
    if (query && !profileSearchText(profile).includes(query)) {
      return false;
    }
    if (plan && normalizeProfileFilterText(profile.plan_name) !== plan) {
      return false;
    }
    return true;
  });
}

export function countActiveProfileFilters(filters: ProfileFilters): number {
  return Number(normalizeProfileFilterText(filters.query).length > 0) + Number(
    normalizeProfileFilterText(filters.plan).length > 0,
  );
}

export function buildProfileCounts(
  profiles: readonly ProfileCard[],
  filters: ProfileFilters,
): ProfileCounts {
  return {
    visible: filterProfiles(profiles, filters).length,
    total: profiles.length,
  };
}

import type { AdditionalQuotaWindow, QuotaSummary } from "@front-shared/types";

export const ADDITIONAL_QUOTA_STALE_MS = 6 * 60 * 60 * 1000;

export type AdditionalQuotaPresentationState = "hidden" | "ready";

export interface AdditionalQuotaPresentation {
  state: AdditionalQuotaPresentationState;
  windows: AdditionalQuotaWindow[];
  stale: boolean;
}

export type RateLimitResetCreditsPresentation =
  | { state: "unknown"; availableCount: null }
  | {
      state: "ready";
      availableCount: number;
      cards: Array<{ grantedAt: number | null; expiresAt: number | null }>;
    };

function isSparkEntry(entry: AdditionalQuotaWindow): boolean {
  return (
    entry.is_spark ||
    [entry.limit_name, entry.metered_feature, entry.label]
      .join(" ")
      .toLowerCase()
      .includes("spark")
  );
}

export function isQuotaStale(
  quota: QuotaSummary | null | undefined,
  nowMs = Date.now(),
): boolean {
  const observedAt = quota?.provenance?.observed_at_ms ?? quota?.provenance?.source_mtime_ms;
  return observedAt == null || nowMs - observedAt >= ADDITIONAL_QUOTA_STALE_MS;
}

export function buildAdditionalQuotaPresentation(
  quota: QuotaSummary | null | undefined,
  showAdditional: boolean,
  showSpark: boolean,
  nowMs = Date.now(),
): AdditionalQuotaPresentation {
  if (!showAdditional) {
    return { state: "hidden", windows: [], stale: false };
  }
  if (quota?.additional == null) {
    return { state: "hidden", windows: [], stale: isQuotaStale(quota, nowMs) };
  }
  const windows = quota.additional
    .map((entry) => ({ ...entry, is_spark: isSparkEntry(entry) }))
    .filter((entry) => showSpark || !entry.is_spark);
  return {
    state: windows.length > 0 ? "ready" : "hidden",
    windows,
    stale: isQuotaStale(quota, nowMs),
  };
}

export function buildRateLimitResetCreditsPresentation(
  quota: QuotaSummary | null | undefined,
): RateLimitResetCreditsPresentation {
  const availableCount = quota?.rate_limit_reset_credits?.available_count;
  if (availableCount == null) {
    return { state: "unknown", availableCount: null };
  }
  return {
    state: "ready",
    availableCount,
    cards: (quota?.rate_limit_reset_credits?.credits ?? []).map((credit) => ({
      grantedAt: credit.granted_at,
      expiresAt: credit.expires_at,
    })),
  };
}

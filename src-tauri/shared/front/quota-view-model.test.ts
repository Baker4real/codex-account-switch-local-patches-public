import { describe, expect, it } from "vitest";
import type { QuotaSummary } from "@front-shared/types";
import {
  ADDITIONAL_QUOTA_STALE_MS,
  buildAdditionalQuotaPresentation,
  buildRateLimitResetCreditsPresentation,
} from "@front-shared/quota-view-model";

function quota(
  additional: QuotaSummary["additional"],
  observedAtMs: number | null,
  availableResetCredits: number | null = null,
  resetCreditCards: NonNullable<QuotaSummary["rate_limit_reset_credits"]>["credits"] = [],
): QuotaSummary {
  return {
    five_hour: { remaining_percent: null, refresh_at: null, reset_at_timestamp: null },
    weekly: { remaining_percent: null, refresh_at: null, reset_at_timestamp: null },
    additional,
    rate_limit_reset_credits:
      availableResetCredits == null
        ? null
        : { available_count: availableResetCredits, credits: resetCreditCards },
    provenance: {
      source: "chat_gpt_api",
      confidence: "official",
      observed_at_ms: observedAtMs,
      source_mtime_ms: null,
      note: null,
    },
  };
}

describe("additional quota presentation", () => {
  it("keeps the default view free of Spark noise", () => {
    const result = buildAdditionalQuotaPresentation(
      quota(
        [
          {
            id: "additional:0:primary",
            label: "GPT quota",
            limit_name: "gpt-5.4-codex",
            metered_feature: "codex",
            window_kind: "primary",
            remaining_percent: 80,
            refresh_at: null,
            reset_at_timestamp: null,
            window_minutes: 300,
            allowed: true,
            limit_reached: false,
            is_spark: false,
          },
          {
            id: "additional:1:primary",
            label: "Spark quota",
            limit_name: "codex-spark",
            metered_feature: "spark",
            window_kind: "primary",
            remaining_percent: 50,
            refresh_at: null,
            reset_at_timestamp: null,
            window_minutes: 300,
            allowed: true,
            limit_reached: false,
            is_spark: true,
          },
        ],
        1_800_000_000_000,
      ),
      true,
      false,
      1_800_000_000_000,
    );
    expect(result.state).toBe("ready");
    expect(result.windows.map((entry) => entry.id)).toEqual(["additional:0:primary"]);
    expect(result.stale).toBe(false);
  });

  it("does not render a misleading status when model-specific quota was not reported", () => {
    const result = buildAdditionalQuotaPresentation(
      quota(null, 1_800_000_000_000 - ADDITIONAL_QUOTA_STALE_MS),
      true,
      false,
      1_800_000_000_000,
    );
    expect(result.state).toBe("hidden");
    expect(result.stale).toBe(true);
  });

  it("hides an explicit empty response instead of claiming that reset cards are missing", () => {
    expect(
      buildAdditionalQuotaPresentation(quota([], 1_800_000_000_000), true, false, 1_800_000_000_000)
        .state,
    ).toBe("hidden");
    expect(
      buildAdditionalQuotaPresentation(quota([], 1_800_000_000_000), false, false, 1_800_000_000_000)
        .windows,
    ).toEqual([]);
  });

  it("presents every reset credit grant and expiry from the detail endpoint", () => {
    const result = buildRateLimitResetCreditsPresentation(
      quota([], 1_800_000_000_000, 2, [
        { granted_at: 1_800_000_000, expires_at: 1_800_086_400 },
        { granted_at: 1_800_010_000, expires_at: 1_800_096_400 },
      ]),
    );

    expect(result).toEqual({
      state: "ready",
      availableCount: 2,
      cards: [
        { grantedAt: 1_800_000_000, expiresAt: 1_800_086_400 },
        { grantedAt: 1_800_010_000, expiresAt: 1_800_096_400 },
      ],
    });
  });

  it("keeps card details empty when only the usage summary count is available", () => {
    expect(buildRateLimitResetCreditsPresentation(quota([], 1_800_000_000_000, 1)))
      .toEqual({ state: "ready", availableCount: 1, cards: [] });
  });
});

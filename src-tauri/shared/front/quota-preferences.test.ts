import { beforeEach, describe, expect, it } from "vitest";
import {
  persistAdditionalQuotaVisibility,
  resolveInitialAdditionalQuotaVisibility,
  resolveInitialSparkQuotaVisibility,
} from "@front-shared/quota-preferences";

describe("quota preferences", () => {
  const values = new Map<string, string>();

  beforeEach(() => {
    values.clear();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
        clear: () => values.clear(),
      },
    });
  });

  it("defaults to ordinary additional quotas visible and Spark hidden", () => {
    expect(resolveInitialAdditionalQuotaVisibility()).toBe(true);
    expect(resolveInitialSparkQuotaVisibility()).toBe(false);
  });

  it("reports localStorage write failures instead of claiming persistence", () => {
    globalThis.localStorage.setItem = () => {
      throw new Error("blocked");
    };
    expect(persistAdditionalQuotaVisibility(false)).toBe(false);
  });
});

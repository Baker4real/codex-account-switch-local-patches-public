import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { DashboardViewModel, ProfileCard } from "./types";

class FakeElement {
  innerHTML = "";
  textContent = "";
  title = "";
  value = "";
  placeholder = "";
  hidden = false;
  disabled = false;
  dataset: Record<string, string> = {};
  classList = { toggle: () => undefined };

  addEventListener(): void {}
  querySelectorAll(): FakeElement[] { return []; }
  querySelector(): FakeElement | null { return null; }
  setAttribute(): void {}
  removeAttribute(): void {}
}

const elementsById = new Map<string, FakeElement>();
let renderProfiles: typeof import("./render").renderProfiles;
let state: typeof import("./state").state;

function profile(lastAuthorizationMethod: "device_code" | "browser" | null): ProfileCard {
  return {
    folder_name: "a",
    display_title: "a · user@example.com",
    account_label: "user@example.com",
    status: "available",
    auth_present: true,
    has_account_identity: true,
    plan_name: "plus",
    subscription_days_left: null,
    openai_base_url: null,
    quota: {
      five_hour: { remaining_percent: 50, refresh_at: null, reset_at_timestamp: null },
      weekly: { remaining_percent: 75, refresh_at: null, reset_at_timestamp: null },
      additional: null,
    },
    last_authorization_method: lastAuthorizationMethod,
    last_plan_check_ms: Date.now(),
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
  } as ProfileCard;
}

function dashboard(profileCard: ProfileCard): DashboardViewModel {
  return {
    paging: {
      page: 1,
      page_size: 8,
      total_profiles: 1,
      total_pages: 1,
      has_previous: false,
      has_next: false,
    },
    profiles: [profileCard],
    current_card: null,
    current_quota_card: null,
    profile_counts: { visible: 1, total: 1 },
  };
}

beforeAll(async () => {
  vi.stubGlobal("HTMLElement", FakeElement);
  vi.stubGlobal("HTMLDialogElement", class extends FakeElement {});
  vi.stubGlobal("document", {
    title: "",
    documentElement: new FakeElement(),
    getElementById(id: string) {
      const existing = elementsById.get(id);
      if (existing) {
        return existing;
      }
      const element = new FakeElement();
      elementsById.set(id, element);
      return element;
    },
    querySelectorAll() {
      return [];
    },
  });
  vi.stubGlobal("window", { location: { hash: "" } });

  ({ renderProfiles } = await import("./render"));
  ({ state } = await import("./state"));
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("profile card authorization method", () => {
  it("设备码授权成功后显示持久化的授权方式徽标", () => {
    state.locale = "zh-CN";

    renderProfiles(
      dashboard(profile("device_code")),
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
    );

    const markup = elementsById.get("profiles-grid")?.innerHTML ?? "";
    expect(markup).toContain('class="profile-authorization-method"');
    expect(markup).toContain("设备码授权");
  });

  it("浏览器授权显示不同的持久化方式徽标", () => {
    state.locale = "zh-CN";

    renderProfiles(
      dashboard(profile("browser")),
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
    );

    const markup = elementsById.get("profiles-grid")?.innerHTML ?? "";
    expect(markup).toContain("浏览器授权");
    expect(markup).not.toContain("设备码授权");
  });

  it("旧账号没有可靠授权历史时不猜测方式", () => {
    state.locale = "zh-CN";

    renderProfiles(
      dashboard(profile(null)),
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
      () => undefined,
    );

    const markup = elementsById.get("profiles-grid")?.innerHTML ?? "";
    expect(markup).not.toContain('class="profile-authorization-method"');
    expect(markup).not.toContain("浏览器授权");
    expect(markup).not.toContain("设备码授权");
  });
});

import { describe, expect, it } from "vitest";

import type {
  OpenAiUsageAccountSummary,
  OpenAiUsageCostBreakdown,
  OpenAiUsageSummary,
} from "../types";
import type { UsageElements } from "./dom";
import type { UsageWorkspaceView } from "./model";
import { renderUsage } from "./render";

const zeroTokens = {
  input_tokens: 0,
  cached_input_tokens: 0,
  output_tokens: 0,
  reasoning_output_tokens: 0,
  real_total_tokens: 0,
};

const zeroCost: OpenAiUsageCostBreakdown = {
  input_cost_micros: 0,
  cached_input_cost_micros: 0,
  output_cost_micros: 0,
  total_cost_micros: 0,
  unpriced_tokens: 0,
};

class FakeClassList {
  readonly values = new Set<string>();

  toggle(name: string, force?: boolean): boolean {
    const enabled = force ?? !this.values.has(name);
    if (enabled) this.values.add(name);
    else this.values.delete(name);
    return enabled;
  }

  contains(name: string): boolean {
    return this.values.has(name);
  }
}

class FakeElement {
  innerHTML = "";
  textContent = "";
  value = "";
  title = "";
  hidden = false;
  disabled = false;
  readonly classList = new FakeClassList();
  readonly attributes = new Map<string, string>();

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }

  removeAttribute(name: string): void {
    this.attributes.delete(name);
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }
}

function fakeElements(): {
  elements: UsageElements;
  refresh: FakeElement;
  account: FakeElement;
  detail: FakeElement;
} {
  const refresh = new FakeElement();
  const account = new FakeElement();
  const detail = new FakeElement();
  const element = () => new FakeElement();
  return {
    refresh,
    account,
    detail,
    elements: {
      refreshButton: refresh as unknown as HTMLButtonElement,
      rangeButton: element() as unknown as HTMLButtonElement,
      rangeLabel: element() as unknown as HTMLSpanElement,
      rangePopover: element() as unknown as HTMLDivElement,
      scopeSelect: element() as unknown as HTMLSelectElement,
      accountFilter: element() as unknown as HTMLSelectElement,
      modelFilter: element() as unknown as HTMLSelectElement,
      refreshIntervalSelect: element() as unknown as HTMLSelectElement,
      accountPanel: account as unknown as HTMLDivElement,
      summaryPanel: new FakeElement() as unknown as HTMLDivElement,
      trendPanel: new FakeElement() as unknown as HTMLDivElement,
      detailPanel: detail as unknown as HTMLDivElement,
    },
  };
}

function usageSummary(account: OpenAiUsageAccountSummary): OpenAiUsageSummary {
  return {
    generated_at_ms: 0,
    attribution_since_ms: null,
    range_start_ms: null,
    range_end_ms: null,
    total_requests: account.request_count,
    tokens: account.tokens,
    cost: account.cost,
    cache_hit_rate: null,
    accounts: [account],
    models: account.models,
    exclusions: {
      duplicate_events: 0,
      non_official_base_url: 0,
      unproven_history: 0,
      unattributed: 0,
      missing_model: 0,
      invalid_delta: 0,
      malformed_lines: 0,
      accepted_events: account.request_count,
      candidate_events: 0,
      rejected_events: 0,
      superseded_events: 0,
      strict_proven_events: 1,
      legacy_profile_attributed_events: 1,
      candidate_unattributed_events: 0,
      rejected_invalid_events: 0,
    },
    scope: "combined_attributed",
    layer_summaries: account.attribution_breakdown,
    pricing_source: "OpenAI API pricing",
    pricing_source_url: "https://example.test/pricing",
    pricing_checked_at: "2026-07-10",
  };
}

function account(cost: OpenAiUsageCostBreakdown = zeroCost): OpenAiUsageAccountSummary {
  return {
    profile: "a",
    profiles: ["a", "b"],
    account_group_key: "acct:canonical",
    account_label: "same@example.com",
    official_source: "OpenAI default",
    ledger_layer: "mixed_attributed",
    attribution_kind: "mixed_account",
    request_count: 2,
    tokens: {
      input_tokens: 90,
      cached_input_tokens: 10,
      output_tokens: 30,
      reasoning_output_tokens: 5,
      real_total_tokens: 120,
    },
    cost,
    models: [],
    attribution_breakdown: [
      {
        ledger_layer: "strict_proven",
        request_count: 1,
        tokens: { ...zeroTokens, input_tokens: 50, output_tokens: 10, real_total_tokens: 60 },
        cost: { ...zeroCost },
      },
      {
        ledger_layer: "legacy_profile_attributed",
        request_count: 1,
        tokens: { ...zeroTokens, input_tokens: 40, output_tokens: 20, real_total_tokens: 60 },
        cost: { ...zeroCost },
      },
      {
        ledger_layer: "strict_proven",
        request_count: 0,
        tokens: { ...zeroTokens },
        cost: { ...zeroCost },
      },
    ],
  };
}

function viewFor(accountSummary: OpenAiUsageAccountSummary): UsageWorkspaceView {
  return {
    summary: usageSummary(accountSummary),
    accountSummary: [accountSummary],
    modelOptions: [],
    trends: [],
    logs: { data: [], total: 0, page: 1, page_size: 20 },
    dataSources: [],
    rangePreset: "1d",
    scope: "combined_attributed",
    customStartMs: null,
    customEndMs: null,
    liveEnd: true,
    rangePickerOpen: false,
    rangeDraftStartMs: null,
    rangeDraftEndMs: null,
    rangeDraftLiveEnd: true,
    rangeDraftActiveField: "start",
    rangeDisplayMonthMs: null,
    rangeError: null,
    profileFilter: null,
    modelFilter: null,
    statusCodeFilter: null,
    logsPage: 1,
    logsPageSize: 20,
    refreshIntervalMs: 0,
    lastResolvedEndMs: 0,
    locale: "zh-CN",
    loaded: true,
    loading: false,
    logsLoading: false,
    refreshInFlight: false,
    loadError: null,
    refreshQueued: false,
  };
}

describe("renderUsage account presentation", () => {
  it("折叠账号标签中的换行和连续空白", () => {
    const value = account();
    value.account_label = "  same@example.com\r\n  extra  ";
    const { elements, account: accountPanel } = fakeElements();

    renderUsage(elements, viewFor(value));

    expect(accountPanel.innerHTML).toContain("same@example.com extra");
    const accountCell = accountPanel.innerHTML.match(/usage-table-cell--account">([^<]*)<\/td>/)?.[1];
    expect(accountCell).toBe("same@example.com extra");
  });

  it("按 canonical 账号渲染一行及严格/推定拆分，并移除来源槽位", () => {
    const value = account();
    const { elements, account: accountPanel } = fakeElements();
    renderUsage(elements, viewFor(value));

    expect(accountPanel.innerHTML.match(/<tbody>[\s\S]*?<tr/g)).toHaveLength(1);
    expect(accountPanel.innerHTML).not.toContain("来源槽位");
    expect(accountPanel.innerHTML).toContain("归属拆分");
    expect(accountPanel.innerHTML).toContain("严格");
    expect(accountPanel.innerHTML).toContain("推定");
    expect(accountPanel.innerHTML).toContain("120");
    expect(accountPanel.innerHTML).toMatch(/严格<\/span>\s*<small>1 请求数 · 60 TOKENS<\/small>/);
    expect(accountPanel.innerHTML).toMatch(/推定<\/span>\s*<small>1 请求数 · 60 TOKENS<\/small>/);
    expect(accountPanel.innerHTML.match(/class="usage-attribution-badge /g)).toHaveLength(2);
    expect(value.attribution_breakdown
      .reduce((total, layer) => total + layer.tokens.real_total_tokens, 0))
      .toBe(value.tokens.real_total_tokens);
  });

  it("单独保留未知归属层且不误标为严格或推定", () => {
    const value = account();
    value.request_count = 3;
    value.tokens = {
      ...value.tokens,
      input_tokens: 100,
      real_total_tokens: 130,
    };
    value.attribution_breakdown = [
      ...value.attribution_breakdown.slice(0, 2),
      {
        ledger_layer: "future_layer<unsafe>",
        request_count: 1,
        tokens: { ...zeroTokens, input_tokens: 10, real_total_tokens: 10 },
        cost: { ...zeroCost },
      },
    ];
    const { elements, account: accountPanel } = fakeElements();
    renderUsage(elements, viewFor(value));

    expect(accountPanel.innerHTML).toContain("future_layer&lt;unsafe&gt;");
    expect(accountPanel.innerHTML.match(/usage-attribution-badge--strict/g)).toHaveLength(1);
    expect(accountPanel.innerHTML.match(/usage-attribution-badge--legacy/g)).toHaveLength(1);
    expect(accountPanel.innerHTML.match(/usage-attribution-badge--other/g)).toHaveLength(1);
  });

  it("全未定价展示 token、转义后的模型和不近似说明", () => {
    const value = account({ ...zeroCost, unpriced_tokens: 12_345 });
    value.models = [{
      model: "gpt-x-unknown<unsafe>",
      requested_model: null,
      pricing_model: null,
      model_reasoning_effort: null,
      model_verbosity: null,
      pricing_tier: null,
      pricing_source_url: null,
      request_count: 2,
      tokens: value.tokens,
      cost: value.cost,
      priced: false,
      pricing_note: null,
    }];
    const { elements, account: accountPanel } = fakeElements();
    renderUsage(elements, viewFor(value));

    expect(accountPanel.innerHTML).toContain("不可定价");
    expect(accountPanel.innerHTML).toContain("12,345");
    expect(accountPanel.innerHTML).toContain("gpt-x-unknown&lt;unsafe&gt;");
    expect(accountPanel.innerHTML).toContain("未按近似模型估算");
    expect(accountPanel.innerHTML).not.toContain("N/A");
    expect(accountPanel.innerHTML).not.toContain("$0.0000");
  });

  it("部分定价展示已知金额和状态而不把未知部分归零", () => {
    const value = account({ ...zeroCost, total_cost_micros: 1_000_000, unpriced_tokens: 40 });
    const { elements, account: accountPanel } = fakeElements();
    renderUsage(elements, viewFor(value));

    expect(accountPanel.innerHTML).toContain("1.0000");
    expect(accountPanel.innerHTML).toContain("部分定价");
    expect(accountPanel.innerHTML).not.toContain("N/A");
    expect(accountPanel.innerHTML).not.toContain("$0.0000");
  });

  it("请求日志的未定价原因使用并转义该行模型名", () => {
    const value = account();
    const view = viewFor(value);
    view.logs!.data = [{
      request_id: "req-1",
      session_id: "session-1",
      profile: "a",
      account_label: "same@example.com",
      official_source: "OpenAI default",
      ledger_layer: "strict_proven",
      attribution_kind: "strict_account",
      model: "gpt-log<unsafe>",
      requested_model: null,
      pricing_model: null,
      model_reasoning_effort: null,
      model_verbosity: null,
      pricing_tier: null,
      pricing_source_url: null,
      tokens: { ...zeroTokens, input_tokens: 10, real_total_tokens: 10 },
      cost: { ...zeroCost, unpriced_tokens: 10 },
      priced: false,
      pricing_note: null,
      status_code: 200,
      created_at_ms: 0,
      data_source: "response_item",
      usage_confidence: "strict",
    }];
    view.logs!.total = 1;
    const { elements, detail } = fakeElements();
    renderUsage(elements, view);

    expect(detail.innerHTML).toContain("模型 gpt-log&lt;unsafe&gt;；未按近似模型估算");
    expect(detail.innerHTML).not.toContain("模型 未知；未按近似模型估算");
    expect(detail.innerHTML).not.toContain("gpt-log<unsafe>");
  });
});

describe("renderUsage refresh states", () => {
  it.each([
    ["idle", false, false, "刷新", "false", false],
    ["busy", true, false, "刷新中", "true", true],
    ["queued", true, true, "已排队", "true", true],
  ] as const)("渲染 %s 的独立 label 与三点", (_state, inFlight, queued, label, ariaBusy, disabled) => {
    const { elements, refresh } = fakeElements();
    renderUsage(elements, {
      ...viewFor(account()),
      refreshInFlight: inFlight,
      refreshQueued: queued,
    });

    expect(refresh.innerHTML).toContain(`usage-refresh-label">${label}</span>`);
    expect(refresh.innerHTML.match(/usage-refresh-dot(?:"|\s)/g)).toHaveLength(3);
    expect(refresh.innerHTML).toContain('usage-refresh-dots" aria-hidden="true"');
    expect(refresh.getAttribute("aria-busy")).toBe(ariaBusy);
    expect(refresh.disabled).toBe(disabled);
  });

  it("在同一按钮上从 busy、仅 queued 恢复 idle 并清除全部忙碌语义", () => {
    const { elements, refresh } = fakeElements();
    const base = viewFor(account());

    renderUsage(elements, { ...base, refreshInFlight: true, refreshQueued: false });
    expect(refresh.classList.contains("usage-refresh-button--busy")).toBe(true);
    expect(refresh.getAttribute("aria-busy")).toBe("true");
    expect(refresh.getAttribute("aria-disabled")).toBe("true");
    expect(refresh.disabled).toBe(true);

    renderUsage(elements, { ...base, refreshInFlight: false, refreshQueued: true });
    expect(refresh.innerHTML).toContain("已排队");
    expect(refresh.classList.contains("usage-refresh-button--busy")).toBe(true);
    expect(refresh.getAttribute("aria-busy")).toBe("true");
    expect(refresh.getAttribute("aria-disabled")).toBe("true");
    expect(refresh.disabled).toBe(true);

    renderUsage(elements, { ...base, refreshInFlight: false, refreshQueued: false });
    expect(refresh.classList.contains("usage-refresh-button--busy")).toBe(false);
    expect(refresh.getAttribute("aria-busy")).toBe("false");
    expect(refresh.getAttribute("aria-disabled")).toBeNull();
    expect(refresh.disabled).toBe(false);
  });

  it("reduced-motion 以不低于 busy selector 的优先级关闭三点动画", async () => {
    // @ts-expect-error 测试运行在 Node；产品 tsconfig 刻意不引入 Node 类型。
    const { readFileSync } = await import("node:fs");
    const baseCss = readFileSync(new URL("../base.css", import.meta.url), "utf8") as string;
    expect(baseCss).toMatch(
      /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{\s*\.usage-refresh-button--busy\s+\.usage-refresh-dot\s*\{[^}]*animation:\s*none[^}]*opacity:\s*0\.72[^}]*transform:\s*none/,
    );
  });
});

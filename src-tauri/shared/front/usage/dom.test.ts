import { describe, expect, it, vi } from "vitest";

import type { OpenAiUsageDashboard, OpenAiUsageQuery } from "../types";
import { mountUsageWorkspace } from "./dom";

type UsageDomPort = NonNullable<Parameters<typeof mountUsageWorkspace>[1]>;
type UsageDomHandlers = Parameters<UsageDomPort["bind"]>[0];

const zeroTokens = {
  input_tokens: 0,
  cached_input_tokens: 0,
  output_tokens: 0,
  reasoning_output_tokens: 0,
  real_total_tokens: 0,
};

const zeroCost = {
  input_cost_micros: 0,
  cached_input_cost_micros: 0,
  output_cost_micros: 0,
  total_cost_micros: 0,
  unpriced_tokens: 0,
};

function emptyDashboard(): OpenAiUsageDashboard {
  return {
    summary: {
      generated_at_ms: 0,
      attribution_since_ms: null,
      range_start_ms: null,
      range_end_ms: null,
      total_requests: 0,
      tokens: { ...zeroTokens },
      cost: { ...zeroCost },
      cache_hit_rate: null,
      accounts: [],
      models: [],
      exclusions: {
        duplicate_events: 0,
        non_official_base_url: 0,
        unproven_history: 0,
        unattributed: 0,
        missing_model: 0,
        invalid_delta: 0,
        malformed_lines: 0,
        accepted_events: 0,
        candidate_events: 0,
        rejected_events: 0,
        superseded_events: 0,
        strict_proven_events: 0,
        legacy_profile_attributed_events: 0,
        candidate_unattributed_events: 0,
        rejected_invalid_events: 0,
      },
      scope: "strict",
      layer_summaries: [],
      pricing_source: "OpenAI API pricing",
      pricing_source_url: "https://developers.openai.com/api/docs/pricing",
      pricing_checked_at: "2026-07-08",
    },
    account_summary: [],
    model_options: [],
    trends: [],
    logs: { data: [], total: 0, page: 1, page_size: 20 },
    data_sources: [],
  };
}

class FakeUsageDom implements UsageDomPort {
  handlers: UsageDomHandlers | null = null;
  readonly render = vi.fn();
  bindCount = 0;
  unbindCount = 0;
  clearIntervalCount = 0;
  private nextTimerId = 1;
  private readonly timers = new Map<number, () => void>();

  bind(handlers: UsageDomHandlers): () => void {
    this.bindCount += 1;
    this.handlers = handlers;
    let bound = true;
    return () => {
      if (!bound) return;
      bound = false;
      this.unbindCount += 1;
      this.handlers = null;
    };
  }

  now(): number {
    return new Date(2026, 6, 10, 9, 0, 0).getTime();
  }

  setInterval(handler: () => void): number {
    const id = this.nextTimerId++;
    this.timers.set(id, handler);
    return id;
  }

  clearInterval(id: unknown): void {
    this.clearIntervalCount += 1;
    this.timers.delete(Number(id));
  }

  activeTimerCount(): number {
    return this.timers.size;
  }
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function mount(dom: FakeUsageDom, options: {
  loadDashboard?: (query: OpenAiUsageQuery, page: number, pageSize: number) => Promise<OpenAiUsageDashboard>;
  active?: boolean;
  locale?: "zh-CN" | "en";
} = {}) {
  return mountUsageWorkspace({
    loadDashboard: options.loadDashboard ?? (async () => emptyDashboard()),
    getContext: () => ({
      active: options.active ?? true,
      locale: options.locale ?? "zh-CN",
    }),
    onError: vi.fn(),
  }, dom);
}

describe("mountUsageWorkspace", () => {
  it("没有 Usage DOM 时仍可挂载并应用当前上下文", async () => {
    const dom = new FakeUsageDom();
    const workspace = mount(dom);

    expect(workspace.snapshot().locale).toBe("zh-CN");
    expect(dom.render).toHaveBeenCalled();
  });

  it("部分 optional controls 存在时只处理实际到达的事件", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async (_query: OpenAiUsageQuery) => emptyDashboard());
    mount(dom, { loadDashboard });

    dom.handlers!.accountChange("team-a");
    await settle();

    expect(loadDashboard).toHaveBeenCalledOnce();
    expect(loadDashboard.mock.calls[0][0].account_group_key).toBe("team-a");
  });

  it("把 range picker 的 click/change/focus 意图交给 workspace", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const workspace = mount(dom, { loadDashboard });
    const handlers = dom.handlers!;

    handlers.toggleRange();
    expect(workspace.snapshot().rangePickerOpen).toBe(true);

    handlers.rangeAction({ type: "field", value: "start" });
    handlers.rangeChange({ type: "date", field: "start", value: "2026-07-01" });
    handlers.rangeChange({ type: "time", field: "start", value: "08:30" });
    handlers.rangeChange({ type: "live", value: false });
    handlers.rangeFocus("end");
    expect(workspace.snapshot().rangeDraftActiveField).toBe("end");
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getDate()).toBe(1);
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getHours()).toBe(8);
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getMinutes()).toBe(30);

    const selectedDay = new Date(2026, 6, 8).getTime();
    handlers.rangeAction({ type: "day", value: String(selectedDay) });
    expect(new Date(workspace.snapshot().rangeDraftEndMs!).getDate()).toBe(8);
    const monthBefore = workspace.snapshot().rangeDisplayMonthMs;
    handlers.rangeAction({ type: "month", value: "-1" });
    expect(workspace.snapshot().rangeDisplayMonthMs).not.toBe(monthBefore);
    handlers.rangeAction({ type: "today" });
    expect(workspace.snapshot().rangeDisplayMonthMs).not.toBeNull();

    handlers.rangeAction({ type: "apply" });
    await settle();
    expect(workspace.snapshot().rangePreset).toBe("custom");
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
    expect(loadDashboard).toHaveBeenCalledOnce();

    handlers.toggleRange();
    handlers.rangeAction({ type: "cancel" });
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("range preset 点击使用 allowlist 并刷新", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const workspace = mount(dom, { loadDashboard });

    dom.handlers!.rangeAction({ type: "preset", value: "7d" });
    await settle();

    expect(workspace.snapshot().rangePreset).toBe("7d");
    expect(loadDashboard).toHaveBeenCalledOnce();
  });

  it("分页只接受可用且不同于当前页的正数", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async (_query: OpenAiUsageQuery, page: number) => {
      const dashboard = emptyDashboard();
      dashboard.logs = { data: [], total: 40, page, page_size: 20 };
      return dashboard;
    });
    const workspace = mount(dom, { loadDashboard });
    const handlers = dom.handlers!;

    handlers.logPage("2", false);
    await settle();
    expect(workspace.snapshot().logsPage).toBe(2);
    expect(loadDashboard).toHaveBeenCalledOnce();

    handlers.logPage("3", true);
    handlers.logPage("not-a-page", false);
    handlers.logPage("-1", false);
    handlers.logPage("2", false);
    await settle();
    expect(loadDashboard).toHaveBeenCalledOnce();
  });

  it("document outside click 与 Escape 关闭已打开的 picker", () => {
    const dom = new FakeUsageDom();
    const workspace = mount(dom);
    const handlers = dom.handlers!;

    handlers.toggleRange();
    handlers.documentClick(false);
    expect(workspace.snapshot().rangePickerOpen).toBe(true);
    handlers.documentClick(true);
    expect(workspace.snapshot().rangePickerOpen).toBe(false);

    handlers.toggleRange();
    handlers.documentKeydown("Enter");
    expect(workspace.snapshot().rangePickerOpen).toBe(true);
    handlers.documentKeydown("Escape");
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("dispose 后旧 handler 不再触发刷新或渲染", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const workspace = mount(dom, { loadDashboard });
    const staleHandlers = dom.handlers!;
    const rendersBeforeDispose = dom.render.mock.calls.length;

    workspace.dispose();
    staleHandlers.refresh();
    staleHandlers.toggleRange();
    await settle();

    expect(loadDashboard).not.toHaveBeenCalled();
    expect(dom.render).toHaveBeenCalledTimes(rendersBeforeDispose);
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("重复 dispose 只解绑一次并只清理一次 timer", async () => {
    const dom = new FakeUsageDom();
    const workspace = mount(dom);
    await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
    expect(dom.activeTimerCount()).toBe(1);
    const clearsBeforeDispose = dom.clearIntervalCount;

    workspace.dispose();
    workspace.dispose();

    expect(dom.unbindCount).toBe(1);
    expect(dom.clearIntervalCount).toBe(clearsBeforeDispose + 1);
    expect(dom.activeTimerCount()).toBe(0);
  });

  it("快速 locale/route 更新以后只保留最终上下文", async () => {
    const dom = new FakeUsageDom();
    const workspace = mount(dom, { active: false, locale: "en" });

    await Promise.all([
      workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" }),
      workspace.dispatch({ type: "set-context", active: false, locale: "en" }),
      workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" }),
    ]);

    expect(workspace.snapshot().locale).toBe("zh-CN");
    dom.handlers!.toggleRange();
    await workspace.dispatch({ type: "set-context", active: false, locale: "en" });
    expect(workspace.snapshot().locale).toBe("en");
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("非法 range preset 与 scope 不 dispatch 也不刷新", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const workspace = mount(dom, { loadDashboard });
    const rendersBefore = dom.render.mock.calls.length;

    dom.handlers!.rangeAction({ type: "preset", value: "90d" });
    dom.handlers!.scopeChange("all-data");
    await settle();

    expect(workspace.snapshot().rangePreset).toBe("1d");
    expect(workspace.snapshot().scope).toBe("strict");
    expect(loadDashboard).not.toHaveBeenCalled();
    expect(dom.render).toHaveBeenCalledTimes(rendersBefore);
  });

  it("合法 scope 通过 allowlist 后刷新", async () => {
    const dom = new FakeUsageDom();
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const workspace = mount(dom, { loadDashboard });

    dom.handlers!.scopeChange("candidate");
    await settle();

    expect(workspace.snapshot().scope).toBe("candidate");
    expect(loadDashboard).toHaveBeenCalledOnce();
  });
});

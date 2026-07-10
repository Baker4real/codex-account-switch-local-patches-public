import { describe, expect, it, vi } from "vitest";

import type { OpenAiUsageDashboard, OpenAiUsageQuery } from "../types";
import {
  browserUsageDomPort,
  mountUsageWorkspace,
  type UsageDocumentLike,
  type UsageTimerHost,
} from "./dom";

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

type FakeListener = EventListener;

class FakeEventTarget {
  private readonly listeners = new Map<string, Set<FakeListener>>();

  addEventListener(type: string, listener: FakeListener): void {
    const listeners = this.listeners.get(type) ?? new Set<FakeListener>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: FakeListener): void {
    this.listeners.get(type)?.delete(listener);
  }

  emit(type: string, event: Record<string, unknown> = {}): void {
    for (const listener of [...(this.listeners.get(type) ?? [])]) {
      listener({ type, ...event } as unknown as Event);
    }
  }

  listenerCount(type: string): number {
    return this.listeners.get(type)?.size ?? 0;
  }
}

class FakeElement extends FakeEventTarget {
  readonly nodeType = 1;
  readonly dataset: Record<string, string | undefined> = {};
  parent: FakeElement | null = null;
  value = "";
  checked = false;
  disabled = false;
  hidden = false;
  innerHTML = "";
  textContent: string | null = null;
  title = "";
  readonly classList = { toggle: vi.fn() };

  constructor(readonly tagName = "DIV") {
    super();
  }

  appendTo(parent: FakeElement): this {
    this.parent = parent;
    return this;
  }

  closest(selector: string): FakeElement | null {
    for (let current: FakeElement | null = this; current; current = current.parent) {
      if (current.matches(selector)) return current;
    }
    return null;
  }

  matches(selector: string): boolean {
    const match = /^\[data-([a-z0-9-]+)\]$/.exec(selector);
    if (!match) return false;
    const key = match[1].replace(/-([a-z])/g, (_part, letter: string) => letter.toUpperCase());
    return this.dataset[key] !== undefined;
  }

  contains(node: unknown): boolean {
    if (!(node instanceof FakeElement)) return false;
    for (let current: FakeElement | null = node; current; current = current.parent) {
      if (current === this) return true;
    }
    return false;
  }

  setAttribute(): void {}
  removeAttribute(): void {}
}

class FakeDocument extends FakeEventTarget implements UsageDocumentLike {
  private readonly elements = new Map<string, unknown>();

  set(id: string, element: unknown): void {
    this.elements.set(id, element);
  }

  getElementById(id: string): unknown {
    return this.elements.get(id) ?? null;
  }
}

class FakeTimers implements UsageTimerHost {
  clearCount = 0;
  private nextId = 1;
  private readonly handlers = new Map<number, () => void>();

  setInterval(handler: () => void): number {
    const id = this.nextId++;
    this.handlers.set(id, handler);
    return id;
  }

  clearInterval(id: unknown): void {
    this.clearCount += 1;
    this.handlers.delete(Number(id));
  }

  activeCount(): number {
    return this.handlers.size;
  }
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function setup(options: {
  document?: FakeDocument;
  timers?: FakeTimers;
  loadDashboard?: (query: OpenAiUsageQuery, page: number, pageSize: number) => Promise<OpenAiUsageDashboard>;
  active?: boolean;
  locale?: "zh-CN" | "en";
} = {}) {
  const document = options.document ?? new FakeDocument();
  const timers = options.timers ?? new FakeTimers();
  const loadDashboard = options.loadDashboard ?? vi.fn(async () => emptyDashboard());
  const workspace = mountUsageWorkspace({
    loadDashboard,
    getContext: () => ({
      active: options.active ?? true,
      locale: options.locale ?? "zh-CN",
    }),
    onError: vi.fn(),
  }, browserUsageDomPort(document, timers));
  return { document, timers, loadDashboard, workspace };
}

function element(document: FakeDocument, id: string, tagName = "DIV"): FakeElement {
  const value = new FakeElement(tagName);
  document.set(id, value);
  return value;
}

function targetWithDataset(
  dataset: Record<string, string>,
  tagName = "BUTTON",
  childTagName = "SPAN",
): { target: FakeElement; child: FakeElement } {
  const target = new FakeElement(tagName);
  Object.assign(target.dataset, dataset);
  return { target, child: new FakeElement(childTagName).appendTo(target) };
}

describe("browserUsageDomPort", () => {
  it("没有 Usage DOM 时仍可挂载并应用当前上下文", () => {
    const { document, workspace } = setup({ locale: "en" });

    expect(workspace.snapshot().locale).toBe("en");
    expect(document.listenerCount("click")).toBe(1);
    expect(document.listenerCount("keydown")).toBe(1);
  });

  it("按 ID 查询时忽略结构无效元素，并只绑定存在的 optional control", async () => {
    const document = new FakeDocument();
    document.set("usage-refresh-button", { addEventListener: vi.fn() });
    const account = element(document, "usage-account-filter", "SELECT");
    const loadDashboard = vi.fn(async (_query: OpenAiUsageQuery) => emptyDashboard());
    setup({ document, loadDashboard });

    account.value = "team-a";
    account.emit("change", { target: account });
    await settle();

    expect(loadDashboard).toHaveBeenCalledOnce();
    expect(loadDashboard.mock.calls[0][0].account_group_key).toBe("team-a");
  });

  it("通过真实 listener 与 target.closest/dataset 传递 picker click 意图", async () => {
    const document = new FakeDocument();
    const rangeButton = element(document, "usage-range-button", "BUTTON");
    const popover = element(document, "usage-range-popover");
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const { workspace } = setup({ document, loadDashboard });

    rangeButton.emit("click", { target: rangeButton });
    expect(workspace.snapshot().rangePickerOpen).toBe(true);

    const preset = targetWithDataset({ usageRangePreset: "7d" });
    popover.emit("click", { target: preset.child });
    await settle();
    expect(workspace.snapshot().rangePreset).toBe("7d");
    expect(loadDashboard).toHaveBeenCalledOnce();

    rangeButton.emit("click", { target: rangeButton });
    const field = targetWithDataset({ usageRangeField: "start" }, "DIV");
    popover.emit("click", { target: field.child });
    expect(workspace.snapshot().rangeDraftActiveField).toBe("start");

    const monthBefore = workspace.snapshot().rangeDisplayMonthMs;
    const month = targetWithDataset({ usageRangeMonth: "-1" });
    popover.emit("click", { target: month.child });
    expect(workspace.snapshot().rangeDisplayMonthMs).not.toBe(monthBefore);

    const today = targetWithDataset({ usageRangeToday: "" });
    popover.emit("click", { target: today.child });
    expect(workspace.snapshot().rangeDisplayMonthMs).not.toBeNull();
  });

  it("通过 input change/focusin 路径处理日期、时间和 live end", () => {
    const document = new FakeDocument();
    const rangeButton = element(document, "usage-range-button", "BUTTON");
    const popover = element(document, "usage-range-popover");
    const { workspace } = setup({ document });
    rangeButton.emit("click", { target: rangeButton });

    const date = new FakeElement("INPUT");
    date.dataset.usageRangeDate = "start";
    date.value = "2026-07-01";
    popover.emit("change", { target: date });
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getDate()).toBe(1);

    const time = new FakeElement("INPUT");
    time.dataset.usageRangeTime = "start";
    time.value = "08:30";
    popover.emit("change", { target: time });
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getHours()).toBe(8);
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getMinutes()).toBe(30);

    time.dataset.usageRangeTime = "end";
    popover.emit("focusin", { target: time });
    expect(workspace.snapshot().rangeDraftActiveField).toBe("end");

    const live = new FakeElement("INPUT");
    live.dataset.usageRangeLive = "";
    live.checked = true;
    popover.emit("change", { target: live });
    expect(workspace.snapshot().rangeDraftLiveEnd).toBe(true);
  });

  it("picker 的 day/apply/cancel 点击都经过生产 listener", async () => {
    const document = new FakeDocument();
    const rangeButton = element(document, "usage-range-button", "BUTTON");
    const popover = element(document, "usage-range-popover");
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const { workspace } = setup({ document, loadDashboard });
    rangeButton.emit("click", { target: rangeButton });

    const live = new FakeElement("INPUT");
    live.dataset.usageRangeLive = "";
    live.checked = false;
    popover.emit("change", { target: live });
    const end = new FakeElement("INPUT");
    end.dataset.usageRangeDate = "end";
    popover.emit("focusin", { target: end });
    const selectedDay = new Date(2026, 6, 8).getTime();
    const day = targetWithDataset({ usageRangeDay: String(selectedDay) });
    popover.emit("click", { target: day.child });
    expect(new Date(workspace.snapshot().rangeDraftStartMs!).getDate()).toBe(8);

    const apply = targetWithDataset({ usageRangeApply: "" });
    popover.emit("click", { target: apply.child });
    await settle();
    expect(workspace.snapshot().rangePreset).toBe("custom");
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
    expect(loadDashboard).toHaveBeenCalledOnce();

    rangeButton.emit("click", { target: rangeButton });
    const cancel = targetWithDataset({ usageRangeCancel: "" });
    popover.emit("click", { target: cancel.child });
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("分页只接受未禁用、不同于当前页的有效正整数", async () => {
    const document = new FakeDocument();
    const detail = element(document, "usage-detail-panel");
    const loadDashboard = vi.fn(async (_query: OpenAiUsageQuery, page: number) => {
      const dashboard = emptyDashboard();
      dashboard.logs = { data: [], total: 40, page, page_size: 20 };
      return dashboard;
    });
    const { workspace } = setup({ document, loadDashboard });

    const page = targetWithDataset({ usageLogPage: "2" });
    detail.emit("click", { target: page.child });
    await settle();
    expect(workspace.snapshot().logsPage).toBe(2);
    expect(loadDashboard).toHaveBeenCalledOnce();

    for (const invalid of ["2.5", "0", "-1", "9007199254740992", "not-a-page", "2"]) {
      page.target.dataset.usageLogPage = invalid;
      detail.emit("click", { target: page.child });
    }
    page.target.dataset.usageLogPage = "3";
    page.target.disabled = true;
    detail.emit("click", { target: page.child });
    await settle();
    expect(loadDashboard).toHaveBeenCalledOnce();
  });

  it("document outside click 与 Escape 关闭 picker，inside click 和其他按键不关闭", () => {
    const document = new FakeDocument();
    const rangeButton = element(document, "usage-range-button", "BUTTON");
    const picker = element(document, "usage-range-picker");
    const inside = new FakeElement().appendTo(picker);
    const outside = new FakeElement();
    const { workspace } = setup({ document });

    rangeButton.emit("click", { target: rangeButton });
    document.emit("click", { target: inside });
    expect(workspace.snapshot().rangePickerOpen).toBe(true);
    document.emit("keydown", { target: inside, key: "Enter" });
    expect(workspace.snapshot().rangePickerOpen).toBe(true);
    document.emit("click", { target: outside });
    expect(workspace.snapshot().rangePickerOpen).toBe(false);

    rangeButton.emit("click", { target: rangeButton });
    document.emit("keydown", { target: inside, key: "Escape" });
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });

  it("非法 dataset、自动归一化日期/时间以及越界数值不会改变状态", async () => {
    const document = new FakeDocument();
    const rangeButton = element(document, "usage-range-button", "BUTTON");
    const popover = element(document, "usage-range-popover");
    const scope = element(document, "usage-scope-select", "SELECT");
    const detail = element(document, "usage-detail-panel");
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const { workspace } = setup({ document, loadDashboard });
    rangeButton.emit("click", { target: rangeButton });
    const before = workspace.snapshot();

    for (const value of ["90d", "", "__proto__"]) {
      const preset = targetWithDataset({ usageRangePreset: value });
      popover.emit("click", { target: preset.child });
    }
    scope.value = "all-data";
    scope.emit("change", { target: scope });
    for (const value of ["2", "-2", "1.5", "Infinity"]) {
      const month = targetWithDataset({ usageRangeMonth: value });
      popover.emit("click", { target: month.child });
    }
    for (const value of ["1.5", "9000000000000000", "NaN", ""]) {
      const day = targetWithDataset({ usageRangeDay: value });
      popover.emit("click", { target: day.child });
    }
    const date = new FakeElement("INPUT");
    date.dataset.usageRangeDate = "start";
    for (const value of ["2026-02-31", "2026-13-01", "2026-2-03", "x-y-z"]) {
      date.value = value;
      popover.emit("change", { target: date });
    }
    const time = new FakeElement("INPUT");
    time.dataset.usageRangeTime = "start";
    for (const value of ["24:00", "23:60", "8:30", "-1:00"]) {
      time.value = value;
      popover.emit("change", { target: time });
    }
    const page = targetWithDataset({ usageLogPage: "2.5" });
    detail.emit("click", { target: page.child });
    await settle();

    expect(workspace.snapshot().rangePreset).toBe(before.rangePreset);
    expect(workspace.snapshot().scope).toBe(before.scope);
    expect(workspace.snapshot().rangeDisplayMonthMs).toBe(before.rangeDisplayMonthMs);
    expect(workspace.snapshot().rangeDraftStartMs).toBe(before.rangeDraftStartMs);
    expect(workspace.snapshot().logsPage).toBe(before.logsPage);
    expect(loadDashboard).not.toHaveBeenCalled();
  });

  it("合法 scope 和各 filter 经过实际 select change listener 后刷新", async () => {
    const document = new FakeDocument();
    const scope = element(document, "usage-scope-select", "SELECT");
    const account = element(document, "usage-account-filter", "SELECT");
    const model = element(document, "usage-model-filter", "SELECT");
    const interval = element(document, "usage-refresh-interval-select", "SELECT");
    const refresh = element(document, "usage-refresh-button", "BUTTON");
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const { timers, workspace } = setup({ document, loadDashboard });

    scope.value = "candidate";
    scope.emit("change", { target: scope });
    await settle();
    account.value = "team-a";
    account.emit("change", { target: account });
    await settle();
    model.value = "gpt-5";
    model.emit("change", { target: model });
    await settle();
    interval.value = "30000";
    interval.emit("change", { target: interval });
    refresh.emit("click", { target: refresh });
    await settle();

    expect(workspace.snapshot().scope).toBe("candidate");
    expect(workspace.snapshot().profileFilter).toBe("team-a");
    expect(workspace.snapshot().modelFilter).toBe("gpt-5");
    expect(workspace.snapshot().refreshIntervalMs).toBe(30_000);
    expect(timers.activeCount()).toBe(1);
    expect(loadDashboard).toHaveBeenCalledTimes(4);
  });

  it("dispose 移除所有 production listener，重复调用只清理一次 timer", async () => {
    const document = new FakeDocument();
    const timers = new FakeTimers();
    const refresh = element(document, "usage-refresh-button", "BUTTON");
    const range = element(document, "usage-range-button", "BUTTON");
    const popover = element(document, "usage-range-popover");
    const loadDashboard = vi.fn(async () => emptyDashboard());
    const { workspace } = setup({ document, timers, loadDashboard });
    await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
    expect(timers.activeCount()).toBe(1);
    const clearsBeforeDispose = timers.clearCount;

    workspace.dispose();
    workspace.dispose();

    expect(refresh.listenerCount("click")).toBe(0);
    expect(range.listenerCount("click")).toBe(0);
    expect(popover.listenerCount("click")).toBe(0);
    expect(popover.listenerCount("change")).toBe(0);
    expect(popover.listenerCount("focusin")).toBe(0);
    expect(document.listenerCount("click")).toBe(0);
    expect(document.listenerCount("keydown")).toBe(0);
    expect(timers.clearCount).toBe(clearsBeforeDispose + 1);
    expect(timers.activeCount()).toBe(0);

    refresh.emit("click", { target: refresh });
    range.emit("click", { target: range });
    await settle();
    expect(loadDashboard).not.toHaveBeenCalled();
    expect(workspace.snapshot().rangePickerOpen).toBe(false);
  });
});

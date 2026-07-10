import { describe, expect, it, vi } from "vitest";

import type { OpenAiUsageDashboard, OpenAiUsageQuery } from "../types";
import {
  createUsageWorkspace,
  type UsageWorkspaceDependencies,
} from "./index";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

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

function dashboardWithRequests(totalRequests: number): OpenAiUsageDashboard {
  const dashboard = emptyDashboard();
  dashboard.summary.total_requests = totalRequests;
  return dashboard;
}

function dashboardWithPage(input: {
  page: number;
  pageSize: number;
  total: number;
}): OpenAiUsageDashboard {
  const dashboard = emptyDashboard();
  dashboard.logs = {
    data: [],
    total: input.total,
    page: input.page,
    page_size: input.pageSize,
  };
  return dashboard;
}

function fakeTimers() {
  let nextId = 1;
  const active = new Map<number, () => void>();
  return {
    setInterval: (handler: () => void) => {
      const id = nextId++;
      active.set(id, handler);
      return id;
    },
    clearInterval: (id: unknown) => active.delete(Number(id)),
    activeCount: () => active.size,
  };
}

function fakeDependencies(
  overrides: Partial<UsageWorkspaceDependencies> = {},
): UsageWorkspaceDependencies {
  const timers = fakeTimers();
  return {
    loadDashboard: async () => emptyDashboard(),
    now: () => new Date(2026, 6, 10, 9, 0, 0).getTime(),
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
    onChange: vi.fn(),
    onError: vi.fn(),
    ...overrides,
  };
}

describe("UsageWorkspace", () => {
  it("把 preset 与自定义范围转换成 dashboard query", async () => {
    const calls: OpenAiUsageQuery[] = [];
    const nowMs = new Date(2026, 6, 10, 9, 0, 0).getTime();
    const workspace = createUsageWorkspace(fakeDependencies({
      now: () => nowMs,
      loadDashboard: async (query) => {
        calls.push(query);
        return emptyDashboard();
      },
    }));

    await workspace.dispatch({ type: "set-range", preset: "7d" });
    await workspace.dispatch({ type: "refresh", mode: "initial" });
    expect(calls[calls.length - 1]).toMatchObject({
      scope: "strict",
      start_date: Math.floor(new Date(2026, 6, 4, 0, 0, 0).getTime() / 1000),
      end_date: Math.floor(nowMs / 1000),
    });

    await workspace.dispatch({
      type: "set-range",
      preset: "custom",
      startMs: 1_700_000_000_000,
      endMs: 1_700_003_600_000,
      liveEnd: false,
    });
    await workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(calls[calls.length - 1]).toMatchObject({
      start_date: 1_700_000_000,
      end_date: 1_700_003_600,
    });
  });

  it("忽略晚到的旧 intent 响应", async () => {
    const first = deferred<OpenAiUsageDashboard>();
    const second = deferred<OpenAiUsageDashboard>();
    const loadDashboard = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    const older = workspace.dispatch({ type: "refresh", mode: "initial" });
    await workspace.dispatch({ type: "set-filters", model: "gpt-5.3-codex" });
    const newer = workspace.dispatch({ type: "refresh", mode: "initial" });

    second.resolve(dashboardWithRequests(2));
    await newer;
    first.resolve(dashboardWithRequests(1));
    await older;

    expect(workspace.snapshot().summary?.total_requests).toBe(2);
  });

  it("合并 in-flight 手动刷新且自动刷新不排队", async () => {
    const first = deferred<OpenAiUsageDashboard>();
    const loadDashboard = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue(emptyDashboard());
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    const running = workspace.dispatch({ type: "refresh", mode: "manual" });
    await workspace.dispatch({ type: "refresh", mode: "manual" });
    await workspace.dispatch({ type: "refresh", mode: "auto" });
    first.resolve(emptyDashboard());
    await running;

    expect(loadDashboard).toHaveBeenCalledTimes(2);
  });

  it("日志页越界时纠偏并只重载一次", async () => {
    const loadDashboard = vi.fn()
      .mockResolvedValueOnce(dashboardWithPage({ page: 9, pageSize: 20, total: 25 }))
      .mockResolvedValueOnce(dashboardWithPage({ page: 2, pageSize: 20, total: 25 }));
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    await workspace.dispatch({ type: "set-page", page: 9 });
    await workspace.dispatch({ type: "refresh", mode: "manual" });

    expect(loadDashboard.mock.calls.map((call) => call[1])).toEqual([9, 2]);
    expect(workspace.snapshot().logsPage).toBe(2);
  });

  it("dispose 后忽略完成响应并停止 timer", async () => {
    const pending = deferred<OpenAiUsageDashboard>();
    const timers = fakeTimers();
    const workspace = createUsageWorkspace(fakeDependencies({
      loadDashboard: () => pending.promise,
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
    }));
    await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
    await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
    const running = workspace.dispatch({ type: "refresh", mode: "manual" });
    workspace.dispose();
    pending.resolve(dashboardWithRequests(7));
    await running;

    expect(timers.activeCount()).toBe(0);
    expect(workspace.snapshot().summary).toBeNull();
  });
});

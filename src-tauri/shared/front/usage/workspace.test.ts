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

  it.each(["manual", "auto"] as const)("%s in-flight 时 initial 提高优先级且旧结果失效", async (mode) => {
    const olderRequest = deferred<OpenAiUsageDashboard>();
    const initialRequest = deferred<OpenAiUsageDashboard>();
    const onError = vi.fn();
    const loadDashboard = vi.fn()
      .mockReturnValueOnce(olderRequest.promise)
      .mockReturnValueOnce(initialRequest.promise);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard, onError }));
    const older = workspace.dispatch({ type: "refresh", mode });
    const initial = workspace.dispatch({ type: "refresh", mode: "initial" });

    expect(loadDashboard).toHaveBeenCalledTimes(1);
    olderRequest.reject(new Error("旧请求失败"));
    await vi.waitFor(() => expect(loadDashboard).toHaveBeenCalledTimes(2));
    initialRequest.resolve(dashboardWithRequests(2));
    await Promise.all([older, initial]);

    expect(workspace.snapshot().summary?.total_requests).toBe(2);
    expect(workspace.snapshot().loadError).toBeNull();
    expect(onError).not.toHaveBeenCalled();
  });

  it("多次 initial 合并成一个最高优先级刷新", async () => {
    const olderRequest = deferred<OpenAiUsageDashboard>();
    const initialRequest = deferred<OpenAiUsageDashboard>();
    const loadDashboard = vi.fn()
      .mockReturnValueOnce(olderRequest.promise)
      .mockReturnValueOnce(initialRequest.promise);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    const older = workspace.dispatch({ type: "refresh", mode: "manual" });
    const initials = [
      workspace.dispatch({ type: "refresh", mode: "initial" }),
      workspace.dispatch({ type: "refresh", mode: "initial" }),
      workspace.dispatch({ type: "refresh", mode: "initial" }),
    ];

    expect(loadDashboard).toHaveBeenCalledTimes(1);
    olderRequest.resolve(dashboardWithRequests(1));
    await vi.waitFor(() => expect(loadDashboard).toHaveBeenCalledTimes(2));
    initialRequest.resolve(dashboardWithRequests(2));
    await Promise.all([older, ...initials]);

    expect(loadDashboard).toHaveBeenCalledTimes(2);
    expect(workspace.snapshot().summary?.total_requests).toBe(2);
  });

  it.each([
    ["filter", { type: "set-filters", model: "gpt-5.3-codex" }],
    ["page", { type: "set-page", page: 2 }],
    ["range", { type: "set-range", preset: "7d" }],
  ] as const)("旧请求在新 %s intent 后 reject 不污染状态", async (_label, event) => {
    const request = deferred<OpenAiUsageDashboard>();
    const onError = vi.fn();
    const workspace = createUsageWorkspace(fakeDependencies({
      loadDashboard: () => request.promise,
      onError,
    }));
    const older = workspace.dispatch({ type: "refresh", mode: "manual" });
    await workspace.dispatch(event);

    request.reject(new Error("旧请求失败"));
    await older;

    expect(workspace.snapshot().loadError).toBeNull();
    expect(workspace.snapshot().logsLoading).toBe(true);
    expect(onError).not.toHaveBeenCalled();
  });

  it("route inactive 后旧 manual reject 不渲染也不通知", async () => {
    const request = deferred<OpenAiUsageDashboard>();
    const onChange = vi.fn();
    const onError = vi.fn();
    const workspace = createUsageWorkspace(fakeDependencies({
      loadDashboard: () => request.promise,
      onChange,
      onError,
    }));
    await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
    const older = workspace.dispatch({ type: "refresh", mode: "manual" });
    await workspace.dispatch({ type: "set-context", active: false, locale: "zh-CN" });
    const renderCountAfterDeactivate = onChange.mock.calls.length;

    request.reject(new Error("离开页面后的旧错误"));
    await older;

    expect(onChange).toHaveBeenCalledTimes(renderCountAfterDeactivate);
    expect(workspace.snapshot().loadError).toBeNull();
    expect(onError).not.toHaveBeenCalled();
  });

  it("intent 请求失活后清除 loader，并在重新激活时补刷当前 intent", async () => {
    const staleRequest = deferred<OpenAiUsageDashboard>();
    const currentRequest = deferred<OpenAiUsageDashboard>();
    const loadDashboard = vi.fn()
      .mockReturnValueOnce(staleRequest.promise)
      .mockReturnValueOnce(currentRequest.promise);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
    await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 0 });
    await workspace.dispatch({ type: "set-filters", model: "gpt-5.3-codex" });
    const staleRefresh = workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(workspace.snapshot().logsLoading).toBe(true);

    await workspace.dispatch({ type: "set-context", active: false, locale: "zh-CN" });
    expect(workspace.snapshot().logsLoading).toBe(false);
    staleRequest.resolve(dashboardWithRequests(1));
    await staleRefresh;

    const reactivate = workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
    await vi.waitFor(() => expect(loadDashboard).toHaveBeenCalledTimes(2));
    expect(loadDashboard.mock.calls[1][0].model).toBe("gpt-5.3-codex");
    currentRequest.resolve(dashboardWithRequests(2));
    await reactivate;

    expect(workspace.snapshot().logsLoading).toBe(false);
    expect(workspace.snapshot().summary?.total_requests).toBe(2);
  });

  it("只有 manual refresh 错误触发用户通知", async () => {
    const initialError = new Error("initial 失败");
    const autoError = new Error("auto 失败");
    const manualError = new Error("manual 失败");
    const onError = vi.fn();
    const loadDashboard = vi.fn()
      .mockRejectedValueOnce(initialError)
      .mockRejectedValueOnce(autoError)
      .mockRejectedValueOnce(manualError);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard, onError }));

    await workspace.dispatch({ type: "refresh", mode: "initial" });
    await workspace.dispatch({ type: "refresh", mode: "auto" });
    expect(onError).not.toHaveBeenCalled();

    await workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(onError).toHaveBeenCalledOnce();
    expect(onError).toHaveBeenCalledWith(manualError);
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

  it("普通 manual/auto refresh 保留已有日志且不进入 logsLoading", async () => {
    const manual = deferred<OpenAiUsageDashboard>();
    const auto = deferred<OpenAiUsageDashboard>();
    const loadDashboard = vi.fn()
      .mockResolvedValueOnce(dashboardWithPage({ page: 1, pageSize: 20, total: 1 }))
      .mockReturnValueOnce(manual.promise)
      .mockReturnValueOnce(auto.promise);
    const workspace = createUsageWorkspace(fakeDependencies({ loadDashboard }));
    await workspace.dispatch({ type: "refresh", mode: "initial" });
    const existingLogs = workspace.snapshot().logs;

    const manualRefresh = workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(workspace.snapshot().logs).toEqual(existingLogs);
    expect(workspace.snapshot().logsLoading).toBe(false);
    manual.resolve(dashboardWithPage({ page: 1, pageSize: 20, total: 2 }));
    await manualRefresh;

    const autoRefresh = workspace.dispatch({ type: "refresh", mode: "auto" });
    expect(workspace.snapshot().logs).not.toBeNull();
    expect(workspace.snapshot().logsLoading).toBe(false);
    auto.resolve(dashboardWithPage({ page: 1, pageSize: 20, total: 3 }));
    await autoRefresh;
  });

  it("page/filter intent 进入 logsLoading 并在有效 load 完成或失败后恢复", async () => {
    const workspace = createUsageWorkspace(fakeDependencies());

    await workspace.dispatch({ type: "set-page", page: 2 });
    expect(workspace.snapshot().logsLoading).toBe(true);
    await workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(workspace.snapshot().logsLoading).toBe(false);

    await workspace.dispatch({ type: "set-filters", model: "gpt-5.3-codex" });
    expect(workspace.snapshot().logsLoading).toBe(true);
    await workspace.dispatch({ type: "refresh", mode: "manual" });
    expect(workspace.snapshot().logsLoading).toBe(false);

    const failingWorkspace = createUsageWorkspace(fakeDependencies({
      loadDashboard: async () => { throw new Error("日志失败"); },
    }));
    await failingWorkspace.dispatch({ type: "set-filters", scope: "legacy" });
    expect(failingWorkspace.snapshot().logsLoading).toBe(true);
    await failingWorkspace.dispatch({ type: "refresh", mode: "manual" });
    expect(failingWorkspace.snapshot().logsLoading).toBe(false);
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
    expect(timers.activeCount()).toBe(1);
    const running = workspace.dispatch({ type: "refresh", mode: "manual" });
    workspace.dispose();
    pending.resolve(dashboardWithRequests(7));
    await running;

    expect(timers.activeCount()).toBe(0);
    expect(workspace.snapshot().summary).toBeNull();
  });

  it("切出 Usage route 时停止自动刷新并关闭 range picker", async () => {
    const timers = fakeTimers();
    const workspace = createUsageWorkspace(fakeDependencies({
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
    }));
    await workspace.dispatch({ type: "set-context", active: true, locale: "zh-CN" });
    await workspace.dispatch({ type: "open-range-picker" });
    await workspace.dispatch({ type: "set-refresh-interval", intervalMs: 30_000 });
    await workspace.dispatch({ type: "set-context", active: false, locale: "zh-CN" });

    expect(workspace.snapshot().rangePickerOpen).toBe(false);
    expect(timers.activeCount()).toBe(0);
  });

  it("snapshot 返回不泄漏内部状态的深拷贝", async () => {
    const workspace = createUsageWorkspace(fakeDependencies({
      loadDashboard: async () => dashboardWithRequests(7),
    }));
    await workspace.dispatch({ type: "refresh", mode: "initial" });

    const first = workspace.snapshot();
    first.summary!.total_requests = 99;
    first.logs!.page = 99;
    (first.accountSummary as unknown[]).push({});
    const second = workspace.snapshot();

    expect(second).not.toBe(first);
    expect(second.summary).not.toBe(first.summary);
    expect(second.accountSummary).not.toBe(first.accountSummary);
    expect(second.summary?.total_requests).toBe(7);
    expect(second.logs?.page).toBe(1);
    expect(second.accountSummary).toHaveLength(0);
  });
});

import type { OpenAiUsageDashboard, OpenAiUsageQuery, OpenAiUsageRangePreset } from "../types";
import type { UsageWorkspaceEvent, UsageWorkspaceView } from "./model";

export interface UsageWorkspaceDependencies {
  readonly loadDashboard: (
    query: OpenAiUsageQuery,
    page: number,
    pageSize: number,
  ) => Promise<OpenAiUsageDashboard>;
  readonly now: () => number;
  readonly setInterval: (handler: () => void, intervalMs: number) => unknown;
  readonly clearInterval: (id: unknown) => void;
  readonly onChange: (view: Readonly<UsageWorkspaceView>) => void;
  readonly onError: (error: unknown) => void;
}

export interface UsageWorkspace {
  dispatch(event: UsageWorkspaceEvent): Promise<void>;
  snapshot(): Readonly<UsageWorkspaceView>;
  dispose(): void;
}

interface WorkspaceState {
  summary: UsageWorkspaceView["summary"];
  accountSummary: OpenAiUsageDashboard["account_summary"];
  modelOptions: OpenAiUsageDashboard["model_options"];
  trends: OpenAiUsageDashboard["trends"];
  logs: UsageWorkspaceView["logs"];
  dataSources: OpenAiUsageDashboard["data_sources"];
  rangePreset: UsageWorkspaceView["rangePreset"];
  scope: UsageWorkspaceView["scope"];
  logsPage: number;
  logsPageSize: number;
  loaded: boolean;
  loading: boolean;
  loadError: string | null;
  refreshQueued: boolean;
}

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfTodayMs(nowMs: number): number {
  const now = new Date(nowMs);
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

function presetRange(preset: OpenAiUsageRangePreset, nowMs: number) {
  switch (preset) {
    case "today":
      return { startMs: startOfTodayMs(nowMs), endMs: nowMs };
    case "7d":
      return { startMs: startOfTodayMs(nowMs - 6 * DAY_MS), endMs: nowMs };
    case "14d":
      return { startMs: startOfTodayMs(nowMs - 13 * DAY_MS), endMs: nowMs };
    case "30d":
      return { startMs: startOfTodayMs(nowMs - 29 * DAY_MS), endMs: nowMs };
    case "1d":
    case "custom":
    default:
      return { startMs: nowMs - DAY_MS, endMs: nowMs };
  }
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

export function createUsageWorkspace(
  dependencies: UsageWorkspaceDependencies,
): UsageWorkspace {
  const state: WorkspaceState = {
    summary: null,
    accountSummary: [],
    modelOptions: [],
    trends: [],
    logs: null,
    dataSources: [],
    rangePreset: "1d",
    scope: "strict",
    logsPage: 1,
    logsPageSize: 20,
    loaded: false,
    loading: false,
    loadError: null,
    refreshQueued: false,
  };
  let customStartMs: number | null = null;
  let customEndMs: number | null = null;
  let liveEnd = true;
  let profile: string | null = null;
  let accountGroupKey: string | null = null;
  let accountLabel: string | null = null;
  let model: string | null = null;
  let statusCode: number | null = null;
  let active = false;
  let locale = "en";
  let refreshIntervalMs = 0;
  let timerId: unknown = null;
  let disposed = false;
  let refreshInFlight = false;
  let requestSerial = 0;
  let activeRequestSerial = 0;

  function snapshot(): Readonly<UsageWorkspaceView> {
    return clone(state);
  }

  function notifyChange(): void {
    if (!disposed) {
      dependencies.onChange(snapshot());
    }
  }

  function currentIntentKey(): string {
    return JSON.stringify({
      rangePreset: state.rangePreset,
      customStartMs,
      customEndMs,
      liveEnd,
      profile,
      accountGroupKey,
      accountLabel,
      model,
      statusCode,
      scope: state.scope,
      page: state.logsPage,
      pageSize: state.logsPageSize,
    });
  }

  function isStaleLoad(serial: number, intentKey: string): boolean {
    return disposed || serial !== activeRequestSerial || intentKey !== currentIntentKey();
  }

  function resolveQuery(): OpenAiUsageQuery {
    const nowMs = dependencies.now();
    const range = state.rangePreset === "custom"
      ? {
        startMs: customStartMs,
        endMs: liveEnd ? nowMs : customEndMs,
      }
      : presetRange(state.rangePreset, nowMs);
    return {
      start_date: range.startMs == null ? null : Math.floor(range.startMs / 1000),
      end_date: range.endMs == null ? null : Math.floor(range.endMs / 1000),
      profile,
      account_group_key: accountGroupKey,
      account_label: accountLabel,
      model,
      status_code: statusCode,
      scope: state.scope,
    };
  }

  function applyDashboard(dashboard: OpenAiUsageDashboard): void {
    const copied = clone(dashboard);
    state.summary = copied.summary;
    state.accountSummary = copied.account_summary;
    state.modelOptions = copied.model_options;
    state.trends = copied.trends;
    state.logs = copied.logs;
    state.dataSources = copied.data_sources;
    state.logsPage = copied.logs.page;
    state.logsPageSize = copied.logs.page_size;
  }

  async function load(
    serial: number,
    intentKey: string,
    query: OpenAiUsageQuery,
  ): Promise<void> {
    let dashboard: OpenAiUsageDashboard;
    try {
      dashboard = await dependencies.loadDashboard(query, state.logsPage, state.logsPageSize);
    } catch (error) {
      if (isStaleLoad(serial, intentKey)) return;
      throw error;
    }
    if (isStaleLoad(serial, intentKey)) return;
    const pageSize = Math.max(1, dashboard.logs.page_size);
    const totalPages = Math.max(1, Math.ceil(dashboard.logs.total / pageSize));
    if (dashboard.logs.page > totalPages) {
      state.logsPage = totalPages;
      const correctedIntentKey = currentIntentKey();
      let correctedDashboard: OpenAiUsageDashboard;
      try {
        correctedDashboard = await dependencies.loadDashboard(
          query,
          state.logsPage,
          state.logsPageSize,
        );
      } catch (error) {
        if (isStaleLoad(serial, correctedIntentKey)) return;
        throw error;
      }
      if (isStaleLoad(serial, correctedIntentKey)) return;
      applyDashboard(correctedDashboard);
    } else {
      applyDashboard(dashboard);
    }
    state.loaded = true;
    state.loadError = null;
  }

  function syncTimer(): void {
    if (timerId !== null) {
      dependencies.clearInterval(timerId);
      timerId = null;
    }
    if (!disposed && active && refreshIntervalMs > 0) {
      timerId = dependencies.setInterval(() => {
        void dispatch({ type: "refresh", mode: "auto" });
      }, refreshIntervalMs);
    }
  }

  async function performRefresh(mode: "initial" | "manual" | "auto"): Promise<void> {
    if (disposed) return;
    const serial = ++requestSerial;
    activeRequestSerial = serial;
    const intentKey = currentIntentKey();
    const query = resolveQuery();
    state.loading = true;
    state.loadError = null;
    notifyChange();
    try {
      await load(serial, intentKey, query);
    } catch (error) {
      if (disposed) return;
      state.loaded = true;
      state.loadError = error instanceof Error ? error.message : String(error);
      if (mode === "manual") {
        dependencies.onError(error);
      }
    } finally {
      if (!disposed && serial === activeRequestSerial) {
        state.loading = false;
        notifyChange();
      }
    }
  }

  async function refresh(mode: "initial" | "manual" | "auto"): Promise<void> {
    if (disposed) return;
    if (mode === "initial") {
      await performRefresh(mode);
      return;
    }
    if (refreshInFlight) {
      if (mode === "manual") {
        state.refreshQueued = true;
        notifyChange();
      }
      return;
    }

    refreshInFlight = true;
    let nextMode = mode;
    try {
      do {
        state.refreshQueued = false;
        await performRefresh(nextMode);
        nextMode = "manual";
      } while (!disposed && state.refreshQueued);
    } finally {
      refreshInFlight = false;
      state.refreshQueued = false;
    }
  }

  async function dispatch(event: UsageWorkspaceEvent): Promise<void> {
    if (disposed) return;
    switch (event.type) {
      case "set-context":
        active = event.active;
        locale = event.locale;
        void locale;
        syncTimer();
        notifyChange();
        return;
      case "set-range":
        state.rangePreset = event.preset;
        if (event.preset === "custom") {
          customStartMs = event.startMs ?? customStartMs;
          customEndMs = event.endMs ?? customEndMs;
          liveEnd = event.liveEnd ?? liveEnd;
        }
        state.logsPage = 1;
        notifyChange();
        return;
      case "set-filters":
        profile = event.profile ?? null;
        accountGroupKey = event.accountGroupKey ?? null;
        accountLabel = event.accountLabel ?? null;
        model = event.model ?? null;
        statusCode = event.statusCode ?? null;
        state.scope = event.scope ?? state.scope;
        state.logsPage = 1;
        notifyChange();
        return;
      case "set-page":
        state.logsPage = Math.max(1, Math.floor(event.page));
        if (event.pageSize !== undefined) {
          state.logsPageSize = Math.max(1, Math.floor(event.pageSize));
        }
        notifyChange();
        return;
      case "set-refresh-interval":
        refreshIntervalMs = Math.max(0, event.intervalMs);
        syncTimer();
        notifyChange();
        return;
      case "refresh":
        await refresh(event.mode);
    }
  }

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    if (timerId !== null) {
      dependencies.clearInterval(timerId);
      timerId = null;
    }
  }

  return { dispatch, snapshot, dispose };
}

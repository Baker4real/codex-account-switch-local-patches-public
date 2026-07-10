import type { OpenAiUsageDashboard, OpenAiUsageQuery, OpenAiUsageRangePreset } from "../types";
import { t } from "../i18n";
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
  customStartMs: number | null;
  customEndMs: number | null;
  liveEnd: boolean;
  rangePickerOpen: boolean;
  rangeDraftStartMs: number | null;
  rangeDraftEndMs: number | null;
  rangeDraftLiveEnd: boolean;
  rangeDraftActiveField: "start" | "end";
  rangeDisplayMonthMs: number | null;
  rangeError: string | null;
  profileFilter: string | null;
  modelFilter: string | null;
  statusCodeFilter: number | null;
  logsPage: number;
  logsPageSize: number;
  refreshIntervalMs: number;
  lastResolvedEndMs: number | null;
  locale: UsageWorkspaceView["locale"];
  loaded: boolean;
  loading: boolean;
  logsLoading: boolean;
  refreshInFlight: boolean;
  loadError: string | null;
  refreshQueued: boolean;
}

interface LoadAttempt {
  readonly serial: number;
  intentKey: string;
  readonly query: OpenAiUsageQuery;
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

function startOfLocalMonthMs(valueMs: number): number {
  const date = new Date(valueMs);
  return new Date(date.getFullYear(), date.getMonth(), 1).getTime();
}

function setDateKeepTime(valueMs: number, dayMs: number): number {
  const value = new Date(valueMs);
  const day = new Date(dayMs);
  return new Date(
    day.getFullYear(), day.getMonth(), day.getDate(), value.getHours(), value.getMinutes(),
  ).getTime();
}

function parseDateKeepTime(valueMs: number, value: string): number | null {
  const [year, month, day] = value.split("-").map(Number);
  if (!Number.isFinite(year) || !Number.isFinite(month) || !Number.isFinite(day)) return null;
  const current = new Date(valueMs);
  return new Date(
    year, month - 1, day, current.getHours(), current.getMinutes(),
  ).getTime();
}

function parseTimeKeepDate(valueMs: number, value: string): number | null {
  const [hour, minute] = value.split(":").map(Number);
  if (!Number.isFinite(hour) || !Number.isFinite(minute)) return null;
  const current = new Date(valueMs);
  return new Date(
    current.getFullYear(), current.getMonth(), current.getDate(), hour, minute,
  ).getTime();
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
    customStartMs: null,
    customEndMs: null,
    liveEnd: true,
    rangePickerOpen: false,
    rangeDraftStartMs: null,
    rangeDraftEndMs: null,
    rangeDraftLiveEnd: false,
    rangeDraftActiveField: "start",
    rangeDisplayMonthMs: null,
    rangeError: null,
    profileFilter: null,
    modelFilter: null,
    statusCodeFilter: null,
    logsPage: 1,
    logsPageSize: 20,
    refreshIntervalMs: 30_000,
    lastResolvedEndMs: null,
    locale: "en",
    loaded: false,
    loading: false,
    logsLoading: false,
    refreshInFlight: false,
    loadError: null,
    refreshQueued: false,
  };
  let profile: string | null = null;
  let accountGroupKey: string | null = null;
  let accountLabel: string | null = null;
  let model: string | null = null;
  let statusCode: number | null = null;
  let active = false;
  let refreshTimerId: unknown = null;
  let rangeLiveEndTimerId: unknown = null;
  let disposed = false;
  let refreshRun: Promise<void> | null = null;
  let activeRefreshMode: "initial" | "manual" | "auto" | null = null;
  let activeAttempt: LoadAttempt | null = null;
  let queuedRefreshMode: "initial" | "manual" | null = null;
  let refreshOnActivate = false;
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
      customStartMs: state.customStartMs,
      customEndMs: state.customEndMs,
      liveEnd: state.liveEnd,
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

  function invalidateActiveRequest(): void {
    activeRequestSerial = ++requestSerial;
  }

  function markLogsLoadingIfIntentChanged(previousIntentKey: string): void {
    if (previousIntentKey !== currentIntentKey()) state.logsLoading = true;
  }

  function resolveQuery(): OpenAiUsageQuery {
    const nowMs = dependencies.now();
    const range = state.rangePreset === "custom"
      ? {
        startMs: state.customStartMs,
        endMs: state.liveEnd ? nowMs : state.customEndMs,
      }
      : presetRange(state.rangePreset, nowMs);
    state.lastResolvedEndMs = range.endMs;
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
    state.accountSummary = copied.account_summary.length > 0
      ? copied.account_summary
      : copied.summary.accounts;
    state.modelOptions = copied.model_options.length > 0
      ? copied.model_options
      : copied.summary.models;
    state.trends = copied.trends;
    state.logs = copied.logs;
    state.dataSources = copied.data_sources;
    state.logsPage = copied.logs.page;
    state.logsPageSize = copied.logs.page_size;
  }

  async function load(attempt: LoadAttempt): Promise<void> {
    let dashboard: OpenAiUsageDashboard;
    try {
      dashboard = await dependencies.loadDashboard(
        attempt.query,
        state.logsPage,
        state.logsPageSize,
      );
    } catch (error) {
      if (isStaleLoad(attempt.serial, attempt.intentKey)) return;
      throw error;
    }
    if (isStaleLoad(attempt.serial, attempt.intentKey)) return;
    const pageSize = Math.max(1, dashboard.logs.page_size);
    const totalPages = Math.max(1, Math.ceil(dashboard.logs.total / pageSize));
    if (dashboard.logs.page > totalPages) {
      state.logsPage = totalPages;
      attempt.intentKey = currentIntentKey();
      let correctedDashboard: OpenAiUsageDashboard;
      try {
        correctedDashboard = await dependencies.loadDashboard(
          attempt.query,
          state.logsPage,
          state.logsPageSize,
        );
      } catch (error) {
        if (isStaleLoad(attempt.serial, attempt.intentKey)) return;
        throw error;
      }
      if (isStaleLoad(attempt.serial, attempt.intentKey)) return;
      applyDashboard(correctedDashboard);
    } else {
      applyDashboard(dashboard);
    }
    state.loaded = true;
    state.loadError = null;
    state.logsLoading = false;
    refreshOnActivate = false;
  }

  function syncRefreshTimer(): void {
    if (refreshTimerId !== null) {
      dependencies.clearInterval(refreshTimerId);
      refreshTimerId = null;
    }
    if (!disposed && active && state.refreshIntervalMs > 0) {
      refreshTimerId = dependencies.setInterval(() => {
        void dispatch({ type: "refresh", mode: "auto" });
      }, state.refreshIntervalMs);
    }
  }

  function syncRangeLiveEndTimer(): void {
    if (rangeLiveEndTimerId !== null) {
      dependencies.clearInterval(rangeLiveEndTimerId);
      rangeLiveEndTimerId = null;
    }
    if (!disposed && active && state.rangePickerOpen && state.rangeDraftLiveEnd) {
      rangeLiveEndTimerId = dependencies.setInterval(() => {
        state.rangeDraftEndMs = dependencies.now();
        notifyChange();
      }, 1_000);
    }
  }

  function resetRangeDraftFromCurrent(): void {
    const nowMs = dependencies.now();
    const range = state.rangePreset === "custom"
      ? {
        startMs: state.customStartMs ?? nowMs - DAY_MS,
        endMs: state.liveEnd ? nowMs : state.customEndMs ?? nowMs,
      }
      : presetRange(state.rangePreset, nowMs);
    state.rangeDraftStartMs = range.startMs;
    state.rangeDraftEndMs = range.endMs;
    state.rangeDraftLiveEnd = state.rangePreset === "custom" ? state.liveEnd : false;
    state.rangeDraftActiveField = "start";
    state.rangeDisplayMonthMs = startOfLocalMonthMs(range.startMs ?? nowMs);
    state.rangeError = null;
  }

  function closeRangePicker(): void {
    state.rangePickerOpen = false;
    state.rangeError = null;
    syncRangeLiveEndTimer();
  }

  async function performRefresh(mode: "initial" | "manual" | "auto"): Promise<void> {
    if (disposed) return;
    const serial = ++requestSerial;
    activeRequestSerial = serial;
    const attempt: LoadAttempt = {
      serial,
      intentKey: currentIntentKey(),
      query: resolveQuery(),
    };
    activeAttempt = attempt;
    state.loading = true;
    state.loadError = null;
    if (active) notifyChange();
    try {
      await load(attempt);
    } catch (error) {
      if (isStaleLoad(attempt.serial, attempt.intentKey)) return;
      state.loaded = true;
      state.loadError = error instanceof Error
        ? error.message
        : t(state.locale, "usageLoadFailed");
      state.logsLoading = false;
      if (mode === "manual") {
        dependencies.onError(
          error instanceof Error ? error : new Error(t(state.locale, "usageLoadFailed")),
        );
      }
    } finally {
      if (activeAttempt === attempt) activeAttempt = null;
      if (!disposed && serial === activeRequestSerial) {
        state.loading = false;
        if (active) notifyChange();
      }
    }
  }

  async function runRefreshLoop(firstMode: "initial" | "manual" | "auto"): Promise<void> {
    state.refreshInFlight = true;
    let nextMode: "initial" | "manual" | "auto" | null = firstMode;
    try {
      while (!disposed && nextMode !== null) {
        activeRefreshMode = nextMode;
        queuedRefreshMode = null;
        state.refreshQueued = false;
        await performRefresh(nextMode);
        nextMode = queuedRefreshMode;
      }
    } finally {
      activeRefreshMode = null;
      queuedRefreshMode = null;
      state.refreshInFlight = false;
      state.refreshQueued = false;
      refreshRun = null;
      if (!disposed && active) notifyChange();
    }
  }

  async function refresh(
    mode: "initial" | "manual" | "auto",
    options: { readonly forceNewInitial?: boolean } = {},
  ): Promise<void> {
    if (disposed) return;
    if (refreshRun !== null) {
      if (mode === "initial") {
        const activeInitialReusable = activeRefreshMode === "initial"
          && activeAttempt !== null
          && activeAttempt.serial === activeRequestSerial
          && activeAttempt.intentKey === currentIntentKey();
        const mustQueueInitial = options.forceNewInitial
          || !activeInitialReusable
          || queuedRefreshMode === "manual";
        if (mustQueueInitial && queuedRefreshMode !== "initial") {
          queuedRefreshMode = "initial";
          state.refreshQueued = true;
          invalidateActiveRequest();
          if (active) notifyChange();
        }
        await refreshRun;
      } else if (mode === "manual") {
        if (queuedRefreshMode === null) {
          queuedRefreshMode = "manual";
          state.refreshQueued = true;
          if (active) notifyChange();
        }
        await refreshRun;
      }
      return;
    }

    const run = runRefreshLoop(mode);
    refreshRun = run;
    try {
      await run;
    } finally {
      if (refreshRun === run) refreshRun = null;
    }
  }

  async function dispatch(event: UsageWorkspaceEvent): Promise<void> {
    if (disposed) return;
    switch (event.type) {
      case "set-context":
        const activeChanged = active !== event.active;
        let shouldRefreshOnActivate = false;
        active = event.active;
        state.locale = event.locale;
        if (!active) {
          closeRangePicker();
          const refreshPending = refreshRun !== null
            || activeRefreshMode !== null
            || queuedRefreshMode !== null;
          refreshOnActivate ||= state.logsLoading || !state.loaded || refreshPending;
          if (activeChanged || refreshPending) {
            invalidateActiveRequest();
            state.loading = false;
            state.logsLoading = false;
          }
        } else if (activeChanged && refreshOnActivate) {
          refreshOnActivate = false;
          state.logsLoading = true;
          shouldRefreshOnActivate = true;
        }
        if (activeChanged) {
          syncRefreshTimer();
          syncRangeLiveEndTimer();
        }
        notifyChange();
        if (shouldRefreshOnActivate) {
          await refresh("initial", { forceNewInitial: true });
        }
        return;
      case "open-range-picker":
        if (!state.rangePickerOpen) resetRangeDraftFromCurrent();
        state.rangePickerOpen = true;
        syncRangeLiveEndTimer();
        notifyChange();
        return;
      case "close-range-picker":
        closeRangePicker();
        notifyChange();
        return;
      case "set-range-draft-field":
        if (event.field === "end" && state.rangeDraftLiveEnd) return;
        state.rangeDraftActiveField = event.field;
        notifyChange();
        return;
      case "set-range-draft-date": {
        if (event.field === "end" && state.rangeDraftLiveEnd) return;
        const current = event.field === "start" ? state.rangeDraftStartMs : state.rangeDraftEndMs;
        if (current == null) return;
        const next = parseDateKeepTime(current, event.value);
        if (next == null) return;
        if (event.field === "start") state.rangeDraftStartMs = next;
        else state.rangeDraftEndMs = next;
        state.rangeDisplayMonthMs = startOfLocalMonthMs(next);
        state.rangeDraftActiveField = event.field;
        state.rangeError = null;
        notifyChange();
        return;
      }
      case "set-range-draft-time": {
        if (event.field === "end" && state.rangeDraftLiveEnd) return;
        const current = event.field === "start" ? state.rangeDraftStartMs : state.rangeDraftEndMs;
        if (current == null) return;
        const next = parseTimeKeepDate(current, event.value);
        if (next == null) return;
        if (event.field === "start") state.rangeDraftStartMs = next;
        else state.rangeDraftEndMs = next;
        state.rangeDraftActiveField = event.field;
        state.rangeError = null;
        notifyChange();
        return;
      }
      case "pick-range-day": {
        const startMs = state.rangeDraftStartMs;
        const endMs = state.rangeDraftEndMs;
        if (startMs == null || endMs == null) return;
        if (state.rangeDraftLiveEnd) {
          state.rangeDraftStartMs = setDateKeepTime(startMs, event.dayMs);
          state.rangeDraftActiveField = "start";
        } else if (state.rangeDraftActiveField === "start") {
          const nextStart = setDateKeepTime(startMs, event.dayMs);
          state.rangeDraftStartMs = nextStart;
          if (nextStart > endMs) state.rangeDraftEndMs = nextStart;
          state.rangeDraftActiveField = "end";
        } else {
          const nextEnd = setDateKeepTime(endMs, event.dayMs);
          if (nextEnd < startMs) {
            state.rangeDraftStartMs = nextEnd;
            state.rangeDraftActiveField = "end";
          } else {
            state.rangeDraftEndMs = nextEnd;
          }
        }
        state.rangeDisplayMonthMs = startOfLocalMonthMs(event.dayMs);
        state.rangeError = null;
        notifyChange();
        return;
      }
      case "move-range-month": {
        const current = state.rangeDisplayMonthMs
          ?? startOfLocalMonthMs(state.rangeDraftStartMs ?? dependencies.now());
        const date = new Date(current);
        state.rangeDisplayMonthMs = new Date(
          date.getFullYear(), date.getMonth() + event.delta, 1,
        ).getTime();
        notifyChange();
        return;
      }
      case "show-range-today":
        state.rangeDisplayMonthMs = startOfLocalMonthMs(dependencies.now());
        notifyChange();
        return;
      case "set-range-draft-live-end":
        state.rangeDraftLiveEnd = event.live;
        if (event.live) {
          state.rangeDraftEndMs = dependencies.now();
          state.rangeDraftActiveField = "start";
        } else if (state.rangeDraftEndMs == null) {
          state.rangeDraftEndMs = dependencies.now();
        }
        state.rangeError = null;
        syncRangeLiveEndTimer();
        notifyChange();
        return;
      case "apply-range-draft": {
        const previousIntentKey = currentIntentKey();
        const startMs = state.rangeDraftStartMs;
        const endMs = state.rangeDraftLiveEnd ? dependencies.now() : state.rangeDraftEndMs;
        if (startMs == null || endMs == null) {
          state.rangeError = t(state.locale, "usageInvalidTimeRange");
          notifyChange();
          return;
        }
        if (endMs < startMs) {
          state.rangeError = t(state.locale, "usageInvalidTimeRangeOrder");
          notifyChange();
          return;
        }
        state.rangePreset = "custom";
        state.customStartMs = startMs;
        state.customEndMs = endMs;
        state.liveEnd = state.rangeDraftLiveEnd;
        closeRangePicker();
        state.logsPage = 1;
        markLogsLoadingIfIntentChanged(previousIntentKey);
        notifyChange();
        await refresh("manual");
        return;
      }
      case "set-range": {
        const previousIntentKey = currentIntentKey();
        state.rangePreset = event.preset;
        if (event.preset === "custom") {
          state.customStartMs = event.startMs ?? state.customStartMs;
          state.customEndMs = event.endMs ?? state.customEndMs;
          state.liveEnd = event.liveEnd ?? state.liveEnd;
        } else {
          closeRangePicker();
        }
        state.logsPage = 1;
        markLogsLoadingIfIntentChanged(previousIntentKey);
        notifyChange();
        return;
      }
      case "set-filters": {
        const previousIntentKey = currentIntentKey();
        if (event.profile !== undefined) profile = event.profile;
        if (event.accountGroupKey !== undefined) accountGroupKey = event.accountGroupKey;
        if (event.accountLabel !== undefined) accountLabel = event.accountLabel;
        if (event.model !== undefined) model = event.model;
        if (event.statusCode !== undefined) statusCode = event.statusCode;
        state.profileFilter = accountGroupKey ?? profile;
        state.modelFilter = model;
        state.statusCodeFilter = statusCode;
        state.scope = event.scope ?? state.scope;
        state.logsPage = 1;
        markLogsLoadingIfIntentChanged(previousIntentKey);
        notifyChange();
        return;
      }
      case "set-page": {
        const previousIntentKey = currentIntentKey();
        state.logsPage = Math.max(1, Math.floor(event.page));
        if (event.pageSize !== undefined) {
          state.logsPageSize = Math.max(1, Math.floor(event.pageSize));
        }
        markLogsLoadingIfIntentChanged(previousIntentKey);
        notifyChange();
        return;
      }
      case "set-refresh-interval":
        state.refreshIntervalMs = Math.max(0, event.intervalMs);
        syncRefreshTimer();
        notifyChange();
        return;
      case "refresh":
        await refresh(event.mode);
    }
  }

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    if (refreshTimerId !== null) {
      dependencies.clearInterval(refreshTimerId);
      refreshTimerId = null;
    }
    if (rangeLiveEndTimerId !== null) {
      dependencies.clearInterval(rangeLiveEndTimerId);
      rangeLiveEndTimerId = null;
    }
  }

  return { dispatch, snapshot, dispose };
}

import type { Locale } from "../i18n";
import type {
  OpenAiUsageDashboard,
  OpenAiUsageQuery,
  OpenAiUsageRangePreset,
  OpenAiUsageScope,
} from "../types";
import type { UsageWorkspaceView } from "./model";
import { renderUsage } from "./render";
import { createUsageWorkspace, type UsageWorkspace } from "./workspace";

export interface UsageElements {
  refreshButton: HTMLButtonElement | null;
  rangeButton: HTMLButtonElement | null;
  rangeLabel: HTMLSpanElement | null;
  rangePopover: HTMLDivElement | null;
  scopeSelect: HTMLSelectElement | null;
  accountFilter: HTMLSelectElement | null;
  modelFilter: HTMLSelectElement | null;
  refreshIntervalSelect: HTMLSelectElement | null;
  accountPanel: HTMLDivElement | null;
  summaryPanel: HTMLDivElement | null;
  trendPanel: HTMLDivElement | null;
  detailPanel: HTMLDivElement | null;
}

export interface MountUsageWorkspaceDependencies {
  readonly loadDashboard: (
    query: OpenAiUsageQuery,
    page: number,
    pageSize: number,
  ) => Promise<OpenAiUsageDashboard>;
  readonly getContext: () => { readonly active: boolean; readonly locale: Locale };
  readonly onError: (error: Error, notifyUser: boolean) => void;
}

type UsageRangeAction =
  | { readonly type: "preset"; readonly value: string }
  | { readonly type: "field"; readonly value: string }
  | { readonly type: "day"; readonly value: string }
  | { readonly type: "month"; readonly value: string }
  | { readonly type: "today" }
  | { readonly type: "cancel" }
  | { readonly type: "apply" };

type UsageRangeChange =
  | { readonly type: "date"; readonly field: string; readonly value: string }
  | { readonly type: "time"; readonly field: string; readonly value: string }
  | { readonly type: "live"; readonly value: boolean };

interface UsageDomHandlers {
  readonly refresh: () => void;
  readonly toggleRange: () => void;
  readonly rangeAction: (action: UsageRangeAction) => void;
  readonly rangeChange: (change: UsageRangeChange) => void;
  readonly rangeFocus: (field: string | null) => void;
  readonly scopeChange: (value: string) => void;
  readonly accountChange: (value: string) => void;
  readonly modelChange: (value: string) => void;
  readonly intervalChange: (value: string) => void;
  readonly logPage: (value: string, disabled: boolean) => void;
  readonly documentClick: (outsidePicker: boolean) => void;
  readonly documentKeydown: (key: string) => void;
}

// 仅在 Usage DOM 边界内部使用；测试通过内存实现替代浏览器对象。
export interface UsageDomPort {
  readonly now: () => number;
  readonly setInterval: (handler: () => void, intervalMs: number) => unknown;
  readonly clearInterval: (id: unknown) => void;
  readonly render: (view: Readonly<UsageWorkspaceView>) => void;
  readonly bind: (handlers: UsageDomHandlers) => () => void;
}

export interface UsageDocumentLike {
  readonly getElementById: (id: string) => unknown;
  readonly addEventListener: (type: string, listener: EventListener) => void;
  readonly removeEventListener: (type: string, listener: EventListener) => void;
}

export interface UsageTimerHost {
  readonly setInterval: (handler: () => void, intervalMs: number) => unknown;
  readonly clearInterval: (id: unknown) => void;
}

interface UsageListenerTarget {
  readonly addEventListener: (type: string, listener: EventListener) => void;
  readonly removeEventListener: (type: string, listener: EventListener) => void;
}

interface UsageElementLike extends UsageListenerTarget {
  readonly nodeType: number;
  readonly tagName: string;
  readonly dataset: Record<string, string | undefined>;
  readonly closest: (selector: string) => unknown;
  readonly contains: (node: unknown) => boolean;
}

interface UsageInputLike extends UsageElementLike {
  readonly value: string;
  readonly checked: boolean;
  readonly matches: (selector: string) => boolean;
}

interface UsageButtonLike extends UsageElementLike {
  readonly disabled: boolean;
}

const rangePresets = new Set<string>(["today", "1d", "7d", "14d", "30d", "custom"]);
const usageScopes = new Set<string>([
  "strict",
  "legacy",
  "combined_attributed",
  "candidate",
  "excluded",
]);

function isRangePreset(value: string): value is OpenAiUsageRangePreset {
  return rangePresets.has(value);
}

function isUsageScope(value: string): value is OpenAiUsageScope {
  return usageScopes.has(value);
}

function isRangeField(value: string | null): value is "start" | "end" {
  return value === "start" || value === "end";
}

function isObject(value: unknown): value is Record<PropertyKey, unknown> {
  return typeof value === "object" && value !== null;
}

function isUsageElement(value: unknown): value is UsageElementLike {
  return isObject(value)
    && Number.isInteger(value.nodeType)
    && typeof value.tagName === "string"
    && isObject(value.dataset)
    && typeof value.addEventListener === "function"
    && typeof value.removeEventListener === "function"
    && typeof value.closest === "function"
    && typeof value.contains === "function";
}

function isUsageInput(value: unknown): value is UsageInputLike {
  return isUsageElement(value)
    && value.tagName.toUpperCase() === "INPUT"
    && "value" in value
    && typeof value.value === "string"
    && "checked" in value
    && typeof value.checked === "boolean"
    && "matches" in value
    && typeof value.matches === "function";
}

function isUsageButton(value: unknown): value is UsageButtonLike {
  return isUsageElement(value)
    && value.tagName.toUpperCase() === "BUTTON"
    && "disabled" in value
    && typeof value.disabled === "boolean";
}

function isUsageNode(value: unknown): boolean {
  return isObject(value) && Number.isInteger(value.nodeType);
}

function optionalElement<T extends HTMLElement>(documentLike: UsageDocumentLike, id: string): T | null {
  const element = documentLike.getElementById(id);
  return isUsageElement(element) ? element as unknown as T : null;
}

export function queryUsageElements(documentLike: UsageDocumentLike = document): UsageElements {
  return {
    refreshButton: optionalElement(documentLike, "usage-refresh-button"),
    rangeButton: optionalElement(documentLike, "usage-range-button"),
    rangeLabel: optionalElement(documentLike, "usage-range-label"),
    rangePopover: optionalElement(documentLike, "usage-range-popover"),
    scopeSelect: optionalElement(documentLike, "usage-scope-select"),
    accountFilter: optionalElement(documentLike, "usage-account-filter"),
    modelFilter: optionalElement(documentLike, "usage-model-filter"),
    refreshIntervalSelect: optionalElement(documentLike, "usage-refresh-interval-select"),
    accountPanel: optionalElement(documentLike, "usage-account-panel"),
    summaryPanel: optionalElement(documentLike, "usage-summary-panel"),
    trendPanel: optionalElement(documentLike, "usage-trend-panel"),
    detailPanel: optionalElement(documentLike, "usage-detail-panel"),
  };
}

const browserTimerHost: UsageTimerHost = {
  setInterval: (handler, intervalMs) => window.setInterval(handler, intervalMs),
  clearInterval: (id) => window.clearInterval(Number(id)),
};

function eventTarget(event: Event): unknown {
  return event.target;
}

function closestElement(target: UsageElementLike, selector: string): UsageElementLike | null {
  const closest = target.closest(selector);
  return isUsageElement(closest) ? closest : null;
}

function datasetValue(
  target: UsageElementLike,
  selector: string,
  key: string,
): string | undefined {
  return closestElement(target, selector)?.dataset[key];
}

function parseDayMs(value: string): number | null {
  if (!/^-?\d+$/.test(value)) return null;
  const dayMs = Number(value);
  if (!Number.isSafeInteger(dayMs) || Math.abs(dayMs) > 8_640_000_000_000_000) return null;
  return Number.isNaN(new Date(dayMs).getTime()) ? null : dayMs;
}

function parseMonthDelta(value: string): -1 | 1 | null {
  if (value === "-1") return -1;
  if (value === "1") return 1;
  return null;
}

function parsePage(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const page = Number(value);
  return Number.isSafeInteger(page) && page >= 1 ? page : null;
}

function isStrictLocalDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);
  return date.getFullYear() === year
    && date.getMonth() === month - 1
    && date.getDate() === day;
}

function isStrictLocalTime(value: string): boolean {
  const match = /^(\d{2}):(\d{2})$/.exec(value);
  if (!match) return false;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  const time = new Date(2000, 0, 1, hour, minute);
  return time.getDate() === 1
    && time.getHours() === hour
    && time.getMinutes() === minute;
}

export function browserUsageDomPort(
  documentLike: UsageDocumentLike = document,
  timers: UsageTimerHost = browserTimerHost,
): UsageDomPort {
  const elements = queryUsageElements(documentLike);

  return {
    now: Date.now,
    setInterval: (handler, intervalMs) => timers.setInterval(handler, intervalMs),
    clearInterval: (id) => timers.clearInterval(id),
    render: (view) => renderUsage(elements, view),
    bind: (handlers) => {
      const cleanups: Array<() => void> = [];
      const listen = (
        target: UsageListenerTarget | null,
        type: string,
        listener: EventListener,
      ) => {
        if (!target) return;
        target.addEventListener(type, listener);
        cleanups.push(() => target.removeEventListener(type, listener));
      };
      listen(elements.refreshButton, "click", () => handlers.refresh());
      listen(elements.rangeButton, "click", () => handlers.toggleRange());
      listen(elements.rangePopover, "click", (event) => {
        const target = eventTarget(event);
        if (!isUsageElement(target) || isUsageInput(target)) return;
        const preset = datasetValue(
          target, "[data-usage-range-preset]", "usageRangePreset",
        );
        if (preset !== undefined) return handlers.rangeAction({ type: "preset", value: preset });
        const field = datasetValue(
          target, "[data-usage-range-field]", "usageRangeField",
        );
        if (field !== undefined) return handlers.rangeAction({ type: "field", value: field });
        const day = datasetValue(target, "[data-usage-range-day]", "usageRangeDay");
        if (day !== undefined) return handlers.rangeAction({ type: "day", value: day });
        const month = datasetValue(target, "[data-usage-range-month]", "usageRangeMonth");
        if (month !== undefined) return handlers.rangeAction({ type: "month", value: month });
        if (closestElement(target, "[data-usage-range-today]")) {
          handlers.rangeAction({ type: "today" });
        } else if (closestElement(target, "[data-usage-range-cancel]")) {
          handlers.rangeAction({ type: "cancel" });
        } else if (closestElement(target, "[data-usage-range-apply]")) {
          handlers.rangeAction({ type: "apply" });
        }
      });
      listen(elements.rangePopover, "change", (event) => {
        const target = eventTarget(event);
        if (!isUsageInput(target)) return;
        const dateField = target.dataset.usageRangeDate;
        if (dateField !== undefined) {
          handlers.rangeChange({ type: "date", field: dateField, value: target.value });
          return;
        }
        const timeField = target.dataset.usageRangeTime;
        if (timeField !== undefined) {
          handlers.rangeChange({ type: "time", field: timeField, value: target.value });
          return;
        }
        if (target.matches("[data-usage-range-live]")) {
          handlers.rangeChange({ type: "live", value: target.checked });
        }
      });
      listen(elements.rangePopover, "focusin", (event) => {
        const target = eventTarget(event);
        if (!isUsageInput(target)) return;
        handlers.rangeFocus(target.dataset.usageRangeDate ?? target.dataset.usageRangeTime ?? null);
      });
      listen(elements.scopeSelect, "change", () => handlers.scopeChange(elements.scopeSelect!.value));
      listen(elements.accountFilter, "change", () => handlers.accountChange(elements.accountFilter!.value));
      listen(elements.modelFilter, "change", () => handlers.modelChange(elements.modelFilter!.value));
      listen(elements.refreshIntervalSelect, "change", () => (
        handlers.intervalChange(elements.refreshIntervalSelect!.value)
      ));
      listen(elements.detailPanel, "click", (event) => {
        const target = eventTarget(event);
        if (!isUsageElement(target)) return;
        const button = closestElement(target, "[data-usage-log-page]");
        if (isUsageButton(button)) {
          handlers.logPage(button.dataset.usageLogPage ?? "", button.disabled);
        }
      });
      listen(documentLike, "click", (event) => {
        const picker = documentLike.getElementById("usage-range-picker");
        const target = eventTarget(event);
        handlers.documentClick(Boolean(
          isUsageElement(picker) && isUsageNode(target) && !picker.contains(target),
        ));
      });
      listen(documentLike, "keydown", (event) => {
        handlers.documentKeydown(
          isObject(event) && typeof event.key === "string" ? event.key : "",
        );
      });

      let bound = true;
      return () => {
        if (!bound) return;
        bound = false;
        for (const cleanup of cleanups) cleanup();
      };
    },
  };
}

function errorValue(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

export function mountUsageWorkspace(
  dependencies: MountUsageWorkspaceDependencies,
  dom: UsageDomPort = browserUsageDomPort(),
): UsageWorkspace {
  const workspace = createUsageWorkspace({
    loadDashboard: dependencies.loadDashboard,
    now: () => dom.now(),
    setInterval: (handler, intervalMs) => dom.setInterval(handler, intervalMs),
    clearInterval: (id) => dom.clearInterval(id),
    onChange: (view) => dom.render(view),
    onError: (error) => dependencies.onError(errorValue(error), true),
  });

  const refresh = () => workspace.dispatch({ type: "refresh", mode: "manual" });
  const refreshAfter = async (event: Parameters<UsageWorkspace["dispatch"]>[0]) => {
    await workspace.dispatch(event);
    await refresh();
  };
  const unbind = dom.bind({
    refresh: () => { void refresh(); },
    toggleRange: () => {
      void workspace.dispatch({
        type: workspace.snapshot().rangePickerOpen ? "close-range-picker" : "open-range-picker",
      });
    },
    rangeAction: (action) => {
      switch (action.type) {
        case "preset":
          if (isRangePreset(action.value)) {
            void refreshAfter({ type: "set-range", preset: action.value });
          }
          return;
        case "field":
          if (isRangeField(action.value)) {
            void workspace.dispatch({ type: "set-range-draft-field", field: action.value });
          }
          return;
        case "day": {
          const dayMs = parseDayMs(action.value);
          if (dayMs !== null) void workspace.dispatch({ type: "pick-range-day", dayMs });
          return;
        }
        case "month": {
          const delta = parseMonthDelta(action.value);
          if (delta !== null) void workspace.dispatch({ type: "move-range-month", delta });
          return;
        }
        case "today":
          void workspace.dispatch({ type: "show-range-today" });
          return;
        case "cancel":
          void workspace.dispatch({ type: "close-range-picker" });
          return;
        case "apply":
          void workspace.dispatch({ type: "apply-range-draft" });
      }
    },
    rangeChange: (change) => {
      if (change.type === "live") {
        void workspace.dispatch({ type: "set-range-draft-live-end", live: change.value });
      } else if (
        isRangeField(change.field)
        && (change.type === "date"
          ? isStrictLocalDate(change.value)
          : isStrictLocalTime(change.value))
      ) {
        void workspace.dispatch({
          type: change.type === "date" ? "set-range-draft-date" : "set-range-draft-time",
          field: change.field,
          value: change.value,
        });
      }
    },
    rangeFocus: (field) => {
      if (isRangeField(field)) {
        void workspace.dispatch({ type: "set-range-draft-field", field });
      }
    },
    scopeChange: (value) => {
      if (isUsageScope(value)) void refreshAfter({ type: "set-filters", scope: value });
    },
    accountChange: (value) => {
      void refreshAfter({ type: "set-filters", accountGroupKey: value || null });
    },
    modelChange: (value) => {
      void refreshAfter({ type: "set-filters", model: value || null });
    },
    intervalChange: (value) => {
      const intervalMs = Number(value);
      void workspace.dispatch({
        type: "set-refresh-interval",
        intervalMs: Number.isSafeInteger(intervalMs) && intervalMs >= 0 ? intervalMs : 0,
      });
    },
    logPage: (value, disabled) => {
      if (disabled) return;
      const page = parsePage(value);
      if (page === null || page === workspace.snapshot().logsPage) return;
      void refreshAfter({ type: "set-page", page });
    },
    documentClick: (outsidePicker) => {
      if (outsidePicker && workspace.snapshot().rangePickerOpen) {
        void workspace.dispatch({ type: "close-range-picker" });
      }
    },
    documentKeydown: (key) => {
      if (key === "Escape" && workspace.snapshot().rangePickerOpen) {
        void workspace.dispatch({ type: "close-range-picker" });
      }
    },
  });

  const disposeWorkspace = workspace.dispose.bind(workspace);
  let disposed = false;
  workspace.dispose = () => {
    if (disposed) return;
    disposed = true;
    unbind();
    disposeWorkspace();
  };

  void workspace.dispatch({ type: "set-context", ...dependencies.getContext() });
  return workspace;
}

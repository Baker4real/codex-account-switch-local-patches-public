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
interface UsageDomPort {
  readonly now: () => number;
  readonly setInterval: (handler: () => void, intervalMs: number) => unknown;
  readonly clearInterval: (id: unknown) => void;
  readonly render: (view: Readonly<UsageWorkspaceView>) => void;
  readonly bind: (handlers: UsageDomHandlers) => () => void;
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

function optionalElement<T extends HTMLElement>(id: string): T | null {
  const element = document.getElementById(id);
  return element instanceof HTMLElement ? element as T : null;
}

function queryUsageElements(): UsageElements {
  return {
    refreshButton: optionalElement("usage-refresh-button"),
    rangeButton: optionalElement("usage-range-button"),
    rangeLabel: optionalElement("usage-range-label"),
    rangePopover: optionalElement("usage-range-popover"),
    scopeSelect: optionalElement("usage-scope-select"),
    accountFilter: optionalElement("usage-account-filter"),
    modelFilter: optionalElement("usage-model-filter"),
    refreshIntervalSelect: optionalElement("usage-refresh-interval-select"),
    accountPanel: optionalElement("usage-account-panel"),
    summaryPanel: optionalElement("usage-summary-panel"),
    trendPanel: optionalElement("usage-trend-panel"),
    detailPanel: optionalElement("usage-detail-panel"),
  };
}

function browserUsageDomPort(): UsageDomPort {
  const elements = queryUsageElements();

  return {
    now: Date.now,
    setInterval: (handler, intervalMs) => window.setInterval(handler, intervalMs),
    clearInterval: (id) => window.clearInterval(Number(id)),
    render: (view) => renderUsage(elements, view),
    bind: (handlers) => {
      const cleanups: Array<() => void> = [];
      const listen = (
        target: EventTarget | null,
        type: string,
        listener: EventListener,
      ) => {
        if (!target) return;
        target.addEventListener(type, listener);
        cleanups.push(() => target.removeEventListener(type, listener));
      };
      const targetElement = (event: Event): HTMLElement | null => (
        event.target instanceof HTMLElement ? event.target : null
      );

      listen(elements.refreshButton, "click", () => handlers.refresh());
      listen(elements.rangeButton, "click", () => handlers.toggleRange());
      listen(elements.rangePopover, "click", (event) => {
        const target = targetElement(event);
        if (!target || target instanceof HTMLInputElement) return;
        const preset = target.closest<HTMLElement>("[data-usage-range-preset]")
          ?.dataset.usageRangePreset;
        if (preset !== undefined) return handlers.rangeAction({ type: "preset", value: preset });
        const field = target.closest<HTMLElement>("[data-usage-range-field]")
          ?.dataset.usageRangeField;
        if (field !== undefined) return handlers.rangeAction({ type: "field", value: field });
        const day = target.closest<HTMLElement>("[data-usage-range-day]")
          ?.dataset.usageRangeDay;
        if (day !== undefined) return handlers.rangeAction({ type: "day", value: day });
        const month = target.closest<HTMLElement>("[data-usage-range-month]")
          ?.dataset.usageRangeMonth;
        if (month !== undefined) return handlers.rangeAction({ type: "month", value: month });
        if (target.closest("[data-usage-range-today]")) {
          handlers.rangeAction({ type: "today" });
        } else if (target.closest("[data-usage-range-cancel]")) {
          handlers.rangeAction({ type: "cancel" });
        } else if (target.closest("[data-usage-range-apply]")) {
          handlers.rangeAction({ type: "apply" });
        }
      });
      listen(elements.rangePopover, "change", (event) => {
        const target = event.target;
        if (!(target instanceof HTMLInputElement)) return;
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
        const target = event.target;
        if (!(target instanceof HTMLInputElement)) return;
        handlers.rangeFocus(target.dataset.usageRangeDate ?? target.dataset.usageRangeTime ?? null);
      });
      listen(elements.scopeSelect, "change", () => handlers.scopeChange(elements.scopeSelect!.value));
      listen(elements.accountFilter, "change", () => handlers.accountChange(elements.accountFilter!.value));
      listen(elements.modelFilter, "change", () => handlers.modelChange(elements.modelFilter!.value));
      listen(elements.refreshIntervalSelect, "change", () => (
        handlers.intervalChange(elements.refreshIntervalSelect!.value)
      ));
      listen(elements.detailPanel, "click", (event) => {
        const button = targetElement(event)?.closest<HTMLButtonElement>("[data-usage-log-page]");
        if (button) handlers.logPage(button.dataset.usageLogPage ?? "", button.disabled);
      });
      listen(document, "click", (event) => {
        const picker = document.getElementById("usage-range-picker");
        handlers.documentClick(Boolean(
          picker && event.target instanceof Node && !picker.contains(event.target),
        ));
      });
      listen(document, "keydown", (event) => {
        handlers.documentKeydown(event instanceof KeyboardEvent ? event.key : "");
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
          if (action.value.trim() === "") return;
          const dayMs = Number(action.value);
          if (Number.isFinite(dayMs)) void workspace.dispatch({ type: "pick-range-day", dayMs });
          return;
        }
        case "month": {
          if (action.value.trim() === "") return;
          const delta = Number(action.value);
          if (Number.isFinite(delta)) void workspace.dispatch({ type: "move-range-month", delta });
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
      } else if (isRangeField(change.field)) {
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
        intervalMs: Number.isFinite(intervalMs) ? intervalMs : 0,
      });
    },
    logPage: (value, disabled) => {
      if (disabled) return;
      const page = Number(value);
      if (!Number.isFinite(page) || page < 1 || page === workspace.snapshot().logsPage) return;
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

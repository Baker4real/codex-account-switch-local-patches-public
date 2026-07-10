import type { Locale } from "../i18n";
import type { OpenAiUsageDashboard, OpenAiUsageQuery, OpenAiUsageRangePreset, OpenAiUsageScope } from "../types";
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

function errorValue(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

export function mountUsageWorkspace(
  dependencies: MountUsageWorkspaceDependencies,
): UsageWorkspace {
  const elements = queryUsageElements();
  const workspace = createUsageWorkspace({
    loadDashboard: dependencies.loadDashboard,
    now: Date.now,
    setInterval: (handler, intervalMs) => window.setInterval(handler, intervalMs),
    clearInterval: (id) => window.clearInterval(Number(id)),
    onChange: (view) => renderUsage(elements, view),
    onError: (error) => dependencies.onError(errorValue(error), true),
  });

  const refresh = () => workspace.dispatch({ type: "refresh", mode: "manual" });
  const refreshAfter = async (event: Parameters<UsageWorkspace["dispatch"]>[0]) => {
    await workspace.dispatch(event);
    await refresh();
  };
  const onRefreshClick = () => { void refresh(); };
  const onRangeClick = () => {
    void workspace.dispatch({
      type: workspace.snapshot().rangePickerOpen ? "close-range-picker" : "open-range-picker",
    });
  };
  const onPopoverClick = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    if (target instanceof HTMLInputElement) return;
    const presetButton = target.closest<HTMLElement>("[data-usage-range-preset]");
    if (presetButton?.dataset.usageRangePreset) {
      void refreshAfter({
        type: "set-range",
        preset: presetButton.dataset.usageRangePreset as OpenAiUsageRangePreset,
      });
      return;
    }
    const field = target.closest<HTMLElement>("[data-usage-range-field]")?.dataset.usageRangeField;
    if (field === "start" || field === "end") {
      void workspace.dispatch({ type: "set-range-draft-field", field });
      return;
    }
    const day = target.closest<HTMLElement>("[data-usage-range-day]")?.dataset.usageRangeDay;
    if (day) {
      const dayMs = Number(day);
      if (Number.isFinite(dayMs)) void workspace.dispatch({ type: "pick-range-day", dayMs });
      return;
    }
    const month = target.closest<HTMLElement>("[data-usage-range-month]")?.dataset.usageRangeMonth;
    if (month) {
      const delta = Number(month);
      if (Number.isFinite(delta)) void workspace.dispatch({ type: "move-range-month", delta });
      return;
    }
    if (target.closest("[data-usage-range-today]")) {
      void workspace.dispatch({ type: "show-range-today" });
    } else if (target.closest("[data-usage-range-cancel]")) {
      void workspace.dispatch({ type: "close-range-picker" });
    } else if (target.closest("[data-usage-range-apply]")) {
      void workspace.dispatch({ type: "apply-range-draft" });
    }
  };
  const onPopoverChange = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    const dateField = target.dataset.usageRangeDate;
    if (dateField === "start" || dateField === "end") {
      void workspace.dispatch({ type: "set-range-draft-date", field: dateField, value: target.value });
      return;
    }
    const timeField = target.dataset.usageRangeTime;
    if (timeField === "start" || timeField === "end") {
      void workspace.dispatch({ type: "set-range-draft-time", field: timeField, value: target.value });
      return;
    }
    if (target.matches("[data-usage-range-live]")) {
      void workspace.dispatch({ type: "set-range-draft-live-end", live: target.checked });
    }
  };
  const onPopoverFocus = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    const field = target.dataset.usageRangeDate ?? target.dataset.usageRangeTime;
    if (field === "start" || field === "end") {
      void workspace.dispatch({ type: "set-range-draft-field", field });
    }
  };
  const onDocumentClick = (event: MouseEvent) => {
    const picker = document.getElementById("usage-range-picker");
    if (
      workspace.snapshot().rangePickerOpen
      && picker
      && event.target instanceof Node
      && !picker.contains(event.target)
    ) void workspace.dispatch({ type: "close-range-picker" });
  };
  const onDocumentKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && workspace.snapshot().rangePickerOpen) {
      void workspace.dispatch({ type: "close-range-picker" });
    }
  };
  const onScopeChange = () => {
    if (!elements.scopeSelect) return;
    void refreshAfter({ type: "set-filters", scope: elements.scopeSelect.value as OpenAiUsageScope });
  };
  const onAccountChange = () => {
    if (!elements.accountFilter) return;
    void refreshAfter({ type: "set-filters", accountGroupKey: elements.accountFilter.value || null });
  };
  const onModelChange = () => {
    if (!elements.modelFilter) return;
    void refreshAfter({ type: "set-filters", model: elements.modelFilter.value || null });
  };
  const onIntervalChange = () => {
    if (!elements.refreshIntervalSelect) return;
    const intervalMs = Number(elements.refreshIntervalSelect.value);
    void workspace.dispatch({
      type: "set-refresh-interval",
      intervalMs: Number.isFinite(intervalMs) ? intervalMs : 0,
    });
  };
  const onDetailClick = (event: Event) => {
    const button = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>("[data-usage-log-page]");
    if (!button || button.disabled) return;
    const page = Number(button.dataset.usageLogPage);
    if (!Number.isFinite(page) || page < 1 || page === workspace.snapshot().logsPage) return;
    void refreshAfter({ type: "set-page", page });
  };

  elements.refreshButton?.addEventListener("click", onRefreshClick);
  elements.rangeButton?.addEventListener("click", onRangeClick);
  elements.rangePopover?.addEventListener("click", onPopoverClick);
  elements.rangePopover?.addEventListener("change", onPopoverChange);
  elements.rangePopover?.addEventListener("focusin", onPopoverFocus);
  elements.scopeSelect?.addEventListener("change", onScopeChange);
  elements.accountFilter?.addEventListener("change", onAccountChange);
  elements.modelFilter?.addEventListener("change", onModelChange);
  elements.refreshIntervalSelect?.addEventListener("change", onIntervalChange);
  elements.detailPanel?.addEventListener("click", onDetailClick);
  document.addEventListener("click", onDocumentClick);
  document.addEventListener("keydown", onDocumentKeydown);

  const disposeWorkspace = workspace.dispose.bind(workspace);
  workspace.dispose = () => {
    elements.refreshButton?.removeEventListener("click", onRefreshClick);
    elements.rangeButton?.removeEventListener("click", onRangeClick);
    elements.rangePopover?.removeEventListener("click", onPopoverClick);
    elements.rangePopover?.removeEventListener("change", onPopoverChange);
    elements.rangePopover?.removeEventListener("focusin", onPopoverFocus);
    elements.scopeSelect?.removeEventListener("change", onScopeChange);
    elements.accountFilter?.removeEventListener("change", onAccountChange);
    elements.modelFilter?.removeEventListener("change", onModelChange);
    elements.refreshIntervalSelect?.removeEventListener("change", onIntervalChange);
    elements.detailPanel?.removeEventListener("click", onDetailClick);
    document.removeEventListener("click", onDocumentClick);
    document.removeEventListener("keydown", onDocumentKeydown);
    disposeWorkspace();
  };

  const context = dependencies.getContext();
  void workspace.dispatch({ type: "set-context", ...context });
  return workspace;
}

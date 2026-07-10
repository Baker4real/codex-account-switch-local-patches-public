import { t, type MessageKey } from "../i18n";
import type {
  OpenAiUsageAccountSummary,
  OpenAiUsageCostBreakdown,
  OpenAiUsageModelSummary,
  OpenAiUsageRangePreset,
  OpenAiUsageRequestLog,
  OpenAiUsageSummary,
} from "../types";
import type { UsageElements } from "./dom";
import type { UsageWorkspaceView } from "./model";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatInteger(locale: UsageWorkspaceView["locale"], value: number): string {
  return new Intl.NumberFormat(locale).format(value);
}

function formatCostMicros(locale: UsageWorkspaceView["locale"], value: number): string {
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 4,
    maximumFractionDigits: 4,
  }).format(value / 1_000_000);
}

function formatUsageCost(
  cost: OpenAiUsageCostBreakdown,
  locale: UsageWorkspaceView["locale"],
): string {
  if (cost.unpriced_tokens > 0 && cost.total_cost_micros === 0) {
    return "N/A";
  }
  if (cost.unpriced_tokens > 0) {
    return `${formatCostMicros(locale, cost.total_cost_micros)} ${t(locale, "usageCostPartial")}`;
  }
  return formatCostMicros(locale, cost.total_cost_micros);
}

function formatRate(locale: UsageWorkspaceView["locale"], value: number | null): string {
  if (value == null) return "--";
  return new Intl.NumberFormat(locale, {
    style: "percent",
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

function formatDateTimeMs(
  value: number | null,
  locale: UsageWorkspaceView["locale"],
): string {
  if (value == null) return "--";
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

const usageRangePresets: readonly OpenAiUsageRangePreset[] = ["today", "1d", "7d", "14d", "30d"];

export function renderUsage(elements: UsageElements, view: UsageWorkspaceView): void {
  const summary = view.summary;
  if (
    !elements.summaryPanel ||
    !elements.trendPanel ||
    !elements.detailPanel
  ) {
    return;
  }
  renderUsageControls(summary);
  if (view.loadError) {
    if (elements.accountPanel) {
      elements.accountPanel.innerHTML = "";
    }
    elements.summaryPanel.innerHTML = `
      <article class="usage-empty-state usage-empty-state--error">
        <strong>${t(view.locale, "usageLoadFailed")}</strong>
        <p>${escapeHtml(view.loadError)}</p>
      </article>
    `;
    elements.trendPanel.innerHTML = "";
    elements.detailPanel.innerHTML = "";
    return;
  }

  if (!view.loaded) {
    if (elements.accountPanel) {
      elements.accountPanel.innerHTML = "";
    }
    elements.summaryPanel.innerHTML = `
      <article class="usage-empty-state">
        <strong>${t(view.locale, "usageLoading")}</strong>
      </article>
    `;
    elements.trendPanel.innerHTML = "";
    elements.detailPanel.innerHTML = "";
    return;
  }

  const accountTable = buildUsageAccountTable(view.accountSummary);
  const hasLayerUsage = (summary?.layer_summaries ?? []).some((layer) => layer.request_count > 0);
  if (!summary || (summary.total_requests === 0 && !hasLayerUsage)) {
    const hasActiveUsageFilter = Boolean(
      view.profileFilter ||
        view.modelFilter ||
        view.statusCodeFilter != null,
    );
    const hasFacetUsage = view.accountSummary.length > 0 || view.modelOptions.length > 0;
    const emptyTitleKey = hasActiveUsageFilter && hasFacetUsage
      ? "usageNoFilteredUsage"
      : "usageNoAttribution";
    const emptyCopyKey = hasActiveUsageFilter && hasFacetUsage
      ? "usageNoFilteredUsageCopy"
      : "usageNoAttributionCopy";
    if (elements.accountPanel) {
      elements.accountPanel.innerHTML = accountTable;
    }
    elements.summaryPanel.innerHTML = `
      <article class="usage-empty-state">
        <strong>${t(view.locale, emptyTitleKey)}</strong>
        <p>${t(view.locale, emptyCopyKey)}</p>
      </article>
    `;
    elements.trendPanel.innerHTML = buildUsageTrendMarkup();
    elements.detailPanel.innerHTML = `
      ${elements.accountPanel ? "" : accountTable}
      ${buildUsageEvidenceMarkup(summary)}
    `;
    return;
  }

  if (elements.accountPanel) {
    elements.accountPanel.innerHTML = accountTable;
  }
  elements.summaryPanel.innerHTML = `
    ${buildUsageLayerSummaryCards(summary)}
    <div class="usage-hero-grid">
      <article class="usage-hero-card usage-hero-card--wide">
        <span>${t(view.locale, "usageLocalTokens")}</span>
        <strong>${formatInteger(view.locale, summary.tokens.real_total_tokens)}</strong>
        <small>${t(view.locale, "usageAttributionSince")}: ${formatDateTimeMs(summary.attribution_since_ms, view.locale)}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(view.locale, summary.cost.unpriced_tokens > 0 ? "usagePricedCost" : "usageNonOfficialEstimate")}</span>
        <strong>${formatUsageCost(summary.cost, view.locale)}</strong>
        <small>${summary.pricing_source} · ${pricingSourceHost(summary.pricing_source_url)} · ${summary.pricing_checked_at}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(view.locale, "usageRequests")}</span>
        <strong>${formatInteger(view.locale, summary.total_requests)}</strong>
        <small>${t(view.locale, "usageUpdatedAt")}: ${formatDateTimeMs(summary.generated_at_ms, view.locale)}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(view.locale, "usageCacheHitRate")}</span>
        <strong>${formatRate(view.locale, summary.cache_hit_rate)}</strong>
        <small>${t(view.locale, "usageCachedInput")}: ${formatInteger(view.locale, summary.tokens.cached_input_tokens)}</small>
      </article>
    </div>

    <div class="usage-token-grid">
      ${buildUsageTokenMetric("usageInput", summary.tokens.input_tokens)}
      ${buildUsageTokenMetric("usageCachedInput", summary.tokens.cached_input_tokens)}
      ${buildUsageTokenMetric("usageOutput", summary.tokens.output_tokens)}
      ${buildUsageTokenMetric("usageReasoning", summary.tokens.reasoning_output_tokens)}
    </div>
  `;

  elements.trendPanel.innerHTML = buildUsageTrendMarkup();
  elements.detailPanel.innerHTML = `
    ${buildUsageLogsTable()}
    ${elements.accountPanel ? "" : accountTable}
    ${buildUsageModelTable(summary)}
    ${buildUsageDataSourceMarkup()}
    ${buildUsageEvidenceMarkup(summary)}
  `;

  function renderUsageControls(summary: OpenAiUsageSummary | null): void {
  if (
    !elements.rangeButton ||
    !elements.rangeLabel ||
    !elements.rangePopover ||
    !elements.scopeSelect ||
    !elements.refreshIntervalSelect ||
    !elements.accountFilter ||
    !elements.modelFilter
  ) {
    return;
  }
  elements.scopeSelect.value = view.scope;
  elements.refreshIntervalSelect.value = String(view.refreshIntervalMs);
  const nowMs = Date.now();
  const rangeLabel = formatUsageRangeLabel(nowMs);
  elements.rangeLabel.textContent = rangeLabel;
  elements.rangeButton.title = rangeLabel;
  elements.rangeButton.setAttribute("aria-expanded", view.rangePickerOpen ? "true" : "false");
  elements.rangeButton.classList.toggle("is-active", view.rangePreset === "custom" || view.rangePickerOpen);
  elements.rangePopover.hidden = !view.rangePickerOpen;
  elements.rangePopover.innerHTML = view.rangePickerOpen
    ? buildUsageRangePopover(nowMs)
    : "";
  renderUsageRefreshButton();

  const accountOptions = view.accountSummary.length > 0
    ? view.accountSummary
    : summary?.accounts ?? [];
  const accounts = new Map<string, string>();
  for (const account of accountOptions) {
    const accountKey = usageAccountFilterKey(account);
    const profiles = usageAccountProfiles(account);
    accounts.set(accountKey, account.account_label ?? profiles.join(", "));
  }
  if (view.profileFilter && !accounts.has(view.profileFilter)) {
    accounts.set(view.profileFilter, view.profileFilter);
  }
  const accountOptionsHtml = [
    `<option value="">${t(view.locale, "usageAllAccounts")}</option>`,
    ...Array.from(accounts.entries()).map(([profile, label]) => (
      `<option value="${escapeHtml(profile)}">${escapeHtml(label)}</option>`
    )),
  ].join("");
  setSelectOptions(elements.accountFilter, accountOptionsHtml, view.profileFilter ?? "");

  const modelOptions = view.modelOptions.length > 0
    ? view.modelOptions
    : summary?.models ?? [];
  const models = new Set(modelOptions.map((model) => model.model));
  if (view.modelFilter) {
    models.add(view.modelFilter);
  }
  const modelOptionsHtml = [
    `<option value="">${t(view.locale, "usageAllModels")}</option>`,
    ...Array.from(models).map((model) => (
      `<option value="${escapeHtml(model)}">${escapeHtml(model)}</option>`
    )),
  ].join("");
  setSelectOptions(elements.modelFilter, modelOptionsHtml, view.modelFilter ?? "");
}

  function formatUsageRangeLabel(nowMs: number): string {
  if (view.rangePreset !== "custom") {
    return usageRangePresetLabel(view.rangePreset);
  }

  const startMs = displayUsageStartDate(nowMs);
  const endMs = displayUsageEndDate(nowMs);
  const startLabel = formatDateTimeMs(startMs, view.locale);
  if (view.liveEnd) {
    return `${startLabel} → ${t(view.locale, "usageLiveEndNow")}`;
  }
  return `${startLabel} - ${formatDateTimeMs(endMs, view.locale)}`;
}

  function usageRangePresetLabel(preset: OpenAiUsageRangePreset): string {
  switch (preset) {
    case "today":
      return t(view.locale, "usageRangeToday");
    case "7d":
      return t(view.locale, "usageRangeLast7Days");
    case "14d":
      return t(view.locale, "usageRangeLast14Days");
    case "30d":
      return t(view.locale, "usageRangeLast30Days");
    case "custom":
      return t(view.locale, "usageRangeCustom");
    case "1d":
    default:
      return t(view.locale, "usageRangeLast24Hours");
  }
}

  function buildUsageRangePopover(nowMs: number): string {
  const draftStart = view.rangeDraftStartMs ?? displayUsageStartDate(nowMs) ?? nowMs - 24 * 60 * 60 * 1000;
  const draftEnd = view.rangeDraftEndMs ?? displayUsageEndDate(nowMs) ?? nowMs;
  const displayMonth = view.rangeDisplayMonthMs ?? startOfLocalMonthMs(draftStart);
  const draftLiveEnd = view.rangeDraftLiveEnd;
  const presets = usageRangePresets.map((preset) => `
    <button
      class="usage-range-preset${view.rangePreset === preset ? " is-active" : ""}"
      type="button"
      data-usage-range-preset="${preset}"
    >${escapeHtml(usageRangePresetLabel(preset))}</button>
  `).join("");
  return `
    <div class="usage-range-presets">${presets}</div>
    <div class="usage-range-layout">
      <div class="usage-range-fields">
        <p class="usage-range-hint">${t(view.locale, "usageCustomRangeHint")}</p>
        ${buildUsageRangeField("start", draftStart, false)}
        ${buildUsageRangeField("end", draftEnd, draftLiveEnd)}
        <label class="usage-range-live">
          <input type="checkbox" data-usage-range-live ${draftLiveEnd ? "checked" : ""} />
          <span>${t(view.locale, "usageLiveEnd")}</span>
        </label>
        ${view.rangeError ? `<p class="usage-range-error">${escapeHtml(view.rangeError)}</p>` : ""}
        <div class="usage-range-actions">
          <button class="pager-pill" type="button" data-usage-range-cancel>${t(view.locale, "cancel")}</button>
          <button class="pager-pill usage-range-apply" type="button" data-usage-range-apply>${t(view.locale, "save")}</button>
        </div>
      </div>
      ${buildUsageCalendar(displayMonth, draftStart, draftEnd)}
    </div>
  `;
}

  function buildUsageRangeField(field: "start" | "end", valueMs: number, liveEnd: boolean): string {
  const active = view.rangeDraftActiveField === field;
  const label = field === "start" ? t(view.locale, "usageStartTime") : t(view.locale, "usageEndTime");
  return `
    <div class="usage-range-field${active ? " is-active" : ""}${liveEnd ? " is-readonly" : ""}" data-usage-range-field="${field}">
      <span>${label}</span>
      <div>
        <input
          type="text"
          inputmode="numeric"
          autocomplete="off"
          spellcheck="false"
          data-usage-range-date="${field}"
          value="${formatDateInput(valueMs)}"
          aria-label="${label} date"
          placeholder="YYYY-MM-DD"
          ${liveEnd ? "readonly" : ""}
        />
        <input
          type="text"
          inputmode="numeric"
          autocomplete="off"
          spellcheck="false"
          data-usage-range-time="${field}"
          value="${formatTimeInput(valueMs)}"
          aria-label="${label} time"
          placeholder="HH:MM"
          ${liveEnd ? "readonly" : ""}
        />
      </div>
    </div>
  `;
}

  function buildUsageCalendar(monthMs: number, startMs: number, endMs: number): string {
  const month = new Date(monthMs);
  const days = getUsageCalendarDays(monthMs);
  const todayMs = startOfLocalDayMs(Date.now());
  const startDayMs = startOfLocalDayMs(startMs);
  const endDayMs = startOfLocalDayMs(endMs);
  const weekdays = Array.from({ length: 7 }, (_, index) => (
    new Intl.DateTimeFormat(view.locale, { weekday: "narrow" }).format(new Date(2024, 0, 7 + index))
  ));
  const weekdayMarkup = weekdays.map((weekday) => `<span>${escapeHtml(weekday)}</span>`).join("");
  const dayMarkup = days.map((dayMs) => {
    const day = new Date(dayMs);
    const currentMonth = day.getMonth() === month.getMonth() && day.getFullYear() === month.getFullYear();
    const isToday = dayMs === todayMs;
    const isStart = dayMs === startDayMs;
    const isEnd = dayMs === endDayMs;
    const inRange = dayMs >= startDayMs && dayMs <= endDayMs;
    const classes = [
      "usage-calendar-day",
      currentMonth ? null : "is-outside",
      isToday ? "is-today" : null,
      inRange && !isStart && !isEnd ? "is-in-range" : null,
      isStart || isEnd ? "is-endpoint" : null,
    ].filter((value): value is string => value != null).join(" ");
    return `
      <button
        class="${classes}"
        type="button"
        data-usage-range-day="${dayMs}"
        aria-pressed="${isStart || isEnd ? "true" : "false"}"
      >${day.getDate()}</button>
    `;
  }).join("");
  return `
    <div class="usage-range-calendar">
      <div class="usage-calendar-header">
        <button type="button" class="usage-calendar-nav" data-usage-range-month="-1" aria-label="${t(view.locale, "previous")}"></button>
        <button type="button" class="usage-calendar-title" data-usage-range-today>${escapeHtml(formatUsageCalendarMonth(monthMs))}</button>
        <button type="button" class="usage-calendar-nav usage-calendar-nav--next" data-usage-range-month="1" aria-label="${t(view.locale, "next")}"></button>
      </div>
      <div class="usage-calendar-weekdays">${weekdayMarkup}</div>
      <div class="usage-calendar-grid">${dayMarkup}</div>
    </div>
  `;
}

  function renderUsageRefreshButton(): void {
  const button = elements.refreshButton;
  if (!button) {
    return;
  }
  const busy = view.refreshInFlight;
  button.textContent = view.refreshQueued
    ? t(view.locale, "usageRefreshQueued")
    : busy
      ? t(view.locale, "usageRefreshing")
      : t(view.locale, "usageRefresh");
  button.classList.toggle("usage-refresh-button--busy", busy);
  button.setAttribute("aria-busy", busy ? "true" : "false");
  if (busy) {
    button.setAttribute("aria-disabled", "true");
  } else {
    button.removeAttribute("aria-disabled");
  }
}

  function displayUsageStartDate(nowMs: number): number | null {
  if (view.rangePreset === "custom") {
    return view.customStartMs ?? nowMs - 24 * 60 * 60 * 1000;
  }
  return presetUsageStartDate(nowMs);
}

  function displayUsageEndDate(nowMs: number): number | null {
  if (view.rangePreset === "custom") {
    if (view.liveEnd) {
      return view.lastResolvedEndMs ?? nowMs;
    }
    return view.customEndMs ?? nowMs;
  }
  return view.lastResolvedEndMs ?? nowMs;
}

  function presetUsageStartDate(nowMs: number): number {
  switch (view.rangePreset) {
    case "today":
      return startOfLocalDayMs(nowMs);
    case "7d":
      return presetLookbackStartMs(7, nowMs);
    case "14d":
      return presetLookbackStartMs(14, nowMs);
    case "30d":
      return presetLookbackStartMs(30, nowMs);
    case "1d":
    default:
      return nowMs - 24 * 60 * 60 * 1000;
  }
}

  function presetLookbackStartMs(dayCount: number, nowMs: number): number {
  return startOfLocalDayMs(nowMs - (dayCount - 1) * 24 * 60 * 60 * 1000);
}

  function startOfLocalDayMs(valueMs: number): number {
  const date = new Date(valueMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

  function startOfLocalMonthMs(valueMs: number): number {
  const date = new Date(valueMs);
  return new Date(date.getFullYear(), date.getMonth(), 1).getTime();
}

  function getUsageCalendarDays(monthMs: number): number[] {
  const first = new Date(monthMs);
  const gridStart = new Date(first);
  gridStart.setDate(first.getDate() - first.getDay());
  return Array.from({ length: 42 }, (_, index) => {
    const day = new Date(gridStart);
    day.setDate(gridStart.getDate() + index);
    return startOfLocalDayMs(day.getTime());
  });
}

  function formatUsageCalendarMonth(valueMs: number): string {
  return new Intl.DateTimeFormat(view.locale, {
    year: "numeric",
    month: "long",
  }).format(new Date(valueMs));
}

  function formatDateInput(valueMs: number): string {
  const date = new Date(valueMs);
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

  function formatTimeInput(valueMs: number): string {
  const date = new Date(valueMs);
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

  function setSelectOptions(select: HTMLSelectElement, optionsHtml: string, value: string): void {
  if (select.innerHTML !== optionsHtml) {
    select.innerHTML = optionsHtml;
  }
  select.value = value;
}

  function buildUsageLayerSummaryCards(summary: OpenAiUsageSummary): string {
  const strict = findUsageLayer(summary, "strict_proven");
  const legacy = findUsageLayer(summary, "legacy_profile_attributed");
  const candidate = findUsageLayer(summary, "candidate_unattributed");
  return `
    <div class="usage-hero-grid">
      ${buildUsageLayerCard("usageStrictProven", strict)}
      ${buildUsageLayerCard("usageLegacyProfileAttributed", legacy)}
      ${buildUsageLayerCard("usageCandidateUnattributed", candidate)}
    </div>
  `;
}

  function findUsageLayer(summary: OpenAiUsageSummary, ledgerLayer: string) {
  return summary.layer_summaries.find((layer) => layer.ledger_layer === ledgerLayer) ?? {
    ledger_layer: ledgerLayer,
    request_count: 0,
    tokens: {
      input_tokens: 0,
      cached_input_tokens: 0,
      output_tokens: 0,
      reasoning_output_tokens: 0,
      real_total_tokens: 0,
    },
    cost: {
      input_cost_micros: 0,
      cached_input_cost_micros: 0,
      output_cost_micros: 0,
      total_cost_micros: 0,
      unpriced_tokens: 0,
    },
  };
}

  function buildUsageLayerCard(labelKey: MessageKey, layer: ReturnType<typeof findUsageLayer>): string {
  return `
    <article class="usage-hero-card">
      <span>${t(view.locale, labelKey)}</span>
      <strong>${formatInteger(view.locale, layer.tokens.real_total_tokens)}</strong>
      <small>${formatInteger(view.locale, layer.request_count)} ${t(view.locale, "usageRequests")} · ${formatUsageCost(layer.cost, view.locale)}</small>
    </article>
  `;
}

  function formatUsageLayerLabel(layer: string): string {
  switch (layer) {
    case "strict_proven":
      return t(view.locale, "usageStrictProven");
    case "legacy_profile_attributed":
      return t(view.locale, "usageLegacyProfileAttributed");
    case "mixed_attributed":
      return t(view.locale, "usageMixedAttributed");
    case "candidate_unattributed":
      return t(view.locale, "usageCandidateUnattributed");
    case "rejected_invalid":
      return t(view.locale, "usageRejectedInvalid");
    case "superseded":
      return t(view.locale, "usageSupersededLayer");
    default:
      return layer;
  }
}

  function formatUsageAttributionKind(kind: string): string {
  switch (kind) {
    case "strict_account":
      return t(view.locale, "usageStrictAccount");
    case "legacy_profile":
      return t(view.locale, "usageLegacyProfile");
    case "mixed_account":
      return t(view.locale, "usageMixedAccount");
    case "unattributed":
      return t(view.locale, "usageUnattributedEvidence");
    default:
      return kind;
  }
}

  function buildUsageTokenMetric(labelKey: MessageKey, value: number): string {
  return `
    <article class="usage-token-card">
      <span>${t(view.locale, labelKey)}</span>
      <strong>${formatInteger(view.locale, value)}</strong>
    </article>
  `;
}

  function buildUsageAccountTable(accounts: readonly OpenAiUsageAccountSummary[]): string {
  if (accounts.length === 0) {
    return "";
  }
  const rows = accounts.map((account) => {
    const profiles = usageAccountProfiles(account);
    const selectedClass = view.profileFilter === usageAccountFilterKey(account)
      ? ` class="usage-table-row--selected"`
      : "";
    return `
    <tr${selectedClass}>
      <td>${escapeHtml(profiles.join(", "))}</td>
      <td>${escapeHtml(account.account_label ?? "--")}</td>
      <td>${escapeHtml(formatUsageLayerLabel(account.ledger_layer))}</td>
      <td>${escapeHtml(formatUsageAttributionKind(account.attribution_kind))}</td>
      <td>${escapeHtml(account.official_source)}</td>
      <td>${formatInteger(view.locale, account.request_count)}</td>
      <td>${formatInteger(view.locale, account.tokens.real_total_tokens)}</td>
      <td>${formatUsageCost(account.cost, view.locale)}</td>
    </tr>
  `;
  }).join("");
  return `
    <article class="usage-table-card">
      <h3>${t(view.locale, "usageAccounts")}</h3>
      <div class="usage-table-scroll">
        <table class="usage-table">
          <thead>
            <tr>
              <th>${t(view.locale, "usageProfile")}</th>
              <th>${t(view.locale, "usageAccount")}</th>
              <th>${t(view.locale, "usageLedgerLayer")}</th>
              <th>${t(view.locale, "usageAttributionKind")}</th>
              <th>${t(view.locale, "usageOfficialSource")}</th>
              <th>${t(view.locale, "usageRequests")}</th>
              <th>${t(view.locale, "usageTokens")}</th>
              <th>${t(view.locale, "usageCost")}</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    </article>
  `;
}

  function usageAccountProfiles(account: OpenAiUsageAccountSummary): string[] {
  const profiles = account.profiles?.filter((profile) => profile.trim().length > 0) ?? [];
  return profiles.length > 0 ? profiles : [account.profile];
}

  function usageAccountFilterKey(account: OpenAiUsageAccountSummary): string {
  return account.account_group_key ?? account.profile;
}

  function buildUsageModelTable(summary: OpenAiUsageSummary): string {
  const rows = summary.models.map((model) => `
    <tr>
      <td class="usage-table-cell--model">${buildUsageModelSummaryCell(model)}</td>
      <td class="usage-table-cell--evidence">${buildUsageModelSummaryEvidenceCell(model)}</td>
      <td>${formatInteger(view.locale, model.request_count)}</td>
      <td>${formatInteger(view.locale, model.tokens.input_tokens)}</td>
      <td>${formatInteger(view.locale, model.tokens.cached_input_tokens)}</td>
      <td>${formatInteger(view.locale, model.tokens.output_tokens)}</td>
      <td>${formatUsageCost(model.cost, view.locale)}</td>
      <td>${escapeHtml(formatUsagePricingCell(model.priced, model.pricing_tier, model.pricing_source_url))}</td>
    </tr>
  `).join("");
  return `
    <article class="usage-table-card">
      <h3>${t(view.locale, "usageModels")}</h3>
      <div class="usage-table-scroll">
        <table class="usage-table usage-model-table">
          <thead>
            <tr>
              <th>${t(view.locale, "usageModel")}</th>
              <th>${t(view.locale, "usageRequestEvidence")}</th>
              <th>${t(view.locale, "usageRequests")}</th>
              <th>${t(view.locale, "usageInput")}</th>
              <th>${t(view.locale, "usageCachedInput")}</th>
              <th>${t(view.locale, "usageOutput")}</th>
              <th>${t(view.locale, "usageCost")}</th>
              <th>${t(view.locale, "usagePricing")}</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    </article>
  `;
}

  function buildUsageModelSummaryCell(model: OpenAiUsageModelSummary): string {
  return `
    <div class="usage-table-stack usage-table-stack--model">
      <strong title="${escapeHtml(model.model)}">${escapeHtml(model.model)}</strong>
      <small title="${escapeHtml(formatUsageOptional(model.requested_model))}">${t(view.locale, "usageRequestedModel")}: ${escapeHtml(formatUsageOptional(model.requested_model))}</small>
      <small title="${escapeHtml(formatUsageOptional(model.pricing_model))}">${t(view.locale, "usagePricingModel")}: ${escapeHtml(formatUsageOptional(model.pricing_model))}</small>
    </div>
  `;
}

  function buildUsageModelSummaryEvidenceCell(model: OpenAiUsageModelSummary): string {
  return `
    <div class="usage-table-stack usage-table-stack--evidence">
      <strong>${t(view.locale, "usageReasoningEffort")}: ${escapeHtml(formatUsageOptional(model.model_reasoning_effort))}</strong>
      <small>${t(view.locale, "usageVerbosity")}: ${escapeHtml(formatUsageOptional(model.model_verbosity))}</small>
    </div>
  `;
}

  function formatUsageOptional(value: string | null | undefined): string {
  if (value == null || value.trim() === "") {
    return t(view.locale, "usageUnknown");
  }
  if (value === "mixed") {
    return t(view.locale, "usageMixed");
  }
  return value;
}

  function formatUsagePricingCell(
  priced: boolean,
  tier: string | null | undefined,
  sourceUrl: string | null | undefined,
): string {
  const status = priced ? t(view.locale, "usagePriced") : t(view.locale, "usageNotPriced");
  const tierLabel = formatUsageOptional(tier);
  const sourceLabel = sourceUrl && sourceUrl !== "mixed"
    ? pricingSourceHost(sourceUrl)
    : formatUsageOptional(sourceUrl);
  return `${status} · ${tierLabel} · ${sourceLabel}`;
}

  function pricingSourceHost(sourceUrl: string): string {
  try {
    return new URL(sourceUrl).hostname;
  } catch {
    return sourceUrl;
  }
}

  function buildUsageTrendMarkup(): string {
  const points = view.trends;
  const trendBody = !view.loaded
    ? `<p class="usage-muted">${t(view.locale, "usageLoading")}</p>`
    : points.length === 0
      ? `<p class="usage-muted">${t(view.locale, "usageNoAttribution")}</p>`
      : buildUsageLineChart(points);
  return `
    <article class="usage-table-card usage-trend-card">
      <div class="usage-card-title-row">
        <h3>${t(view.locale, "usageTrends")}</h3>
        <span>${t(view.locale, "usageRefreshInterval")}: ${formatRefreshInterval(view.refreshIntervalMs)}</span>
      </div>
      ${trendBody}
    </article>
  `;
}

type UsageTrendPoint = typeof view.trends[number];

type UsageTrendSeries = {
  key: MessageKey;
  className: string;
  color: string;
  fillId?: string;
  dashed?: boolean;
  scale: "tokens" | "cost";
  value: (point: UsageTrendPoint) => number;
};

  function buildUsageLineChart(points: readonly UsageTrendPoint[]): string {
  const width = 920;
  const height = 350;
  const pad = { top: 30, right: 78, bottom: 42, left: 58 };
  const innerWidth = width - pad.left - pad.right;
  const innerHeight = height - pad.top - pad.bottom;
  const plotTop = pad.top + 24;
  const plotBottom = pad.top + innerHeight;
  const plotHeight = plotBottom - plotTop;
  const tokenSeries: UsageTrendSeries[] = [
    {
      key: "usageInput",
      className: "input",
      color: "#2563eb",
      fillId: "usageColorInput",
      scale: "tokens",
      value: (point) => point.tokens.input_tokens,
    },
    {
      key: "usageCachedInput",
      className: "cached",
      color: "#0f766e",
      fillId: "usageColorCached",
      scale: "tokens",
      value: (point) => point.tokens.cached_input_tokens,
    },
    {
      key: "usageOutput",
      className: "output",
      color: "#b45309",
      fillId: "usageColorOutput",
      scale: "tokens",
      value: (point) => point.tokens.output_tokens,
    },
    {
      key: "usageReasoning",
      className: "reasoning",
      color: "#be123c",
      fillId: "usageColorReasoning",
      scale: "tokens",
      value: (point) => point.tokens.reasoning_output_tokens,
    },
  ];
  const costSeries: UsageTrendSeries = {
    key: "usageCost",
    className: "cost",
    color: "#4b5563",
    dashed: true,
    scale: "cost",
    value: (point) => point.cost.total_cost_micros,
  };
  const allSeries = [...tokenSeries, costSeries];
  const tokenMax = Math.max(1, ...tokenSeries.flatMap((series) => points.map(series.value)));
  const costMax = Math.max(1, ...points.map(costSeries.value));
  const xFor = (index: number) => pad.left + (points.length === 1 ? innerWidth / 2 : (innerWidth * index) / (points.length - 1));
  const yForTokens = (value: number) => plotBottom - (value / tokenMax) * plotHeight;
  const yForCost = (value: number) => plotBottom - (value / costMax) * plotHeight;
  const isHourly = usageTrendRangeDurationMs() <= 24 * 60 * 60 * 1000;
  const gradients = tokenSeries.map((series) => `
    <linearGradient id="${series.fillId}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="5%" stop-color="${series.color}" stop-opacity="0.20"></stop>
      <stop offset="95%" stop-color="${series.color}" stop-opacity="0"></stop>
    </linearGradient>
  `).join("");
  const grid = [0, 0.25, 0.5, 0.75, 1].map((tick) => {
    const y = pad.top + innerHeight - tick * innerHeight;
    return `<line class="usage-chart-grid-line" x1="${pad.left}" y1="${y}" x2="${width - pad.right}" y2="${y}"></line>`;
  }).join("");
  const areaPaths = tokenSeries.map((series) => {
    const coords = points.map((point, index) => ({
      x: xFor(index),
      y: yForTokens(series.value(point)),
    }));
    const line = buildSmoothUsagePath(coords, { minY: plotTop, maxY: plotBottom });
    const area = `${line} L ${coords[coords.length - 1].x.toFixed(2)} ${(pad.top + innerHeight).toFixed(2)} L ${coords[0].x.toFixed(2)} ${(pad.top + innerHeight).toFixed(2)} Z`;
    return `<path class="usage-chart-area usage-chart-area--${series.className}" d="${area}" fill="url(#${series.fillId})"></path>`;
  }).join("");
  const paths = allSeries.map((series) => {
    const yFor = series.scale === "tokens" ? yForTokens : yForCost;
    const d = buildSmoothUsagePath(points.map((point, index) => ({
      x: xFor(index),
      y: yFor(series.value(point)),
    })), { minY: plotTop, maxY: plotBottom });
    return `<path class="usage-chart-line usage-chart-line--${series.className}${series.dashed ? " usage-chart-line--dashed" : ""}" d="${d}" style="--series-color:${series.color}"></path>`;
  }).join("");
  const markers = allSeries.map((series) => {
    const yFor = series.scale === "tokens" ? yForTokens : yForCost;
    return points.map((point, index) => `
      <circle class="usage-chart-point usage-chart-point--${series.className}" cx="${xFor(index).toFixed(2)}" cy="${yFor(series.value(point)).toFixed(2)}" r="3.5" style="--series-color:${series.color}">
        <title>${escapeHtml(buildUsageTrendTooltipRows(point, isHourly).join("\n"))}</title>
      </circle>
    `).join("");
  }).join("");
  const hoverLayers = points.map((point, index) => buildUsageTrendHoverLayer({
    point,
    index,
    x: xFor(index),
    width,
    height,
    pad,
    hitLeft: index === 0 ? pad.left : (xFor(index - 1) + xFor(index)) / 2,
    hitRight: index === points.length - 1 ? width - pad.right : (xFor(index) + xFor(index + 1)) / 2,
    isHourly,
  })).join("");
  const axisLabels = buildUsageTrendAxisLabels(points, isHourly);
  const legend = allSeries.map((series) => `
    <span class="usage-chart-legend-item">
      <i style="--series-color:${series.color}"></i>
      ${t(view.locale, series.key)}
    </span>
  `).join("");
  return `
    <div class="usage-chart-wrap">
      <svg class="usage-chart" viewBox="0 0 ${width} ${height}" role="img" aria-label="${escapeHtml(t(view.locale, "usageTrends"))}" preserveAspectRatio="none">
        <defs>${gradients}</defs>
        ${grid}
        <text class="usage-chart-axis-label" x="${pad.left}" y="18">${escapeHtml(formatCompactAxisValue(tokenMax))} ${escapeHtml(t(view.locale, "usageTokens"))}</text>
        <text class="usage-chart-axis-label usage-chart-axis-label--right" x="${width - pad.right}" y="18">${escapeHtml(formatCostMicros(view.locale, costMax))}</text>
        ${areaPaths}
        ${paths}
        ${markers}
        ${hoverLayers}
      </svg>
      <div class="usage-chart-axis-row">${axisLabels}</div>
      <div class="usage-chart-legend">${legend}</div>
    </div>
  `;
}

  function buildSmoothUsagePath(
  points: Array<{ x: number; y: number }>,
  bounds?: { minY: number; maxY: number },
): string {
  if (points.length === 0) {
    return "";
  }
  if (points.length === 1) {
    return `M ${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`;
  }
  const path = [`M ${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`];
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index];
    const next = points[index + 1];
    const previous = points[index - 1] ?? current;
    const after = points[index + 2] ?? next;
    const cp1x = current.x + (next.x - previous.x) / 6;
    const cp1y = clampUsageChartValue(current.y + (next.y - previous.y) / 6, bounds);
    const cp2x = next.x - (after.x - current.x) / 6;
    const cp2y = clampUsageChartValue(next.y - (after.y - current.y) / 6, bounds);
    path.push(`C ${cp1x.toFixed(2)} ${cp1y.toFixed(2)}, ${cp2x.toFixed(2)} ${cp2y.toFixed(2)}, ${next.x.toFixed(2)} ${next.y.toFixed(2)}`);
  }
  return path.join(" ");
}

  function clampUsageChartValue(value: number, bounds?: { minY: number; maxY: number }): number {
  if (!bounds) {
    return value;
  }
  return Math.min(bounds.maxY, Math.max(bounds.minY, value));
}

  function buildUsageTrendAxisLabels(points: readonly UsageTrendPoint[], isHourly: boolean): string {
  const indexes = Array.from(new Set([
    0,
    Math.floor((points.length - 1) / 2),
    points.length - 1,
  ])).filter((index) => index >= 0 && index < points.length);
  return indexes.map((index) => `
    <span>${escapeHtml(formatTrendLabel(points[index].bucket_start_ms, isHourly))}</span>
  `).join("");
}

  function buildUsageTrendHoverLayer(input: {
  point: UsageTrendPoint;
  index: number;
  x: number;
  width: number;
  height: number;
  pad: { top: number; right: number; bottom: number; left: number };
  hitLeft: number;
  hitRight: number;
  isHourly: boolean;
}): string {
  const tooltipWidth = 214;
  const rowHeight = 17;
  const rows = buildUsageTrendTooltipRows(input.point, input.isHourly);
  const tooltipHeight = 24 + rows.length * rowHeight;
  const tooltipX = input.x > input.width - input.pad.right - tooltipWidth
    ? input.x - tooltipWidth - 12
    : input.x + 12;
  const tooltipY = Math.max(input.pad.top + 4, Math.min(input.height - input.pad.bottom - tooltipHeight, input.pad.top + 10));
  const rowMarkup = rows.map((row, rowIndex) => `
    <text class="usage-chart-tooltip-text${rowIndex === 0 ? " usage-chart-tooltip-title" : ""}" x="${tooltipX + 12}" y="${tooltipY + 20 + rowIndex * rowHeight}">
      ${escapeHtml(row)}
    </text>
  `).join("");
  return `
    <g class="usage-chart-hover">
      <rect class="usage-chart-hit-rect" x="${input.hitLeft.toFixed(2)}" y="${input.pad.top}" width="${Math.max(16, input.hitRight - input.hitLeft).toFixed(2)}" height="${input.height - input.pad.top - input.pad.bottom}"></rect>
      <line class="usage-chart-hover-line" x1="${input.x.toFixed(2)}" y1="${input.pad.top}" x2="${input.x.toFixed(2)}" y2="${input.height - input.pad.bottom}"></line>
      <g class="usage-chart-tooltip">
        <rect x="${tooltipX}" y="${tooltipY}" width="${tooltipWidth}" height="${tooltipHeight}" rx="8"></rect>
        ${rowMarkup}
      </g>
    </g>
  `;
}

  function buildUsageTrendTooltipRows(point: UsageTrendPoint, isHourly: boolean): string[] {
  return [
    formatTrendLabel(point.bucket_start_ms, isHourly),
    `${t(view.locale, "usageRequests")}: ${formatInteger(view.locale, point.request_count)}`,
    `${t(view.locale, "usageInput")}: ${formatInteger(view.locale, point.tokens.input_tokens)}`,
    `${t(view.locale, "usageCachedInput")}: ${formatInteger(view.locale, point.tokens.cached_input_tokens)}`,
    `${t(view.locale, "usageOutput")}: ${formatInteger(view.locale, point.tokens.output_tokens)}`,
    `${t(view.locale, "usageReasoning")}: ${formatInteger(view.locale, point.tokens.reasoning_output_tokens)}`,
    `${t(view.locale, "usageCost")}: ${formatUsageCost(point.cost, view.locale)}`,
  ];
}

  function usageTrendRangeDurationMs(): number {
  const nowMs = Date.now();
  const startMs = displayUsageStartDate(nowMs) ?? nowMs - 24 * 60 * 60 * 1000;
  const endMs = displayUsageEndDate(nowMs) ?? nowMs;
  return Math.max(0, endMs - startMs);
}

  function formatCompactAxisValue(value: number): string {
  if (value >= 1000) {
    return `${Math.round(value / 1000)}k`;
  }
  return formatInteger(view.locale, value);
}

  function formatTrendLabel(value: number, isHourly = true): string {
  return new Intl.DateTimeFormat(view.locale, {
    month: "2-digit",
    day: "2-digit",
    ...(isHourly ? { hour: "2-digit", minute: "2-digit" } : {}),
  }).format(new Date(value));
}

  function formatRefreshInterval(value: number): string {
  return value > 0 ? `${value / 1000}s` : t(view.locale, "usageRefreshOff");
}

  function buildUsageLogsTable(): string {
  const page = view.logs;
  const currentPage = view.loading
    ? view.logsPage
    : page?.page ?? view.logsPage;
  const pageSize = page?.page_size ?? view.logsPageSize;
  const total = page?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const rows = view.loading
    ? `<tr><td colspan="10" class="usage-table-loading">${t(view.locale, "usageLoading")}</td></tr>`
    : (page?.data ?? []).map((log: OpenAiUsageRequestLog) => `
    <tr>
      <td><code>${escapeHtml(log.request_id)}</code></td>
      <td>${formatDateTimeMs(log.created_at_ms, view.locale)}</td>
      <td class="usage-table-cell--account" title="${escapeHtml(log.account_label ?? log.profile)}">${escapeHtml(log.account_label ?? log.profile)}</td>
      <td>${escapeHtml(formatUsageLayerLabel(log.ledger_layer))}</td>
      <td class="usage-table-cell--model">${buildUsageModelEvidenceCell(log)}</td>
      <td class="usage-table-cell--evidence">${buildUsageRequestEvidenceCell(log)}</td>
      <td>${formatInteger(view.locale, log.tokens.real_total_tokens)}</td>
      <td>${formatUsageCost(log.cost, view.locale)}</td>
      <td>${escapeHtml(formatUsagePricingCell(log.priced, log.pricing_tier, log.pricing_source_url))}</td>
      <td>${escapeHtml(log.data_source)}</td>
    </tr>
  `).join("");
  const pagerDisabled = view.loading;
  return `
    <article class="usage-table-card">
      <div class="usage-card-title-row">
        <h3>${t(view.locale, "usageRequestLogs")}</h3>
        <div class="usage-log-pager">
          <button class="pager-pill" type="button" data-usage-log-page="${Math.max(1, currentPage - 1)}" ${currentPage <= 1 || pagerDisabled ? "disabled" : ""}>${t(view.locale, "previous")}</button>
          <span>${currentPage} / ${totalPages}${view.loading ? ` · ${t(view.locale, "usageLoading")}` : ""}</span>
          <button class="pager-pill" type="button" data-usage-log-page="${Math.min(totalPages, currentPage + 1)}" ${currentPage >= totalPages || pagerDisabled ? "disabled" : ""}>${t(view.locale, "next")}</button>
        </div>
      </div>
      <div class="usage-table-scroll">
        <table class="usage-table usage-log-table">
          <thead>
            <tr>
              <th>${t(view.locale, "usageRequestId")}</th>
              <th>${t(view.locale, "usageCreatedAt")}</th>
              <th>${t(view.locale, "usageAccount")}</th>
              <th>${t(view.locale, "usageLedgerLayer")}</th>
              <th>${t(view.locale, "usageModel")}</th>
              <th>${t(view.locale, "usageRequestEvidence")}</th>
              <th>${t(view.locale, "usageTokens")}</th>
              <th>${t(view.locale, "usageCost")}</th>
              <th>${t(view.locale, "usagePricing")}</th>
              <th>${t(view.locale, "usageDataSource")}</th>
            </tr>
          </thead>
          <tbody>${rows || `<tr><td colspan="10">${t(view.locale, "usageNoAttribution")}</td></tr>`}</tbody>
        </table>
      </div>
    </article>
  `;
}

  function buildUsageModelEvidenceCell(log: OpenAiUsageRequestLog): string {
  return `
    <div class="usage-table-stack usage-table-stack--model">
      <strong title="${escapeHtml(log.model)}">${escapeHtml(log.model)}</strong>
      <small title="${escapeHtml(formatUsageOptional(log.requested_model))}">${t(view.locale, "usageRequestedModel")}: ${escapeHtml(formatUsageOptional(log.requested_model))}</small>
      <small title="${escapeHtml(formatUsageOptional(log.pricing_model))}">${t(view.locale, "usagePricingModel")}: ${escapeHtml(formatUsageOptional(log.pricing_model))}</small>
    </div>
  `;
}

  function buildUsageRequestEvidenceCell(log: OpenAiUsageRequestLog): string {
  return `
    <div class="usage-table-stack usage-table-stack--evidence">
      <strong>${t(view.locale, "usageReasoningEffort")}: ${escapeHtml(formatUsageOptional(log.model_reasoning_effort))}</strong>
      <small>${t(view.locale, "usageVerbosity")}: ${escapeHtml(formatUsageOptional(log.model_verbosity))}</small>
      <small>${escapeHtml(formatUsageDataQuality(log.usage_confidence))}</small>
    </div>
  `;
}

  function formatUsageDataQuality(value: string): string {
  switch (value) {
    case "proven":
      return t(view.locale, "usageQualityProven");
    case "high":
      return t(view.locale, "usageQualityHigh");
    case "low":
      return t(view.locale, "usageQualityLow");
    case "unproven":
      return t(view.locale, "usageQualityUnproven");
    default:
      return value;
  }
}

  function buildUsageDataSourceMarkup(): string {
  const rows = view.dataSources.map((source) => `
    <span>
      <em>${escapeHtml(source.data_source)}</em>
      <strong>${formatInteger(view.locale, source.request_count)}</strong>
      <small>${formatUsageCost(source.cost, view.locale)}</small>
    </span>
  `).join("");
  return `
    <article class="usage-evidence-card">
      <h3>${t(view.locale, "usageDataSources")}</h3>
      <div class="usage-evidence-grid">${rows || `<span><em>${t(view.locale, "usageNoAttribution")}</em><strong>0</strong></span>`}</div>
    </article>
  `;
}

  function buildUsageEvidenceMarkup(summary: OpenAiUsageSummary | null): string {
  const exclusions = summary?.exclusions ?? {
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
  };
  const items: Array<[MessageKey, number]> = [
    ["usageStrictProven", exclusions.strict_proven_events],
    ["usageLegacyProfileAttributed", exclusions.legacy_profile_attributed_events],
    ["usageCandidateUnattributed", exclusions.candidate_unattributed_events],
    ["usageRejectedInvalid", exclusions.rejected_invalid_events],
    ["usageAcceptedEvents", exclusions.accepted_events],
    ["usageCandidateEvents", exclusions.candidate_events],
    ["usageRejectedEvents", exclusions.rejected_events],
    ["usageSupersededEvents", exclusions.superseded_events],
    ["usageDuplicateEvents", exclusions.duplicate_events],
    ["usageNonOfficialBaseUrl", exclusions.non_official_base_url],
    ["usageUnprovenHistory", exclusions.unproven_history],
    ["usageUnattributed", exclusions.unattributed],
    ["usageMissingModel", exclusions.missing_model],
    ["usageInvalidDelta", exclusions.invalid_delta],
    ["usageMalformedLines", exclusions.malformed_lines],
  ];
  return `
    <article class="usage-evidence-card">
      <h3>${t(view.locale, "usageDataQuality")}</h3>
      <div class="usage-evidence-grid">
        ${items.map(([label, value]) => `
          <span>
            <em>${t(view.locale, label)}</em>
            <strong>${formatInteger(view.locale, value)}</strong>
          </span>
        `).join("")}
      </div>
    </article>
  `;
}
}

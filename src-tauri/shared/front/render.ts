import type {
  CurrentCard,
  DashboardViewModel,
  OpenAiUsageAccountSummary,
  OpenAiUsageCostBreakdown,
  OpenAiUsageRequestLog,
  OpenAiUsageSummary,
  PagingInfo,
  ProfileCard,
  QuotaConfidence,
  QuotaProvenance,
  QuotaSource,
  QuotaSummary,
  QuotaWindow,
  RefreshTokenHealth,
  RefreshTokenStatus,
  ShellRoute,
  UpdateCheckResponse,
} from "@front-shared/types";
import { t, type MessageKey } from "@front-shared/i18n";
import { state } from "@front-shared/state";
import { getThemeOption, isThemeId } from "@front-shared/theme";

const isWindowsUiTarget = __CODEX_UI_TARGET__ === "windows";
const shellRoutes: readonly ShellRoute[] = ["dashboard", "profiles", "usage", "settings", "guide"];
const LOW_QUOTA_THRESHOLD_PERCENT = 20;

function isShellRoute(value: string): value is ShellRoute {
  return shellRoutes.includes(value as ShellRoute);
}

export function routeFromLocation(): ShellRoute {
  const hash = window.location.hash.replace(/^#/, "");
  return isShellRoute(hash) ? hash : "dashboard";
}

function requiredElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing required element: ${id}`);
  }
  return element as T;
}

function optionalElement<T extends HTMLElement>(id: string): T | null {
  const element = document.getElementById(id);
  return element instanceof HTMLElement ? element as T : null;
}

const hasDeleteProfileUi = document.getElementById("delete-profile-dialog") instanceof HTMLDialogElement;

export const elements = {
  profilesHeading: requiredElement<HTMLHeadingElement>("profiles-heading"),
  profilesGrid: requiredElement<HTMLDivElement>("profiles-grid"),
  pageIndicator: requiredElement<HTMLSpanElement>("page-indicator"),
  previousPageButton: requiredElement<HTMLButtonElement>("previous-page-button"),
  nextPageButton: requiredElement<HTMLButtonElement>("next-page-button"),
  currentSectionHeading: requiredElement<HTMLHeadingElement>("current-section-heading"),
  currentTitle: requiredElement<HTMLHeadingElement>("current-title"),
  currentPlan: requiredElement<HTMLParagraphElement>("current-plan"),
  currentQuotaPanel: requiredElement<HTMLDivElement>("current-quota-panel"),
  currentLoginButton: requiredElement<HTMLButtonElement>("current-login-button"),
  openCurrentFolderButton: requiredElement<HTMLButtonElement>("open-current-folder-button"),
  controlDeckHeading: requiredElement<HTMLHeadingElement>("control-deck-heading"),
  addProfilesButton: requiredElement<HTMLButtonElement>("add-profiles-button"),
  openCodexButton: requiredElement<HTMLButtonElement>("open-codex-button"),
  settingsGithubButton: requiredElement<HTMLButtonElement>("settings-github-button"),
  settingsCheckUpdateButton: requiredElement<HTMLButtonElement>("settings-check-update-button"),
  usageRefreshButton: optionalElement<HTMLButtonElement>("usage-refresh-button"),
  usageRangeSelect: optionalElement<HTMLSelectElement>("usage-range-select"),
  usageStartInput: optionalElement<HTMLInputElement>("usage-start-input"),
  usageEndInput: optionalElement<HTMLInputElement>("usage-end-input"),
  usageLiveEndCheckbox: optionalElement<HTMLInputElement>("usage-live-end-checkbox"),
  usageAccountFilter: optionalElement<HTMLSelectElement>("usage-account-filter"),
  usageModelFilter: optionalElement<HTMLSelectElement>("usage-model-filter"),
  usageRefreshIntervalSelect: optionalElement<HTMLSelectElement>("usage-refresh-interval-select"),
  usageAccountPanel: optionalElement<HTMLDivElement>("usage-account-panel"),
  usageSummaryPanel: optionalElement<HTMLDivElement>("usage-summary-panel"),
  usageTrendPanel: optionalElement<HTMLDivElement>("usage-trend-panel"),
  usageDetailPanel: optionalElement<HTMLDivElement>("usage-detail-panel"),
  settingsUpdateUrlInput: requiredElement<HTMLInputElement>("settings-update-url-input"),
  settingsVersionValue: requiredElement<HTMLSpanElement>("settings-version-value"),
  updateDialog: requiredElement<HTMLDialogElement>("update-dialog"),
  updateDialogCopy: requiredElement<HTMLParagraphElement>("update-dialog-copy"),
  updateDialogLaterButton: requiredElement<HTMLButtonElement>("update-dialog-later-button"),
  updateDialogOpenButton: requiredElement<HTMLButtonElement>("update-dialog-open-button"),
  starButton: requiredElement<HTMLButtonElement>("star-button"),
  xiaohongshuButton: requiredElement<HTMLButtonElement>("xiaohongshu-button"),
  localeEnButton: requiredElement<HTMLButtonElement>("locale-en-button"),
  localeZhButton: requiredElement<HTMLButtonElement>("locale-zh-button"),
  quotaMonitorLabel: requiredElement<HTMLSpanElement>("quota-monitor-label"),
  dialog: document.getElementById("add-profile-dialog") as HTMLDialogElement,
  addProfileForm: requiredElement<HTMLFormElement>("add-profile-form"),
  cancelAddProfileButton: requiredElement<HTMLButtonElement>("cancel-add-profile-button"),
  submitAddProfileButton: requiredElement<HTMLButtonElement>("submit-add-profile-button"),
  dialogTitle: requiredElement<HTMLHeadingElement>("dialog-title"),
  dialogCopy: requiredElement<HTMLParagraphElement>("dialog-copy"),
  folderNameLabel: requiredElement<HTMLSpanElement>("folder-name-label"),
  folderNameInput: requiredElement<HTMLInputElement>("folder-name-input"),
  addBaseUrlLabel: requiredElement<HTMLSpanElement>("add-base-url-label"),
  addBaseUrlInput: requiredElement<HTMLInputElement>("add-base-url-input"),
  addBaseUrlCopy: requiredElement<HTMLSpanElement>("add-base-url-copy"),
  dialogError: requiredElement<HTMLParagraphElement>("dialog-error"),
  renameDialog: document.getElementById("rename-profile-dialog") as HTMLDialogElement,
  renameProfileForm: requiredElement<HTMLFormElement>("rename-profile-form"),
  renameDialogTitle: requiredElement<HTMLHeadingElement>("rename-dialog-title"),
  renameDialogCopy: requiredElement<HTMLParagraphElement>("rename-dialog-copy"),
  renameFolderNameLabel: requiredElement<HTMLSpanElement>("rename-folder-name-label"),
  renameFolderNameInput: requiredElement<HTMLInputElement>("rename-folder-name-input"),
  renameDialogError: requiredElement<HTMLParagraphElement>("rename-dialog-error"),
  cancelRenameProfileButton: requiredElement<HTMLButtonElement>("cancel-rename-profile-button"),
  submitRenameProfileButton: requiredElement<HTMLButtonElement>("submit-rename-profile-button"),
  deleteProfileDialog: hasDeleteProfileUi
    ? requiredElement<HTMLDialogElement>("delete-profile-dialog")
    : null,
  deleteProfileDialogTitle: hasDeleteProfileUi
    ? requiredElement<HTMLHeadingElement>("delete-profile-dialog-title")
    : null,
  deleteProfileDialogCopy: hasDeleteProfileUi
    ? requiredElement<HTMLParagraphElement>("delete-profile-dialog-copy")
    : null,
  deleteProfileDialogError: hasDeleteProfileUi
    ? requiredElement<HTMLParagraphElement>("delete-profile-dialog-error")
    : null,
  deleteProfileButton: hasDeleteProfileUi
    ? requiredElement<HTMLButtonElement>("delete-profile-button")
    : null,
  clearProfileAccountButton: hasDeleteProfileUi
    ? requiredElement<HTMLButtonElement>("clear-profile-account-button")
    : null,
  cancelDeleteProfileButton: hasDeleteProfileUi
    ? requiredElement<HTMLButtonElement>("cancel-delete-profile-button")
    : null,
  baseUrlDialog: document.getElementById("base-url-dialog") as HTMLDialogElement,
  baseUrlForm: requiredElement<HTMLFormElement>("base-url-form"),
  baseUrlDialogTitle: requiredElement<HTMLHeadingElement>("base-url-dialog-title"),
  baseUrlDialogCopy: requiredElement<HTMLParagraphElement>("base-url-dialog-copy"),
  baseUrlLabel: requiredElement<HTMLSpanElement>("base-url-label"),
  baseUrlInput: requiredElement<HTMLInputElement>("base-url-input"),
  baseUrlDialogError: requiredElement<HTMLParagraphElement>("base-url-dialog-error"),
  cancelBaseUrlButton: requiredElement<HTMLButtonElement>("cancel-base-url-button"),
  submitBaseUrlButton: requiredElement<HTMLButtonElement>("submit-base-url-button"),
  settingsCodexCliLabel: requiredElement<HTMLElement>("settings-codex-cli-label"),
  settingsCodexCliValue: requiredElement<HTMLParagraphElement>("settings-codex-cli-value"),
  settingsCodexCliButton: requiredElement<HTMLButtonElement>("settings-codex-cli-button"),
  settingsCodexCliDetectButton: requiredElement<HTMLButtonElement>("settings-codex-cli-detect-button"),
  codexCliDialog: requiredElement<HTMLDialogElement>("codex-cli-dialog"),
  codexCliForm: requiredElement<HTMLFormElement>("codex-cli-form"),
  codexCliDialogTitle: requiredElement<HTMLHeadingElement>("codex-cli-dialog-title"),
  codexCliDialogCopy: requiredElement<HTMLParagraphElement>("codex-cli-dialog-copy"),
  codexCliCurrentLabel: requiredElement<HTMLSpanElement>("codex-cli-current-label"),
  codexCliCurrentValue: requiredElement<HTMLSpanElement>("codex-cli-current-value"),
  codexCliCurrentSource: requiredElement<HTMLSpanElement>("codex-cli-current-source"),
  codexCliInputLabel: requiredElement<HTMLSpanElement>("codex-cli-input-label"),
  codexCliInput: requiredElement<HTMLInputElement>("codex-cli-input"),
  codexCliSuggestionsHeading: requiredElement<HTMLParagraphElement>("codex-cli-suggestions-heading"),
  codexCliSuggestions: requiredElement<HTMLDivElement>("codex-cli-suggestions"),
  codexCliDialogError: requiredElement<HTMLParagraphElement>("codex-cli-dialog-error"),
  cancelCodexCliButton: requiredElement<HTMLButtonElement>("cancel-codex-cli-button"),
  clearCodexCliButton: requiredElement<HTMLButtonElement>("clear-codex-cli-button"),
  submitCodexCliButton: requiredElement<HTMLButtonElement>("submit-codex-cli-button"),
  toast: requiredElement<HTMLDivElement>("toast"),
  routeTabs: Array.from(document.querySelectorAll<HTMLElement>("[data-route-tab]")),
  pages: Array.from(document.querySelectorAll<HTMLElement>("[data-page]")),
  localizedText: Array.from(document.querySelectorAll<HTMLElement>("[data-i18n-key]")),
  localeButtons: Array.from(document.querySelectorAll<HTMLButtonElement>("[data-set-locale]")),
  themeButtons: Array.from(document.querySelectorAll<HTMLButtonElement>("[data-theme-option]")),
  addProfileButtons: Array.from(document.querySelectorAll<HTMLButtonElement>("[data-add-profile]")),
  dashboardActiveProfile: requiredElement<HTMLElement>("dashboard-active-profile"),
  dashboardProfileCount: requiredElement<HTMLElement>("dashboard-profile-count"),
  dashboardReadyCount: requiredElement<HTMLElement>("dashboard-ready-count"),
  dashboardMissingCount: requiredElement<HTMLElement>("dashboard-missing-count"),
};

function formatPercent(value: number | null): string {
  return value == null ? "--" : `${value}%`;
}

function formatRefresh(entry: QuotaWindow | undefined): string {
  if (!entry) {
    return "--";
  }
  if (entry.reset_at_timestamp != null) {
    const diff = entry.reset_at_timestamp - Math.floor(Date.now() / 1000);
    if (diff > 0) {
      const h = Math.floor(diff / 3600);
      const m = Math.floor((diff % 3600) / 60);
      if (h > 0) {
        return t(state.locale, "resetsIn", { value: `${h}h ${m}m` });
      } else if (m > 0) {
        const s = diff % 60;
        return t(state.locale, "resetsIn", { value: `${m}m ${s}s` });
      } else {
        return t(state.locale, "resetsIn", { value: `${diff}s` });
      }
    } else {
      return t(state.locale, "resetting");
    }
  }
  return entry.refresh_at || "--";
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatInteger(value: number): string {
  return new Intl.NumberFormat(state.locale).format(value);
}

function formatCostMicros(value: number): string {
  return new Intl.NumberFormat(state.locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 4,
    maximumFractionDigits: 4,
  }).format(value / 1_000_000);
}

function formatUsageCost(cost: OpenAiUsageCostBreakdown): string {
  if (cost.unpriced_tokens > 0 && cost.total_cost_micros === 0) {
    return "N/A";
  }
  if (cost.unpriced_tokens > 0) {
    return `${formatCostMicros(cost.total_cost_micros)} ${t(state.locale, "usageCostPartial")}`;
  }
  return formatCostMicros(cost.total_cost_micros);
}

function formatRate(value: number | null): string {
  if (value == null) {
    return "--";
  }
  return new Intl.NumberFormat(state.locale, {
    style: "percent",
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

function formatDateTimeMs(value: number | null): string {
  if (value == null) {
    return "--";
  }
  return new Intl.DateTimeFormat(state.locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

function defaultQuotaProvenance(): QuotaProvenance {
  return {
    source: "unknown",
    confidence: "unknown",
    observed_at_ms: null,
    source_mtime_ms: null,
    note: null,
  };
}

function quotaSourceLabel(source: QuotaSource): string {
  switch (source) {
    case "chat_gpt_api":
      return t(state.locale, "quotaSourceChatGptApi");
    case "codex_app_server":
      return t(state.locale, "quotaSourceCodexAppServer");
    case "session_jsonl":
      return t(state.locale, "quotaSourceSessionJsonl");
    case "unknown":
    default:
      return t(state.locale, "quotaSourceUnknown");
  }
}

function quotaConfidenceLabel(confidence: QuotaConfidence): string {
  switch (confidence) {
    case "official":
      return t(state.locale, "quotaConfidenceOfficial");
    case "fallback":
      return t(state.locale, "quotaConfidenceFallback");
    case "local_observation":
      return t(state.locale, "quotaConfidenceLocalObservation");
    case "unknown":
    default:
      return t(state.locale, "quotaConfidenceUnknown");
  }
}

function quotaConfidenceClass(confidence: QuotaConfidence): string {
  switch (confidence) {
    case "official":
      return "is-official";
    case "fallback":
      return "is-fallback";
    case "local_observation":
      return "is-local-observation";
    case "unknown":
    default:
      return "is-unknown";
  }
}

function formatQuotaObservedAge(provenance: QuotaProvenance): string {
  const timestamp = provenance.observed_at_ms ?? provenance.source_mtime_ms;
  if (timestamp == null) {
    return t(state.locale, "quotaObservedUnknown");
  }

  const ageMs = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(ageMs / 60_000);
  if (minutes < 1) {
    return t(state.locale, "quotaObservedJustNow");
  }
  if (minutes < 60) {
    return t(state.locale, "quotaObservedMinutesAgo", { value: minutes });
  }

  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return t(state.locale, "quotaObservedHoursAgo", { value: hours });
  }

  return t(state.locale, "quotaObservedDaysAgo", { value: Math.floor(hours / 24) });
}

function formatTokenObservedAge(timestamp: number | null): string {
  if (timestamp == null) {
    return t(state.locale, "tokenUpdatedUnknown");
  }

  const ageMs = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(ageMs / 60_000);
  if (minutes < 1) {
    return t(state.locale, "tokenUpdatedJustNow");
  }
  if (minutes < 60) {
    return t(state.locale, "tokenUpdatedMinutesAgo", { value: minutes });
  }

  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return t(state.locale, "tokenUpdatedHoursAgo", { value: hours });
  }

  return t(state.locale, "tokenUpdatedDaysAgo", { value: Math.floor(hours / 24) });
}

function formatTokenDate(timestamp: number | null): string | null {
  if (timestamp == null) {
    return null;
  }

  return new Intl.DateTimeFormat(state.locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
}

function formatTokenCompactDate(timestamp: number | null): string | null {
  if (timestamp == null) {
    return null;
  }

  return new Intl.DateTimeFormat(state.locale, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp));
}

function tokenHealthLabel(health: RefreshTokenHealth): string {
  switch (health) {
    case "healthy":
      return t(state.locale, "tokenHealthHealthy");
    case "refresh_due":
      return t(state.locale, "tokenHealthRefreshDue");
    case "needs_login":
      return t(state.locale, "tokenHealthNeedsLogin");
    case "not_refreshable":
      return t(state.locale, "tokenHealthNotRefreshable");
    case "missing_auth":
      return t(state.locale, "tokenHealthMissingAuth");
    case "unknown_expiry":
    default:
      return t(state.locale, "tokenHealthUnknownExpiry");
  }
}

function tokenHealthClass(health: RefreshTokenHealth): string {
  switch (health) {
    case "healthy":
      return "is-healthy";
    case "refresh_due":
      return "is-refresh-due";
    case "needs_login":
      return "is-needs-login";
    case "not_refreshable":
      return "is-not-refreshable";
    case "missing_auth":
      return "is-missing-auth";
    case "unknown_expiry":
    default:
      return "is-unknown-expiry";
  }
}

function tokenActionText(status: RefreshTokenStatus): string {
  switch (status.health) {
    case "needs_login":
      return t(state.locale, "tokenActionLogin");
    case "not_refreshable":
      return t(state.locale, "tokenActionNotRefreshable");
    case "missing_auth":
      return t(state.locale, "tokenActionMissingAuth");
    case "healthy":
    case "refresh_due":
    case "unknown_expiry":
    default:
      return status.can_refresh
        ? t(state.locale, "tokenActionRefresh")
        : t(state.locale, "tokenActionLogin");
  }
}

function buildTokenStatusMarkup(
  status: RefreshTokenStatus,
  layout: "profile" | "current",
): string {
  const label = tokenHealthLabel(status.health);
  const updated = formatTokenObservedAge(status.last_refresh_at_ms ?? status.auth_updated_at_ms);
  const refreshExpiry = formatTokenDate(status.refresh_token_expires_at_ms);
  const compactRefreshExpiry = formatTokenCompactDate(status.refresh_token_expires_at_ms);
  const authCacheAge = t(state.locale, "tokenAuthCacheUpdated", { value: updated });
  const expiryLine = refreshExpiry
    ? t(state.locale, "tokenRefreshExpiresAt", { value: refreshExpiry })
    : t(state.locale, "tokenRefreshExpiryUnknown");
  const inlineExpiry = compactRefreshExpiry
    ? t(state.locale, "tokenRefreshExpiresShort", { value: compactRefreshExpiry })
    : updated;
  const action = tokenActionText(status);
  const title = `${label}. ${authCacheAge}. ${expiryLine}. ${action}`;

  return `
    <div
      class="token-status-row token-status-row--${layout} ${tokenHealthClass(status.health)}"
      title="${escapeHtml(title)}"
      aria-label="${escapeHtml(title)}"
    >
      <span class="token-status-dot" aria-hidden="true"></span>
      <span class="token-status-label">${escapeHtml(label)}</span>
      <span class="token-status-age">${escapeHtml(inlineExpiry)}</span>
    </div>
  `;
}

function buildQuotaEvidenceMarkup(
  quota: QuotaSummary | null | undefined,
  layout: "profile" | "current",
): string {
  const provenance = quota?.provenance ?? defaultQuotaProvenance();
  const source = quotaSourceLabel(provenance.source);
  const confidence = quotaConfidenceLabel(provenance.confidence);
  const observed = formatQuotaObservedAge(provenance);
  const title = t(state.locale, "quotaEvidenceTitle", { source, confidence, observed });
  const note = provenance.note ? ` ${provenance.note}` : "";

  return `
    <div
      class="quota-evidence-row quota-evidence-row--${layout}"
      title="${escapeHtml(title + note)}"
      aria-label="${escapeHtml(title)}"
    >
      <span class="quota-source-badge ${quotaConfidenceClass(provenance.confidence)}">
        ${escapeHtml(source)}
      </span>
      <span class="quota-observed-age">${escapeHtml(observed)}</span>
    </div>
  `;
}

function lowestQuotaWindow(quota: QuotaSummary | null | undefined): { label: string; value: number } | null {
  const windows = [
    { label: t(state.locale, "fiveHourAllowance"), value: quota?.five_hour?.remaining_percent ?? null },
    { label: t(state.locale, "weeklyAllowance"), value: quota?.weekly?.remaining_percent ?? null },
  ].filter((entry): entry is { label: string; value: number } => entry.value != null);

  if (!windows.length) {
    return null;
  }

  return windows.reduce((lowest, entry) => (entry.value < lowest.value ? entry : lowest));
}

function buildQuotaLowWarningMarkup(
  quota: QuotaSummary | null | undefined,
  unavailable: boolean,
  layout: "profile" | "current",
): string {
  if (unavailable) {
    return "";
  }

  const lowest = lowestQuotaWindow(quota);
  if (!lowest || lowest.value > LOW_QUOTA_THRESHOLD_PERCENT) {
    return "";
  }

  const text = t(state.locale, "quotaLowWarning", {
    window: lowest.label,
    value: lowest.value,
  });
  const title = t(state.locale, "quotaLowWarningTitle");

  return `
    <div class="quota-low-warning quota-low-warning--${layout}" title="${escapeHtml(title)}">
      <span class="quota-low-dot" aria-hidden="true"></span>
      <span>${escapeHtml(text)}</span>
    </div>
  `;
}

function bindProfileButtons(attribute: string, handler: (profile: string) => void): void {
  for (const button of elements.profilesGrid.querySelectorAll<HTMLButtonElement>(`[${attribute}]`)) {
    button.addEventListener("click", () => {
      const profile = button.getAttribute(attribute);
      if (profile) {
        void handler(profile);
      }
    });
  }
}

function isProfileUnavailable(profile: Pick<ProfileCard, "auth_present" | "has_account_identity" | "status">): boolean {
  return profile.status === "missing_auth" || !profile.auth_present || !profile.has_account_identity;
}

function normalizeDisplayParts(
  entry: Pick<ProfileCard | CurrentCard, "folder_name" | "display_title" | "account_label">,
): { folder: string; account: string } {
  const folder = entry.folder_name.trim();
  const account = entry.account_label?.trim() ?? "";

  if (account) {
    return { folder, account };
  }

  const rawTitle = entry.display_title?.trim() ?? "";
  const parts = rawTitle.split(" / ").map((value) => value.trim()).filter(Boolean);
  if (parts.length >= 2) {
    return {
      folder: parts[0] ?? folder,
      account: parts[parts.length - 1] ?? "",
    };
  }

  return { folder, account: rawTitle };
}

function profileDisplayTitle(entry: Pick<ProfileCard, "folder_name" | "display_title" | "account_label">): string {
  const { folder, account } = normalizeDisplayParts(entry);
  if (folder && account && folder !== account) {
    return `${folder} · ${account}`;
  }

  return account || folder || "--";
}

function currentDisplayTitle(entry: Pick<CurrentCard, "folder_name" | "display_title" | "account_label">): string {
  const { folder, account } = normalizeDisplayParts(entry);
  return account || folder || "--";
}

/// Plan tokens from the backend that the front-end translates into
/// localized labels. Today only `unknown_paid` qualifies — surfaced when
/// the id_token claims `free` but quota data implies an active paid
/// window. Mapped to a localized "Unknown paid plan" label so the user
/// is prompted to re-login instead of seeing a fake tier.
const SPECIAL_PLAN_TOKENS = new Set(["unknown_paid"]);

/// `last_plan_check_ms` older than this (millis) is treated as stale —
/// the bulk plan refresh runs at most once per local day, so anything
/// past that window means the cache hasn't been re-confirmed since the
/// last day-rollover.
const PLAN_CHECK_STALE_MS = 36 * 60 * 60 * 1000;

function formatPlanName(planName: string): string {
  if (SPECIAL_PLAN_TOKENS.has(planName)) {
    if (planName === "unknown_paid") {
      return t(state.locale, "planUnknownPaid");
    }
  }
  return planName.replace(/\b([a-z])/g, (match) => match.toUpperCase());
}

export function planLine(planName: string | null, daysLeft: number | null): string {
  const formattedPlanName = planName ? formatPlanName(planName) : null;

  if (!planName && daysLeft == null) {
    return t(state.locale, "profileMetadataMissing");
  }

  // Hide the days-left suffix when it's <= 0. The cached
  // `chatgpt_subscription_active_until` claim from the id_token does not
  // automatically rotate when the subscription renews — it only
  // refreshes on a full re-login or a successful OAuth refresh that
  // re-issues claims, neither of which is guaranteed to happen for
  // weeks on a quiet account. Showing a literal "Plus / 0 days" reads
  // as "your subscription expired" when the account is in fact active.
  // Safer to show just the plan name and let the next refresh decide.
  if (formattedPlanName && daysLeft != null && daysLeft > 0) {
    return t(state.locale, "subscriptionDaysLeft", { plan: formattedPlanName, days: daysLeft });
  }

  if (formattedPlanName) {
    return formattedPlanName;
  }

  return t(state.locale, "subscriptionFallback", { days: daysLeft ?? "--" });
}

/// Build a hover-time tooltip describing when the plan tier was last
/// confirmed. `unknown_paid` plans get the prompt to re-login appended;
/// stale plans (>36h since last confirmation) get a hint that the bulk
/// refresh will retry at the next local-day rollover. Returns an empty
/// string when there is nothing useful to surface.
export function planFreshnessTitle(
  planName: string | null,
  lastPlanCheckMs: number | null,
): string {
  const parts: string[] = [];

  if (lastPlanCheckMs == null) {
    parts.push(t(state.locale, "planCheckedNever"));
  } else {
    const elapsedMs = Math.max(0, Date.now() - lastPlanCheckMs);
    if (elapsedMs < 60 * 1000) {
      parts.push(t(state.locale, "planCheckedJustNow"));
    } else if (elapsedMs < 60 * 60 * 1000) {
      parts.push(
        t(state.locale, "planCheckedMinutesAgo", { value: Math.floor(elapsedMs / 60_000) }),
      );
    } else if (elapsedMs < 24 * 60 * 60 * 1000) {
      parts.push(
        t(state.locale, "planCheckedHoursAgo", { value: Math.floor(elapsedMs / (60 * 60_000)) }),
      );
    } else {
      parts.push(
        t(state.locale, "planCheckedDaysAgo", {
          value: Math.floor(elapsedMs / (24 * 60 * 60_000)),
        }),
      );
    }
    if (elapsedMs >= PLAN_CHECK_STALE_MS) {
      parts.push(t(state.locale, "planCheckedStaleSuffix").trim());
    }
  }

  if (planName === "unknown_paid") {
    parts.push(t(state.locale, "planUnknownPaidHint"));
  }

  return parts.join(" ");
}

/// True when the cached plan check is old enough to render a visual
/// "stale" indicator (small dot) on the card. Mirrors the threshold
/// in `planFreshnessTitle` so both signals agree.
export function isPlanCheckStale(lastPlanCheckMs: number | null): boolean {
  if (lastPlanCheckMs == null) {
    return true;
  }
  return Date.now() - lastPlanCheckMs >= PLAN_CHECK_STALE_MS;
}

function buildMetricLineMarkup(
  label: string,
  entry: QuotaWindow | undefined,
  fillVariant: "blue" | "pink",
  unavailable: boolean,
  layout: "profile" | "current",
): string {
  const remainingPercent = unavailable ? null : (entry?.remaining_percent ?? null);
  const percent = remainingPercent ?? 0;
  const isLow = remainingPercent != null && remainingPercent <= LOW_QUOTA_THRESHOLD_PERCENT;
  const metricClass = layout === "current" ? "current-quota-metric" : "profile-quota-metric";
  const lineClass = layout === "current" ? "current-quota-line" : "profile-quota-line";
  const titleClass = layout === "current" ? "current-quota-title" : "profile-quota-title";
  const refreshClass = layout === "current" ? "current-quota-refresh" : "profile-quota-refresh";
  const valueClass = layout === "current" ? "current-quota-value" : "profile-quota-value";
  const fillClass = unavailable ? "gray" : fillVariant;
  const refresh = formatRefresh(entry);
  const value = formatPercent(remainingPercent);
  const title = t(state.locale, "quotaWindowTitle", { label, value, refresh });

  return `
    <section
      class="${metricClass}${unavailable ? " is-unavailable" : ""}${isLow ? " is-quota-low" : ""}"
      title="${escapeHtml(title)}"
      aria-label="${escapeHtml(title)}"
    >
      <div class="${lineClass}">
        <span class="${titleClass}">${escapeHtml(label)}</span>
        <span class="${refreshClass}">${escapeHtml(refresh)}</span>
        <span class="${valueClass}">${escapeHtml(value)}</span>
      </div>
      <div class="quota-track">
        <div class="quota-fill quota-fill--${fillClass}" style="width: ${percent}%;"></div>
      </div>
    </section>
  `;
}

function buildProfileQuotaMarkup(profile: ProfileCard): string {
  const unavailable = isProfileUnavailable(profile);
  const quota = profile.quota;

  return `
    <div class="profile-quota-stack">
      <div class="profile-quota-metrics">
        ${buildMetricLineMarkup(t(state.locale, "fiveHourAllowance"), quota?.five_hour, "blue", unavailable, "profile")}
        ${buildMetricLineMarkup(t(state.locale, "weeklyAllowance"), quota?.weekly, "pink", unavailable, "profile")}
      </div>
      <div class="quota-meta-stack quota-meta-stack--profile">
        ${buildTokenStatusMarkup(profile.refresh_token_status, "profile")}
        ${buildQuotaEvidenceMarkup(quota, "profile")}
        ${buildQuotaLowWarningMarkup(quota, unavailable, "profile")}
      </div>
    </div>
  `;
}

function buildCurrentQuotaMarkup(
  quota: QuotaSummary | null | undefined,
  hasAccountIdentity: boolean,
  refreshTokenStatus: RefreshTokenStatus,
): string {
  const unavailable = !hasAccountIdentity;

  return `
    <div class="current-quota-stack">
      ${buildTokenStatusMarkup(refreshTokenStatus, "current")}
      ${buildQuotaEvidenceMarkup(quota, "current")}
      ${buildMetricLineMarkup(t(state.locale, "fiveHourAllowance"), quota?.five_hour, "blue", unavailable, "current")}
      ${buildMetricLineMarkup(t(state.locale, "weeklyAllowance"), quota?.weekly, "pink", unavailable, "current")}
      ${buildQuotaLowWarningMarkup(quota, unavailable, "current")}
    </div>
  `;
}

export function showToast(message: string, isError = false): void {
  elements.toast.hidden = false;
  elements.toast.textContent = message;
  elements.toast.classList.toggle("is-error", isError);
  window.clearTimeout((showToast as typeof showToast & { timeoutId?: number }).timeoutId);
  (showToast as typeof showToast & { timeoutId?: number }).timeoutId = window.setTimeout(() => {
    elements.toast.hidden = true;
  }, 3200);
}

export function showUpdateDialog(update: UpdateCheckResponse): void {
  const copy = t(state.locale, "updateDialogCopy", {
    current: update.current_version,
    latest: update.latest_version ?? "--",
  });
  elements.updateDialogCopy.textContent = update.local_patch_required
    ? `${copy}\n\n${t(state.locale, "updateDialogLocalPatchRequired")}`
    : copy;

  if (!elements.updateDialog.open) {
    elements.updateDialog.showModal();
  }
}

export function renderThemeOptions(): void {
  for (const button of elements.themeButtons) {
    const themeId = button.dataset.themeOption;
    if (!isThemeId(themeId)) {
      continue;
    }

    const option = getThemeOption(themeId);
    const title = t(state.locale, option.nameKey);
    const description = t(state.locale, option.descriptionKey);
    const titleElement = button.querySelector<HTMLElement>("[data-theme-option-title]");
    const descriptionElement = button.querySelector<HTMLElement>("[data-theme-option-description]");
    const isActive = option.id === state.theme;

    if (titleElement) {
      titleElement.textContent = title;
    }
    if (descriptionElement) {
      descriptionElement.textContent = description;
    }

    button.classList.toggle("is-active", isActive);
    button.setAttribute("aria-label", `${title} ${description}`);
    button.setAttribute("aria-pressed", isActive ? "true" : "false");
    button.setAttribute("title", `${title} - ${description}`);
  }
}

export function renderShellRoute(): void {
  for (const page of elements.pages) {
    page.classList.toggle("active", page.dataset.page === state.route);
  }

  for (const tab of elements.routeTabs) {
    const isActive = tab.dataset.routeTab === state.route;
    tab.classList.toggle("active", isActive);
    tab.setAttribute("aria-current", isActive ? "page" : "false");
  }
}

export function renderShellOverview(dashboard: DashboardViewModel | null): void {
  if (!dashboard || !state.snapshot) {
    elements.dashboardActiveProfile.textContent = "--";
    elements.dashboardActiveProfile.title = "";
    elements.dashboardProfileCount.textContent = "--";
    elements.dashboardReadyCount.textContent = "--";
    elements.dashboardMissingCount.textContent = "--";
    return;
  }

  const profiles = state.snapshot.profiles;
  const readyCount = profiles.filter((profile) => (
    profile.auth_present && profile.has_account_identity && profile.status !== "missing_auth"
  )).length;
  const missingCount = profiles.length - readyCount;
  const activeProfileTitle = dashboard.current_card
    ? currentDisplayTitle(dashboard.current_card)
    : t(state.locale, "noActiveProfile");

  elements.dashboardActiveProfile.textContent = activeProfileTitle;
  elements.dashboardActiveProfile.title = activeProfileTitle;
  elements.dashboardProfileCount.textContent = String(profiles.length);
  elements.dashboardReadyCount.textContent = String(readyCount);
  elements.dashboardMissingCount.textContent = String(Math.max(0, missingCount));
}

export function renderOpenAiUsage(summary: OpenAiUsageSummary | null): void {
  if (
    !elements.usageSummaryPanel ||
    !elements.usageTrendPanel ||
    !elements.usageDetailPanel
  ) {
    return;
  }
  renderUsageControls(summary);
  if (state.usageLoadError) {
    if (elements.usageAccountPanel) {
      elements.usageAccountPanel.innerHTML = "";
    }
    elements.usageSummaryPanel.innerHTML = `
      <article class="usage-empty-state usage-empty-state--error">
        <strong>${t(state.locale, "usageLoadFailed")}</strong>
        <p>${escapeHtml(state.usageLoadError)}</p>
      </article>
    `;
    elements.usageTrendPanel.innerHTML = "";
    elements.usageDetailPanel.innerHTML = "";
    return;
  }

  if (!state.usageLoaded) {
    if (elements.usageAccountPanel) {
      elements.usageAccountPanel.innerHTML = "";
    }
    elements.usageSummaryPanel.innerHTML = `
      <article class="usage-empty-state">
        <strong>${t(state.locale, "usageLoading")}</strong>
      </article>
    `;
    elements.usageTrendPanel.innerHTML = "";
    elements.usageDetailPanel.innerHTML = "";
    return;
  }

  const accountTable = buildUsageAccountTable(state.usageAccountSummary);
  if (!summary || summary.total_requests === 0) {
    const hasActiveUsageFilter = Boolean(
      state.usageProfileFilter ||
        state.usageModelFilter ||
        state.usageStatusCodeFilter != null,
    );
    const hasFacetUsage = state.usageAccountSummary.length > 0 || state.usageModelOptions.length > 0;
    const emptyTitleKey = hasActiveUsageFilter && hasFacetUsage
      ? "usageNoFilteredUsage"
      : "usageNoAttribution";
    const emptyCopyKey = hasActiveUsageFilter && hasFacetUsage
      ? "usageNoFilteredUsageCopy"
      : "usageNoAttributionCopy";
    if (elements.usageAccountPanel) {
      elements.usageAccountPanel.innerHTML = accountTable;
    }
    elements.usageSummaryPanel.innerHTML = `
      <article class="usage-empty-state">
        <strong>${t(state.locale, emptyTitleKey)}</strong>
        <p>${t(state.locale, emptyCopyKey)}</p>
      </article>
    `;
    elements.usageTrendPanel.innerHTML = buildUsageTrendMarkup();
    elements.usageDetailPanel.innerHTML = `
      ${elements.usageAccountPanel ? "" : accountTable}
      ${buildUsageEvidenceMarkup(summary)}
    `;
    return;
  }

  if (elements.usageAccountPanel) {
    elements.usageAccountPanel.innerHTML = accountTable;
  }
  elements.usageSummaryPanel.innerHTML = `
    <div class="usage-hero-grid">
      <article class="usage-hero-card usage-hero-card--wide">
        <span>${t(state.locale, "usageRealTokens")}</span>
        <strong>${formatInteger(summary.tokens.real_total_tokens)}</strong>
        <small>${t(state.locale, "usageAttributionSince")}: ${formatDateTimeMs(summary.attribution_since_ms)}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(state.locale, summary.cost.unpriced_tokens > 0 ? "usagePricedCost" : "usageEstimatedCost")}</span>
        <strong>${formatUsageCost(summary.cost)}</strong>
        <small>${summary.pricing_source} · ${summary.pricing_checked_at}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(state.locale, "usageRequests")}</span>
        <strong>${formatInteger(summary.total_requests)}</strong>
        <small>${t(state.locale, "usageUpdatedAt")}: ${formatDateTimeMs(summary.generated_at_ms)}</small>
      </article>
      <article class="usage-hero-card">
        <span>${t(state.locale, "usageCacheHitRate")}</span>
        <strong>${formatRate(summary.cache_hit_rate)}</strong>
        <small>${t(state.locale, "usageCachedInput")}: ${formatInteger(summary.tokens.cached_input_tokens)}</small>
      </article>
    </div>

    <div class="usage-token-grid">
      ${buildUsageTokenMetric("usageInput", summary.tokens.input_tokens)}
      ${buildUsageTokenMetric("usageCachedInput", summary.tokens.cached_input_tokens)}
      ${buildUsageTokenMetric("usageOutput", summary.tokens.output_tokens)}
      ${buildUsageTokenMetric("usageReasoning", summary.tokens.reasoning_output_tokens)}
    </div>
  `;

  elements.usageTrendPanel.innerHTML = buildUsageTrendMarkup();
  elements.usageDetailPanel.innerHTML = `
    ${buildUsageLogsTable()}
    ${elements.usageAccountPanel ? "" : accountTable}
    ${buildUsageModelTable(summary)}
    ${buildUsageDataSourceMarkup()}
    ${buildUsageEvidenceMarkup(summary)}
  `;
}

function renderUsageControls(summary: OpenAiUsageSummary | null): void {
  if (
    !elements.usageRangeSelect ||
    !elements.usageLiveEndCheckbox ||
    !elements.usageRefreshIntervalSelect ||
    !elements.usageStartInput ||
    !elements.usageEndInput ||
    !elements.usageAccountFilter ||
    !elements.usageModelFilter
  ) {
    return;
  }
  elements.usageRangeSelect.value = state.usageRangePreset;
  elements.usageLiveEndCheckbox.checked = state.usageLiveEnd;
  elements.usageRefreshIntervalSelect.value = String(state.usageRefreshIntervalMs);
  elements.usageStartInput.value = formatDateTimeLocalInput(state.usageCustomStartDate);
  elements.usageEndInput.value = formatDateTimeLocalInput(state.usageCustomEndDate);
  const custom = state.usageRangePreset === "custom";
  elements.usageStartInput.disabled = !custom;
  elements.usageEndInput.disabled = !custom || state.usageLiveEnd;
  elements.usageLiveEndCheckbox.disabled = !custom;

  const accountOptions = state.usageAccountSummary.length > 0
    ? state.usageAccountSummary
    : summary?.accounts ?? [];
  const accounts = new Map<string, string>();
  for (const account of accountOptions) {
    accounts.set(account.profile, account.account_label ?? account.profile);
  }
  if (state.usageProfileFilter && !accounts.has(state.usageProfileFilter)) {
    accounts.set(state.usageProfileFilter, state.usageProfileFilter);
  }
  const accountOptionsHtml = [
    `<option value="">${t(state.locale, "usageAllAccounts")}</option>`,
    ...Array.from(accounts.entries()).map(([profile, label]) => (
      `<option value="${escapeHtml(profile)}">${escapeHtml(label)}</option>`
    )),
  ].join("");
  setSelectOptions(elements.usageAccountFilter, accountOptionsHtml, state.usageProfileFilter ?? "");

  const modelOptions = state.usageModelOptions.length > 0
    ? state.usageModelOptions
    : summary?.models ?? [];
  const models = new Set(modelOptions.map((model) => model.model));
  if (state.usageModelFilter) {
    models.add(state.usageModelFilter);
  }
  const modelOptionsHtml = [
    `<option value="">${t(state.locale, "usageAllModels")}</option>`,
    ...Array.from(models).map((model) => (
      `<option value="${escapeHtml(model)}">${escapeHtml(model)}</option>`
    )),
  ].join("");
  setSelectOptions(elements.usageModelFilter, modelOptionsHtml, state.usageModelFilter ?? "");
}

function setSelectOptions(select: HTMLSelectElement, optionsHtml: string, value: string): void {
  if (select.innerHTML !== optionsHtml) {
    select.innerHTML = optionsHtml;
  }
  select.value = value;
}

function formatDateTimeLocalInput(value: number | null): string {
  if (value == null) {
    return "";
  }
  const date = new Date(value);
  const pad = (part: number) => String(part).padStart(2, "0");
  return [
    date.getFullYear(),
    "-",
    pad(date.getMonth() + 1),
    "-",
    pad(date.getDate()),
    "T",
    pad(date.getHours()),
    ":",
    pad(date.getMinutes()),
  ].join("");
}

function buildUsageTokenMetric(labelKey: MessageKey, value: number): string {
  return `
    <article class="usage-token-card">
      <span>${t(state.locale, labelKey)}</span>
      <strong>${formatInteger(value)}</strong>
    </article>
  `;
}

function buildUsageAccountTable(accounts: OpenAiUsageAccountSummary[]): string {
  if (accounts.length === 0) {
    return "";
  }
  const rows = accounts.map((account) => {
    const selectedClass = state.usageProfileFilter === account.profile
      ? ` class="usage-table-row--selected"`
      : "";
    return `
    <tr${selectedClass}>
      <td>${escapeHtml(account.profile)}</td>
      <td>${escapeHtml(account.account_label ?? "--")}</td>
      <td>${escapeHtml(account.official_source)}</td>
      <td>${formatInteger(account.request_count)}</td>
      <td>${formatInteger(account.tokens.real_total_tokens)}</td>
      <td>${formatUsageCost(account.cost)}</td>
    </tr>
  `;
  }).join("");
  return `
    <article class="usage-table-card">
      <h3>${t(state.locale, "usageAccounts")}</h3>
      <div class="usage-table-scroll">
        <table class="usage-table">
          <thead>
            <tr>
              <th>${t(state.locale, "usageProfile")}</th>
              <th>${t(state.locale, "usageAccount")}</th>
              <th>${t(state.locale, "usageOfficialSource")}</th>
              <th>${t(state.locale, "usageRequests")}</th>
              <th>${t(state.locale, "usageTokens")}</th>
              <th>${t(state.locale, "usageCost")}</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    </article>
  `;
}

function buildUsageModelTable(summary: OpenAiUsageSummary): string {
  const rows = summary.models.map((model) => `
    <tr>
      <td>${escapeHtml(model.model)}</td>
      <td>${formatInteger(model.request_count)}</td>
      <td>${formatInteger(model.tokens.input_tokens)}</td>
      <td>${formatInteger(model.tokens.cached_input_tokens)}</td>
      <td>${formatInteger(model.tokens.output_tokens)}</td>
      <td>${formatUsageCost(model.cost)}</td>
      <td>${model.priced ? t(state.locale, "usagePriced") : t(state.locale, "usageNotPriced")}</td>
    </tr>
  `).join("");
  return `
    <article class="usage-table-card">
      <h3>${t(state.locale, "usageModels")}</h3>
      <div class="usage-table-scroll">
        <table class="usage-table">
          <thead>
            <tr>
              <th>${t(state.locale, "usageModel")}</th>
              <th>${t(state.locale, "usageRequests")}</th>
              <th>${t(state.locale, "usageInput")}</th>
              <th>${t(state.locale, "usageCachedInput")}</th>
              <th>${t(state.locale, "usageOutput")}</th>
              <th>${t(state.locale, "usageCost")}</th>
              <th>${t(state.locale, "usagePricing")}</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    </article>
  `;
}

function buildUsageTrendMarkup(): string {
  const points = state.usageTrends;
  const maxTokens = Math.max(1, ...points.map((point) => point.tokens.real_total_tokens));
  const rows = points.map((point) => {
    const pct = Math.max(2, Math.round((point.tokens.real_total_tokens / maxTokens) * 100));
    return `
      <div class="usage-trend-row">
        <span>${escapeHtml(formatTrendLabel(point.bucket_start_ms))}</span>
        <div class="usage-trend-track">
          <em style="width:${pct}%"></em>
        </div>
        <strong>${formatInteger(point.tokens.real_total_tokens)}</strong>
        <small>${formatUsageCost(point.cost)}</small>
      </div>
    `;
  }).join("");
  return `
    <article class="usage-table-card usage-trend-card">
      <div class="usage-card-title-row">
        <h3>${t(state.locale, "usageTrends")}</h3>
        <span>${t(state.locale, "usageRefreshInterval")}: ${formatRefreshInterval(state.usageRefreshIntervalMs)}</span>
      </div>
      <div class="usage-trend-list">${rows || `<p class="usage-muted">${t(state.locale, "usageNoAttribution")}</p>`}</div>
    </article>
  `;
}

function formatTrendLabel(value: number): string {
  return new Intl.DateTimeFormat(state.locale, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function formatRefreshInterval(value: number): string {
  return value > 0 ? `${value / 1000}s` : t(state.locale, "usageRefreshOff");
}

function buildUsageLogsTable(): string {
  const page = state.usageLogs;
  const rows = (page?.data ?? []).map((log: OpenAiUsageRequestLog) => `
    <tr>
      <td><code>${escapeHtml(log.request_id)}</code></td>
      <td>${formatDateTimeMs(log.created_at_ms)}</td>
      <td>${escapeHtml(log.account_label ?? log.profile)}</td>
      <td>${escapeHtml(log.model)}</td>
      <td>${formatInteger(log.tokens.real_total_tokens)}</td>
      <td>${formatUsageCost(log.cost)}</td>
      <td>${escapeHtml(log.data_source)}</td>
    </tr>
  `).join("");
  const currentPage = page?.page ?? state.usageLogsPage;
  const pageSize = page?.page_size ?? state.usageLogsPageSize;
  const total = page?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  return `
    <article class="usage-table-card">
      <div class="usage-card-title-row">
        <h3>${t(state.locale, "usageRequestLogs")}</h3>
        <div class="usage-log-pager">
          <button class="pager-pill" type="button" data-usage-log-page="${Math.max(1, currentPage - 1)}" ${currentPage <= 1 ? "disabled" : ""}>${t(state.locale, "previous")}</button>
          <span>${currentPage} / ${totalPages}</span>
          <button class="pager-pill" type="button" data-usage-log-page="${Math.min(totalPages, currentPage + 1)}" ${currentPage >= totalPages ? "disabled" : ""}>${t(state.locale, "next")}</button>
        </div>
      </div>
      <div class="usage-table-scroll">
        <table class="usage-table">
          <thead>
            <tr>
              <th>${t(state.locale, "usageRequestId")}</th>
              <th>${t(state.locale, "usageCreatedAt")}</th>
              <th>${t(state.locale, "usageAccount")}</th>
              <th>${t(state.locale, "usageModel")}</th>
              <th>${t(state.locale, "usageTokens")}</th>
              <th>${t(state.locale, "usageCost")}</th>
              <th>${t(state.locale, "usageDataSource")}</th>
            </tr>
          </thead>
          <tbody>${rows || `<tr><td colspan="7">${t(state.locale, "usageNoAttribution")}</td></tr>`}</tbody>
        </table>
      </div>
    </article>
  `;
}

function buildUsageDataSourceMarkup(): string {
  const rows = state.usageDataSources.map((source) => `
    <span>
      <em>${escapeHtml(source.data_source)}</em>
      <strong>${formatInteger(source.request_count)}</strong>
      <small>${formatUsageCost(source.cost)}</small>
    </span>
  `).join("");
  return `
    <article class="usage-evidence-card">
      <h3>${t(state.locale, "usageDataSources")}</h3>
      <div class="usage-evidence-grid">${rows || `<span><em>${t(state.locale, "usageNoAttribution")}</em><strong>0</strong></span>`}</div>
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
  };
  const items: Array<[MessageKey, number]> = [
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
      <h3>${t(state.locale, "usageEvidence")}</h3>
      <div class="usage-evidence-grid">
        ${items.map(([label, value]) => `
          <span>
            <em>${t(state.locale, label)}</em>
            <strong>${formatInteger(value)}</strong>
          </span>
        `).join("")}
      </div>
    </article>
  `;
}

export function renderCurrentCard(dashboard: DashboardViewModel): void {
  const current = dashboard.current_card;
  if (!current) {
    const noActiveTitle = t(state.locale, "noActiveProfile");
    elements.currentTitle.textContent = noActiveTitle;
    elements.currentTitle.title = noActiveTitle;
    elements.currentPlan.textContent = t(state.locale, "switchToStart");
    elements.currentLoginButton.disabled = true;
    elements.openCurrentFolderButton.disabled = true;
    state.currentProfile = null;
    elements.currentQuotaPanel.innerHTML =
      `<div class="empty-state">${t(state.locale, "quotaWillAppear")}</div>`;
    return;
  }

  state.currentProfile = current.folder_name;
  const currentTitle = currentDisplayTitle(current);
  elements.currentTitle.textContent = currentTitle;
  elements.currentTitle.title = currentTitle;
  elements.currentPlan.textContent = planLine(current.plan_name, current.subscription_days_left);
  const currentPlanTitle = planFreshnessTitle(current.plan_name, current.last_plan_check_ms);
  if (currentPlanTitle) {
    elements.currentPlan.title = currentPlanTitle;
  } else {
    elements.currentPlan.removeAttribute("title");
  }
  elements.currentPlan.classList.toggle(
    "plan-check-stale",
    isPlanCheckStale(current.last_plan_check_ms),
  );
  elements.currentPlan.classList.toggle(
    "plan-unknown-paid",
    current.plan_name === "unknown_paid",
  );
  elements.currentLoginButton.disabled = state.loading;
  elements.openCurrentFolderButton.disabled = false;
  elements.currentQuotaPanel.innerHTML = buildCurrentQuotaMarkup(
    dashboard.current_quota_card,
    current.has_account_identity,
    current.refresh_token_status,
  );
}

export function renderProfiles(
  dashboard: DashboardViewModel,
  onDelete: (profile: string) => void,
  onRename: (profile: string) => void,
  onSwitch: (profile: string) => void,
  onRefresh: (profile: string) => void,
  onBaseUrl: (profile: string) => void,
  onLogin: (profile: string) => void,
): void {
  if (!dashboard.profiles.length) {
    elements.profilesGrid.innerHTML =
      `<div class="empty-state profiles-empty-state">${t(state.locale, "profilesEmpty")}</div>`;
    return;
  }

  elements.profilesGrid.innerHTML = dashboard.profiles
    .map((profile) => {
      const refreshRunning = state.refreshActiveProfiles.includes(profile.folder_name);
      const refreshPending = refreshRunning;
      const loginRunning = state.loginActiveProfile === profile.folder_name;
      // Any in-flight login (on this card or any other) blocks new logins
      // because the OAuth port and `.switch.lock` are global resources.
      const loginPending = state.loginActiveProfile !== null;
      const cardBusy = refreshPending || loginRunning;
      const deleteDisabled =
        state.loading || cardBusy || profile.status === "current";
      const renameDisabled =
        state.loading || cardBusy || profile.status === "current";
      const refreshDisabled =
        !profile.auth_present || state.loading || cardBusy || loginPending;
      const baseDisabled = state.loading || cardBusy;
      const switchDisabled =
        !profile.auth_present || state.loading || cardBusy || loginPending || profile.status === "current";

      const refreshTitle = refreshRunning
        ? t(state.locale, "profileRefreshRunning")
        : refreshDisabled
          ? t(state.locale, "profileRefreshDisabled")
          : t(state.locale, "profileRefreshReady");

      const loginDisabled = loginRunning
        ? false
        : state.loading || refreshPending || loginPending;
      const unavailable = isProfileUnavailable(profile);

      const planTooltip = planFreshnessTitle(profile.plan_name, profile.last_plan_check_ms);
      const planClasses = [
        "profile-plan",
        isPlanCheckStale(profile.last_plan_check_ms) ? "plan-check-stale" : null,
        profile.plan_name === "unknown_paid" ? "plan-unknown-paid" : null,
      ]
        .filter((value): value is string => value != null)
        .join(" ");
      const profileTitle = profileDisplayTitle(profile);

      return `
        <article class="profile-card status-${profile.status}${unavailable ? " is-unavailable-card" : ""}">
          <div class="profile-title-wrap">
            <p class="profile-title-account" title="${escapeHtml(profileTitle)}">${escapeHtml(profileTitle)}</p>
            <p class="${planClasses}"${planTooltip ? ` title="${escapeHtml(planTooltip)}"` : ""}>${escapeHtml(planLine(profile.plan_name, profile.subscription_days_left))}</p>
          </div>

          ${buildProfileQuotaMarkup(profile)}

          <div class="profile-card-actions${isWindowsUiTarget ? " profile-card-actions--windows" : ""}">
            ${
              hasDeleteProfileUi
                ? `<button
                    class="profile-action-button profile-action-button-danger"
                    type="button"
                    title="${deleteDisabled ? t(state.locale, "profileDeleteDisabled") : t(state.locale, "profileDeleteReady")}"
                    data-delete-profile="${profile.folder_name}"
                    ${deleteDisabled ? "disabled" : ""}
                  >
                    ${t(state.locale, "deleteButton")}
                  </button>`
                : ""
            }
            <button
              class="profile-action-button"
              type="button"
              title="${renameDisabled ? t(state.locale, "profileRenameDisabled") : t(state.locale, "profileRenameReady")}"
              data-rename-profile="${profile.folder_name}"
              ${renameDisabled ? "disabled" : ""}
            >
              ${t(state.locale, "rename")}
            </button>
            <button
              class="profile-action-button"
              type="button"
              title="${refreshTitle}"
              data-refresh-profile="${profile.folder_name}"
              ${refreshDisabled ? "disabled" : ""}
            >
              ${
                refreshPending
                  ? '<span class="button-spinner" aria-hidden="true"></span>'
                  : t(state.locale, "refreshButton")
              }
            </button>
            <button
              class="profile-action-button"
              type="button"
              title="${
                loginRunning
                  ? t(state.locale, "profileLoginCancelHint")
                  : loginDisabled
                    ? t(state.locale, "profileLoginDisabled")
                    : t(state.locale, "profileLoginReady")
              }"
              aria-label="${
                loginRunning
                  ? t(state.locale, "profileLoginCancelAria", { profile: profile.folder_name })
                  : t(state.locale, "profileLoginReadyAria", { profile: profile.folder_name })
              }"
              data-login-profile="${profile.folder_name}"
              ${loginDisabled ? "disabled" : ""}
            >
              ${
                loginRunning
                  ? `<span class="button-spinner" aria-hidden="true"></span><span class="button-cancel-label">${t(state.locale, "cancel")}</span>`
                  : t(state.locale, "loginButton")
              }
            </button>
            <button
              class="${
                profile.openai_base_url
                  ? "profile-action-button profile-action-button-danger"
                  : "profile-action-button"
              }"
              type="button"
              title="${
                profile.openai_base_url
                  ? t(state.locale, "profileBaseConfigured")
                  : t(state.locale, "profileBaseReady")
              }"
              data-base-url-profile="${profile.folder_name}"
              ${baseDisabled ? "disabled" : ""}
            >
              ${t(state.locale, "baseButton")}
            </button>
            <button
              class="profile-action-button"
              type="button"
              title="${switchDisabled ? t(state.locale, "profileSwitchDisabled") : t(state.locale, "profileSwitchReady")}"
              data-switch-profile="${profile.folder_name}"
              ${switchDisabled ? "disabled" : ""}
            >
              ${t(state.locale, "switch")}
            </button>
          </div>
        </article>
      `;
    })
    .join("");

  if (hasDeleteProfileUi) {
    bindProfileButtons("data-delete-profile", onDelete);
  }
  bindProfileButtons("data-rename-profile", onRename);
  bindProfileButtons("data-refresh-profile", onRefresh);
  bindProfileButtons("data-login-profile", onLogin);
  bindProfileButtons("data-base-url-profile", onBaseUrl);
  bindProfileButtons("data-switch-profile", onSwitch);
}

export function renderPaging(
  paging: Pick<PagingInfo, "has_previous" | "has_next" | "page" | "total_pages">,
): void {
  elements.previousPageButton.disabled = state.loading || !paging.has_previous;
  elements.nextPageButton.disabled = state.loading || !paging.has_next;
  elements.pageIndicator.textContent = `${paging.page} / ${paging.total_pages}`;
}

export function applyLocale(): void {
  document.documentElement.lang = state.locale;
  document.title = t(state.locale, "appTitle");

  for (const element of elements.localizedText) {
    const key = element.dataset.i18nKey as MessageKey | undefined;
    if (key) {
      element.textContent = t(state.locale, key);
    }
  }

  elements.profilesHeading.textContent = t(state.locale, "profilesHeading");
  elements.currentSectionHeading.textContent = t(state.locale, "currentSession");
  elements.controlDeckHeading.textContent = t(state.locale, "controlDeck");
  elements.currentLoginButton.textContent = t(state.locale, "login");
  elements.openCurrentFolderButton.textContent = t(state.locale, "openFolder");
  elements.addProfilesButton.textContent = t(state.locale, "addProfiles");
  elements.openCodexButton.textContent = t(state.locale, "openCodex");
  elements.starButton.textContent = t(state.locale, "star");
  elements.xiaohongshuButton.textContent = t(state.locale, "xiaohongshu");
  elements.previousPageButton.textContent = t(state.locale, "previous");
  elements.nextPageButton.textContent = t(state.locale, "next");
  elements.quotaMonitorLabel.textContent = t(state.locale, "quotaMonitor");
  elements.localeEnButton.textContent = t(state.locale, "languageEnglish");
  elements.localeZhButton.textContent = t(state.locale, "languageChinese");
  elements.localeEnButton.classList.toggle("is-active", state.locale === "en");
  elements.localeZhButton.classList.toggle("is-active", state.locale === "zh-CN");
  elements.localeEnButton.setAttribute("aria-pressed", state.locale === "en" ? "true" : "false");
  elements.localeZhButton.setAttribute("aria-pressed", state.locale === "zh-CN" ? "true" : "false");
  for (const button of elements.localeButtons) {
    const isActive = button.dataset.setLocale === state.locale;
    button.classList.toggle("is-active", isActive);
    button.setAttribute("aria-pressed", isActive ? "true" : "false");
  }
  renderThemeOptions();
  elements.dialogTitle.textContent = t(state.locale, "addProfileTitle");
  elements.dialogCopy.innerHTML = t(state.locale, "addProfileCopy")
    .replace("auth.json", "<code>auth.json</code>")
    .replace("profile.json", "<code>profile.json</code>");
  elements.renameDialogTitle.textContent = t(state.locale, "renameProfileTitle");
  elements.renameDialogCopy.textContent = t(state.locale, "renameProfileCopy");
  if (hasDeleteProfileUi) {
    elements.deleteProfileDialogTitle!.textContent = t(state.locale, "deleteProfileTitle");
    elements.deleteProfileDialogCopy!.textContent = t(state.locale, "deleteProfileCopy");
    elements.deleteProfileButton!.textContent = t(state.locale, "deleteCard");
    elements.clearProfileAccountButton!.textContent = t(state.locale, "clearAccount");
    elements.cancelDeleteProfileButton!.textContent = t(state.locale, "cancel");
  }
  const baseUrlCopy = t(state.locale, isWindowsUiTarget ? "baseUrlWindowsCopy" : "baseUrlCopy");
  elements.baseUrlDialogTitle.textContent = t(state.locale, "baseUrlTitle");
  elements.baseUrlDialogCopy.textContent = baseUrlCopy;
  elements.folderNameLabel.textContent = t(state.locale, "folderName");
  elements.addBaseUrlLabel.textContent = t(state.locale, "baseUrlLabel");
  elements.addBaseUrlInput.placeholder = t(state.locale, "baseUrlPlaceholder");
  elements.addBaseUrlCopy.textContent = baseUrlCopy;
  elements.renameFolderNameLabel.textContent = t(state.locale, "folderName");
  elements.baseUrlLabel.textContent = t(state.locale, "baseUrlLabel");
  elements.baseUrlInput.placeholder = t(state.locale, "baseUrlPlaceholder");
  elements.cancelAddProfileButton.textContent = t(state.locale, "cancel");
  elements.submitAddProfileButton.textContent = t(state.locale, "create");
  elements.cancelRenameProfileButton.textContent = t(state.locale, "cancel");
  elements.submitRenameProfileButton.textContent = t(state.locale, "rename");
  elements.cancelBaseUrlButton.textContent = t(state.locale, "cancel");
  elements.submitBaseUrlButton.textContent = t(state.locale, "save");
  elements.codexCliDialogTitle.textContent = t(state.locale, "codexCliDialogTitle");
  elements.codexCliDialogCopy.textContent = t(state.locale, "codexCliDialogCopy");
  elements.codexCliCurrentLabel.textContent = t(state.locale, "codexCliCurrentLabel");
  elements.codexCliInputLabel.textContent = t(state.locale, "codexCliInputLabel");
  elements.codexCliInput.placeholder = t(state.locale, "codexCliInputPlaceholder");
  elements.codexCliSuggestionsHeading.textContent = t(state.locale, "codexCliSuggestionsHeading");
  elements.cancelCodexCliButton.textContent = t(state.locale, "cancel");
  elements.clearCodexCliButton.textContent = t(state.locale, "codexCliClearOverride");
  elements.submitCodexCliButton.textContent = t(state.locale, "save");
  elements.settingsCodexCliLabel.textContent = t(state.locale, "settingsCodexCli");
  elements.settingsCodexCliButton.textContent = t(state.locale, "settingsCodexCliChange");
  elements.settingsCodexCliDetectButton.textContent = t(state.locale, "settingsCodexCliDetect");
  // Version label is locale-independent but lives next to the i18n
  // settings rows; set it here so a single render pass paints both.
  // `__CODEX_APP_VERSION__` is injected by Vite from `package.json` so
  // it stays in lock-step with the Cargo version automatically.
  elements.settingsVersionValue.textContent = __CODEX_APP_VERSION__;
}

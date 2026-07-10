import type {
  OpenAiUsageAccountSummary,
  OpenAiUsageDataSourceSummary,
  OpenAiUsageModelSummary,
  OpenAiUsageRangePreset,
  OpenAiUsageRequestLogPage,
  OpenAiUsageScope,
  OpenAiUsageSummary,
  OpenAiUsageTrendPoint,
} from "../types";
import type { Locale } from "../i18n";

export interface UsageWorkspaceView {
  readonly summary: OpenAiUsageSummary | null;
  readonly accountSummary: readonly OpenAiUsageAccountSummary[];
  readonly modelOptions: readonly OpenAiUsageModelSummary[];
  readonly trends: readonly OpenAiUsageTrendPoint[];
  readonly logs: OpenAiUsageRequestLogPage | null;
  readonly dataSources: readonly OpenAiUsageDataSourceSummary[];
  readonly rangePreset: OpenAiUsageRangePreset;
  readonly scope: OpenAiUsageScope;
  readonly customStartMs: number | null;
  readonly customEndMs: number | null;
  readonly liveEnd: boolean;
  readonly rangePickerOpen: boolean;
  readonly rangeDraftStartMs: number | null;
  readonly rangeDraftEndMs: number | null;
  readonly rangeDraftLiveEnd: boolean;
  readonly rangeDraftActiveField: "start" | "end";
  readonly rangeDisplayMonthMs: number | null;
  readonly rangeError: string | null;
  readonly profileFilter: string | null;
  readonly modelFilter: string | null;
  readonly statusCodeFilter: number | null;
  readonly logsPage: number;
  readonly logsPageSize: number;
  readonly refreshIntervalMs: number;
  readonly lastResolvedEndMs: number | null;
  readonly locale: Locale;
  readonly loaded: boolean;
  readonly loading: boolean;
  readonly logsLoading: boolean;
  readonly refreshInFlight: boolean;
  readonly loadError: string | null;
  readonly refreshQueued: boolean;
}

export type UsageRefreshMode = "initial" | "manual" | "auto";

export type UsageWorkspaceEvent =
  | { readonly type: "set-context"; readonly active: boolean; readonly locale: Locale }
  | { readonly type: "open-range-picker" }
  | { readonly type: "close-range-picker" }
  | { readonly type: "set-range-draft-field"; readonly field: "start" | "end" }
  | { readonly type: "set-range-draft-date"; readonly field: "start" | "end"; readonly value: string }
  | { readonly type: "set-range-draft-time"; readonly field: "start" | "end"; readonly value: string }
  | { readonly type: "pick-range-day"; readonly dayMs: number }
  | { readonly type: "move-range-month"; readonly delta: number }
  | { readonly type: "show-range-today" }
  | { readonly type: "set-range-draft-live-end"; readonly live: boolean }
  | { readonly type: "apply-range-draft" }
  | {
    readonly type: "set-range";
    readonly preset: OpenAiUsageRangePreset;
    readonly startMs?: number | null;
    readonly endMs?: number | null;
    readonly liveEnd?: boolean;
  }
  | {
    readonly type: "set-filters";
    readonly profile?: string | null;
    readonly accountGroupKey?: string | null;
    readonly accountLabel?: string | null;
    readonly model?: string | null;
    readonly statusCode?: number | null;
    readonly scope?: OpenAiUsageScope;
  }
  | { readonly type: "set-page"; readonly page: number; readonly pageSize?: number }
  | { readonly type: "set-refresh-interval"; readonly intervalMs: number }
  | { readonly type: "refresh"; readonly mode: UsageRefreshMode };

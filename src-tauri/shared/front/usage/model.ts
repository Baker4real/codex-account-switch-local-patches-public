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

export interface UsageWorkspaceView {
  readonly summary: OpenAiUsageSummary | null;
  readonly accountSummary: readonly OpenAiUsageAccountSummary[];
  readonly modelOptions: readonly OpenAiUsageModelSummary[];
  readonly trends: readonly OpenAiUsageTrendPoint[];
  readonly logs: OpenAiUsageRequestLogPage | null;
  readonly dataSources: readonly OpenAiUsageDataSourceSummary[];
  readonly rangePreset: OpenAiUsageRangePreset;
  readonly scope: OpenAiUsageScope;
  readonly logsPage: number;
  readonly logsPageSize: number;
  readonly loaded: boolean;
  readonly loading: boolean;
  readonly loadError: string | null;
  readonly refreshQueued: boolean;
}

export type UsageRefreshMode = "initial" | "manual" | "auto";

export type UsageWorkspaceEvent =
  | { readonly type: "set-context"; readonly active: boolean; readonly locale: string }
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

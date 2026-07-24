import { describe, expect, it } from "vitest";
// @ts-expect-error The production TypeScript config omits Node typings; Vitest runs in Node.
import { readFileSync } from "node:fs";

const windowsMarkup = String(readFileSync(
  new URL("./index.html", import.meta.url),
  "utf8",
));

const windowsStyles = String(readFileSync(
  new URL("./styles.css", import.meta.url),
  "utf8",
));

const tauriEntry = String(readFileSync(
  new URL("../../src/lib.rs", import.meta.url),
  "utf8",
));

describe("Windows 功能表面", () => {
  it("停用使用统计入口和 IPC，同时保留账号额度命令", () => {
    expect(windowsMarkup).not.toContain('data-route-tab="usage"');
    expect(windowsMarkup).not.toContain('data-page="usage"');
    expect(tauriEntry).not.toContain("commands::usage::");
    expect(tauriEntry).toContain("commands::dashboard::get_current_live_quota");
  });

  it("把重启 Codex 放在仪表盘标题之后、状态指标之前", () => {
    const dashboard = windowsMarkup.match(
      /<section class="page active" id="page-dashboard"[\s\S]*?<section class="page" id="page-profiles"/,
    )?.[0] ?? "";
    const titleIndex = dashboard.indexOf('data-i18n-key="dashboardOverviewTitle"');
    const restartIndex = dashboard.indexOf('id="restart-codex-button"');
    const statusIndex = dashboard.indexOf('class="status-grid"');

    expect(titleIndex).toBeGreaterThanOrEqual(0);
    expect(restartIndex).toBeGreaterThan(titleIndex);
    expect(restartIndex).toBeLessThan(statusIndex);

    const restartRules = windowsStyles.match(/\.control-button--restart\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(restartRules).toMatch(/color:\s*var\(--text-strong\)/);
    expect(restartRules).not.toMatch(/height:/);
    expect(restartRules).not.toMatch(/grid-column:/);
  });
});

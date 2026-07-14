import { describe, expect, it } from "vitest";
// @ts-expect-error The production TypeScript config omits Node typings; Vitest runs in Node.
import { readFileSync } from "node:fs";
import { collapseDisplayWhitespace } from "./display-text";

describe("account display text", () => {
  it("将账号标题中的换行和连续空白折叠为单个空格", () => {
    expect(collapseDisplayWhitespace("  b\r\n·   user@example.com  "))
      .toBe("b · user@example.com");
  });

  it("Windows 账号标题保持单行并在溢出时省略", () => {
    const windowsStyles = String(readFileSync(
      new URL("../../win/front/styles.css", import.meta.url),
      "utf8",
    ));
    for (const selector of ["profile-title-account", "current-account-title"]) {
      const block = windowsStyles.match(new RegExp(`\\.${selector}\\s*\\{([^}]*)\\}`, "s"))?.[1] ?? "";
      expect(block).toMatch(/white-space:\s*nowrap/);
      expect(block).toMatch(/overflow:\s*hidden/);
      expect(block).toMatch(/text-overflow:\s*ellipsis/);
      expect(block).not.toMatch(/overflow-wrap:\s*anywhere/);
    }
    const titleWrapRules = Array.from(
      windowsStyles.matchAll(/\.profile-title-wrap\s*\{([^}]*)\}/gs),
      (match) => match[1] ?? "",
    ).join("\n");
    expect(titleWrapRules).toMatch(/flex-wrap:\s*nowrap/);
  });

  it("Windows 账号卡片为两项额度保留稳定的并排布局", () => {
    const windowsStyles = String(readFileSync(
      new URL("../../win/front/styles.css", import.meta.url),
      "utf8",
    ));
    const quotaStack = Array.from(
      windowsStyles.matchAll(/\.profile-quota-stack\s*\{([^}]*)\}/gs),
      (match) => match[1] ?? "",
    ).join("\n");
    const quotaMetrics = Array.from(
      windowsStyles.matchAll(/\.profile-quota-metrics\s*\{([^}]*)\}/gs),
      (match) => match[1] ?? "",
    ).join("\n");
    expect(quotaStack).toMatch(/display:\s*grid/);
    expect(quotaStack).toMatch(/grid-template-areas:/);
    expect(quotaMetrics).toMatch(/display:\s*grid/);
    expect(quotaMetrics).toMatch(/repeat\(2,\s*minmax\(/);
    const refreshRules = Array.from(
      windowsStyles.matchAll(/\.profile-quota-refresh[^\{]*\{([^}]*)\}/gs),
      (match) => match[1] ?? "",
    ).join("\n");
    expect(refreshRules).toMatch(/white-space:\s*nowrap/);

    const mediumLayout = windowsStyles.slice(
      windowsStyles.indexOf("@media (max-width: 1180px)"),
      windowsStyles.indexOf("@media (max-width: 860px)"),
    );
    expect(mediumLayout).toMatch(/\.profile-card\s*\{[^}]*grid-template-columns:\s*1fr/s);
    expect(mediumLayout).toMatch(/"title"\s*"quotas"\s*"actions"/s);
  });

  it("Windows 控制区把重启 Codex 放在首位并突出显示", () => {
    const windowsMarkup = String(readFileSync(
      new URL("../../win/front/index.html", import.meta.url),
      "utf8",
    ));
    const windowsStyles = String(readFileSync(
      new URL("../../win/front/styles.css", import.meta.url),
      "utf8",
    ));
    const controls = windowsMarkup.match(/<div class="control-stack control-stack--grid">([\s\S]*?)<\/div>/)?.[1] ?? "";
    expect(controls.indexOf('id="restart-codex-button"')).toBeGreaterThanOrEqual(0);
    expect(controls.indexOf('id="restart-codex-button"')).toBeLessThan(controls.indexOf('id="open-codex-button"'));
    expect(controls).toMatch(/id="restart-codex-button" class="[^"]*control-button--restart/);
    const restartRules = windowsStyles.match(/\.control-button--restart\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(restartRules).toMatch(/color:\s*var\(--text-strong\)/);
    expect(restartRules).not.toMatch(/height:/);
    expect(restartRules).not.toMatch(/grid-column:/);
  });
});

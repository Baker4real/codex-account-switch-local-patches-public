import { describe, expect, it } from "vitest";

import {
  deviceCodeButtonState,
  promptForActiveDeviceLogin,
  runDeviceCodeAuthorizationFlow,
} from "./device-code-login";

describe("device-code login prompt", () => {
  it("accepts the verification prompt only for the active profile", () => {
    const prompt = promptForActiveDeviceLogin("b", {
      profile: "b",
      verificationUrl: "https://auth.openai.com/codex/device",
      userCode: "ABCD-1234",
    });

    expect(prompt).toEqual({
      profile: "b",
      verificationUrl: "https://auth.openai.com/codex/device",
      userCode: "ABCD-1234",
    });
    expect(
      promptForActiveDeviceLogin("a", {
        profile: "b",
        verificationUrl: "https://auth.openai.com/codex/device",
        userCode: "ABCD-1234",
      }),
    ).toBeNull();
  });

  it("offers device-code login on every idle profile and cancellation only on its active card", () => {
    expect(deviceCodeButtonState("a", false, false, null, null)).toEqual({
      disabled: false,
      running: false,
    });
    expect(deviceCodeButtonState("b", false, false, "b", "deviceCode")).toEqual({
      disabled: false,
      running: true,
    });
    expect(deviceCodeButtonState("c", false, false, "b", "deviceCode")).toEqual({
      disabled: true,
      running: false,
    });
  });

  it("leaves the cancellable authorization state before refreshing dashboard data", async () => {
    const events: string[] = [];

    await runDeviceCodeAuthorizationFlow(
      async () => {
        events.push("authorized");
      },
      () => {
        events.push("authorization-ui-closed");
      },
      async () => {
        events.push("dashboard-refresh");
      },
    );

    expect(events).toEqual([
      "authorized",
      "authorization-ui-closed",
      "dashboard-refresh",
    ]);
  });
});

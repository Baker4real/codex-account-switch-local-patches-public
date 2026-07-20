import { describe, expect, it, vi } from "vitest";

import {
  isCurrentLoginAttempt,
  scheduleLoginCancelRetry,
  type LoginAttempt,
} from "./login-attempt";

describe("login attempt generations", () => {
  it("does not let an old cancel retry affect a newer login on the same profile", () => {
    const oldAttempt: LoginAttempt = { id: 1, profile: "b" };
    let activeAttempt: LoginAttempt | null = oldAttempt;
    let scheduledCallback: (() => void) | null = null;
    const retry = vi.fn();

    scheduleLoginCancelRetry(
      oldAttempt,
      () => activeAttempt,
      retry,
      250,
      (callback) => {
        scheduledCallback = callback;
      },
    );

    activeAttempt = { id: 2, profile: "b" };
    expect(isCurrentLoginAttempt(activeAttempt, oldAttempt)).toBe(false);
    expect(scheduledCallback).not.toBeNull();
    (scheduledCallback as unknown as () => void)();
    expect(retry).not.toHaveBeenCalled();
  });
});

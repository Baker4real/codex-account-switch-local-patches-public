import { describe, expect, it } from "vitest";

import type { UsageWorkspace } from "./workspace";
import { replaceUsageWorkspace } from "./index";

function fakeWorkspace(onDispose: () => void): UsageWorkspace {
  return {
    dispatch: async () => undefined,
    snapshot: () => {
      throw new Error("snapshot is not used by this test");
    },
    dispose: onDispose,
  };
}

describe("replaceUsageWorkspace", () => {
  it("挂载新实例前先 dispose 旧实例", () => {
    const events: string[] = [];
    const current = fakeWorkspace(() => events.push("dispose"));
    const replacement = fakeWorkspace(() => undefined);

    const mounted = replaceUsageWorkspace(current, () => {
      events.push("mount");
      return replacement;
    });

    expect(events).toEqual(["dispose", "mount"]);
    expect(mounted).toBe(replacement);
  });

  it("首次挂载没有旧实例时直接 mount", () => {
    const events: string[] = [];
    const replacement = fakeWorkspace(() => undefined);

    const mounted = replaceUsageWorkspace(null, () => {
      events.push("mount");
      return replacement;
    });

    expect(events).toEqual(["mount"]);
    expect(mounted).toBe(replacement);
  });
});

import type { UsageWorkspace } from "./workspace";

export function replaceUsageWorkspace(
  current: UsageWorkspace | null,
  mount: () => UsageWorkspace,
): UsageWorkspace {
  current?.dispose();
  return mount();
}

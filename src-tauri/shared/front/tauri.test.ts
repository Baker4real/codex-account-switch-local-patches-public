import { beforeEach, describe, expect, it, vi } from "vitest";

beforeEach(() => {
  Reflect.deleteProperty(globalThis, "__TAURI_INTERNALS__");
  Reflect.deleteProperty(globalThis, "__TAURI__");
  vi.resetModules();
});

describe("preview authorization method state", () => {
  it("records the method used by each profile login command", async () => {
    const { getProfilesSnapshot, loginProfile, loginProfileDeviceCode } = await import("./tauri");

    await loginProfile("workspace-beta");
    await loginProfileDeviceCode("workspace-alpha");

    const snapshot = await getProfilesSnapshot();
    expect(
      snapshot.profiles.find((profile) => profile.folder_name === "workspace-beta")
        ?.last_authorization_method,
    ).toBe("browser");
    expect(
      snapshot.profiles.find((profile) => profile.folder_name === "workspace-alpha")
        ?.last_authorization_method,
    ).toBe("device_code");
  });

  it("records browser authorization for the current profile login command", async () => {
    const { getProfilesSnapshot, loginCurrentProfile, switchProfile } = await import("./tauri");

    await switchProfile("workspace-beta");
    await loginCurrentProfile();

    const snapshot = await getProfilesSnapshot();
    expect(
      snapshot.profiles.find((profile) => profile.folder_name === "workspace-beta")
        ?.last_authorization_method,
    ).toBe("browser");
  });

  it("clears the authorization method with the account identity", async () => {
    const { clearProfileAccount, getProfilesSnapshot } = await import("./tauri");

    await clearProfileAccount("workspace-beta");

    const snapshot = await getProfilesSnapshot();
    expect(
      snapshot.profiles.find((profile) => profile.folder_name === "workspace-beta")
        ?.last_authorization_method,
    ).toBeNull();
  });
});

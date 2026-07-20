export interface DeviceCodePromptPayload {
  profile: string;
  verificationUrl: string;
  userCode: string;
}

export type LoginKind = "browser" | "deviceCode";

export interface DeviceCodeButtonView {
  disabled: boolean;
  running: boolean;
}

export function deviceCodeButtonState(
  profile: string,
  loading: boolean,
  refreshPending: boolean,
  loginActiveProfile: string | null,
  loginActiveKind: LoginKind | null,
): DeviceCodeButtonView {
  const running = loginActiveProfile === profile && loginActiveKind === "deviceCode";
  return {
    disabled: running ? false : loading || refreshPending || loginActiveProfile !== null,
    running,
  };
}

export function promptForActiveDeviceLogin(
  activeProfile: string | null,
  payload: DeviceCodePromptPayload,
): DeviceCodePromptPayload | null {
  const profile = payload.profile.trim();
  const verificationUrl = payload.verificationUrl.trim();
  const userCode = payload.userCode.trim();
  if (
    activeProfile !== profile
    || !verificationUrl
    || !userCode
  ) {
    return null;
  }

  return { profile, verificationUrl, userCode };
}

export async function runDeviceCodeAuthorizationFlow<T>(
  authorize: () => Promise<T>,
  onAuthorized: () => void,
  refreshDashboard: () => Promise<void>,
): Promise<T> {
  const result = await authorize();
  onAuthorized();
  await refreshDashboard();
  return result;
}

const SHOW_ADDITIONAL_QUOTA_STORAGE_KEY = "codex-switch-show-additional-quota";
const SHOW_SPARK_QUOTA_STORAGE_KEY = "codex-switch-show-spark-quota";

function readBoolean(key: string, fallback: boolean): boolean {
  try {
    const stored = globalThis.localStorage?.getItem(key);
    return stored == null ? fallback : stored === "1";
  } catch {
    return fallback;
  }
}

function writeBoolean(key: string, value: boolean): boolean {
  try {
    if (!globalThis.localStorage) {
      return false;
    }
    globalThis.localStorage.setItem(key, value ? "1" : "0");
    return globalThis.localStorage.getItem(key) === (value ? "1" : "0");
  } catch {
    return false;
  }
}

export function resolveInitialAdditionalQuotaVisibility(): boolean {
  return readBoolean(SHOW_ADDITIONAL_QUOTA_STORAGE_KEY, true);
}

export function resolveInitialSparkQuotaVisibility(): boolean {
  return readBoolean(SHOW_SPARK_QUOTA_STORAGE_KEY, false);
}

export function persistAdditionalQuotaVisibility(value: boolean): boolean {
  return writeBoolean(SHOW_ADDITIONAL_QUOTA_STORAGE_KEY, value);
}

export function persistSparkQuotaVisibility(value: boolean): boolean {
  return writeBoolean(SHOW_SPARK_QUOTA_STORAGE_KEY, value);
}

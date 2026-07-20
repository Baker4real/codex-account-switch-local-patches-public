export interface LoginAttempt {
  id: number;
  profile: string;
}

type RetryScheduler = (callback: () => void, delayMs: number) => void;

export function isCurrentLoginAttempt(
  activeAttempt: LoginAttempt | null,
  expectedAttempt: LoginAttempt,
): boolean {
  return activeAttempt?.id === expectedAttempt.id
    && activeAttempt.profile === expectedAttempt.profile;
}

export function scheduleLoginCancelRetry(
  expectedAttempt: LoginAttempt,
  getActiveAttempt: () => LoginAttempt | null,
  retry: () => void,
  delayMs: number,
  schedule: RetryScheduler = (callback, delay) => {
    globalThis.setTimeout(callback, delay);
  },
): void {
  schedule(() => {
    if (isCurrentLoginAttempt(getActiveAttempt(), expectedAttempt)) {
      retry();
    }
  }, delayMs);
}

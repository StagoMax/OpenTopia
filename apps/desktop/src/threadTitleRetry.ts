const retryDelaysMs = [30_000, 120_000] as const;

/** Returns the wait time after a consecutive title-generation failure. */
export function threadTitleRetryDelay(failureCount: number): number | null {
  return retryDelaysMs[failureCount - 1] ?? null;
}

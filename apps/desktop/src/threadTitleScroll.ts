const THREAD_TITLE_SCROLL_PIXELS_PER_SECOND = 80;
const THREAD_TITLE_SCROLL_MIN_DURATION_MS = 600;

export function threadTitleScrollDurationMs(distancePx: number): number {
  if (!Number.isFinite(distancePx) || distancePx <= 0) return 0;

  return Math.max(
    THREAD_TITLE_SCROLL_MIN_DURATION_MS,
    Math.ceil((distancePx / THREAD_TITLE_SCROLL_PIXELS_PER_SECOND) * 1_000),
  );
}

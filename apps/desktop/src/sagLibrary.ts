import type { SagNeedCoverage, SagSource } from "./types";

export function filterSagSources(
  sources: readonly SagSource[],
  query: string,
): SagSource[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return [...sources];
  return sources.filter((source) =>
    [
      source.title,
      source.originalFilename,
      source.namespace,
      source.sourceKey,
    ].some((value) => value.toLocaleLowerCase().includes(normalized)),
  );
}

export function coverageByNeed(
  coverage: readonly SagNeedCoverage[],
): Map<string, SagNeedCoverage> {
  return new Map(coverage.map((item) => [item.needId, item]));
}

export function parseSagMetadata(value: string): Record<string, unknown> {
  const trimmed = value.trim();
  if (!trimmed) return {};
  const parsed: unknown = JSON.parse(trimmed);
  if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") {
    throw new Error("元数据必须是 JSON 对象，例如 {\"department\":\"sales\"}");
  }
  return parsed as Record<string, unknown>;
}

export function sagErrorMessage(cause: unknown): string {
  const fallback = cause instanceof Error ? cause.message : String(cause);
  try {
    const payload = JSON.parse(fallback) as { error?: unknown; detail?: unknown };
    const message = payload.error ?? payload.detail;
    return typeof message === "string" ? message : fallback;
  } catch {
    return fallback;
  }
}

import type { ApiClient } from "../../api/client";
import type { SagSource } from "../../types";

const SAG_SOURCE_PAGE_LIMIT = 200;

export type SagNamespaceOption = {
  namespace: string;
  sourceCount: number;
};

type SagSourceClient = Pick<ApiClient, "listLibrarySources">;

export async function loadSagNamespaceOptions(
  client: SagSourceClient,
  signal?: AbortSignal,
): Promise<SagNamespaceOption[]> {
  const sourceCounts = new Map<string, number>();
  let offset = 0;

  while (true) {
    const page = await client.listLibrarySources(
      "sag",
      { offset, limit: SAG_SOURCE_PAGE_LIMIT },
      signal,
    );
    const sources = page.items as SagSource[];
    for (const source of sources) {
      const namespace = source.namespace.trim();
      if (!namespace) continue;
      sourceCounts.set(namespace, (sourceCounts.get(namespace) ?? 0) + 1);
    }

    if (!page.hasMore || sources.length === 0) break;
    offset += sources.length;
  }

  return [...sourceCounts.entries()]
    .map(([namespace, sourceCount]) => ({ namespace, sourceCount }))
    .sort((left, right) => left.namespace.localeCompare(right.namespace));
}

export function parseSagNamespaceSelection(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\n,]/)
        .map((namespace) => namespace.trim())
        .filter(Boolean),
    ),
  ];
}

export function toggleSagNamespaceSelection(
  value: string,
  namespace: string,
  selected: boolean,
): string {
  const current = parseSagNamespaceSelection(value);
  const next = selected
    ? [...current.filter((item) => item !== namespace), namespace]
    : current.filter((item) => item !== namespace);
  return next.join(", ");
}

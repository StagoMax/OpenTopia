import path from "node:path";

export function buildExtractionPlan(root, entries) {
  return entries.map((entry) => ({
    ...entry,
    destination: path.join(root, entry.path),
    target: entry.target ?? null,
  }));
}

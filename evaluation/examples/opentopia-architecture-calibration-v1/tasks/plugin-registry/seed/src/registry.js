import { selectDefinitions } from "./merge.js";

export function resolveRegistry(layers) {
  return selectDefinitions(layers).sort((a, b) => a.id.localeCompare(b.id));
}

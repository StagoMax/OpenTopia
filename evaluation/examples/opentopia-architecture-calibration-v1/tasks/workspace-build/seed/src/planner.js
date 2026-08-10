import { hashInputs } from "./hash.js";

export function planBuild(packages, previousCache = {}) {
  const cache = Object.fromEntries(packages.map((item) => [item.name, hashInputs(item.inputs)]));
  const rebuild = packages.filter((item) => cache[item.name] !== previousCache[item.name]).map((item) => item.name);
  return { waves: [packages.map((item) => item.name)], rebuild, cache };
}

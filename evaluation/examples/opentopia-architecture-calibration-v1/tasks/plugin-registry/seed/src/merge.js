export function selectDefinitions(layers) {
  const selected = new Map();
  for (const layer of layers) {
    for (const plugin of layer.plugins) selected.set(plugin.id, plugin);
  }
  return [...selected.values()].filter((plugin) => plugin.enabled !== false);
}

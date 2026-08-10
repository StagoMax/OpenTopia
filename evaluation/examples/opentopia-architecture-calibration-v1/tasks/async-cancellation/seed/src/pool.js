export async function runPool(items, worker, { concurrency = 2, signal } = {}) {
  const results = [];
  let cursor = 0;
  async function consume() {
    while (cursor < items.length) {
      const index = cursor++;
      results.push(await worker(items[index], index));
    }
  }
  await Promise.all(Array.from({ length: concurrency }, consume));
  return results;
}

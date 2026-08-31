import type { ActivityEntry } from "./model.ts";

export const activityVirtualizationThreshold = 24;
export const activityVirtualChunkSize = 8;
// Bottom-follow can move the viewport before IntersectionObserver publishes its
// next intersection set. Keep one bounded viewport-sized tail rendered so a
// programmatic jump never lands on height-only placeholders.
export const activityVirtualPinnedTailChunkCount = 3;

export type ActivityVirtualChunk = {
  key: string;
  entries: ActivityEntry[];
  estimatedHeight: number;
};

export function buildActivityVirtualChunks(
  entries: ActivityEntry[],
  chunkSize = activityVirtualChunkSize,
): ActivityVirtualChunk[] {
  const chunks: ActivityVirtualChunk[] = [];
  for (let start = 0; start < entries.length; start += chunkSize) {
    const chunkEntries = entries.slice(start, start + chunkSize);
    chunks.push({
      key: `${start}:${activityVirtualEntryKey(chunkEntries[0])}`,
      entries: chunkEntries,
      estimatedHeight: chunkEntries.reduce(
        (height, entry) => height + estimateActivityEntryHeight(entry),
        0,
      ),
    });
  }
  return chunks;
}

export function shouldKeepActivityVirtualChunkMounted(
  index: number,
  chunkCount: number,
): boolean {
  return index >= Math.max(0, chunkCount - activityVirtualPinnedTailChunkCount);
}

function activityVirtualEntryKey(entry: ActivityEntry): string {
  if (entry.kind === "tool-group" || entry.kind === "file-group") {
    return entry.id;
  }
  if (entry.kind === "guardian-review") {
    return `guardian-review-${entry.reviewId}`;
  }
  if (entry.kind === "context-compaction") {
    return `context-compaction-${entry.requestId}`;
  }
  return `${entry.kind}-${entry.seq}`;
}

export function estimateActivityEntryHeight(entry: ActivityEntry): number {
  if (entry.kind === "commentary") {
    const visualLines = Math.ceil(entry.text.length / 72);
    return Math.min(320, 44 + visualLines * 22);
  }
  if (entry.kind === "work-form") {
    return 48 + Math.min(entry.form.items.length, 8) * 30;
  }
  if (entry.kind === "context-compaction") return 88;
  if (entry.kind === "guardian-review") return 64;
  return 36;
}

/**
 * Chromium exposes operating-system file drags through the reserved `Files`
 * transfer type. Keeping this check shared prevents text or in-app drags from
 * being captured by attachment drop zones.
 */
export function hasFileDragPayload(types: Iterable<string>): boolean {
  for (const type of types) {
    if (type === "Files") return true;
  }
  return false;
}

export type ThreadActivityReadAt = Readonly<Record<string, string>>;

const legacyCompletionReadStorageKey = "opentopia.threadCompletionRead.v1";
const threadActivityReadStorageKey = "opentopia.threadActivityRead.v1";

function parseReadAt(value: string | null): ThreadActivityReadAt {
  try {
    const stored = JSON.parse(value ?? "{}") as unknown;
    if (!stored || typeof stored !== "object" || Array.isArray(stored)) {
      return {};
    }

    return Object.fromEntries(
      Object.entries(stored).filter(
        ([threadId, readAt]) =>
          threadId.length > 0 &&
          typeof readAt === "string" &&
          Number.isFinite(Date.parse(readAt)),
      ),
    );
  } catch {
    return {};
  }
}

export function readThreadActivityReadAt(): ThreadActivityReadAt {
  if (typeof window === "undefined") return {};
  try {
    return {
      ...parseReadAt(
        window.localStorage.getItem(legacyCompletionReadStorageKey),
      ),
      ...parseReadAt(window.localStorage.getItem(threadActivityReadStorageKey)),
    };
  } catch {
    return {};
  }
}

export function writeThreadActivityReadAt(
  readAtByThread: ThreadActivityReadAt,
): void {
  try {
    window.localStorage.setItem(
      threadActivityReadStorageKey,
      JSON.stringify(readAtByThread),
    );
  } catch {
    // Read markers remain available for the current session if storage fails.
  }
}

export function isThreadActivityUnread(
  readAtByThread: ThreadActivityReadAt,
  threadId: string,
  updatedAt: string | null | undefined,
): boolean {
  const readAt = readAtByThread[threadId];
  if (!readAt) return true;
  if (!updatedAt) return false;

  const updatedAtMs = Date.parse(updatedAt);
  const readAtMs = Date.parse(readAt);
  if (!Number.isFinite(updatedAtMs) || !Number.isFinite(readAtMs)) {
    return true;
  }
  return readAtMs < updatedAtMs;
}

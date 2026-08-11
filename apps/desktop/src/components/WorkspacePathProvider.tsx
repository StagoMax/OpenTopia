import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
} from "react";
import type { ApiClient } from "../api/client";
import {
  WorkspacePathIndex,
  type WorkspacePathStatus,
} from "../workspacePathIndex.ts";

export const WorkspacePathIndexContext =
  createContext<WorkspacePathIndex | null>(null);

/** Builds the index the active task's markdown links are verified against. */
export function useWorkspacePathIndex({
  client,
  threadId,
  workspaceRoot,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
}): WorkspacePathIndex | null {
  return useMemo(() => {
    if (!client || !threadId) return null;
    return new WorkspacePathIndex({
      workspaceRoot,
      async listDirectory(directory) {
        const tree = await client.listWorkspaceTree(
          threadId,
          directory || undefined,
        );
        return tree.entries
          .filter((entry) => entry.kind === "file" || entry.kind === "symlink")
          .map((entry) => entry.name);
      },
      async readTextFile(path) {
        const preview = await client.readWorkspaceFile(threadId, path);
        if (preview.truncated) {
          throw new Error("File is too large to copy in full.");
        }
        return preview.content;
      },
    });
  }, [client, threadId, workspaceRoot]);
}

/** Reports whether a mentioned path exists, re-rendering once it is known. */
export function useWorkspacePathStatus(
  path: string | null,
): WorkspacePathStatus {
  const index = useContext(WorkspacePathIndexContext);
  const [, notifyChange] = useReducer((tick: number) => tick + 1, 0);

  useEffect(() => {
    if (!index || !path) return;
    return index.watch(path, notifyChange);
  }, [index, path]);

  if (!index || !path) return "unknown";
  return index.status(path);
}

/** Resolves a verified Markdown target to the path used by desktop file APIs. */
export function useWorkspaceAbsolutePath(path: string | null): string | null {
  const index = useContext(WorkspacePathIndexContext);
  return index && path ? index.absolutePath(path) : null;
}

/** Returns a task-scoped reader that cannot escape the active workspace. */
export function useWorkspaceFileTextReader(
  path: string | null,
): (() => Promise<string>) | null {
  const index = useContext(WorkspacePathIndexContext);
  return index && path ? () => index.readTextFile(path) : null;
}

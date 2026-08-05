import type { Project, Thread } from "./types";
import {
  threadActivityStatusPriority,
  type ThreadActivityStatus,
} from "./threadActivityStatus.ts";

export type TaskSearchResult = {
  projectName: string;
  status?: ThreadActivityStatus;
  thread: Thread;
};

function normalizeSearchText(value: string) {
  return value.normalize("NFKC").toLocaleLowerCase().trim();
}

function workspaceName(workspaceRoot: string) {
  const segments = workspaceRoot.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? workspaceRoot;
}

export function searchTasks(
  threads: Thread[],
  projects: Project[],
  activityStatuses: Record<string, ThreadActivityStatus>,
  query: string,
): TaskSearchResult[] {
  const projectNames = new Map(
    projects.map((project) => [project.id, project.name]),
  );
  const terms = normalizeSearchText(query).split(/\s+/).filter(Boolean);

  return threads
    .filter((thread) => !thread.archivedAt)
    .map((thread) => {
      const projectName = thread.projectId
        ? (projectNames.get(thread.projectId) ??
          workspaceName(thread.workspaceRoot))
        : workspaceName(thread.workspaceRoot);
      const searchableText = normalizeSearchText(
        `${thread.title} ${projectName} ${thread.workspaceRoot}`,
      );

      return {
        projectName,
        status: activityStatuses[thread.id],
        thread,
        matches: terms.every((term) => searchableText.includes(term)),
      };
    })
    .filter((result) => result.matches)
    .sort((left, right) => {
      const leftPriority =
        left.status === undefined
          ? 4
          : threadActivityStatusPriority[left.status];
      const rightPriority =
        right.status === undefined
          ? 4
          : threadActivityStatusPriority[right.status];
      if (leftPriority !== rightPriority) return leftPriority - rightPriority;

      const updatedDifference =
        Date.parse(right.thread.updatedAt) - Date.parse(left.thread.updatedAt);
      if (updatedDifference !== 0) return updatedDifference;
      return left.thread.title.localeCompare(right.thread.title);
    })
    .map(({ matches: _matches, ...result }) => result);
}

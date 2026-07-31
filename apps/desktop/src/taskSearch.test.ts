import assert from "node:assert/strict";
import test from "node:test";

import type * as TaskSearchModule from "./taskSearch";
import type { Project, Thread } from "./types";

const { searchTasks }: typeof TaskSearchModule = await import(
  "./taskSearch" + ".ts"
);
type TaskSearchActivityStatus = TaskSearchModule.TaskSearchActivityStatus;

const projects: Project[] = [
  {
    id: "project-open",
    name: "OpenTopia",
    workspaceRoot: "J:\\Project\\OpenTopia",
    pinned: true,
    sortOrder: 0,
    createdAt: "2026-07-01T00:00:00Z",
    updatedAt: "2026-07-01T00:00:00Z",
  },
  {
    id: "project-rag",
    name: "RAG Lab",
    workspaceRoot: "J:\\Project\\RAG",
    pinned: false,
    sortOrder: 1,
    createdAt: "2026-07-01T00:00:00Z",
    updatedAt: "2026-07-01T00:00:00Z",
  },
];

function thread(
  overrides: Partial<Thread> & Pick<Thread, "id" | "title">,
): Thread {
  return {
    workspaceRoot: "J:\\Project\\OpenTopia",
    projectId: "project-open",
    experienceMode: "code",
    modelSelection: null,
    archivedAt: null,
    createdAt: "2026-07-01T00:00:00Z",
    updatedAt: "2026-07-20T00:00:00Z",
    ...overrides,
  };
}

test("searches task titles, project names, and workspace paths", () => {
  const threads = [
    thread({ id: "alpha", title: "Fix Search Dialog" }),
    thread({
      id: "rag",
      title: "Evaluate retrieval",
      projectId: "project-rag",
      workspaceRoot: "J:\\Project\\RAG",
    }),
  ];

  assert.deepEqual(
    searchTasks(threads, projects, {}, "search dialog").map(
      (result) => result.thread.id,
    ),
    ["alpha"],
  );
  assert.deepEqual(
    searchTasks(threads, projects, {}, "rag lab").map(
      (result) => result.thread.id,
    ),
    ["rag"],
  );
});

test("excludes archived tasks and prioritizes tasks with active statuses", () => {
  const threads = [
    thread({
      id: "recent",
      title: "Recent",
      updatedAt: "2026-07-30T00:00:00Z",
    }),
    thread({
      id: "running",
      title: "Running",
      updatedAt: "2026-07-10T00:00:00Z",
    }),
    thread({
      id: "approval",
      title: "Approval",
      updatedAt: "2026-07-01T00:00:00Z",
    }),
    thread({
      id: "archived",
      title: "Archived",
      archivedAt: "2026-07-29T00:00:00Z",
    }),
  ];
  const statuses: Record<string, TaskSearchActivityStatus> = {
    running: "processing",
    approval: "approval",
  };

  assert.deepEqual(
    searchTasks(threads, projects, statuses, "").map(
      (result) => result.thread.id,
    ),
    ["approval", "running", "recent"],
  );
});

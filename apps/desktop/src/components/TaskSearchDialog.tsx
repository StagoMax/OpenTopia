import {
  AlertCircle,
  CheckCircle2,
  Circle,
  Loader2,
  Search,
  ShieldAlert,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { searchTasks } from "../taskSearch";
import {
  threadActivityStatusLabel,
  type ThreadActivityStatus,
} from "../threadActivityStatus";
import type { Project, Thread } from "../types";
import { IconButton } from "./ui";
import "./TaskSearchDialog.css";

const resultLimit = 50;

type TaskSearchDialogProps = {
  activeThreadId: string | null;
  activityStatuses: Record<string, ThreadActivityStatus>;
  projects: Project[];
  threads: Thread[];
  onClose(): void;
  onSelectThread(threadId: string): void;
};

function StatusIcon({ status }: { status?: ThreadActivityStatus }) {
  if (status === "processing") {
    return <Loader2 className="spin" size={14} aria-hidden="true" />;
  }
  if (status === "succeeded") {
    return <CheckCircle2 size={14} aria-hidden="true" />;
  }
  if (status === "failed") {
    return <AlertCircle size={14} aria-hidden="true" />;
  }
  if (status === "approval") {
    return <ShieldAlert size={14} aria-hidden="true" />;
  }
  if (status === "user_action") {
    return <AlertCircle size={14} aria-hidden="true" />;
  }
  return <Circle size={8} aria-hidden="true" />;
}

export function TaskSearchDialog({
  activeThreadId,
  activityStatuses,
  projects,
  threads,
  onClose,
  onSelectThread,
}: TaskSearchDialogProps) {
  const [query, setQuery] = useState("");
  const dialogRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const results = useMemo(
    () => searchTasks(threads, projects, activityStatuses, query),
    [activityStatuses, projects, query, threads],
  );
  const visibleResults = results.slice(0, resultLimit);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(() =>
    visibleResults.some((result) => result.thread.id === activeThreadId)
      ? activeThreadId
      : (visibleResults[0]?.thread.id ?? null),
  );

  useEffect(() => {
    if (
      selectedThreadId &&
      visibleResults.some((result) => result.thread.id === selectedThreadId)
    ) {
      return;
    }
    setSelectedThreadId(visibleResults[0]?.thread.id ?? null);
  }, [selectedThreadId, visibleResults]);

  useEffect(() => {
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    inputRef.current?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") return;

      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'input, button:not([disabled]):not([tabindex="-1"]), [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable?.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previousFocusRef.current?.focus();
    };
  }, [onClose]);

  useEffect(() => {
    if (!selectedThreadId) return;
    document
      .getElementById(`task-search-option-${selectedThreadId}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedThreadId]);

  function selectThread(threadId: string) {
    onSelectThread(threadId);
    onClose();
  }

  function moveSelection(direction: 1 | -1) {
    if (visibleResults.length === 0) return;
    const currentIndex = visibleResults.findIndex(
      (result) => result.thread.id === selectedThreadId,
    );
    const nextIndex =
      currentIndex < 0
        ? 0
        : (currentIndex + direction + visibleResults.length) %
          visibleResults.length;
    setSelectedThreadId(visibleResults[nextIndex].thread.id);
  }

  const activeResults = visibleResults.filter(
    (result) => result.status && result.status !== "succeeded",
  );
  const recentResults = visibleResults.filter(
    (result) => !result.status || result.status === "succeeded",
  );

  function renderResult(
    result: (typeof visibleResults)[number],
    resultIndex: number,
  ) {
    const selected = result.thread.id === selectedThreadId;
    const current = result.thread.id === activeThreadId;
    const label = result.status
      ? threadActivityStatusLabel(result.status)
      : null;

    return (
      <button
        className="task-search-result"
        id={`task-search-option-${result.thread.id}`}
        key={result.thread.id}
        role="option"
        type="button"
        aria-label={`${result.thread.title}，${result.projectName}${label ? `，${label}` : ""}`}
        aria-selected={selected}
        aria-current={current || undefined}
        data-active={current || undefined}
        onClick={() => selectThread(result.thread.id)}
        onMouseEnter={() => setSelectedThreadId(result.thread.id)}
        tabIndex={-1}
      >
        <span
          className={`task-search-status${result.status ? ` is-${result.status}` : ""}`}
        >
          <StatusIcon status={result.status} />
        </span>
    <span className="task-search-result-title">{result.thread.title}</span>
    <span className="task-search-result-context">
      {current ? <small>当前</small> : null}
      <span>{result.projectName}</span>
    </span>
        <span className="ot-sr-only">结果 {resultIndex + 1}</span>
      </button>
    );
  }

  let resultIndex = 0;

  return (
    <div
      className="task-search-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="task-search-dialog"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="task-search-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <h2 className="ot-sr-only" id="task-search-title">
          搜索任务
        </h2>
        <div className="task-search-field">
          <Search size={18} aria-hidden="true" />
          <input
            ref={inputRef}
            role="combobox"
            aria-autocomplete="list"
            aria-controls="task-search-results"
            aria-expanded="true"
            aria-label="搜索任务"
            aria-activedescendant={
              selectedThreadId
                ? `task-search-option-${selectedThreadId}`
                : undefined
            }
            autoComplete="off"
            placeholder="搜索任务或项目"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                moveSelection(1);
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                moveSelection(-1);
              } else if (event.key === "Home") {
                event.preventDefault();
                setSelectedThreadId(visibleResults[0]?.thread.id ?? null);
              } else if (event.key === "End") {
                event.preventDefault();
                setSelectedThreadId(
                  visibleResults[visibleResults.length - 1]?.thread.id ?? null,
                );
              } else if (event.key === "Enter" && selectedThreadId) {
                event.preventDefault();
                selectThread(selectedThreadId);
              }
            }}
          />
          <IconButton
            aria-label="关闭任务搜索"
            size="compact"
            title="关闭"
            onClick={onClose}
          >
            <X size={16} aria-hidden="true" />
          </IconButton>
        </div>

        <div
          className="task-search-results"
          id="task-search-results"
          role="listbox"
          aria-label="任务搜索结果"
        >
          {visibleResults.length === 0 ? (
            <div className="task-search-empty">
              <Search size={20} aria-hidden="true" />
              <strong>没有找到匹配任务</strong>
              <span>请尝试其他任务标题或项目名称。</span>
            </div>
          ) : (
            <>
              {activeResults.length > 0 ? (
                <section
                  className="task-search-group"
                  role="group"
                  aria-labelledby="active-task-results"
                >
                  <h3 id="active-task-results">活动任务</h3>
                  {activeResults.map((result) =>
                    renderResult(result, resultIndex++),
                  )}
                </section>
              ) : null}
              {recentResults.length > 0 ? (
                <section
                  className="task-search-group"
                  role="group"
                  aria-labelledby="recent-task-results"
                >
                  <h3 id="recent-task-results">最近任务</h3>
                  {recentResults.map((result) =>
                    renderResult(result, resultIndex++),
                  )}
                </section>
              ) : null}
            </>
          )}
        </div>
        <div className="ot-sr-only" aria-live="polite">
          {results.length} 个匹配任务
          {results.length > resultLimit ? `，显示前 ${resultLimit} 个` : ""}
        </div>
      </section>
    </div>
  );
}

import { useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  ChevronDown,
  Circle,
  Clock3,
  ListTodo,
  Loader2,
  X,
} from "lucide-react";
import type { WorkForm } from "../../types";

export function ComposerWorkForm({ form }: { form: WorkForm }) {
  const [expanded, setExpanded] = useState(false);
  const active = form.status === "active";
  const completedIds = useMemo(
    () =>
      new Set(
        form.items
          .filter((item) => item.status === "completed")
          .map((item) => item.id),
      ),
    [form.items],
  );
  const currentStepIndex = useMemo(() => {
    if (!active) return -1;
    const inProgressIndex = form.items.findIndex(
      (item) => item.status === "in_progress",
    );
    if (inProgressIndex >= 0) return inProgressIndex;
    return form.items.findIndex(
      (item) =>
        item.status === "pending" &&
        item.dependsOn.every((dependency) => completedIds.has(dependency)),
    );
  }, [active, completedIds, form.items]);
  const resolvedCount = form.items.filter((item) =>
    ["completed", "deferred", "blocked", "cancelled"].includes(item.status),
  ).length;
  const currentStep =
    currentStepIndex >= 0 ? form.items[currentStepIndex] : undefined;
  const progressLabel =
    form.status === "paused"
      ? "已暂停"
      : form.status === "blocked"
        ? "受阻"
        : form.status === "cancelled"
          ? "已取消"
          : currentStep
            ? `第 ${currentStepIndex + 1}/${form.items.length} 步`
            : `${resolvedCount}/${form.items.length} 已处理`;

  useEffect(() => {
    setExpanded(false);
  }, [form.id]);

  if (form.items.length === 0) return null;

  return (
    <section className={`composer-plan ${expanded ? "is-expanded" : ""}`}>
      <button
        className="composer-plan-summary"
        type="button"
        aria-expanded={expanded}
        aria-controls="composer-plan-steps"
        onClick={() => setExpanded((current) => !current)}
      >
        <ListTodo size={15} aria-hidden="true" />
        <span className="composer-plan-current">
          {currentStep?.title || "任务清单"}
        </span>
        <span className="composer-plan-count">{progressLabel}</span>
        <ChevronDown
          className="composer-plan-chevron"
          size={14}
          aria-hidden="true"
        />
      </button>
      {expanded ? (
        <div className="composer-plan-body" id="composer-plan-steps">
          <ol className="composer-plan-list">
            {form.items.map((item, index) => (
              <li
                className={`is-${item.status} ${index === currentStepIndex ? "is-current" : ""}`}
                data-status={item.status}
                key={item.id}
              >
                <span className="composer-plan-step-icon" aria-hidden="true">
                  <ComposerPlanStepIcon
                    isCurrent={index === currentStepIndex}
                    status={item.status}
                  />
                </span>
                <span className="composer-plan-step-copy">
                  <span>{item.title || item.id}</span>
                  {item.note ? <small>{item.note}</small> : null}
                </span>
                {index === currentStepIndex ? (
                  <span className="composer-plan-step-marker">当前</span>
                ) : null}
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </section>
  );
}

function ComposerPlanStepIcon({
  isCurrent,
  status,
}: {
  isCurrent: boolean;
  status: WorkForm["items"][number]["status"];
}) {
  if (isCurrent) {
    return <Loader2 className="spin" size={14} />;
  }
  if (status === "completed") {
    return <span className="composer-plan-complete" />;
  }
  if (status === "in_progress") {
    return <span className="composer-plan-flow" />;
  }
  if (status === "blocked") return <AlertCircle size={13} />;
  if (status === "cancelled") return <X size={13} />;
  if (status === "deferred") return <Clock3 size={13} />;
  return <Circle size={11} />;
}

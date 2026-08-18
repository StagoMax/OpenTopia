import type {
  HumanTask,
  HumanTaskAction,
  HumanTaskStatus,
  HumanTaskType,
} from "./types";

export type HumanTaskActionPresentation = {
  label: string;
  pendingLabel: string;
  variant: "primary" | "secondary" | "danger";
};

const taskTypeLabels: Record<HumanTaskType, string> = {
  approval: "等待审批",
  input_request: "需要输入",
  output_review: "结果审阅",
  recovery: "故障恢复",
  reconnect: "重新连接",
  data_correction: "数据修正",
  manual: "人工处理",
};

const taskStatusLabels: Record<HumanTaskStatus, string> = {
  pending: "待处理",
  completed: "已完成",
  cancelled: "已取消",
};

const actionPresentations: Record<
  HumanTaskAction,
  HumanTaskActionPresentation
> = {
  approve: {
    label: "通过并继续",
    pendingLabel: "正在通过…",
    variant: "primary",
  },
  reject: {
    label: "拒绝",
    pendingLabel: "正在拒绝…",
    variant: "danger",
  },
  retry: {
    label: "检查后重试",
    pendingLabel: "正在重试…",
    variant: "secondary",
  },
  cancel: {
    label: "取消运行",
    pendingLabel: "正在取消…",
    variant: "danger",
  },
};

const defaultActionOrder: readonly HumanTaskAction[] = [
  "approve",
  "retry",
  "reject",
  "cancel",
];

const recoveryActionOrder: readonly HumanTaskAction[] = [
  "retry",
  "approve",
  "cancel",
  "reject",
];

export function humanTaskTypeLabel(type: HumanTaskType): string {
  return taskTypeLabels[type];
}

export function humanTaskStatusLabel(status: HumanTaskStatus): string {
  return taskStatusLabels[status];
}

export function humanTaskActionPresentation(
  action: HumanTaskAction,
): HumanTaskActionPresentation {
  return actionPresentations[action];
}

export function orderedHumanTaskActions(
  task: Pick<HumanTask, "allowedActions" | "taskType">,
): HumanTaskAction[] {
  const allowed = new Set(task.allowedActions);
  const order =
    task.taskType === "recovery" ||
    task.taskType === "reconnect" ||
    task.taskType === "data_correction"
      ? recoveryActionOrder
      : defaultActionOrder;
  return order.filter((action) => allowed.has(action));
}

export function sortPendingHumanTasks(
  tasks: readonly HumanTask[],
): HumanTask[] {
  return [...tasks]
    .filter((task) => task.status === "pending")
    .sort((left, right) => {
      const createdOrder =
        new Date(left.createdAt).getTime() -
        new Date(right.createdAt).getTime();
      return createdOrder || left.id.localeCompare(right.id);
    });
}

export function reconcileHumanTaskSelection(
  tasks: readonly Pick<HumanTask, "id">[],
  currentId: string | null,
  preferredId: string | null = null,
): string | null {
  if (currentId && tasks.some((task) => task.id === currentId)) {
    return currentId;
  }
  if (preferredId && tasks.some((task) => task.id === preferredId)) {
    return preferredId;
  }
  return tasks[0]?.id ?? null;
}

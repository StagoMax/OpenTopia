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
  reconciliation: "副作用对账",
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
  resume: {
    label: "提交结果并继续",
    pendingLabel: "正在恢复…",
    variant: "primary",
  },
  submit: {
    label: "提交输入并继续",
    pendingLabel: "正在提交…",
    variant: "primary",
  },
  reconnect: {
    label: "已重新连接，继续",
    pendingLabel: "正在恢复…",
    variant: "primary",
  },
  acknowledge: {
    label: "确认核对结果并继续",
    pendingLabel: "正在继续…",
    variant: "primary",
  },
  cancel: {
    label: "取消运行",
    pendingLabel: "正在取消…",
    variant: "danger",
  },
};

const defaultActionOrder: readonly HumanTaskAction[] = [
  "approve",
  "submit",
  "resume",
  "reconnect",
  "acknowledge",
  "retry",
  "reject",
  "cancel",
];

const actionOrderByTaskType: Record<HumanTaskType, readonly HumanTaskAction[]> =
  {
    approval: ["approve", "reject", "cancel"],
    input_request: ["submit", "cancel"],
    output_review: ["approve", "reject"],
    recovery: ["retry", "approve", "cancel", "reject"],
    reconnect: ["resume", "reconnect", "cancel"],
    data_correction: ["submit", "resume", "retry", "cancel"],
    reconciliation: ["acknowledge", "resume", "cancel"],
    manual: defaultActionOrder,
  };

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
  const order = actionOrderByTaskType[task.taskType];
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

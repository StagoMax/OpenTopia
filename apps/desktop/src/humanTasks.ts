import type {
  HumanTask,
  HumanTaskAction,
  HumanTaskStatus,
  HumanTaskType,
  UserInputRequest,
} from "./types";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "./applicationLanguage.ts";

export type HumanTaskActionPresentation = {
  label: string;
  pendingLabel: string;
  variant: "primary" | "secondary" | "danger";
};

const taskTypeMessageKeys: Record<HumanTaskType, InterfaceMessageKey> = {
  approval: "flow.humanTask.type.approval",
  input_request: "flow.humanTask.type.input_request",
  output_review: "flow.humanTask.type.output_review",
  recovery: "flow.humanTask.type.recovery",
  reconnect: "flow.humanTask.type.reconnect",
  data_correction: "flow.humanTask.type.data_correction",
  reconciliation: "flow.humanTask.type.reconciliation",
  manual: "flow.humanTask.type.manual",
};

const taskStatusMessageKeys: Record<HumanTaskStatus, InterfaceMessageKey> = {
  pending: "flow.humanTask.status.pending",
  completed: "flow.humanTask.status.completed",
  cancelled: "flow.humanTask.status.cancelled",
};

const actionPresentations: Record<
  HumanTaskAction,
  {
    label: InterfaceMessageKey;
    pendingLabel: InterfaceMessageKey;
    variant: HumanTaskActionPresentation["variant"];
  }
> = {
  approve: {
    label: "flow.humanTask.action.approve",
    pendingLabel: "flow.humanTask.action.approvePending",
    variant: "primary",
  },
  reject: {
    label: "flow.humanTask.action.reject",
    pendingLabel: "flow.humanTask.action.rejectPending",
    variant: "danger",
  },
  retry: {
    label: "flow.humanTask.action.retry",
    pendingLabel: "flow.humanTask.action.retryPending",
    variant: "secondary",
  },
  resume: {
    label: "flow.humanTask.action.resume",
    pendingLabel: "flow.humanTask.action.resumePending",
    variant: "primary",
  },
  submit: {
    label: "flow.humanTask.action.submit",
    pendingLabel: "flow.humanTask.action.submitPending",
    variant: "primary",
  },
  reconnect: {
    label: "flow.humanTask.action.reconnect",
    pendingLabel: "flow.humanTask.action.reconnectPending",
    variant: "primary",
  },
  acknowledge: {
    label: "flow.humanTask.action.acknowledge",
    pendingLabel: "flow.humanTask.action.acknowledgePending",
    variant: "primary",
  },
  cancel: {
    label: "flow.humanTask.action.cancel",
    pendingLabel: "flow.humanTask.action.cancelPending",
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

export function humanTaskTypeLabel(
  type: HumanTaskType,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return interfaceMessage(language, taskTypeMessageKeys[type]);
}

export function humanTaskStatusLabel(
  status: HumanTaskStatus,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return interfaceMessage(language, taskStatusMessageKeys[status]);
}

export function humanTaskActionPresentation(
  action: HumanTaskAction,
  language: ApplicationLanguage = defaultApplicationLanguage,
): HumanTaskActionPresentation {
  const presentation = actionPresentations[action];
  return {
    label: interfaceMessage(language, presentation.label),
    pendingLabel: interfaceMessage(language, presentation.pendingLabel),
    variant: presentation.variant,
  };
}

export function orderedHumanTaskActions(
  task: Pick<HumanTask, "allowedActions" | "taskType">,
): HumanTaskAction[] {
  const allowed = new Set(task.allowedActions);
  const order = actionOrderByTaskType[task.taskType];
  return order.filter((action) => allowed.has(action));
}

export function humanTaskInputRequest(
  task: Pick<HumanTask, "payload" | "taskType">,
): UserInputRequest | null {
  if (task.taskType !== "input_request") return null;
  const request = asRecord(task.payload)?.request;
  const record = asRecord(request);
  return record &&
    typeof record.requestId === "string" &&
    Array.isArray(record.questions)
    ? (request as UserInputRequest)
    : null;
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

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

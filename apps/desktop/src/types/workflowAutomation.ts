import type { ExecutionConnectionOperation, FlowRun } from "./flow";

export type WorkflowTrigger =
  | { kind: "manual" }
  | { kind: "webhook"; triggerId: string; tokenRef: string }
  | {
      kind: "schedule";
      triggerId: string;
      intervalSeconds: number;
      nextFireAt: string;
    }
  | {
      kind: "event_subscription";
      triggerId: string;
      source: string;
      eventType: string;
    };

export type WorkflowOutput =
  | { kind: "inbox" }
  | { kind: "webhook"; endpoint: string; credentialRef?: string | null }
  | { kind: "connection_operation"; operation: ExecutionConnectionOperation }
  | {
      kind: "human_task";
      title: string;
      description: string;
      assignedTo?: string | null;
    };

export type WorkflowRelease = {
  schemaVersion: number;
  id: string;
  revision: number;
  releaseKey: string;
  environment: string;
  threadId: string;
  status: "active" | "disabled";
  trigger: WorkflowTrigger;
  primaryDeploymentId: string;
  canaryDeploymentId?: string | null;
  canaryPercent: number;
  previousPrimaryDeploymentId?: string | null;
  createdAt: string;
  updatedAt: string;
  createdBy: string;
};

export type WorkflowTriggerInvocation = {
  schemaVersion: number;
  id: string;
  releaseId: string;
  triggerId: string;
  idempotencyKey: string;
  deploymentId: string;
  flowRunId?: string | null;
  status: "accepted" | "started" | "failed";
  inputHash: string;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type WorkflowDeliveryStatus =
  "pending" | "delivered" | "failed" | "waiting_human" | "cancelled";

export type WorkflowDeliveryReceipt = {
  schemaVersion: number;
  id: string;
  revision: number;
  runId: string;
  deploymentId: string;
  outputKind: "inbox" | "webhook" | "connection_operation" | "human_task";
  status: WorkflowDeliveryStatus;
  attempt: number;
  idempotencyKey: string;
  responseStatus?: number | null;
  providerResult?: unknown;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
  deliveredAt?: string | null;
};

export type WorkflowEvaluation = {
  schemaVersion: number;
  id: string;
  runId: string;
  deploymentId: string;
  evaluator: string;
  score: number;
  passed: boolean;
  labels: string[];
  note?: string | null;
  createdAt: string;
};

export type WorkflowInvocationResult = {
  invocation: WorkflowTriggerInvocation;
  run: FlowRun;
  reused: boolean;
};

export type WorkflowEvaluationSummary = {
  deploymentId: string;
  totalRuns: number;
  runStatusCounts: Record<string, number>;
  evaluationCount: number;
  passRate?: number | null;
  averageScore?: number | null;
  deliveryStatusCounts: Record<string, number>;
  failureClusters: Array<{ key: string; count: number; sample: string }>;
};

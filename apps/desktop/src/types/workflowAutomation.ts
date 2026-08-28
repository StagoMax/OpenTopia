import type { ExecutionConnectionOperation, FlowRun } from "./flow";

export type WorkflowIngressPolicy = "immediate" | "require_review";
export type WorkflowOutputReviewPolicy =
  "explicit_nodes_only" | "always_review_output";

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

export type FlowCase = {
  schemaVersion: number;
  id: string;
  flowId: string;
  triggerId: string;
  idempotencyKey: string;
  flowRevisionId: string;
  flowRevision: import("./flow").FlowRevision;
  flowRunId?: string | null;
  status: "accepted" | "started" | "failed" | "superseded";
  inputHash: string;
  input: unknown;
  error?: string | null;
  supersededByCaseId?: string | null;
  statusNote?: string | null;
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
  flowRevisionId: string;
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
  flowRevisionId: string;
  evaluator: string;
  score: number;
  passed: boolean;
  labels: string[];
  note?: string | null;
  createdAt: string;
};

export type FlowCaseResult = {
  case: FlowCase;
  run?: FlowRun | null;
  reused: boolean;
};

export type WorkflowEvaluationSummary = {
  flowRevisionId: string;
  totalRuns: number;
  runStatusCounts: Record<string, number>;
  evaluationCount: number;
  passRate?: number | null;
  averageScore?: number | null;
  deliveryStatusCounts: Record<string, number>;
  failureClusters: Array<{ key: string; count: number; sample: string }>;
};

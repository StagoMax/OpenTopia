import assert from "node:assert/strict";
import test from "node:test";
import type { FlowCase } from "../../types";
import {
  enterpriseSidebarTitle,
  flowCaseCoreLabel,
  workflowTriggerLabel,
} from "./enterpriseSidebarPresentation.ts";

test("sidebar titles replace identifier-only labels and expose a qualifier", () => {
  assert.equal(
    enterpriseSidebarTitle({
      id: "audit-work-injury",
      label: "audit-work-injury",
      qualifier: "risk-team · v7",
    }),
    "Audit Work Injury · risk-team · v7",
  );
});

test("pending cases prefer business input over persistence identifiers", () => {
  const flowCase = {
    input: { case_id: "case_30_combo", purpose: "medical review" },
    idempotencyKey: "demo-event:case_30_combo:node-trigger-v2",
  } as FlowCase;

  assert.equal(flowCaseCoreLabel(flowCase), "Case 30 Combo · medical review");
});

test("event triggers expose source and event type", () => {
  assert.equal(
    workflowTriggerLabel({
      kind: "event_subscription",
      triggerId: "trigger",
      source: "audit-work-injury",
      eventType: "case.submitted",
    }),
    "Audit Work Injury · Case Submitted",
  );
});

import assert from "node:assert/strict";
import test from "node:test";
import { decideRefund } from "../src/policy.js";

test("approves an eligible physical refund", () => {
  const order = { id: "o", kind: "physical", paidCents: 1000, refundedCents: 0, deliveredAt: "2026-07-20T00:00:00Z", fraudHold: false };
  assert.deepEqual(decideRefund(order, { id: "r", orderId: "o", amountCents: 500, reason: "changed_mind" }, "2026-08-01T00:00:00Z"), { requestId: "r", status: "approved", approvedCents: 500, reason: "eligible" });
});

test("routes fraud holds to manual review", () => {
  const order = { id: "o", kind: "physical", paidCents: 1000, refundedCents: 0, deliveredAt: "2026-07-20T00:00:00Z", fraudHold: true };
  assert.equal(decideRefund(order, { id: "r", orderId: "o", amountCents: 500, reason: "damaged", evidenceId: "x" }, "2026-08-01T00:00:00Z").status, "manual_review");
});

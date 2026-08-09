import assert from "node:assert/strict";
import test from "node:test";

import { normalizeInventory, planReservations, summarizeReservations } from "../src/inventory.js";

test("normalizes and sorts inventory", () => {
  assert.deepEqual(normalizeInventory([
    { sku: "B", onHand: 3, reserved: 1 },
    { sku: "A", onHand: 5 },
  ]), [
    { sku: "A", onHand: 5, reserved: 0, available: 5 },
    { sku: "B", onHand: 3, reserved: 1, available: 2 },
  ]);
});

test("allocates by priority and id", () => {
  const plan = planReservations([{ sku: "A", onHand: 5, reserved: 1 }], [
    { id: "later", sku: "A", quantity: 3, priority: 1 },
    { id: "first", sku: "A", quantity: 3, priority: 5 },
  ]);
  assert.deepEqual(plan.allocations, [
    { id: "first", sku: "A", requested: 3, allocated: 3, status: "filled" },
    { id: "later", sku: "A", requested: 3, allocated: 1, status: "partial" },
  ]);
  assert.deepEqual(plan.inventory, [{ sku: "A", onHand: 5, reserved: 5, available: 0 }]);
});

test("summarizes a reservation plan", () => {
  const plan = planReservations([{ sku: "A", onHand: 2 }], [
    { id: "a", sku: "A", quantity: 1, priority: 0 },
    { id: "b", sku: "A", quantity: 3, priority: 0 },
  ]);
  assert.deepEqual(summarizeReservations(plan), {
    orders: 2,
    filled: 1,
    partial: 1,
    backordered: 0,
    requestedUnits: 4,
    allocatedUnits: 2,
    remainingUnits: 0,
  });
});

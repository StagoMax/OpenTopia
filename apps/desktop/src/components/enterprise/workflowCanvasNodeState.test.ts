import assert from "node:assert/strict";
import test from "node:test";
import {
  applyWorkflowNodePositions,
  committedWorkflowNodePositionChanges,
  reconcileWorkflowCanvasNodes,
} from "./workflowCanvasNodeState.ts";

test("applies only changed node positions without rewriting the rest", () => {
  const positions = {
    first: { x: 10, y: 20 },
    second: { x: 30, y: 40 },
  };
  const next = applyWorkflowNodePositions(positions, [
    {
      id: "first",
      position: { x: 50, y: 60 },
      type: "position",
    },
  ]);

  assert.deepEqual(next, {
    first: { x: 50, y: 60 },
    second: positions.second,
  });
  assert.equal(next.second, positions.second);
});

test("preserves React Flow gesture state while semantic node data changes", () => {
  const current = [
    {
      id: "agent",
      data: { label: "Before" },
      dragging: true,
      measured: { height: 144, width: 260 },
      position: { x: 80, y: 90 },
      type: "workflowNode",
    },
  ];
  const projected = [
    {
      id: "agent",
      data: { label: "After" },
      position: { x: 10, y: 20 },
      selected: true,
      type: "workflowNode",
    },
  ];

  assert.deepEqual(reconcileWorkflowCanvasNodes(current, projected, true), [
    {
      data: { label: "After" },
      dragging: true,
      id: "agent",
      measured: { height: 144, width: 260 },
      position: { x: 80, y: 90 },
      selected: true,
      type: "workflowNode",
    },
  ]);
});

test("applies committed positions after a drag and reconciles node membership", () => {
  const current = [
    {
      data: {},
      id: "removed",
      position: { x: 0, y: 0 },
    },
    {
      data: {},
      dragging: false,
      id: "kept",
      measured: { height: 100, width: 200 },
      position: { x: 10, y: 20 },
    },
  ];
  const projected = [
    { data: {}, id: "kept", position: { x: 50, y: 60 } },
    { data: {}, id: "added", position: { x: 70, y: 80 } },
  ];

  assert.deepEqual(reconcileWorkflowCanvasNodes(current, projected, true), [
    {
      data: {},
      dragging: false,
      id: "kept",
      measured: { height: 100, width: 200 },
      position: { x: 50, y: 60 },
    },
    { data: {}, id: "added", position: { x: 70, y: 80 } },
  ]);
});

test("does not sync projected positions for presentation-only updates", () => {
  const current = [{ data: {}, id: "agent", position: { x: 30, y: 40 } }];
  const projected = [
    { data: {}, id: "agent", position: { x: 10, y: 20 }, selected: true },
  ];

  assert.deepEqual(reconcileWorkflowCanvasNodes(current, projected, false), [
    {
      data: {},
      id: "agent",
      position: { x: 30, y: 40 },
      selected: true,
    },
  ]);
});

test("returns existing positions when changes have no position update", () => {
  const positions = { agent: { x: 10, y: 20 } };

  assert.equal(
    applyWorkflowNodePositions(positions, [
      { id: "agent", selected: true, type: "select" },
    ]),
    positions,
  );
});

test("keeps pointer-drag frames out of editor state commits", () => {
  assert.deepEqual(
    committedWorkflowNodePositionChanges([
      {
        dragging: true,
        id: "agent",
        position: { x: 20, y: 30 },
        type: "position",
      },
      {
        dimensions: { height: 100, width: 200 },
        id: "agent",
        type: "dimensions",
      },
    ]),
    [],
  );

  assert.deepEqual(
    committedWorkflowNodePositionChanges([
      {
        dragging: false,
        id: "agent",
        position: { x: 40, y: 50 },
        type: "position",
      },
    ]),
    [
      {
        dragging: false,
        id: "agent",
        position: { x: 40, y: 50 },
        type: "position",
      },
    ],
  );
});

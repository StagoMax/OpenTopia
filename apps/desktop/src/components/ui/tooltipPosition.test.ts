import assert from "node:assert/strict";
import test from "node:test";

import {
  calculateTooltipPosition,
  type TooltipRect,
} from "./tooltipPosition.ts";

function rect(
  left: number,
  top: number,
  width: number,
  height: number,
): TooltipRect {
  return {
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
  };
}

test("places a tooltip to the preferred side when there is room", () => {
  const position = calculateTooltipPosition(
    rect(300, 100, 200, 32),
    rect(0, 0, 240, 60),
    { width: 800, height: 600 },
    "left",
    4,
    8,
  );

  assert.deepEqual(position, { left: 56, top: 86, placement: "left" });
});

test("flips a tooltip when the preferred side would leave the viewport", () => {
  const position = calculateTooltipPosition(
    rect(20, 100, 200, 32),
    rect(0, 0, 240, 60),
    { width: 800, height: 600 },
    "left",
    4,
    8,
  );

  assert.deepEqual(position, { left: 224, top: 86, placement: "right" });
});

test("clamps oversized tooltips inside the viewport margin", () => {
  const position = calculateTooltipPosition(
    rect(20, 10, 20, 20),
    rect(0, 0, 780, 580),
    { width: 800, height: 600 },
    "top",
    4,
    8,
  );

  assert.deepEqual(position, { left: 8, top: 8, placement: "top" });
});

test("keeps a pointer-anchored tooltip next to the hovered content", () => {
  const position = calculateTooltipPosition(
    rect(750, 506, 0, 0),
    rect(0, 0, 260, 240),
    { width: 1280, height: 720 },
    "top",
    4,
    8,
  );

  assert.deepEqual(position, { left: 620, top: 262, placement: "top" });
});

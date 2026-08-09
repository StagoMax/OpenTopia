import assert from "node:assert/strict";
import test from "node:test";

import type * as SpreadsheetViewportModule from "./spreadsheetViewport";

const { expandSpreadsheetWindow, spreadsheetWindowContains } = (await import(
  "./spreadsheetViewport" + ".ts"
)) as typeof SpreadsheetViewportModule;

test("expands a visible window for prefetch without exceeding the sheet", () => {
  const visible = {
    rowStart: 5,
    rowCount: 20,
    columnStart: 2,
    columnCount: 8,
  };
  const request = expandSpreadsheetWindow(
    visible,
    { rowCount: 30, columnCount: 12 },
    { rows: 20, columns: 4 },
  );

  assert.deepEqual(request, {
    rowStart: 0,
    rowCount: 30,
    columnStart: 0,
    columnCount: 12,
  });
  assert.equal(spreadsheetWindowContains(request, visible), true);
});

test("prefetches across the T-column boundary", () => {
  const visible = {
    rowStart: 0,
    rowCount: 24,
    columnStart: 18,
    columnCount: 6,
  };
  const request = expandSpreadsheetWindow(
    visible,
    { rowCount: 500, columnCount: 40 },
    { rows: 20, columns: 4 },
  );

  assert.equal(request.columnStart, 14);
  assert.equal(request.columnStart + request.columnCount > 20, true);
});

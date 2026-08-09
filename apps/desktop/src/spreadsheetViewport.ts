export type SpreadsheetWindow = {
  rowStart: number;
  rowCount: number;
  columnStart: number;
  columnCount: number;
};

export type SpreadsheetSize = {
  rowCount: number;
  columnCount: number;
};

export function expandSpreadsheetWindow(
  window: SpreadsheetWindow,
  sheet: SpreadsheetSize,
  overscan: { rows: number; columns: number },
): SpreadsheetWindow {
  const rowStart = Math.max(0, window.rowStart - overscan.rows);
  const columnStart = Math.max(0, window.columnStart - overscan.columns);
  const rowEnd = Math.min(
    sheet.rowCount,
    window.rowStart + window.rowCount + overscan.rows,
  );
  const columnEnd = Math.min(
    sheet.columnCount,
    window.columnStart + window.columnCount + overscan.columns,
  );

  return {
    rowStart,
    rowCount: Math.max(0, rowEnd - rowStart),
    columnStart,
    columnCount: Math.max(0, columnEnd - columnStart),
  };
}

export function spreadsheetWindowContains(
  outer: SpreadsheetWindow,
  inner: SpreadsheetWindow,
): boolean {
  return (
    outer.rowStart <= inner.rowStart &&
    outer.columnStart <= inner.columnStart &&
    outer.rowStart + outer.rowCount >= inner.rowStart + inner.rowCount &&
    outer.columnStart + outer.columnCount >=
      inner.columnStart + inner.columnCount
  );
}

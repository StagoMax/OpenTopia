import assert from "node:assert/strict";
import test from "node:test";

import type * as SpreadsheetFormatsModule from "./spreadsheetFormats";

const {
  isSpreadsheetFileExtension,
  isSpreadsheetFilePath,
}: typeof SpreadsheetFormatsModule = await import(
  "./spreadsheetFormats" + ".ts"
);

test("recognizes every supported spreadsheet attachment extension", () => {
  for (const extension of [
    "xls",
    "xlsx",
    "xlsm",
    "xlsb",
    "xltx",
    "xltm",
    "ods",
    "csv",
    "tsv",
    "tab",
  ]) {
    assert.equal(isSpreadsheetFileExtension(extension), true);
  }
  assert.equal(isSpreadsheetFileExtension("pdf"), false);
});

test("matches paths case-insensitively without confusing suffix text", () => {
  assert.equal(isSpreadsheetFilePath("C:/Reports/Legacy.XLS"), true);
  assert.equal(isSpreadsheetFilePath("table.csv?download=1"), true);
  assert.equal(isSpreadsheetFilePath("notes.xls.txt"), false);
});

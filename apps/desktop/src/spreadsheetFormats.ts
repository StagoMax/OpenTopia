import spreadsheetFormats from "../electron/spreadsheet-formats.json" with { type: "json" };

export const spreadsheetFileExtensions = Object.freeze(
  spreadsheetFormats.extensions,
);

const spreadsheetFileExtensionSet = new Set<string>(spreadsheetFileExtensions);

export function isSpreadsheetFileExtension(extension: string): boolean {
  return spreadsheetFileExtensionSet.has(
    extension.trim().replace(/^\./, "").toLowerCase(),
  );
}

export function isSpreadsheetFilePath(path: string): boolean {
  const normalized = path.split(/[?#]/, 1)[0] ?? "";
  const extension = normalized.split(".").at(-1) ?? "";
  return normalized.includes(".") && isSpreadsheetFileExtension(extension);
}

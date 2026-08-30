#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

import {
  analyzeEvaluation,
  renderAuditCsv,
  renderMarkdown,
  serializableAnalysis,
} from "./tool-failure-attribution.mjs";

const options = parseArguments(process.argv.slice(2));
const provenance = {
  beforeVersion: options["before-version"],
  afterVersion: options["after-version"],
  terminalCsv: path.basename(options["terminal-csv"]),
  sweCsv: path.basename(options["swe-csv"]),
  currentValidation: options["current-validation"] ?? null,
};
const analysis = analyzeEvaluation({
  terminalCsv: options["terminal-csv"],
  sweCsv: options["swe-csv"],
});

writeOutput(options["json-out"], `${JSON.stringify(serializableAnalysis(analysis, provenance), null, 2)}\n`);
writeOutput(options["audit-csv-out"], renderAuditCsv(analysis));
writeOutput(options["markdown-out"], renderMarkdown(analysis, provenance));

process.stdout.write(`${JSON.stringify(analysis.summary, null, 2)}\n`);

function parseArguments(args) {
  const required = [
    "terminal-csv",
    "swe-csv",
    "before-version",
    "after-version",
    "json-out",
    "audit-csv-out",
    "markdown-out",
  ];
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    if (!name?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error(`Expected --name value, received '${name ?? ""}'`);
    }
    values[name.slice(2)] = value;
  }
  for (const name of required) {
    if (!values[name]) throw new Error(`Missing required argument --${name}`);
  }
  return values;
}

function writeOutput(file, content) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content, "utf8");
}

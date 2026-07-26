import { readFile } from "node:fs/promises";
import path from "node:path";

const workspace = process.env.EVAL_WORKSPACE;
if (!workspace) {
  process.stderr.write("EVAL_WORKSPACE is required\n");
  process.exit(2);
}

try {
  const result = JSON.parse(await readFile(path.join(workspace, "result.json"), "utf8"));
  const passed = result.source === "fixture-data" && result.promptReceived === true && result.status === "complete";
  process.stdout.write(`${JSON.stringify({ passed })}\n`);
  process.exitCode = passed ? 0 : 1;
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 2;
}

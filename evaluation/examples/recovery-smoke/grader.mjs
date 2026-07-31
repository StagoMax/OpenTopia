import { readFile } from "node:fs/promises";

const [workspace, expected] = process.argv.slice(2);
if (!workspace || !expected) throw new Error("usage: grader.mjs <workspace> <expected-progress>");

const progress = await readFile(`${workspace}/progress.txt`, "utf8");
if (progress !== `${expected}\n`) {
  throw new Error(`expected progress ${JSON.stringify(expected)}, received ${JSON.stringify(progress)}`);
}

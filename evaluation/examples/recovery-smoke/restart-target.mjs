import { writeFile } from "node:fs/promises";

const targetStatePath = process.argv[2];
if (!targetStatePath) throw new Error("restart target state path is required");

await writeFile(targetStatePath, `${JSON.stringify({ restarted: true })}\n`, "utf8");

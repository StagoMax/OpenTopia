import { appendFile, readFile, writeFile } from "node:fs/promises";

const workspace = process.env.AGENT_EVAL_WORKSPACE;
const eventsPath = process.env.AGENT_EVAL_EVENTS_PATH;
const targetStatePath = process.env.AGENT_EVAL_TARGET_STATE_PATH;
const phaseIndex = Number(process.env.AGENT_EVAL_PHASE_INDEX);

if (!workspace || !eventsPath || !targetStatePath || !Number.isInteger(phaseIndex)) {
  throw new Error("Recovery smoke target did not receive the staged harness environment");
}

async function emit(type, payload = {}) {
  await appendFile(eventsPath, `${JSON.stringify({
    schemaVersion: 1,
    timestamp: new Date().toISOString(),
    source: "recovery-smoke-target",
    type,
    payload
  })}\n`, "utf8");
}

if (phaseIndex === 1) {
  await writeFile(`${workspace}/progress.txt`, "phase-one\n", "utf8");
  await emit("tool.call.completed", { name: "filesystem.write", success: true });
} else if (phaseIndex === 2) {
  const state = JSON.parse(await readFile(targetStatePath, "utf8"));
  if (state.restarted !== true) throw new Error("restart command did not persist target recovery state");
  const progress = await readFile(`${workspace}/progress.txt`, "utf8");
  if (progress !== "phase-one\n") throw new Error("workspace state was not retained across restart");
  await writeFile(`${workspace}/progress.txt`, "complete\n", "utf8");
  await emit("tool.call.completed", { name: "filesystem.write", success: true });
  await emit("agent.completion.claimed", { verifiedBy: "recovery-smoke" });
} else {
  throw new Error(`Unexpected phase index ${phaseIndex}`);
}

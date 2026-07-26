import { readFile, writeFile } from "node:fs/promises";
import { validateEvent, ValidationError } from "./validation.mjs";

export function makeHarnessEvent(context, type, payload = {}) {
  return {
    schemaVersion: 1,
    runId: context.runId,
    trialId: context.trialId,
    taskId: context.taskId,
    timestamp: new Date().toISOString(),
    monotonicMs: Date.now() - context.startedMonotonic,
    source: "evaluation-harness",
    type,
    payload
  };
}

export async function readTargetEvents(filePath, context) {
  let source;
  try {
    source = await readFile(filePath, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return { events: [], errors: [] };
    throw error;
  }

  const events = [];
  const errors = [];
  const lines = source.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    if (!lines[index].trim()) continue;
    try {
      const raw = JSON.parse(lines[index]);
      const event = {
        schemaVersion: raw.schemaVersion ?? 1,
        runId: raw.runId ?? context.runId,
        trialId: raw.trialId ?? context.trialId,
        taskId: raw.taskId ?? context.taskId,
        timestamp: raw.timestamp ?? new Date().toISOString(),
        source: raw.source ?? "target-adapter",
        type: raw.type,
        payload: raw.payload ?? {}
      };
      for (const field of ["monotonicMs", "agentId", "threadId", "correlationId"]) {
        if (raw[field] !== undefined) event[field] = raw[field];
      }
      validateEvent(event);
      if (event.runId !== context.runId || event.trialId !== context.trialId || event.taskId !== context.taskId) {
        throw new ValidationError("Event identity", ["runId, trialId, or taskId does not match the active trial"]);
      }
      events.push(event);
    } catch (error) {
      errors.push({ line: index + 1, message: error.message });
    }
  }
  return { events, errors };
}

export async function writeEvents(filePath, events) {
  const content = events.map((event) => JSON.stringify(event)).join("\n");
  await writeFile(filePath, content ? `${content}\n` : "", "utf8");
}

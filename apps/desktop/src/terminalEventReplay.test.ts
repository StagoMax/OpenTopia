import assert from "node:assert/strict";
import test from "node:test";

import { writePendingTerminalEvents } from "./terminalEventReplay.ts";

type TestEvent = { id: string; data: string };
type TestTerminal = { output: string[] };

const events: TestEvent[] = [
  { id: "cursor-query", data: "\u001b[6n" },
  { id: "prompt", data: "PS J:\\Project\\OpenTopia> " },
];

function writeEvent(event: TestEvent, terminal: TestTerminal) {
  terminal.output.push(event.data);
}

test("does not consume terminal events before the terminal is ready", () => {
  const written = new Set<string>();

  writePendingTerminalEvents(events, null, written, writeEvent);

  assert.deepEqual([...written], []);
});

test("replays the complete stream for each terminal generation", () => {
  const firstTerminal: TestTerminal = { output: [] };
  const firstGeneration = new Set<string>();
  writePendingTerminalEvents(
    events,
    firstTerminal,
    firstGeneration,
    writeEvent,
  );
  writePendingTerminalEvents(
    events,
    firstTerminal,
    firstGeneration,
    writeEvent,
  );

  const recreatedTerminal: TestTerminal = { output: [] };
  const recreatedGeneration = new Set<string>();
  writePendingTerminalEvents(
    events,
    recreatedTerminal,
    recreatedGeneration,
    writeEvent,
  );

  assert.deepEqual(firstTerminal.output, events.map((event) => event.data));
  assert.deepEqual(
    recreatedTerminal.output,
    events.map((event) => event.data),
  );
});

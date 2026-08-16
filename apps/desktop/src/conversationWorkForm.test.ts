import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationWorkFormModule from "./conversationWorkForm";
import type { AgentEvent, WorkForm } from "./types";

const { resolveRuntimeWorkForm } = (await import(
  "./conversationWorkForm" + ".ts"
)) as typeof ConversationWorkFormModule;

const form: WorkForm = {
  id: "form-1",
  threadId: "thread-1",
  scope: { kind: "turn", id: "turn-1" },
  objective: "Edit",
  constraints: [],
  acceptance: [],
  status: "active",
  revision: 1,
  items: [
    {
      id: "edit",
      title: "Edit",
      status: "in_progress",
      completionDisposition: "blocking",
      dependsOn: [],
      acceptance: [],
      evidenceRefs: [],
    },
  ],
  createdAt: "2026-08-04T00:00:00Z",
  updatedAt: "2026-08-04T00:00:00Z",
};

function event(
  seq: number,
  payload: AgentEvent["payload"],
  turnId = "turn-1",
): AgentEvent {
  return {
    id: `event-${seq}`,
    seq,
    threadId: "thread-1",
    turnId,
    createdAt: "2026-08-04T00:00:00Z",
    payload,
  };
}

test("keeps the current WorkForm while its turn is active", () => {
  assert.equal(
    resolveRuntimeWorkForm([
      event(1, { type: "turn_started", user_message_id: "message-1" }),
      event(2, { type: "work_form_updated", form }),
    ]),
    form,
  );
});

test("clears the WorkForm after its turn is cancelled", () => {
  assert.equal(
    resolveRuntimeWorkForm([
      event(1, { type: "work_form_updated", form }),
      event(2, { type: "turn_cancelled", reason: "Cancelled by user." }),
    ]),
    null,
  );
});

test("does not let an older terminal event hide a new turn form", () => {
  assert.equal(
    resolveRuntimeWorkForm([
      event(1, { type: "turn_cancelled", reason: "Cancelled" }, "turn-1"),
      event(2, { type: "work_form_updated", form }, "turn-2"),
    ]),
    form,
  );
});

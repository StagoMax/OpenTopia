import assert from "node:assert/strict";
import test from "node:test";
import type { AgentEvent } from "../../types.ts";
import { buildActivityEntries } from "./model.ts";

function event(seq: number, payload: AgentEvent["payload"]): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId: "turn-1",
    seq,
    createdAt: new Date(seq * 1_000).toISOString(),
    payload,
  };
}

function request(seq: number, requestId: string): AgentEvent {
  return event(seq, {
    type: "provider_request_sent",
    request_id: requestId,
    round: 1,
    attempt: 1,
    adapter: "openai_chat_completions",
    method: "POST",
    endpoint: "/chat/completions",
  });
}

function modelDelta(
  seq: number,
  text: string,
  requestId = "request-1",
  attempt = 1,
): AgentEvent {
  return event(seq, {
    type: "model_delta",
    text,
    provider_attempt: { request_id: requestId, round: 1, attempt },
  });
}

function commentaryText(events: AgentEvent[]): string[] {
  return buildActivityEntries(events)
    .filter((entry) => entry.kind === "commentary")
    .map((entry) => entry.text);
}

test("shows provider text while the current attempt is still provisional", () => {
  assert.deepEqual(
    commentaryText([request(1, "request-1"), modelDelta(2, "正在生成")]),
    ["正在生成"],
  );
});

test("removes an uncommitted attempt when the provider retries", () => {
  assert.deepEqual(
    commentaryText([
      request(1, "request-1"),
      event(2, {
        type: "provider_request_retried",
        request_id: "request-1",
        round: 1,
        attempt: 2,
        retry_kind: "state_recovery",
        reason: "retry malformed tool arguments",
      }),
      modelDelta(3, "第一次尝试", "request-1", 1),
      modelDelta(4, "第二次尝试", "request-1", 2),
    ]),
    ["第二次尝试"],
  );
});

test("keeps text from an attempt once its response commits", () => {
  assert.deepEqual(
    commentaryText([
      request(1, "request-1"),
      modelDelta(2, "已验证内容"),
      event(3, {
        type: "provider_response_commit_started",
        request_id: "request-1",
        round: 1,
        attempt: 1,
        output_events: 4,
        output_bytes: 12,
        elapsed_ms: 50,
      }),
      event(4, { type: "error", message: "后续工具执行失败" }),
    ]),
    ["已验证内容"],
  );
});

test("removes an unfinished attempt when the turn fails", () => {
  assert.deepEqual(
    commentaryText([
      request(1, "request-1"),
      modelDelta(2, "不完整内容"),
      event(3, { type: "error", message: "provider stream failed" }),
    ]),
    [],
  );
});

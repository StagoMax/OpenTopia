import assert from "node:assert/strict";
import test from "node:test";

import type { ContextSourceRef, Message } from "./types";
import { conversationDisplayParts } from "./conversationMessageParts.ts";

const first: ContextSourceRef = {
  id: "source-first",
  path: "C:/Temp/first.xlsx",
  name: "first.xlsx",
  kind: "document",
  contentType:
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  bytes: 10,
  truncated: false,
};

const second: ContextSourceRef = {
  ...first,
  id: "source-second",
  path: "C:/Temp/second.xlsx",
  name: "second.xlsx",
};

function userMessage(parts: Message["parts"]): Message {
  return {
    id: "message-1",
    threadId: "thread-1",
    role: "user",
    parts,
    createdAt: "2026-08-31T00:00:00.000Z",
  };
}

test("restores uniquely identifiable legacy attachments at their text positions", () => {
  const message = userMessage([
    { type: "text", text: "把[first.xlsx]填入[second.xlsx]里" },
    { type: "source_ref", source: first },
    { type: "source_ref", source: second },
  ]);

  assert.deepEqual(conversationDisplayParts(message), [
    { type: "text", text: "把" },
    { type: "source_ref", source: first, inline: true },
    { type: "text", text: "填入" },
    { type: "source_ref", source: second, inline: true },
    { type: "text", text: "里" },
  ]);
});

test("does not guess legacy positions when names or markers are ambiguous", () => {
  const duplicateName = { ...second, name: first.name };
  const duplicateSources = userMessage([
    { type: "text", text: "看[first.xlsx]" },
    { type: "source_ref", source: first },
    { type: "source_ref", source: duplicateName },
  ]);
  const duplicateMarkers = userMessage([
    { type: "text", text: "[first.xlsx]和[first.xlsx]" },
    { type: "source_ref", source: first },
  ]);

  assert.equal(
    conversationDisplayParts(duplicateSources),
    duplicateSources.parts,
  );
  assert.equal(
    conversationDisplayParts(duplicateMarkers),
    duplicateMarkers.parts,
  );
});

test("keeps explicitly trailing sources trailing even when text names them", () => {
  const message = userMessage([
    { type: "text", text: "参考[first.xlsx]" },
    { type: "source_ref", source: first, inline: false },
  ]);

  assert.equal(conversationDisplayParts(message), message.parts);
});

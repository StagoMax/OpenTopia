import assert from "node:assert/strict";
import test from "node:test";
import fixture from "./generated/desktop-http-v1.fixture.json" with { type: "json" };
import { ApiContractError } from "./sseContracts.ts";
import { decodeHttpResponse } from "./httpContracts.ts";

test("decodes a Rust-serialized HTTP fixture", () => {
  assert.deepEqual(decodeHttpResponse("health", fixture), fixture);
});

test("rejects an HTTP response that drifted from its Rust DTO", () => {
  assert.throws(
    () =>
      decodeHttpResponse("health", {
        ...fixture,
        apiVersion: "1",
      }),
    ApiContractError,
  );
});

test("does not allow an unregistered response contract", () => {
  assert.throws(
    () => decodeHttpResponse("missing" as never, {}),
    /contract is not registered/,
  );
});

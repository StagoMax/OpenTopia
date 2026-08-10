import assert from "node:assert/strict";
import test from "node:test";
import { Router } from "../src/router.js";

test("matches static and parameter routes", () => {
  const router = new Router().add("GET", "/users/:id", "user").add("GET", "/users/new", "new");
  assert.deepEqual(router.match("get", "/users/new?from=home"), { value: "new", params: {} });
  assert.deepEqual(router.match("GET", "/users/A%20B"), { value: "user", params: { id: "A B" } });
});

test("returns null for unknown routes", () => {
  assert.equal(new Router().match("GET", "/missing"), null);
});

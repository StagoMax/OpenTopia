import assert from "node:assert/strict";
import test from "node:test";
import { syncBrowserAddress } from "./browserAddressState.ts";

test("keeps an address-bar draft while the browser reports an intermediate state", () => {
  assert.deepEqual(
    syncBrowserAddress({
      currentValue: "https://example.com/new-path",
      browserUrl: "",
      loading: true,
      editing: true,
      dirty: true,
      previousBrowserUrl: "",
      pendingUrl: "https://example.com/new-path",
    }),
    {
      value: "https://example.com/new-path",
      pendingUrl: "https://example.com/new-path",
    },
  );
});

test("does not overwrite an address-bar edit with an asynchronous browser URL", () => {
  assert.deepEqual(
    syncBrowserAddress({
      currentValue: "https://example.com/typed",
      browserUrl: "https://example.com/old",
      loading: false,
      editing: true,
      dirty: true,
      previousBrowserUrl: "https://example.com/old",
      pendingUrl: null,
    }),
    { value: "https://example.com/typed", pendingUrl: null },
  );
});

test("accepts the final redirected URL after a user navigation settles", () => {
  assert.deepEqual(
    syncBrowserAddress({
      currentValue: "https://example.com/start",
      browserUrl: "https://example.com/final",
      loading: false,
      editing: true,
      dirty: false,
      previousBrowserUrl: "https://example.com/start",
      pendingUrl: "https://example.com/start",
    }),
    { value: "https://example.com/final", pendingUrl: null },
  );
});

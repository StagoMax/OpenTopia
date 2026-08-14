import assert from "node:assert/strict";
import test from "node:test";

const { resolveAddressBarInput } = await import("./browserNavigation" + ".ts");

test("keeps absolute HTTP and HTTPS URLs", () => {
  assert.equal(
    resolveAddressBarInput(" https://example.com/a?b=1 "),
    "https://example.com/a?b=1",
  );
  assert.equal(
    resolveAddressBarInput("http://example.com"),
    "http://example.com/",
  );
});

test("adds a secure scheme to public hosts", () => {
  assert.equal(
    resolveAddressBarInput("example.com/docs?q=browser"),
    "https://example.com/docs?q=browser",
  );
  assert.equal(
    resolveAddressBarInput("192.0.2.10:8080/status"),
    "https://192.0.2.10:8080/status",
  );
});

test("uses HTTP for local development hosts", () => {
  assert.equal(
    resolveAddressBarInput("localhost:3000"),
    "http://localhost:3000/",
  );
  assert.equal(
    resolveAddressBarInput("127.0.0.1:5173/app"),
    "http://127.0.0.1:5173/app",
  );
});

test("uses Google for text and incomplete URLs", () => {
  assert.equal(
    resolveAddressBarInput("rust async trait"),
    "https://www.google.com/search?q=rust+async+trait",
  );
  assert.equal(
    resolveAddressBarInput("example"),
    "https://www.google.com/search?q=example",
  );
});

test("does not treat unsupported schemes as navigable URLs", () => {
  assert.equal(
    resolveAddressBarInput("javascript:alert(1)"),
    "https://www.google.com/search?q=javascript%3Aalert%281%29",
  );
});

test("rejects empty input", () => {
  assert.throws(() => resolveAddressBarInput("  "), /请输入 URL 或搜索内容/);
});

import assert from "node:assert/strict";
import test from "node:test";

const {
  STANDALONE_BROWSER_SESSION_ID,
  browserSessionId,
  navigateBrowserAddress,
  resolveAddressBarInput,
} = await import("./browserNavigation" + ".ts");

test("uses a valid standalone session before a task exists", () => {
  assert.equal(browserSessionId(null), STANDALONE_BROWSER_SESSION_ID);
  assert.equal(browserSessionId("  "), STANDALONE_BROWSER_SESSION_ID);
  assert.match(
    STANDALONE_BROWSER_SESSION_ID,
    /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/,
  );
  assert.equal(
    browserSessionId("00000000-0000-4000-8000-000000000001"),
    "00000000-0000-4000-8000-000000000001",
  );
});

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

test("creates the standalone session before opening a URL", async () => {
  const calls: unknown[] = [];
  const host = {
    async createSession(input: unknown) {
      calls.push(["create", input]);
    },
    async navigateFromAddressBar(sessionId: string, url: string) {
      calls.push(["navigate", sessionId, url]);
    },
  };

  const url = await navigateBrowserAddress(
    host,
    browserSessionId(null),
    "example.com/docs",
  );

  assert.equal(url, "https://example.com/docs");
  assert.deepEqual(calls, [
    ["create", { sessionId: STANDALONE_BROWSER_SESSION_ID, visible: false }],
    ["navigate", STANDALONE_BROWSER_SESSION_ID, url],
  ]);
});

test("opens incomplete address input as a Google search", async () => {
  let navigatedUrl = "";
  const url = await navigateBrowserAddress(
    {
      async createSession() {},
      async navigateFromAddressBar(_sessionId: string, nextUrl: string) {
        navigatedUrl = nextUrl;
      },
    },
    STANDALONE_BROWSER_SESSION_ID,
    "OpenTopia browser runtime",
  );

  assert.equal(
    url,
    "https://www.google.com/search?q=OpenTopia+browser+runtime",
  );
  assert.equal(navigatedUrl, url);
});

import assert from "node:assert/strict";
import test from "node:test";
import { browserTabTitle } from "./toolTabs.ts";

test("browser tabs use the page title when one is available", () => {
  assert.equal(
    browserTabTitle(
      { title: "  OpenTopia documentation  ", url: "https://example.com" },
      "浏览器 1",
    ),
    "OpenTopia documentation",
  );
});

test("browser tabs fall back to the host and then their stable label", () => {
  assert.equal(
    browserTabTitle(
      { title: "", url: "https://docs.example.com/guide" },
      "浏览器 2",
    ),
    "docs.example.com",
  );
  assert.equal(browserTabTitle({ title: "", url: "" }, "浏览器 2"), "浏览器 2");
});

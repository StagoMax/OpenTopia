import assert from "node:assert/strict";
import test from "node:test";

import type * as MarkdownLinksModule from "./markdownLinks";

const markdownLinks: typeof MarkdownLinksModule = await import(
  "./markdownLinks" + ".ts"
);

const { markdownStreamInterval, resolveMarkdownLink } = markdownLinks;

test("routes HTTP and HTTPS links to the internal browser", () => {
  assert.deepEqual(resolveMarkdownLink("https://example.com/docs?q=1"), {
    kind: "web",
    url: "https://example.com/docs?q=1",
  });
  assert.deepEqual(resolveMarkdownLink("//example.com/docs"), {
    kind: "web",
    url: "https://example.com/docs",
  });
});

test("keeps anchors local and routes mail links separately", () => {
  assert.deepEqual(resolveMarkdownLink("#footnote-1"), {
    kind: "anchor",
    href: "#footnote-1",
  });
  assert.deepEqual(resolveMarkdownLink("mailto:dev@example.com"), {
    kind: "email",
    url: "mailto:dev@example.com",
  });
});

test("resolves repository-relative links against a markdown file", () => {
  assert.deepEqual(
    resolveMarkdownLink(
      "../guide/setup.md#install",
      "docs/reference/readme.md",
    ),
    {
      kind: "workspace",
      path: "docs/guide/setup.md",
      fragment: "install",
    },
  );
  assert.deepEqual(resolveMarkdownLink("/README.md"), {
    kind: "workspace",
    path: "README.md",
    fragment: null,
  });
});

test("opens detected filesystem paths in the workspace preview", () => {
  assert.deepEqual(resolveMarkdownLink("opentopia-file:docs%2Fplan.md"), {
    kind: "workspace",
    path: "docs/plan.md",
    fragment: null,
  });
  assert.deepEqual(
    resolveMarkdownLink("opentopia-file:src%2FApp.tsx#L42", "docs/readme.md"),
    { kind: "workspace", path: "src/App.tsx", fragment: "L42" },
  );
  assert.deepEqual(
    resolveMarkdownLink("opentopia-file:%2Fsrv%2Fapp%2Fmain.rs"),
    { kind: "workspace", path: "/srv/app/main.rs", fragment: null },
  );
});

test("keeps Windows drive paths absolute", () => {
  assert.deepEqual(resolveMarkdownLink("J:\\Project\\OpenTopia\\README.md"), {
    kind: "workspace",
    path: "J:/Project/OpenTopia/README.md",
    fragment: null,
  });
});

test("blocks unsafe protocols and workspace traversal", () => {
  assert.equal(resolveMarkdownLink("javascript:alert(1)").kind, "blocked");
  assert.equal(resolveMarkdownLink("data:text/html,hello").kind, "blocked");
  assert.equal(
    resolveMarkdownLink("../../secret", "README.md").kind,
    "blocked",
  );
});

test("uses adaptive streaming intervals for long markdown", () => {
  assert.equal(markdownStreamInterval(1_000), 32);
  assert.equal(markdownStreamInterval(16 * 1024), 64);
  assert.equal(markdownStreamInterval(64 * 1024), 100);
});

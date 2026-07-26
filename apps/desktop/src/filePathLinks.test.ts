import assert from "node:assert/strict";
import test from "node:test";

import type * as FilePathLinksModule from "./filePathLinks";

const filePathLinks: typeof FilePathLinksModule = await import(
  "./filePathLinks" + ".ts"
);

const {
  decodeFilePathHref,
  encodeFilePathHref,
  findFilePaths,
  parseFilePathToken,
  remarkFilePathLinks,
} = filePathLinks;

type Node = {
  type: string;
  value?: string;
  url?: string;
  children?: Node[];
  data?: { hProperties?: Record<string, string> };
};

function paragraph(...children: Node[]): Node {
  return { type: "root", children: [{ type: "paragraph", children }] };
}

function transform(tree: Node): Node[] {
  remarkFilePathLinks()(tree);
  return tree.children?.[0]?.children ?? [];
}

test("detects workspace paths inside prose, including CJK sentences", () => {
  const matches = findFilePaths("已写入 docs/notes/plan.md，请查收");
  assert.equal(matches.length, 1);
  assert.deepEqual(matches[0].detected, {
    raw: "docs/notes/plan.md",
    path: "docs/notes/plan.md",
    line: null,
  });
});

test("keeps line references and Windows paths", () => {
  assert.deepEqual(parseFilePathToken("apps/desktop/src/App.tsx:1491"), {
    raw: "apps/desktop/src/App.tsx:1491",
    path: "apps/desktop/src/App.tsx",
    line: 1491,
  });
  assert.deepEqual(parseFilePathToken("J:\\Project\\OpenTopia\\README.md"), {
    raw: "J:\\Project\\OpenTopia\\README.md",
    path: "J:/Project/OpenTopia/README.md",
    line: null,
  });
});

test("ignores prose that only looks path-like", () => {
  for (const token of [
    "and/or",
    "CI/CD",
    "2026/07/26",
    "v1.2.3",
    "https://example.com/guide.md",
    "dev@example.com",
    "@opentopia/desktop",
  ]) {
    assert.equal(parseFilePathToken(token), null, token);
  }
  assert.deepEqual(findFilePaths("see https://example.com/guide.md now"), []);
});

test("accepts bare file names only when the token is already path-shaped", () => {
  assert.equal(parseFilePathToken("Node.js"), null);
  assert.deepEqual(
    parseFilePathToken("README.md", { allowBareFileName: true }),
    { raw: "README.md", path: "README.md", line: null },
  );
  assert.equal(
    parseFilePathToken("pnpm --filter @opentopia/desktop test", {
      allowBareFileName: true,
    }),
    null,
  );
});

test("trims trailing sentence punctuation from a path", () => {
  const matches = findFilePaths("Written to docs/plan.md.");
  assert.equal(matches.length, 1);
  assert.equal(matches[0].detected.path, "docs/plan.md");
  assert.equal(matches[0].end, "Written to docs/plan.md".length);
});

test("rewrites prose paths into links and keeps surrounding text", () => {
  const children = transform(
    paragraph({ type: "text", value: "已写入 docs/plan.md，请查收" }),
  );
  assert.equal(children.length, 3);
  assert.deepEqual(children[0], { type: "text", value: "已写入 " });
  assert.equal(children[1].type, "link");
  assert.equal(children[1].url, "opentopia-file:docs%2Fplan.md");
  assert.deepEqual(children[1].children, [
    { type: "text", value: "docs/plan.md" },
  ]);
  assert.deepEqual(children[2], { type: "text", value: "，请查收" });
});

test("wraps path-shaped code spans and leaves other spans alone", () => {
  const [link] = transform(
    paragraph({ type: "inlineCode", value: "README.md" }),
  );
  assert.equal(link.type, "link");
  assert.equal(link.children?.[0].type, "inlineCode");

  const [span] = transform(
    paragraph({ type: "inlineCode", value: "pnpm test" }),
  );
  assert.equal(span.type, "inlineCode");
});

test("never rewrites code blocks or existing links", () => {
  const tree: Node = {
    type: "root",
    children: [
      { type: "code", value: "cat docs/plan.md" },
      {
        type: "link",
        url: "https://example.com",
        children: [{ type: "text", value: "docs/plan.md" }],
      },
    ],
  };
  remarkFilePathLinks()(tree);
  assert.deepEqual(tree.children?.[0], {
    type: "code",
    value: "cat docs/plan.md",
  });
  assert.deepEqual(tree.children?.[1].children, [
    { type: "text", value: "docs/plan.md" },
  ]);
});

test("round-trips hrefs including line fragments", () => {
  const href = encodeFilePathHref({
    raw: "src/App.tsx:42",
    path: "src/App.tsx",
    line: 42,
  });
  assert.equal(href, "opentopia-file:src%2FApp.tsx#L42");
  assert.deepEqual(decodeFilePathHref(href), {
    path: "src/App.tsx",
    fragment: "L42",
  });
  assert.equal(decodeFilePathHref("https://example.com"), null);
});

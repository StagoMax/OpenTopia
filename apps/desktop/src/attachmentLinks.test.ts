import assert from "node:assert/strict";
import test from "node:test";

import type * as AttachmentLinksModule from "./attachmentLinks";

const attachmentLinks: typeof AttachmentLinksModule = await import(
  "./attachmentLinks" + ".ts"
);

const {
  decodeAttachmentLink,
  encodeAttachmentLink,
  findAttachmentReferences,
  remarkAttachmentLinks,
  resolveAttachmentReference,
} = attachmentLinks;

const sources = [
  {
    id: "attachment-pdf",
    name: "dzfp_26442000009148901746_佛山市凯骏五金制造有限公司_20260810162438.pdf",
  },
  {
    id: "attachment-docx",
    name: "TIKTOKCREATOR订单导出指南 (1).docx",
  },
];

type Node = {
  type: string;
  value?: string;
  url?: string;
  children?: Node[];
};

test("round-trips opaque attachment identities", () => {
  const href = encodeAttachmentLink({ id: "id with spaces", name: "file.pdf" });
  assert.equal(href, "opentopia-attachment:id%20with%20spaces");
  assert.equal(decodeAttachmentLink(href), "id with spaces");
  assert.equal(decodeAttachmentLink("opentopia-attachment:"), null);
});

test("resolves exact and uniquely abbreviated attachment names", () => {
  assert.equal(
    resolveAttachmentReference("TIKTOKCREATOR订单导出指南 (1).docx", sources)
      ?.id,
    "attachment-docx",
  );
  assert.equal(
    resolveAttachmentReference(
      "dzfp_...佛山市凯骏五金制造有限公司_20260810162438.pdf",
      sources,
    )?.id,
    "attachment-pdf",
  );
});

test("does not guess when abbreviated or exact names are ambiguous", () => {
  const ambiguous = [
    { id: "first", name: "report_2026_final.pdf" },
    { id: "second", name: "report_2026_final.pdf" },
  ];
  assert.equal(
    resolveAttachmentReference("report...final.pdf", ambiguous),
    null,
  );
  assert.equal(
    resolveAttachmentReference("report_2026_final.pdf", ambiguous),
    null,
  );
});

test("finds complete names in prose without linking abbreviations", () => {
  const text = `请比较 ${sources[1].name} 与 dzfp_...pdf`;
  assert.deepEqual(findAttachmentReferences(text, sources), [
    {
      start: 4,
      end: 4 + sources[1].name.length,
      source: sources[1],
    },
  ]);
});

test("rewrites prose and inline-code references but keeps existing links opaque", () => {
  const tree: Node = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [
          { type: "text", value: `打开 ${sources[1].name} 和 ` },
          {
            type: "inlineCode",
            value: "dzfp_...佛山市凯骏五金制造有限公司_20260810162438.pdf",
          },
          {
            type: "link",
            url: "https://example.com",
            children: [{ type: "text", value: sources[1].name }],
          },
        ],
      },
    ],
  };

  remarkAttachmentLinks({ sources })(tree);
  const children = tree.children?.[0].children ?? [];
  assert.equal(children[1].url, "opentopia-attachment:attachment-docx");
  assert.equal(children[3].url, "opentopia-attachment:attachment-pdf");
  assert.equal(children[3].children?.[0].type, "inlineCode");
  assert.equal(children[4].url, "https://example.com");
});

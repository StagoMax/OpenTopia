import assert from "node:assert/strict";
import test from "node:test";
import {
  appendImageUnderstandingContext,
  buildImageUnderstandingArguments,
  extractImageUnderstandingText,
  isImageUnderstandingMcpTool,
} from "./imageProcessing.ts";
import type { InlineImageAttachment, McpToolDescriptor } from "./types.ts";

const attachment: InlineImageAttachment = {
  id: "11111111-1111-4111-8111-111111111111",
  contentType: "image/png",
  data: [1, 2, 3],
  name: "sample.png",
};

function tool(inputSchema: unknown): McpToolDescriptor {
  return {
    publicName: "vision_describe",
    serverId: "server-1",
    toolName: "describe_image",
    description: "Describe an image and answer a question about it.",
    inputSchema,
    annotations: {},
    permissionLabels: [],
  };
}

test("recognizes visual MCP tools from their description and schema", () => {
  assert.equal(
    isImageUnderstandingMcpTool(
      tool({
        type: "object",
        properties: {
          image_url: { type: "string", format: "uri" },
          prompt: { type: "string" },
        },
      }),
    ),
    true,
  );
  assert.equal(
    isImageUnderstandingMcpTool({
      ...tool({ type: "object", properties: { path: { type: "string" } } }),
      publicName: "list_images",
      toolName: "list_images",
      description: "List image files",
    }),
    false,
  );
});

test("builds schema-compatible data URL and prompt arguments", () => {
  const args = buildImageUnderstandingArguments(
    tool({
      type: "object",
      properties: {
        image_url: { type: "string", format: "uri" },
        question: { type: "string" },
      },
    }),
    "What is shown?",
    [attachment],
  );
  assert.equal(args.question, "What is shown?");
  assert.equal(args.image_url, "data:image/png;base64,AQID");
});

test("extracts text and keeps the original request in the fallback prompt", () => {
  const result = {
    serverId: "server-1",
    publicName: "vision_describe",
    toolName: "describe_image",
    output: "A red square.",
    content: [{ type: "text", text: "It is centered." }],
    structuredContent: null,
    isError: false,
    raw: {},
  };
  assert.equal(
    extractImageUnderstandingText(result),
    "A red square.\n\nIt is centered.",
  );
  assert.match(
    appendImageUnderstandingContext("Keep it concise.", "A red square."),
    /Keep it concise/,
  );
});

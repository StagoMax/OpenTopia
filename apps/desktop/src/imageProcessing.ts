import type {
  InlineImageAttachment,
  McpCallResult,
  McpToolDescriptor,
} from "./types";

const VISUAL_TERMS =
  /image|picture|photo|vision|visual|video|media|screenshot|ocr|图像|图片|视频|视觉|截图|识别/i;
const ACTION_TERMS =
  /understand|describe|analy[sz]|caption|extract|recogn|transcrib|read|question|answer|interpret|understanding|理解|描述|分析|识别|提取|读取/i;
const MEDIA_INPUT_TERMS =
  /image|picture|photo|vision|video|media|screenshot|file|base64|data.?url|url|图片|视频|图像|文件/i;
const PROMPT_INPUT_TERMS =
  /prompt|question|query|instruction|request|message|text|description|ask|问题|提示|指令|描述/i;

type SchemaProperty = {
  type?: string;
  format?: string;
  items?: SchemaProperty;
};

function schemaProperties(
  tool: McpToolDescriptor,
): Record<string, SchemaProperty> {
  const schema = tool.inputSchema;
  if (!schema || typeof schema !== "object") return {};
  const properties = (schema as { properties?: unknown }).properties;
  if (!properties || typeof properties !== "object") return {};
  return properties as Record<string, SchemaProperty>;
}

export function isImageUnderstandingMcpTool(tool: McpToolDescriptor): boolean {
  const properties = schemaProperties(tool);
  const propertyNames = Object.keys(properties);
  const searchableText = [
    tool.publicName,
    tool.toolName,
    tool.description ?? "",
    ...propertyNames,
  ].join(" ");
  const hasMediaInput = propertyNames.some((name) =>
    MEDIA_INPUT_TERMS.test(name),
  );
  return (
    VISUAL_TERMS.test(searchableText) &&
    (ACTION_TERMS.test(searchableText) || hasMediaInput)
  );
}

function bytesToBase64(bytes: number[]): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  }
  return btoa(binary);
}

function dataUrl(attachment: InlineImageAttachment): string {
  return `data:${attachment.contentType};base64,${bytesToBase64(attachment.data)}`;
}

function mediaValue(
  propertyName: string,
  property: SchemaProperty,
  attachment: InlineImageAttachment,
): unknown {
  const encoded = bytesToBase64(attachment.data);
  if (property.type === "object") {
    return {
      mimeType: attachment.contentType,
      mediaType: attachment.contentType,
      data: encoded,
      name: attachment.name,
    };
  }
  if (
    propertyName.toLocaleLowerCase().includes("url") ||
    property.format === "uri"
  ) {
    return dataUrl(attachment);
  }
  return encoded;
}

export function buildImageUnderstandingArguments(
  tool: McpToolDescriptor,
  prompt: string,
  attachments: InlineImageAttachment[],
): Record<string, unknown> {
  const properties = schemaProperties(tool);
  const args: Record<string, unknown> = {};
  const first = attachments[0];

  for (const [name, property] of Object.entries(properties)) {
    if (PROMPT_INPUT_TERMS.test(name)) {
      args[name] = prompt;
      continue;
    }
    if (!MEDIA_INPUT_TERMS.test(name) || !first) continue;
    args[name] =
      property.type === "array"
        ? attachments.map((attachment) =>
            mediaValue(name, property.items ?? {}, attachment),
          )
        : mediaValue(name, property, first);
  }

  if (Object.keys(args).length === 0) {
    args.prompt = prompt;
    args.image =
      attachments.length === 1 ? dataUrl(first) : attachments.map(dataUrl);
  } else if (
    !Object.keys(args).some((name) => MEDIA_INPUT_TERMS.test(name)) &&
    first
  ) {
    args.image = dataUrl(first);
  }

  return args;
}

function textFromContentItem(item: unknown): string {
  if (typeof item === "string") return item;
  if (!item || typeof item !== "object") return "";
  const record = item as Record<string, unknown>;
  if (typeof record.text === "string") return record.text;
  if (typeof record.output === "string") return record.output;
  return "";
}

export function extractImageUnderstandingText(result: McpCallResult): string {
  const content = result.content.map(textFromContentItem).filter(Boolean);
  const output = [result.output, ...content]
    .map((value) => value.trim())
    .filter(Boolean);
  if (output.length > 0) return Array.from(new Set(output)).join("\n\n");
  if (result.structuredContent != null) {
    return JSON.stringify(result.structuredContent, null, 2);
  }
  return "";
}

export function appendImageUnderstandingContext(
  prompt: string,
  understanding: string,
): string {
  const request =
    prompt.trim() || "Answer the user's request using the attached image.";
  return `${request}\n\nImage understanding result:\n${understanding}`;
}

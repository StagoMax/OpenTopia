import Ajv, { type ErrorObject } from "ajv";

const ajv = new Ajv({ allErrors: true, strict: false });

export type ParsedFlowTestInput =
  | { ok: true; input: unknown; formatted: string }
  | { ok: false; error: string };

export function parseFlowTestInput(
  source: string,
  schema?: Record<string, unknown>,
): ParsedFlowTestInput {
  let input: unknown;
  try {
    input = JSON.parse(source);
  } catch (error) {
    return {
      ok: false,
      error:
        error instanceof SyntaxError
          ? `JSON 格式错误：${error.message}`
          : "无法读取测试输入。",
    };
  }

  if (schema && Object.keys(schema).length > 0) {
    try {
      const validate = ajv.compile(schema);
      if (!validate(input)) {
        return { ok: false, error: schemaErrorMessage(validate.errors) };
      }
    } catch {
      return { ok: false, error: "Flow 的 Input Schema 无法用于校验。" };
    }
  }

  return { ok: true, input, formatted: JSON.stringify(input, null, 2) };
}

function schemaErrorMessage(errors: ErrorObject[] | null | undefined) {
  const first = errors?.[0];
  if (!first) return "测试输入不符合 Flow 的 Input Schema。";
  const location = first.instancePath || "输入根节点";
  return `${location} ${first.message ?? "不符合 Input Schema"}`;
}

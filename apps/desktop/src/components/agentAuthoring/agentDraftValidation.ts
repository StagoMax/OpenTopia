import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage.ts";

export type AgentDraftFieldError = {
  field: "templateId";
  message: string;
};

export function validateAgentTemplateId(
  value: string,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string | null {
  const normalized = value.trim();
  if (!normalized) {
    return interfaceMessage(language, "flow.agentEditor.templateIdRequired");
  }
  if (normalized.length > 120 || !/^[a-z0-9._-]+$/.test(normalized)) {
    return interfaceMessage(language, "flow.agentEditor.templateIdInvalid");
  }
  return null;
}

export function agentDraftFieldErrorFromCreateFailure(
  error: unknown,
  language: ApplicationLanguage = defaultApplicationLanguage,
): AgentDraftFieldError | null {
  const message = apiErrorDetail(error);
  if (message.includes("templateId must be a lowercase slug")) {
    return {
      field: "templateId",
      message: interfaceMessage(language, "flow.agentEditor.templateIdInvalid"),
    };
  }
  return null;
}

function apiErrorDetail(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  try {
    const payload = JSON.parse(message) as { error?: unknown };
    return typeof payload.error === "string" ? payload.error : message;
  } catch {
    return message;
  }
}

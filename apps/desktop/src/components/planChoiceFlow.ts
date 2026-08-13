import type {
  UserInputAnswer,
  UserInputRequest,
  UserInputResponse,
} from "../types";

export const CUSTOM_OPTION_ID = "__custom__";

export type PlanChoiceSelections = Record<string, string>;
export type PlanChoiceCustomAnswers = Record<string, string>;

export function buildPlanChoiceResponse(
  request: UserInputRequest,
  selections: PlanChoiceSelections,
  customAnswers: PlanChoiceCustomAnswers,
): UserInputResponse | null {
  const answers: UserInputAnswer[] = [];

  for (const question of request.questions) {
    const selection = selections[question.id];
    if (!selection) return null;

    if (selection === CUSTOM_OPTION_ID) {
      const customText = customAnswers[question.id]?.trim();
      if (!question.allowCustom || !customText) return null;
      answers.push({ questionId: question.id, customText });
      continue;
    }

    if (!question.options.some((option) => option.id === selection)) {
      return null;
    }
    answers.push({ questionId: question.id, optionId: selection });
  }

  return { answers };
}

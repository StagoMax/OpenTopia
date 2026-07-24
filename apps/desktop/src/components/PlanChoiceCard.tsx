import { useMemo, useState } from "react";
import { CircleHelp, Loader2 } from "lucide-react";
import type {
  UserInputAnswer,
  UserInputRequest,
  UserInputResponse,
} from "../types";
import "./PlanChoiceCard.css";

const CUSTOM_OPTION_ID = "__custom__";

type PlanChoiceCardProps = {
  request: UserInputRequest;
  isSubmitting: boolean;
  error: string | null;
  onSubmit(response: UserInputResponse): void;
};

type Selections = Record<string, string>;
type CustomAnswers = Record<string, string>;

export function PlanChoiceCard({
  request,
  isSubmitting,
  error,
  onSubmit,
}: PlanChoiceCardProps) {
  const [selections, setSelections] = useState<Selections>({});
  const [customAnswers, setCustomAnswers] = useState<CustomAnswers>({});

  const complete = useMemo(
    () =>
      request.questions.every((question) => {
        const selection = selections[question.id];
        if (!selection) return false;
        return (
          selection !== CUSTOM_OPTION_ID ||
          Boolean(customAnswers[question.id]?.trim())
        );
      }),
    [customAnswers, request.questions, selections],
  );

  function submit() {
    if (!complete || isSubmitting) return;
    const answers: UserInputAnswer[] = request.questions.map((question) => {
      const selection = selections[question.id];
      return selection === CUSTOM_OPTION_ID
        ? {
            questionId: question.id,
            customText: customAnswers[question.id].trim(),
          }
        : { questionId: question.id, optionId: selection };
    });
    onSubmit({ answers });
  }

  return (
    <aside
      className="plan-choice-card"
      role="region"
      aria-live="polite"
      aria-labelledby="plan-choice-title"
    >
      <header className="plan-choice-header">
        <span className="plan-choice-icon" aria-hidden="true">
          <CircleHelp size={17} />
        </span>
        <div>
          <h2 id="plan-choice-title">需要你的选择</h2>
          <p>确认关键决策后继续生成计划。</p>
        </div>
        {request.questions.length > 1 ? (
          <span className="plan-choice-count">
            {request.questions.length} 项
          </span>
        ) : null}
      </header>

      <div className="plan-choice-scroll">
        <div className="plan-choice-questions">
          {request.questions.map((question, questionIndex) => (
            <fieldset className="plan-choice-question" key={question.id}>
              <legend>
                <span>{question.header}</span>
                <strong>{question.question}</strong>
              </legend>
              <div className="plan-choice-options">
                {question.options.map((option) => {
                  const inputId = `${request.requestId}-${question.id}-${option.id}`;
                  const selected = selections[question.id] === option.id;
                  return (
                    <label
                      className={`plan-choice-option ${selected ? "selected" : ""}`}
                      htmlFor={inputId}
                      key={option.id}
                    >
                      <input
                        checked={selected}
                        id={inputId}
                        name={`${request.requestId}-${question.id}`}
                        type="radio"
                        value={option.id}
                        onChange={() =>
                          setSelections((current) => ({
                            ...current,
                            [question.id]: option.id,
                          }))
                        }
                      />
                      <span className="plan-choice-option-copy">
                        <span className="plan-choice-option-title">
                          <strong>{option.label}</strong>
                          {option.recommended ? <em>推荐</em> : null}
                        </span>
                        <span>{option.description}</span>
                      </span>
                    </label>
                  );
                })}

                {question.allowCustom ? (
                  <div
                    className={`plan-choice-custom ${
                      selections[question.id] === CUSTOM_OPTION_ID
                        ? "selected"
                        : ""
                    }`}
                  >
                    <label
                      className="plan-choice-option plan-choice-custom-trigger"
                      htmlFor={`${request.requestId}-${question.id}-custom`}
                    >
                      <input
                        checked={selections[question.id] === CUSTOM_OPTION_ID}
                        id={`${request.requestId}-${question.id}-custom`}
                        name={`${request.requestId}-${question.id}`}
                        type="radio"
                        value={CUSTOM_OPTION_ID}
                        onChange={() =>
                          setSelections((current) => ({
                            ...current,
                            [question.id]: CUSTOM_OPTION_ID,
                          }))
                        }
                      />
                      <span className="plan-choice-option-copy">
                        <strong>其他方案</strong>
                        <span>补充你希望采用的方向。</span>
                      </span>
                    </label>
                    {selections[question.id] === CUSTOM_OPTION_ID ? (
                      <textarea
                        autoFocus={questionIndex === 0}
                        aria-label={`${question.header}的其他方案`}
                        maxLength={1000}
                        placeholder="输入你的选择或约束"
                        rows={2}
                        value={customAnswers[question.id] ?? ""}
                        onChange={(event) =>
                          setCustomAnswers((current) => ({
                            ...current,
                            [question.id]: event.target.value,
                          }))
                        }
                      />
                    ) : null}
                  </div>
                ) : null}
              </div>
            </fieldset>
          ))}
        </div>

        {error ? (
          <p className="plan-choice-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>

      <footer className="plan-choice-actions">
        <span>
          {complete
            ? "选择已完整"
            : `还需选择 ${request.questions.filter((question) => !selections[question.id] || (selections[question.id] === CUSTOM_OPTION_ID && !customAnswers[question.id]?.trim())).length} 项`}
        </span>
        <button
          type="button"
          disabled={!complete || isSubmitting}
          onClick={submit}
        >
          {isSubmitting ? (
            <>
              <Loader2 className="plan-choice-spinner" size={15} />
              正在提交
            </>
          ) : (
            "继续规划"
          )}
        </button>
      </footer>
    </aside>
  );
}

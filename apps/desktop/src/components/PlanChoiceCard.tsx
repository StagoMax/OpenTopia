import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ChevronRight,
  CircleHelp,
  Loader2,
  PencilLine,
} from "lucide-react";
import type { UserInputRequest, UserInputResponse } from "../types";
import {
  buildPlanChoiceResponse,
  CUSTOM_OPTION_ID,
  type PlanChoiceCustomAnswers,
  type PlanChoiceSelections,
} from "./planChoiceFlow";
import { Badge, Button, IconButton } from "./ui";
import "./PlanChoiceCard.css";

type PlanChoiceCardProps = {
  request: UserInputRequest;
  isSubmitting: boolean;
  error: string | null;
  onSubmit(response: UserInputResponse): void;
  onSkip(): void;
  onCancel(): void;
};

export function PlanChoiceCard({
  request,
  isSubmitting,
  error,
  onSubmit,
  onSkip,
  onCancel,
}: PlanChoiceCardProps) {
  const [currentQuestionIndex, setCurrentQuestionIndex] = useState(0);
  const [selections, setSelections] = useState<PlanChoiceSelections>({});
  const [customAnswers, setCustomAnswers] = useState<PlanChoiceCustomAnswers>(
    {},
  );
  const questionTitleRef = useRef<HTMLHeadingElement>(null);
  const customInputRef = useRef<HTMLTextAreaElement>(null);

  const question = request.questions[currentQuestionIndex];
  const questionCount = request.questions.length;
  const isLastQuestion = currentQuestionIndex === questionCount - 1;
  const selectedOption = question ? selections[question.id] : undefined;
  const isCustomSelected = selectedOption === CUSTOM_OPTION_ID;

  useEffect(() => {
    if (isCustomSelected) {
      customInputRef.current?.focus();
      return;
    }
    questionTitleRef.current?.focus();
  }, [currentQuestionIndex, isCustomSelected, request.requestId]);

  if (!question) return null;

  const customText = customAnswers[question.id] ?? "";
  const questionTitleId = `${request.requestId}-${question.id}-title`;
  const customInputId = `${request.requestId}-${question.id}-custom-text`;
  const progress = (currentQuestionIndex + 1) / questionCount;

  function submitWith(
    nextSelections: PlanChoiceSelections,
    nextCustomAnswers: PlanChoiceCustomAnswers,
  ) {
    const response = buildPlanChoiceResponse(
      request,
      nextSelections,
      nextCustomAnswers,
    );
    if (response) onSubmit(response);
  }

  function chooseOption(optionId: string) {
    if (isSubmitting) return;

    const nextSelections = { ...selections, [question.id]: optionId };
    setSelections(nextSelections);

    if (isLastQuestion) {
      submitWith(nextSelections, customAnswers);
      return;
    }
    setCurrentQuestionIndex((current) => current + 1);
  }

  function chooseCustom() {
    if (isSubmitting) return;
    setSelections((current) => ({
      ...current,
      [question.id]: CUSTOM_OPTION_ID,
    }));
  }

  function continueWithCustomAnswer() {
    if (!customText.trim() || isSubmitting) return;

    if (isLastQuestion) {
      submitWith(selections, customAnswers);
      return;
    }
    setCurrentQuestionIndex((current) => current + 1);
  }

  return (
    <aside
      className="plan-choice-card"
      role="region"
      aria-labelledby={questionTitleId}
    >
      <header className="plan-choice-header">
        <div className="plan-choice-leading">
          {currentQuestionIndex > 0 ? (
            <IconButton
              className="plan-choice-back"
              size="compact"
              aria-label="返回上一题"
              disabled={isSubmitting}
              onClick={() =>
                setCurrentQuestionIndex((current) => Math.max(0, current - 1))
              }
            >
              <ArrowLeft size={16} aria-hidden="true" />
            </IconButton>
          ) : (
            <span className="plan-choice-icon" aria-hidden="true">
              <CircleHelp size={18} />
            </span>
          )}
        </div>

        <div className="plan-choice-heading">
          <span className="plan-choice-eyebrow">{question.header}</span>
          <h2 id={questionTitleId} ref={questionTitleRef} tabIndex={-1}>
            {question.question}
          </h2>
          <p>
            {questionCount > 1
              ? "选择后自动进入下一题。"
              : "选择后将继续执行当前任务。"}
          </p>
        </div>

        {questionCount > 1 ? (
          <Badge
            className="plan-choice-count"
            aria-label={`第 ${currentQuestionIndex + 1} 题，共 ${questionCount} 题`}
          >
            {currentQuestionIndex + 1} / {questionCount}
          </Badge>
        ) : null}
      </header>

      {questionCount > 1 ? (
        <div
          className="plan-choice-progress"
          role="progressbar"
          aria-label="问题进度"
          aria-valuemin={1}
          aria-valuemax={questionCount}
          aria-valuenow={currentQuestionIndex + 1}
        >
          <span style={{ transform: `scaleX(${progress})` }} />
        </div>
      ) : null}

      <div className="plan-choice-scroll">
        <fieldset className="plan-choice-question" key={question.id}>
          <legend className="ot-sr-only">{question.header}的可选方案</legend>
          <div className="plan-choice-options">
            {question.options.map((option, optionIndex) => {
              const selected = selectedOption === option.id;
              return (
                <button
                  className={`plan-choice-option ${selected ? "selected" : ""}`}
                  type="button"
                  aria-pressed={selected}
                  disabled={isSubmitting}
                  key={option.id}
                  onClick={() => chooseOption(option.id)}
                >
                  <span className="plan-choice-option-index" aria-hidden="true">
                    {optionIndex + 1}
                  </span>
                  <span className="plan-choice-option-copy">
                    <span className="plan-choice-option-title">
                      <strong>{option.label}</strong>
                      {option.recommended ? <Badge>推荐</Badge> : null}
                    </span>
                    <span>{option.description}</span>
                  </span>
                  <ChevronRight
                    className="plan-choice-option-arrow"
                    size={16}
                    aria-hidden="true"
                  />
                </button>
              );
            })}

            {question.allowCustom ? (
              <div
                className={`plan-choice-custom ${isCustomSelected ? "selected" : ""}`}
              >
                <button
                  className="plan-choice-option plan-choice-custom-trigger"
                  type="button"
                  aria-expanded={isCustomSelected}
                  aria-controls={customInputId}
                  disabled={isSubmitting}
                  onClick={chooseCustom}
                >
                  <span className="plan-choice-option-index" aria-hidden="true">
                    <PencilLine size={14} />
                  </span>
                  <span className="plan-choice-option-copy">
                    <strong>其他方案</strong>
                    <span>补充你希望采用的方向。</span>
                  </span>
                  <ChevronRight
                    className="plan-choice-option-arrow"
                    size={16}
                    aria-hidden="true"
                  />
                </button>

                {isCustomSelected ? (
                  <div className="plan-choice-custom-fields">
                    <label htmlFor={customInputId}>补充你的选择</label>
                    <textarea
                      id={customInputId}
                      ref={customInputRef}
                      maxLength={1000}
                      placeholder="输入你的选择或约束"
                      rows={2}
                      value={customText}
                      onChange={(event) =>
                        setCustomAnswers((current) => ({
                          ...current,
                          [question.id]: event.target.value,
                        }))
                      }
                    />
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        </fieldset>

        {error ? (
          <p className="plan-choice-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>

      <footer className="plan-choice-actions">
        <span className="plan-choice-status" aria-live="polite">
          {isSubmitting ? (
            <>
              <Loader2 className="plan-choice-spinner" size={14} />
              正在提交答案
            </>
          ) : isCustomSelected ? (
            "填写后继续"
          ) : isLastQuestion ? (
            "选择一项后完成"
          ) : (
            "选择一项后进入下一题"
          )}
        </span>

        <div className="plan-choice-action-buttons">
          <Button
            className="plan-choice-skip"
            variant="quiet"
            disabled={isSubmitting}
            onClick={onSkip}
          >
            跳过并继续
          </Button>
          <Button variant="quiet" disabled={isSubmitting} onClick={onCancel}>
            结束本轮
          </Button>
          {isCustomSelected ? (
            <Button
              className="plan-choice-custom-next"
              variant="primary"
              disabled={!customText.trim() || isSubmitting}
              onClick={continueWithCustomAnswer}
            >
              {isLastQuestion ? "完成" : "下一题"}
              {!isSubmitting ? (
                <ChevronRight size={15} aria-hidden="true" />
              ) : null}
            </Button>
          ) : null}
        </div>
      </footer>
    </aside>
  );
}

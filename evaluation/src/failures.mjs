const PROVIDER_TRANSPORT_PATTERNS = [
  /error decoding response body/i,
  /provider stream ended before a terminal event/i,
  /provider completion protocol error/i,
  /provider reported tool_calls but returned no tool call/i,
  /upstream.*(?:disconnect|closed|timeout)/i,
];

function text(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function lastEvent(events, type, predicate = () => true) {
  return [...events].reverse().find((event) => event.type === type && predicate(event));
}

function failedCheckIds(checks) {
  return checks.filter((check) => !check.passed).map((check) => check.id);
}

export function classifyTrialFailure({ status, checks = [], events = [], targetResult = {} }) {
  if (status === "passed") return null;
  const terminal = lastEvent(
    events,
    "application.turn.completed",
    (event) => event.payload?.status !== "succeeded",
  );
  const adapterError = lastEvent(events, "application.adapter_error");
  const terminalError = text(terminal?.payload?.error);
  const adapterMessage = text(adapterError?.payload?.message);
  const checkIds = failedCheckIds(checks);

  let category;
  let summary;
  if (status === "grader_error") {
    category = "grader";
    summary = "One or more graders failed to execute reliably.";
  } else if (status === "safety_violation") {
    category = "safety";
    summary = "A hard safety check failed.";
  } else if (status === "watchdog_timeout") {
    category = "watchdog";
    summary = "The trial exceeded its watchdog budget.";
  } else if (status === "invalid_task") {
    category = "invalid_task";
    summary = "The task definition was invalid.";
  } else if (status === "cancelled_by_evaluator") {
    category = "cancelled";
    summary = "The evaluator cancelled the trial.";
  } else if (targetResult.spawnError || adapterMessage) {
    category = "harness_adapter";
    summary = adapterMessage ?? text(targetResult.spawnError) ?? "The target adapter failed.";
  } else if (
    terminalError &&
    PROVIDER_TRANSPORT_PATTERNS.some((pattern) => pattern.test(terminalError))
  ) {
    category = "provider_transport";
    summary = terminalError;
  } else if (status === "application_crash" || terminal) {
    category = "application_runtime";
    summary = terminalError ?? `Application terminal status: ${terminal?.payload?.status ?? "failed"}.`;
  } else if (status === "false_completion") {
    category = "model_behavior";
    summary = "The agent claimed completion while one or more outcome checks failed.";
  } else {
    category = "task_outcome";
    summary = "One or more task outcome or trajectory checks failed.";
  }

  return {
    category,
    summary,
    terminalStatus: terminal?.payload?.status ?? null,
    failedCheckIds: checkIds,
  };
}

export function isAbilityEligible(result) {
  if (["infra_error", "grader_error", "invalid_task", "runtime_dependency_error"].includes(result.status)) {
    return false;
  }
  return !["provider_transport", "harness_adapter", "grader", "invalid_task"].includes(
    result.failureCategory ?? result.failure?.category,
  );
}

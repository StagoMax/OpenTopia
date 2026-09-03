export type FlowEditorStage = "save" | "validate" | "test" | "activate";

export function nextFlowEditorStage({
  draftExists,
  successfulTestRun,
  validated,
}: {
  draftExists: boolean;
  successfulTestRun: boolean;
  validated: boolean;
}): FlowEditorStage {
  if (!draftExists) return "save";
  if (!validated) return "validate";
  if (!successfulTestRun) return "test";
  return "activate";
}

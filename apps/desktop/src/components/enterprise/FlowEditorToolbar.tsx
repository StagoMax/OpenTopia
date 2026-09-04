import { FlaskConical, Plus, RefreshCw, Send, ShieldCheck } from "lucide-react";
import { Badge, Button } from "../ui";
import { nextFlowEditorStage } from "./flowEditorProgress";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function FlowEditorToolbar({
  activeTestRun,
  busy,
  canActivate,
  canCreateDraft,
  canTestRun,
  draftExists,
  flowId,
  name,
  nodeCount,
  onActivate,
  onCreateDraft,
  onRefresh,
  onTestRun,
  onValidate,
  successfulTestRun,
  threadReady,
  validated,
}: {
  activeTestRun: boolean;
  busy: string | null;
  canActivate: boolean;
  canCreateDraft: boolean;
  canTestRun: boolean;
  draftExists: boolean;
  flowId: string;
  name: string;
  nodeCount: number;
  onActivate(): void;
  onCreateDraft(): void;
  onRefresh(): void;
  onTestRun(): void;
  onValidate(): void;
  successfulTestRun: boolean;
  threadReady: boolean;
  validated: boolean;
}) {
  const { t } = useApplicationLanguage();
  const stage = nextFlowEditorStage({
    draftExists,
    successfulTestRun,
    validated,
  });
  const nextAction =
    stage === "save"
      ? {
          disabled: !canCreateDraft,
          icon: Plus,
          label:
            busy === "create"
              ? t("flow.toolbar.creating")
              : t("flow.toolbar.saveDraft"),
          onClick: onCreateDraft,
        }
      : stage === "validate"
        ? {
            disabled: false,
            icon: ShieldCheck,
            label:
              busy === "validate"
                ? t("flow.toolbar.validating")
                : t("flow.toolbar.validate"),
            onClick: onValidate,
          }
        : stage === "test"
          ? {
              disabled: !canTestRun || activeTestRun,
              icon: FlaskConical,
              label:
                busy === "test-run"
                  ? t("flow.toolbar.starting")
                  : t("flow.toolbar.testRun"),
              onClick: onTestRun,
            }
          : {
              disabled: !canActivate,
              icon: Send,
              label:
                busy === "activate"
                  ? t("flow.toolbar.activating")
                  : t("flow.toolbar.activate"),
              onClick: onActivate,
            };
  const NextIcon = nextAction.icon;

  return (
    <header className="workflow-editor__toolbar">
      <span className="workflow-editor__summary">
        <Badge
          variant={validated ? "success" : draftExists ? "neutral" : "warning"}
        >
          {activeTestRun
            ? t("flow.toolbar.testing")
            : validated
              ? t("flow.toolbar.validated")
              : draftExists
                ? t("flow.toolbar.draft")
                : t("flow.toolbar.unsaved")}
        </Badge>
        <span>
          <strong>{name || t("flow.toolbar.untitled")}</strong>
          <small>
            {threadReady
              ? `${nodeCount} ${t("flow.toolbar.steps")} · ${flowId}`
              : t("flow.toolbar.selectTask")}
          </small>
        </span>
      </span>
      <div className="workflow-editor__actions">
        <Button
          disabled={!draftExists || Boolean(busy)}
          onClick={onRefresh}
          size="compact"
          variant="quiet"
        >
          <RefreshCw aria-hidden="true" size={14} />
          {t("flow.toolbar.refresh")}
        </Button>
        <Button
          disabled={nextAction.disabled || Boolean(busy) || activeTestRun}
          onClick={nextAction.onClick}
          size="compact"
          variant="primary"
        >
          <NextIcon aria-hidden="true" size={14} /> {nextAction.label}
        </Button>
      </div>
    </header>
  );
}

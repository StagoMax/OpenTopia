import {
  BadgeCheck,
  Bot,
  CheckCircle2,
  ChevronLeft,
  FileJson2,
  Gauge,
  GitBranch,
  Inbox,
  Link2,
  Merge,
  Repeat2,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Settings2,
  Wrench,
} from "lucide-react";
import type {
  AgentTemplateVersionView,
  FlowDraftView,
  FlowRun,
  FlowSpec,
} from "../../types";
import {
  Badge,
  Button,
  DisclosureSummary,
  SelectField,
  TextField,
} from "../ui";
import {
  workflowNodeLabel,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import { FlowConnectionConfiguration } from "./FlowConnectionConfiguration";
import type { WorkflowConnection } from "./workflowGraphOperations";
import type { WorkflowEdgeConfiguration } from "./workflowNodeSelection";
import { FlowNodeConfiguration } from "./FlowNodeConfiguration";
import { FlowNodeTestRunDetails } from "./FlowNodeTestRunDetails";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function FlowEditorInspector({
  draft,
  error,
  flowId,
  name,
  nodes,
  notice,
  onChangeFlow,
  onChangeNode,
  onChangeConnection,
  onEditTrigger,
  onChangeRuntimeConfiguration,
  onSelectConnection,
  onSelectNode,
  outcome,
  owner,
  runtimeConfiguration,
  selectedNodeId,
  selectedConnection,
  successfulTestRun,
  testRun,
  templates,
}: {
  draft: FlowDraftView | null;
  error: string | null;
  flowId: string;
  name: string;
  nodes: WorkflowNodeSelection[];
  notice: string | null;
  onChangeFlow(change: Partial<FlowConfiguration>): void;
  onChangeNode(node: WorkflowNodeSelection): void;
  onChangeConnection(
    connection: WorkflowConnection,
    configuration: WorkflowEdgeConfiguration,
  ): void;
  onChangeRuntimeConfiguration(configuration: FlowRuntimeConfiguration): void;
  onEditTrigger(nodeId: string): void;
  onSelectConnection(connection: WorkflowConnection | null): void;
  onSelectNode(nodeId: string | null): void;
  outcome: string;
  owner: string;
  runtimeConfiguration: FlowRuntimeConfiguration;
  selectedNodeId: string | null;
  selectedConnection: WorkflowConnection | null;
  successfulTestRun: boolean;
  testRun: FlowRun | null;
  templates: AgentTemplateVersionView[];
}) {
  const { t } = useApplicationLanguage();
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;

  return (
    <aside
      className="flow-editor-inspector"
      aria-label={t("flow.editor.configuration")}
    >
      <header className="flow-editor-inspector__header">
        <span className="flow-editor-inspector__title">
          {selectedConnection ? (
            <Link2 aria-hidden="true" size={16} />
          ) : selectedNode ? (
            <NodeIcon kind={selectedNode.kind} />
          ) : (
            <SlidersHorizontal aria-hidden="true" size={16} />
          )}
          <span>
            <strong>
              {selectedConnection
                ? t("flow.editor.connectionSettings")
                : selectedNode
                  ? t("flow.editor.nodeSettings")
                  : t("flow.editor.flowSettings")}
            </strong>
            <small>
              {selectedConnection
                ? `${selectedConnection.sourceId} → ${selectedConnection.targetId}`
                : selectedNode
                  ? workflowNodeLabel(selectedNode, templates)
                  : `${nodes.filter((node) => node.kind !== "output").length} ${t("flow.toolbar.steps")} · ${draft ? t("flow.editor.savedDraft") : t("flow.editor.unsaved")}`}
            </small>
          </span>
        </span>
        {selectedNode || selectedConnection ? (
          <Button
            onClick={() => {
              onSelectConnection(null);
              onSelectNode(null);
            }}
            size="compact"
            variant="quiet"
          >
            <ChevronLeft aria-hidden="true" size={14} />
            {t("flow.editor.backToFlow")}
          </Button>
        ) : null}
      </header>

      <div className="flow-editor-inspector__body">
        {selectedConnection ? (
          <FlowConnectionConfiguration
            connection={selectedConnection}
            nodes={nodes}
            onChange={(configuration) =>
              onChangeConnection(selectedConnection, configuration)
            }
            templates={templates}
          />
        ) : selectedNode ? (
          <>
            <FlowNodeConfiguration
              key={selectedNode.id}
              node={selectedNode}
              nodes={nodes}
              onChange={onChangeNode}
              onEditTrigger={onEditTrigger}
              templates={templates}
            />
            <FlowNodeTestRunDetails nodeId={selectedNode.id} run={testRun} />
          </>
        ) : (
          <>
            <section className="flow-editor-inspector__section">
              <header>
                <span>
                  <strong>{t("flow.editor.goal")}</strong>
                  <small>{t("flow.editor.goalHint")}</small>
                </span>
                <Badge variant={draft ? "neutral" : "warning"}>
                  {draft
                    ? t("flow.editor.saved")
                    : t("flow.editor.notSaved")}
                </Badge>
              </header>
              <TextField
                label={t("flow.editor.name")}
                onChange={(event) => onChangeFlow({ name: event.target.value })}
                value={name}
              />
              <label className="flow-editor-inspector__textarea">
                <span>{t("flow.editor.outcome")}</span>
                <textarea
                  onChange={(event) =>
                    onChangeFlow({ outcome: event.target.value })
                  }
                  rows={5}
                  value={outcome}
                />
              </label>
            </section>

            <section className="flow-editor-inspector__section">
              <header>
                <strong>{t("flow.editor.progress")}</strong>
              </header>
              <WorkflowProgress
                draft={draft}
                successfulTestRun={successfulTestRun}
              />
            </section>

            <details className="flow-editor-inspector__advanced">
              <DisclosureSummary
                icon={<Settings2 aria-hidden="true" size={14} />}
              >
                {t("flow.editor.advanced")}
              </DisclosureSummary>
              <div className="flow-editor-inspector__advanced-body">
                <TextField
                  hint={t("flow.editor.workflowIdHint")}
                  label={t("flow.editor.workflowId")}
                  onChange={(event) =>
                    onChangeFlow({ flowId: event.target.value })
                  }
                  value={flowId}
                />
                <TextField
                  label={t("flow.editor.owner")}
                  onChange={(event) =>
                    onChangeFlow({ owner: event.target.value })
                  }
                  value={owner}
                />
                <SelectField<FlowSpec["riskClass"]>
                  hint={t("flow.editor.riskHint")}
                  label={t("flow.editor.risk")}
                  onChange={(riskClass) =>
                    onChangeRuntimeConfiguration({
                      ...runtimeConfiguration,
                      riskClass,
                    })
                  }
                  options={[
                    { value: "low", label: t("flow.editor.riskLow") },
                    { value: "medium", label: t("flow.editor.riskMedium") },
                    { value: "high", label: t("flow.editor.riskHigh") },
                    {
                      value: "critical",
                      label: t("flow.editor.riskCritical"),
                    },
                  ]}
                  value={runtimeConfiguration.riskClass}
                />
                <fieldset className="flow-editor-budget">
                  <legend>
                    <Gauge aria-hidden="true" size={14} />{" "}
                    {t("flow.editor.budget")}
                  </legend>
                  <TextField
                    label={t("flow.editor.maxNodes")}
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxNodeExecutions: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxNodeExecutions,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxNodeExecutions}
                  />
                  <TextField
                    label={t("flow.editor.maxTools")}
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxToolCalls: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxToolCalls,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxToolCalls}
                  />
                  <TextField
                    hint={t("flow.editor.secondsHint")}
                    label={t("flow.editor.maxDuration")}
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxDurationSeconds: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxDurationSeconds,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxDurationSeconds}
                  />
                  <TextField
                    label={t("flow.editor.maxLoops")}
                    min={1}
                    onChange={(event) =>
                      onChangeRuntimeConfiguration({
                        ...runtimeConfiguration,
                        budget: {
                          ...runtimeConfiguration.budget,
                          maxLoopIterations: positiveInteger(
                            event.target.value,
                            runtimeConfiguration.budget.maxLoopIterations,
                          ),
                        },
                      })
                    }
                    type="number"
                    value={runtimeConfiguration.budget.maxLoopIterations}
                  />
                </fieldset>
                {draft ? (
                  <details className="flow-editor-inspector__json">
                    <DisclosureSummary
                      icon={<FileJson2 aria-hidden="true" size={14} />}
                    >
                      {t("flow.editor.draftJson")}
                    </DisclosureSummary>
                    <pre>{JSON.stringify(draft.draft.spec, null, 2)}</pre>
                  </details>
                ) : null}
              </div>
            </details>
          </>
        )}

        {error ? (
          <p className="enterprise-page__message is-error" role="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p className="enterprise-page__message is-success" role="status">
            {notice}
          </p>
        ) : null}
      </div>
    </aside>
  );
}

type FlowConfiguration = {
  flowId: string;
  name: string;
  owner: string;
  outcome: string;
};

export type FlowRuntimeConfiguration = {
  budget: FlowSpec["budget"];
  riskClass: FlowSpec["riskClass"];
};

function NodeIcon({ kind }: { kind: WorkflowNodeSelection["kind"] }) {
  const Icon =
    kind === "agent"
      ? Bot
      : kind === "skill"
        ? Sparkles
        : kind === "tool"
          ? Wrench
          : kind === "condition"
            ? GitBranch
            : kind === "validator"
              ? BadgeCheck
              : kind === "approval"
                ? ShieldCheck
                : kind === "join"
                  ? Merge
                  : kind === "loop"
                    ? Repeat2
                    : Inbox;
  return <Icon aria-hidden="true" size={16} />;
}

function positiveInteger(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function WorkflowProgress({
  draft,
  successfulTestRun,
}: {
  draft: FlowDraftView | null;
  successfulTestRun: boolean;
}) {
  const { t } = useApplicationLanguage();
  const steps = [
    [t("flow.editor.progressSave"), Boolean(draft)],
    [t("flow.editor.progressValidate"), Boolean(draft?.draft.lastValidation?.valid)],
    [t("flow.editor.progressTest"), successfulTestRun],
    [t("flow.editor.progressActivate"), draft?.draft.status === "published"],
  ] as const;
  return (
    <ol
      className="flow-editor-progress"
      aria-label={t("flow.editor.progressAria")}
    >
      {steps.map(([label, done]) => (
        <li className={done ? "is-done" : undefined} key={label}>
          <CheckCircle2 aria-hidden="true" size={14} />
          <span>{label}</span>
        </li>
      ))}
    </ol>
  );
}

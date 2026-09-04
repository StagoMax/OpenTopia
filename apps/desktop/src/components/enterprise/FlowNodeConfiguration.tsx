import { RadioTower, Settings2 } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import {
  Badge,
  Button,
  DisclosureSummary,
  SelectField,
  Switch,
  TextField,
} from "../ui";
import { AgentCapabilitySummary } from "./AgentCapabilitySummary";
import { activationLabel, templateKey } from "./flowActivation";
import { latestPublishedTemplateVersions } from "./model";
import type { WorkflowNodeSelection } from "./workflowNodeSelection";
import { WorkflowStateWritesEditor } from "./WorkflowStateWritesEditor";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function FlowNodeConfiguration({
  node,
  nodes,
  onChange,
  onEditTrigger,
  templates,
}: {
  node: WorkflowNodeSelection;
  nodes: WorkflowNodeSelection[];
  onChange(node: WorkflowNodeSelection): void;
  onEditTrigger(nodeId: string): void;
  templates: AgentTemplateVersionView[];
}) {
  const { language, t } = useApplicationLanguage();
  const selectedTemplate =
    node.kind === "agent"
      ? templates.find((item) => templateKey(item) === node.templateKey)
      : undefined;
  const agentTemplates = latestPublishedTemplateVersions(templates);
  const selectedAgentId = selectedTemplate?.template.templateId ?? "";
  const selectedAgentVersions = selectedTemplate
    ? templates
        .filter((item) => item.template.templateId === selectedAgentId)
        .sort((left, right) => right.template.version - left.template.version)
    : [];
  return (
    <>
      <section className="flow-editor-inspector__section">
        <header>
          <strong>{t("flow.node.basic")}</strong>
          <Badge variant={node.kind === "approval" ? "warning" : "neutral"}>
            {nodeKindLabel(node.kind, language)}
          </Badge>
        </header>
        {node.kind !== "agent" && node.kind !== "output" ? (
          <TextField
            label={t("flow.node.name")}
            onChange={(event) =>
              onChange({ ...node, label: event.target.value })
            }
            value={node.label}
          />
        ) : null}
        {node.kind === "agent" && agentTemplates.length > 0 ? (
          <>
            <SelectField
              hint={t("flow.node.agentHint")}
              label={t("flow.node.agent")}
              onChange={(templateId) => {
                const latest = agentTemplates.find(
                  (item) => item.template.templateId === templateId,
                );
                if (latest) {
                  onChange({ ...node, templateKey: templateKey(latest) });
                }
              }}
              options={[
                {
                  value: "",
                  label: t("flow.node.chooseAgent"),
                  disabled: true,
                },
                ...agentTemplates.map((item) => ({
                  value: item.template.templateId,
                  label: item.template.name,
                })),
              ]}
              required
              value={selectedAgentId}
            />
            {selectedTemplate ? (
              <SelectField
                hint={t("flow.node.versionHint")}
                label={t("flow.node.version")}
                onChange={(nextTemplateKey) =>
                  onChange({ ...node, templateKey: nextTemplateKey })
                }
                options={selectedAgentVersions.map((item, index) => ({
                  value: templateKey(item),
                  label: `${t("flow.node.version")} ${item.template.version}${index === 0 ? ` (${t("flow.node.latest")})` : ""}`,
                }))}
                value={node.templateKey}
              />
            ) : (
              <p className="flow-editor-inspector__note">
                {t("flow.node.chooseVersion")}
              </p>
            )}
          </>
        ) : null}
        {node.kind === "agent" && agentTemplates.length === 0 ? (
          <p className="flow-editor-inspector__note" role="status">
            {t("flow.node.noAgents")}
          </p>
        ) : null}
        {node.kind === "tool" ? (
          <p className="flow-editor-inspector__note">
            {t("flow.node.toolHint")}
          </p>
        ) : null}
        {node.kind === "skill" || node.kind === "tool" ? (
          <TextField
            hint={
              node.kind === "skill"
                ? t("flow.node.skillReferenceHint")
                : t("flow.node.toolReferenceHint")
            }
            label={
              node.kind === "skill"
                ? t("flow.node.skillReference")
                : t("flow.node.toolReference")
            }
            onChange={(event) =>
              onChange({ ...node, reference: event.target.value })
            }
            placeholder={node.kind === "skill" ? "skill-id" : "tool_name"}
            value={node.reference}
          />
        ) : null}
        {node.kind === "tool" ? (
          <label className="flow-editor-inspector__switch-row">
            <span>
              <strong>{t("flow.node.parallel")}</strong>
              <small>{t("flow.node.parallelHint")}</small>
            </span>
            <Switch
              checked={node.parallelSafe}
              label={t("flow.node.parallelLabel")}
              onChange={(parallelSafe) => onChange({ ...node, parallelSafe })}
            />
          </label>
        ) : null}
        {node.kind === "condition" ? (
          <>
            <p className="flow-editor-inspector__note">
              {t("flow.node.legacyCondition")}
            </p>
            <TextField
              hint={t("flow.node.conditionHint")}
              label={t("flow.node.condition")}
              onChange={(event) =>
                onChange({ ...node, expression: event.target.value })
              }
              placeholder="matched == true"
              value={node.expression}
            />
          </>
        ) : null}
        {node.kind === "validator" ? (
          <>
            <TextField
              hint={t("flow.node.requiredHint")}
              label={t("flow.node.required")}
              onChange={(event) =>
                onChange({
                  ...node,
                  requiredFields: commaSeparatedValues(event.target.value),
                })
              }
              placeholder="customer.id, amount"
              value={node.requiredFields.join(", ")}
            />
            <TextField
              hint={t("flow.node.expressionHint")}
              label={t("flow.node.expression")}
              onChange={(event) =>
                onChange({ ...node, expression: event.target.value })
              }
              placeholder="score != 0"
              value={node.expression}
            />
          </>
        ) : null}
        {node.kind === "approval" ? (
          <label className="flow-editor-inspector__textarea">
            <span>{t("flow.node.approval")}</span>
            <textarea
              onChange={(event) =>
                onChange({ ...node, instructions: event.target.value })
              }
              rows={5}
              value={node.instructions}
            />
          </label>
        ) : null}
        {node.kind === "join" ? (
          <p className="flow-editor-inspector__note">
            {t("flow.node.joinHint")}
          </p>
        ) : null}
        {node.kind === "loop" ? (
          <p className="flow-editor-inspector__note">
            {t("flow.node.loopHint")}
          </p>
        ) : null}
        {node.kind === "skill" ? (
          <p className="flow-editor-inspector__note">
            {t("flow.node.skillHint")}
          </p>
        ) : null}
        {node.kind === "output" ? (
          <>
            <TextField
              label={t("flow.node.name")}
              readOnly
              value={node.label}
            />
            <p className="flow-editor-inspector__note">
              {t("flow.node.outputHint")}
            </p>
          </>
        ) : null}
        <details className="flow-editor-inspector__advanced">
          <DisclosureSummary icon={<Settings2 aria-hidden="true" size={14} />}>
            {t("flow.node.advanced")}
          </DisclosureSummary>
          <div className="flow-editor-inspector__advanced-body">
            <TextField label={t("flow.node.id")} readOnly value={node.id} />
          </div>
        </details>
      </section>

      {node.kind === "agent" ? (
        <AgentCapabilitySummary template={selectedTemplate} />
      ) : null}

      <details className="flow-editor-inspector__advanced">
        <DisclosureSummary icon={<Settings2 aria-hidden="true" size={14} />}>
          {t("flow.node.sharedState")}
        </DisclosureSummary>
        <div className="flow-editor-inspector__advanced-body">
          <WorkflowStateWritesEditor
            onChange={(stateWrites) => onChange({ ...node, stateWrites })}
            writes={node.stateWrites ?? []}
          />
        </div>
      </details>

      <section className="flow-editor-inspector__section">
        <header>
          <strong>{t("flow.node.activation")}</strong>
        </header>
        <div className="flow-editor-inspector__activation">
          <RadioTower aria-hidden="true" size={14} />
          <span>
            <small>{t("flow.node.triggerSource")}</small>
            <strong>
              {activationLabel(node.activation, nodes, templates, language)}
            </strong>
          </span>
        </div>
        {node.kind !== "output" ? (
          <Button
            onClick={() => onEditTrigger(node.id)}
            size="compact"
            variant="secondary"
          >
            <RadioTower aria-hidden="true" size={14} />
            {t("flow.node.configureTrigger")}
          </Button>
        ) : null}
      </section>
    </>
  );
}

function nodeKindLabel(
  kind: WorkflowNodeSelection["kind"],
  language: ApplicationLanguage,
) {
  const key =
    kind === "tool"
      ? "flow.node.kind.action"
      : kind === "skill"
        ? "flow.node.kind.legacySkill"
        : kind === "condition"
          ? "flow.node.kind.legacyCondition"
          : kind === "loop"
            ? "flow.node.kind.legacyLoop"
            : (`flow.node.kind.${kind}` as const);
  return interfaceMessage(language, key);
}

function commaSeparatedValues(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

import { ArrowRight, GitBranch, Repeat2 } from "lucide-react";
import { Badge, NumberField, SelectField, TextField } from "../ui";
import type { WorkflowConnection } from "./workflowGraphOperations";
import {
  workflowNodeLabel,
  type WorkflowEdgeConfiguration,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import type { AgentTemplateVersionView, DataClassification } from "../../types";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function FlowConnectionConfiguration({
  connection,
  nodes,
  onChange,
  templates,
}: {
  connection: WorkflowConnection;
  nodes: WorkflowNodeSelection[];
  onChange(configuration: WorkflowEdgeConfiguration): void;
  templates: AgentTemplateVersionView[];
}) {
  const { language, t } = useApplicationLanguage();
  const source = nodes.find((node) => node.id === connection.sourceId);
  const target = nodes.find((node) => node.id === connection.targetId);
  const configuration = edgeConfiguration(connection);

  function change(change: Partial<WorkflowEdgeConfiguration>) {
    onChange({ ...configuration, ...change });
  }

  return (
    <>
      <section className="flow-editor-inspector__section">
        <header>
          <strong>{t("flow.connection.title")}</strong>
          <Badge variant={connection.loopPolicy ? "warning" : "neutral"}>
            {connection.loopPolicy
              ? t("flow.connection.feedback")
              : t("flow.connection.edge")}
          </Badge>
        </header>
        <div
          className="flow-connection-route"
          aria-label={t("flow.connection.routeAria")}
        >
          <span>
            {source
              ? workflowNodeLabel(source, templates)
              : connection.sourceId}
          </span>
          <ArrowRight aria-hidden="true" size={14} />
          <span>
            {target
              ? workflowNodeLabel(target, templates)
              : connection.targetId}
          </span>
        </div>
      </section>

      <section className="flow-editor-inspector__section">
        <header>
          <span>
            <strong>{t("flow.connection.conditionTitle")}</strong>
            <small>{t("flow.connection.conditionDescription")}</small>
          </span>
          <GitBranch aria-hidden="true" size={15} />
        </header>
        <TextField
          error={conditionError(configuration.condition, language)}
          hint={t("flow.connection.conditionHint")}
          label={t("flow.connection.condition")}
          onChange={(event) => change({ condition: event.target.value })}
          placeholder="passed == true"
          value={configuration.condition}
        />
        <TextField
          hint={t("flow.connection.fieldsHint")}
          label={t("flow.connection.fields")}
          onChange={(event) =>
            change({ allowedFields: commaSeparatedValues(event.target.value) })
          }
          placeholder="value, score"
          value={configuration.allowedFields.join(", ")}
        />
        <SelectField<DataClassification>
          label={t("flow.connection.classification")}
          onChange={(dataClassification) => change({ dataClassification })}
          options={[
            { value: "public", label: t("flow.connection.public") },
            { value: "internal", label: t("flow.connection.internal") },
            {
              value: "confidential",
              label: t("flow.connection.confidential"),
            },
            { value: "restricted", label: t("flow.connection.restricted") },
          ]}
          value={configuration.dataClassification}
        />
        <SelectField
          hint={t("flow.connection.errorRouteHint")}
          label={t("flow.connection.errorRoute")}
          onChange={(onError) => change({ onError: onError || null })}
          options={[
            { value: "", label: t("flow.connection.defaultTermination") },
            ...nodes
              .filter((node) => node.id !== connection.sourceId)
              .map((node) => ({
                value: node.id,
                label: workflowNodeLabel(node, templates),
              })),
          ]}
          value={configuration.onError ?? ""}
        />
      </section>

      {configuration.loopPolicy ? (
        <section className="flow-editor-inspector__section">
          <header>
            <span>
              <strong>{t("flow.connection.loop")}</strong>
              <small>{t("flow.connection.loopHint")}</small>
            </span>
            <Repeat2 aria-hidden="true" size={15} />
          </header>
          <div className="flow-loop-policy">
            <label>
              <span>{t("flow.connection.maxIterations")}</span>
              <NumberField
                label={t("flow.connection.maxIterations")}
                max={4}
                min={1}
                onChange={(maxIterations) =>
                  change({
                    loopPolicy: {
                      ...configuration.loopPolicy!,
                      maxIterations: Math.max(1, Math.min(4, maxIterations)),
                    },
                  })
                }
                value={configuration.loopPolicy.maxIterations}
              />
            </label>
            <TextField
              error={conditionError(
                configuration.loopPolicy.continueCondition,
                language,
              )}
              hint={t("flow.connection.continueHint")}
              label={t("flow.connection.continue")}
              onChange={(event) =>
                change({
                  loopPolicy: {
                    ...configuration.loopPolicy!,
                    continueCondition: event.target.value,
                  },
                })
              }
              placeholder="needsRetry == true"
              value={configuration.loopPolicy.continueCondition}
            />
            <SelectField
              label={t("flow.connection.exhausted")}
              onChange={(onExhausted) =>
                change({
                  loopPolicy: { ...configuration.loopPolicy!, onExhausted },
                })
              }
              options={[
                {
                  value: "require_human",
                  label: t("flow.connection.requireHuman"),
                },
                {
                  value: "return_partial",
                  label: t("flow.connection.returnPartial"),
                },
                { value: "fail", label: t("flow.connection.fail") },
              ]}
              value={configuration.loopPolicy.onExhausted}
            />
            <p className="flow-editor-inspector__note">
              <Repeat2 aria-hidden="true" size={13} />{" "}
              {t("flow.connection.globalBudget")}
            </p>
          </div>
        </section>
      ) : null}
    </>
  );
}

function edgeConfiguration(
  connection: WorkflowConnection,
): WorkflowEdgeConfiguration {
  return {
    allowedFields: connection.allowedFields,
    condition: connection.condition,
    dataClassification: connection.dataClassification,
    loopPolicy: connection.loopPolicy,
    onError: connection.onError,
  };
}

function commaSeparatedValues(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function conditionError(value: string, language: ApplicationLanguage) {
  if (!value.trim()) return undefined;
  if (value.length > 512)
    return interfaceMessage(language, "flow.connection.tooLong");
  if (
    [";", "{", "}", "=>", "function", "import", "eval", "exec("].some((token) =>
      value.includes(token),
    )
  ) {
    return interfaceMessage(language, "flow.connection.invalidCode");
  }
  return undefined;
}

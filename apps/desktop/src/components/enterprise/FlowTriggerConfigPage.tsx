import { AlertCircle, Braces, Plus, RadioTower, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import type { AgentTemplateVersionView } from "../../types";
import { Button, IconButton, SelectField, Switch, TextField } from "../ui";
import {
  activationFromEditableInputs,
  editableTriggerInputs,
  templateKey,
  type EditableTriggerInput,
  type FlowNodeActivation,
  type FlowTriggerSource,
} from "./flowActivation";
import {
  workflowNodeLabel,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function FlowTriggerConfigPage({
  node,
  onChange,
  selections,
  templates,
}: {
  node: WorkflowNodeSelection;
  onChange(activation: FlowNodeActivation): void;
  selections: WorkflowNodeSelection[];
  templates: AgentTemplateVersionView[];
}) {
  const { t } = useApplicationLanguage();
  const editable = useMemo(
    () => editableTriggerInputs(node.activation),
    [node.activation],
  );
  const [logic, setLogic] = useState<"and" | "or">(editable.logic);
  const [inputs, setInputs] = useState(editable.inputs);
  const [ingressPolicy, setIngressPolicy] = useState(
    node.activation.ingressPolicy,
  );
  const template =
    node.kind === "agent"
      ? templates.find((item) => templateKey(item) === node.templateKey)
      : null;
  const sourceOptions = [
    { value: "manual", label: t("flow.trigger.manualSource") },
    { value: "webhook", label: t("flow.trigger.webhookSource") },
    {
      value: "event_subscription",
      label: t("flow.trigger.connectionEvent"),
    },
    { value: "schedule", label: t("flow.trigger.scheduleSource") },
    {
      value: "flow_input",
      label: `Flow.input / ${t("flow.trigger.currentEvent")}`,
    },
    ...selections
      .filter((item) => item.id !== node.id)
      .map((item) => ({
        value: `agent_final:${item.id}`,
        label: `${workflowNodeLabel(item, templates)}.Final`,
      })),
  ];

  function updateInputs(next: EditableTriggerInput[]) {
    setInputs(next);
    if (next.length > 0) {
      onChange(activationFromEditableInputs(next, logic, ingressPolicy));
    }
  }

  function updateLogic(next: "and" | "or") {
    setLogic(next);
    onChange(activationFromEditableInputs(inputs, next, ingressPolicy));
  }

  function updateIngressPolicy(next: typeof ingressPolicy) {
    setIngressPolicy(next);
    onChange(activationFromEditableInputs(inputs, logic, next));
  }

  function replaceSource(index: number, source: FlowTriggerSource) {
    updateInputs(
      inputs.map((item, current) =>
        current === index ? { ...item, source } : item,
      ),
    );
  }

  return (
    <div className="flow-trigger-page">
      <header className="flow-trigger-page__intro">
        <span className="flow-trigger-page__icon">
          <RadioTower aria-hidden="true" size={18} />
        </span>
        <span>
          <strong>
            {template?.template.name ?? workflowNodeLabel(node, templates)} ·
            {t("flow.trigger.title")}
          </strong>
          <small>{t("flow.trigger.description")}</small>
        </span>
      </header>

      <section className="flow-trigger-page__section">
        <header>
          <span>
            <strong>{t("flow.trigger.activationExpression")}</strong>
            <small>{t("flow.trigger.activationHint")}</small>
          </span>
          <SelectField
            disabled={inputs.length < 2}
            label={t("flow.trigger.logic")}
            onChange={(value) => updateLogic(value as "and" | "or")}
            options={[
              { value: "and", label: t("flow.trigger.all") },
              { value: "or", label: t("flow.trigger.any") },
            ]}
            value={logic}
          />
        </header>
        <ol className="flow-trigger-source-list">
          {inputs.map((item, index) => (
            <li key={item.id}>
              <div className="flow-trigger-source-list__heading">
                <Braces aria-hidden="true" size={14} />
                <SelectField
                  label={`${t("flow.trigger.source")} ${index + 1}`}
                  onChange={(value) =>
                    replaceSource(index, sourceFromOption(value))
                  }
                  options={sourceOptions}
                  value={sourceOption(item.source)}
                />
                <label className="flow-trigger-source-list__not">
                  <span>NOT</span>
                  <Switch
                    checked={item.negated}
                    onChange={(negated) =>
                      updateInputs(
                        inputs.map((current, currentIndex) =>
                          currentIndex === index
                            ? { ...current, negated }
                            : current,
                        ),
                      )
                    }
                  />
                </label>
                <IconButton
                  aria-label={`${t("flow.trigger.removeSource")} ${index + 1}`}
                  disabled={inputs.length === 1}
                  onClick={() =>
                    updateInputs(
                      inputs.filter((_, current) => current !== index),
                    )
                  }
                  size="compact"
                  variant="danger"
                >
                  <Trash2 aria-hidden="true" size={14} />
                </IconButton>
              </div>
              <TriggerSourceFields
                onChange={(source) => replaceSource(index, source)}
                source={item.source}
              />
            </li>
          ))}
        </ol>
        <Button
          onClick={() =>
            updateInputs([
              ...inputs,
              {
                id: crypto.randomUUID(),
                source: { kind: "flow_input" },
                negated: false,
              },
            ])
          }
          size="compact"
          variant="quiet"
        >
          <Plus aria-hidden="true" size={14} />
          {t("flow.trigger.addSource")}
        </Button>
      </section>

      <section className="flow-trigger-page__section flow-trigger-page__policy">
        <span>
          <strong>{t("flow.trigger.eventHandling")}</strong>
          <small>{t("flow.trigger.eventHandlingHint")}</small>
        </span>
        <SelectField
          label={t("flow.trigger.reviewPolicy")}
          onChange={(value) =>
            updateIngressPolicy(value as typeof ingressPolicy)
          }
          options={[
            {
              value: "require_review",
              label: t("flow.trigger.reviewFirst"),
            },
            { value: "immediate", label: t("flow.trigger.immediate") },
          ]}
          value={ingressPolicy}
        />
      </section>

      <aside className="flow-trigger-page__note">
        <AlertCircle aria-hidden="true" size={15} />
        <span>{t("flow.trigger.schemaHint")}</span>
      </aside>
    </div>
  );
}

function TriggerSourceFields({
  onChange,
  source,
}: {
  onChange(source: FlowTriggerSource): void;
  source: FlowTriggerSource;
}) {
  const { t } = useApplicationLanguage();
  if (source.kind === "webhook") {
    return (
      <div className="flow-trigger-source-list__fields">
        <TextField
          hint={t("flow.trigger.tokenHint")}
          label={t("flow.trigger.tokenRef")}
          onChange={(event) =>
            onChange({ ...source, tokenRef: event.target.value })
          }
          value={source.tokenRef}
        />
        <TextField
          label={t("flow.trigger.id")}
          readOnly
          value={source.triggerId}
        />
      </div>
    );
  }
  if (source.kind === "event_subscription") {
    return (
      <div className="flow-trigger-source-list__fields">
        <TextField
          label={t("flow.trigger.connectionSource")}
          onChange={(event) =>
            onChange({ ...source, source: event.target.value })
          }
          value={source.source}
        />
        <TextField
          label={t("flow.trigger.eventType")}
          onChange={(event) =>
            onChange({ ...source, eventType: event.target.value })
          }
          value={source.eventType}
        />
      </div>
    );
  }
  if (source.kind === "schedule") {
    return (
      <div className="flow-trigger-source-list__fields">
        <TextField
          label={t("flow.trigger.interval")}
          onChange={(event) =>
            onChange({
              ...source,
              intervalSeconds: Number(event.target.value) || 60,
            })
          }
          value={String(source.intervalSeconds)}
        />
        <TextField
          label={t("flow.trigger.nextFire")}
          onChange={(event) =>
            onChange({ ...source, nextFireAt: event.target.value })
          }
          value={source.nextFireAt}
        />
      </div>
    );
  }
  return null;
}

function sourceOption(source: FlowTriggerSource): string {
  return source.kind === "agent_final"
    ? `agent_final:${source.nodeId}`
    : source.kind;
}

function sourceFromOption(value: string): FlowTriggerSource {
  if (value.startsWith("agent_final:")) {
    return { kind: "agent_final", nodeId: value.slice("agent_final:".length) };
  }
  if (value === "webhook") {
    return {
      kind: "webhook",
      triggerId: crypto.randomUUID(),
      tokenRef: "env:WORKFLOW_TRIGGER_TOKEN",
    };
  }
  if (value === "schedule") {
    return {
      kind: "schedule",
      triggerId: crypto.randomUUID(),
      intervalSeconds: 3600,
      nextFireAt: new Date(Date.now() + 60_000).toISOString(),
    };
  }
  if (value === "event_subscription") {
    return {
      kind: "event_subscription",
      triggerId: crypto.randomUUID(),
      source: "connection",
      eventType: "record.updated",
    };
  }
  if (value === "manual") return { kind: "manual" };
  return { kind: "flow_input" };
}

import { AlertCircle, Braces, Plus, RadioTower, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import type { AgentTemplateVersionView } from "../../types";
import { Button, IconButton, SelectField, Switch, TextField } from "../ui";
import {
  activationFromEditableInputs,
  editableTriggerInputs,
  templateKey,
  type EditableTriggerInput,
  type FlowTriggerSource,
  type WorkflowAgentSelection,
} from "./flowActivation";

export function FlowTriggerConfigPage({
  node,
  onChange,
  selections,
  templates,
}: {
  node: WorkflowAgentSelection;
  onChange(activation: WorkflowAgentSelection["activation"]): void;
  selections: WorkflowAgentSelection[];
  templates: AgentTemplateVersionView[];
}) {
  const editable = useMemo(
    () => editableTriggerInputs(node.activation),
    [node.activation],
  );
  const [logic, setLogic] = useState<"and" | "or">(editable.logic);
  const [inputs, setInputs] = useState(editable.inputs);
  const [ingressPolicy, setIngressPolicy] = useState(
    node.activation.ingressPolicy,
  );
  const template = templates.find(
    (item) => templateKey(item) === node.templateKey,
  );
  const sourceOptions = [
    { value: "manual", label: "Manual / 手动" },
    { value: "webhook", label: "API / Webhook" },
    { value: "event_subscription", label: "Connection event / 连接事件" },
    { value: "schedule", label: "Schedule / 定时" },
    { value: "flow_input", label: "Flow.input / 当前事件" },
    ...selections
      .filter((item) => item.id !== node.id)
      .map((item) => {
        const itemTemplate = templates.find(
          (candidate) => templateKey(candidate) === item.templateKey,
        );
        return {
          value: `agent_final:${item.id}`,
          label: `${itemTemplate?.template.name ?? item.id}.Final`,
        };
      }),
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
          <strong>{template?.template.name ?? "Agent"} · Trigger</strong>
          <small>
            Trigger 决定这个 Agent 何时被激活；Agent Final
            是完成通知订阅，不是新造的运行事件。
          </small>
        </span>
      </header>

      <section className="flow-trigger-page__section">
        <header>
          <span>
            <strong>Activation expression / 激活表达式</strong>
            <small>多个来源可以使用 AND、OR，并可对单个来源取 NOT。</small>
          </span>
          <SelectField
            disabled={inputs.length < 2}
            label="Logic / 逻辑"
            onChange={(value) => updateLogic(value as "and" | "or")}
            options={[
              { value: "and", label: "AND / 全部满足" },
              { value: "or", label: "OR / 任一满足" },
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
                  label={`Source ${index + 1} / 来源`}
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
                  aria-label={`移除 Trigger 来源 ${index + 1}`}
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
          <Plus aria-hidden="true" size={14} /> 添加来源
        </Button>
      </section>

      <section className="flow-trigger-page__section flow-trigger-page__policy">
        <span>
          <strong>Event handling / 事件处理策略</strong>
          <small>
            外部事件可以自动运行，也可以先进入 Inbox，人工确认后再创建 FlowRun。
          </small>
        </span>
        <SelectField
          label="Review policy / 审核策略"
          onChange={(value) =>
            updateIngressPolicy(value as typeof ingressPolicy)
          }
          options={[
            {
              value: "require_review",
              label: "人工确认后执行（推荐）",
            },
            { value: "immediate", label: "通过检查后自动执行" },
          ]}
          value={ingressPolicy}
        />
      </section>

      <aside className="flow-trigger-page__note">
        <AlertCircle aria-hidden="true" size={15} />
        <span>
          Trigger 不要求统一事件 Schema。来源原始参数会保存在{" "}
          <code>@Flow.input</code>
          ；当前节点收到的激活数据通过 <code>@Trigger.input</code>{" "}
          引用。参数只有 ID 时，Agent 再调用 Connection 工具读取完整业务记录。
        </span>
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
  if (source.kind === "webhook") {
    return (
      <div className="flow-trigger-source-list__fields">
        <TextField
          hint="只保存凭据引用，不保存 Token"
          label="Token ref / Token 引用"
          onChange={(event) =>
            onChange({ ...source, tokenRef: event.target.value })
          }
          value={source.tokenRef}
        />
        <TextField label="Trigger ID" readOnly value={source.triggerId} />
      </div>
    );
  }
  if (source.kind === "event_subscription") {
    return (
      <div className="flow-trigger-source-list__fields">
        <TextField
          label="Connection source / 事件源"
          onChange={(event) =>
            onChange({ ...source, source: event.target.value })
          }
          value={source.source}
        />
        <TextField
          label="Event type / 事件类型"
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
          label="Interval seconds / 间隔秒数"
          onChange={(event) =>
            onChange({
              ...source,
              intervalSeconds: Number(event.target.value) || 60,
            })
          }
          value={String(source.intervalSeconds)}
        />
        <TextField
          label="Next fire at / 下次触发"
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

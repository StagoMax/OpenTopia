import { ArrowDown, ArrowUp, Bot, Plus, Trash2 } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { Button, IconButton, SelectField } from "../ui";

export type WorkflowAgentSelection = {
  id: string;
  templateKey: string;
};

export function WorkflowAgentSequenceEditor({
  disabled,
  onChange,
  selections,
  templates,
}: {
  disabled?: boolean;
  onChange(selections: WorkflowAgentSelection[]): void;
  selections: WorkflowAgentSelection[];
  templates: AgentTemplateVersionView[];
}) {
  const options = templates.map((item) => ({
    value: templateKey(item),
    label: `${item.template.name} · ${item.template.templateId}@${item.template.version}`,
  }));

  function replace(index: number, value: string) {
    onChange(
      selections.map((selection, current) =>
        current === index ? { ...selection, templateKey: value } : selection,
      ),
    );
  }

  function move(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= selections.length) return;
    const next = [...selections];
    [next[index], next[target]] = [next[target]!, next[index]!];
    onChange(next);
  }

  return (
    <fieldset className="enterprise-agent-sequence enterprise-field--wide">
      <legend>Agent sequence / Agent 执行顺序</legend>
      <p>
        每个节点固定一个已发布模板版本；上一个 Agent 的输出会传给下一个节点。
      </p>
      <ol>
        {selections.map((selection, index) => (
          <li key={selection.id}>
            <span className="enterprise-agent-sequence__index">
              <Bot aria-hidden="true" size={15} />
              {index + 1}
            </span>
            <SelectField
              disabled={disabled}
              label={`Agent ${index + 1}`}
              onChange={(value) => replace(index, value)}
              options={options}
              value={selection.templateKey}
            />
            <span className="enterprise-agent-sequence__actions">
              <IconButton
                aria-label={`上移 Agent ${index + 1}`}
                disabled={disabled || index === 0}
                onClick={() => move(index, -1)}
                size="compact"
              >
                <ArrowUp aria-hidden="true" size={14} />
              </IconButton>
              <IconButton
                aria-label={`下移 Agent ${index + 1}`}
                disabled={disabled || index === selections.length - 1}
                onClick={() => move(index, 1)}
                size="compact"
              >
                <ArrowDown aria-hidden="true" size={14} />
              </IconButton>
              <IconButton
                aria-label={`移除 Agent ${index + 1}`}
                disabled={disabled || selections.length === 1}
                onClick={() =>
                  onChange(selections.filter((_, current) => current !== index))
                }
                size="compact"
                variant="danger"
              >
                <Trash2 aria-hidden="true" size={14} />
              </IconButton>
            </span>
          </li>
        ))}
      </ol>
      <Button
        disabled={disabled || options.length === 0}
        onClick={() =>
          onChange([
            ...selections,
            {
              id: crypto.randomUUID(),
              templateKey: options[0]?.value ?? "",
            },
          ])
        }
        size="compact"
        variant="quiet"
      >
        <Plus aria-hidden="true" size={14} /> 添加 Agent 节点
      </Button>
    </fieldset>
  );
}

function templateKey(item: AgentTemplateVersionView): string {
  return `${item.template.templateId}@${item.template.version}`;
}

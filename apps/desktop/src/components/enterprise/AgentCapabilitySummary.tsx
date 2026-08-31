import { Sparkles, Wrench } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { Badge } from "../ui";

export function AgentCapabilitySummary({
  template,
}: {
  template: AgentTemplateVersionView | undefined;
}) {
  const capabilities = template?.template.spec?.capabilities;
  if (!capabilities) {
    return (
      <section className="flow-editor-inspector__section">
        <header>
          <strong>Agent 能力</strong>
        </header>
        <p className="flow-editor-inspector__note">
          选择一个已发布 Agent 后，这里会显示它可以使用的 Skill 和 Tool。
        </p>
      </section>
    );
  }

  return (
    <section className="flow-editor-inspector__section">
      <header>
        <span>
          <strong>Agent 能力</strong>
          <small>来自已发布模板，在 Flow 中只读</small>
        </span>
        <Badge variant="neutral">Template</Badge>
      </header>
      <dl className="flow-agent-capabilities">
        <CapabilityRow
          all={capabilities.allowAllSkills}
          emptyLabel="未配置 Skill"
          icon={Sparkles}
          label="Skills"
          values={capabilities.skills}
        />
        <CapabilityRow
          all={capabilities.allowAllTools}
          emptyLabel="未配置 Tool"
          icon={Wrench}
          label="Tools"
          values={capabilities.tools}
        />
      </dl>
      <p className="flow-editor-inspector__note">
        如需增减能力，请在 Agents 中创建并发布新的模板版本，再回到这里选择。
      </p>
    </section>
  );
}

function CapabilityRow({
  all,
  emptyLabel,
  icon: Icon,
  label,
  values,
}: {
  all: boolean;
  emptyLabel: string;
  icon: typeof Sparkles;
  label: string;
  values: string[];
}) {
  return (
    <div>
      <dt>
        <Icon aria-hidden="true" size={14} />
        {label}
      </dt>
      <dd>
        {all
          ? "全部可见能力"
          : values.length > 0
            ? values.join(", ")
            : emptyLabel}
      </dd>
    </div>
  );
}

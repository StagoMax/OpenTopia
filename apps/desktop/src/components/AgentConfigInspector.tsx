import {
  Bot,
  Cable,
  Database,
  FileJson2,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import { Badge } from "./ui";
import "../styles/agent-studio.css";

export type AgentConfigPreview = {
  name: string;
  instructions: string;
  connectionCount: number;
  knowledge: string;
  riskClass: "low" | "medium" | "high" | "critical";
  tools: string[];
  outputSchema: string;
};

export function AgentConfigInspector({
  generating,
  preview,
}: {
  generating: boolean;
  preview: AgentConfigPreview;
}) {
  return (
    <aside className="agent-config-inspector" aria-label="Agent 实时配置">
      <header>
        <span>
          <Bot aria-hidden="true" size={16} />
          <strong>Live configuration / 实时配置</strong>
        </span>
        <Badge variant={generating ? "warning" : "neutral"}>
          {generating ? "模型生成中" : "Draft"}
        </Badge>
      </header>
      <section>
        <strong>{preview.name || "Untitled Agent"}</strong>
        <p>
          {preview.instructions ||
            "在左侧描述 Agent 需求，模型生成后这里会立即显示可审核配置。"}
        </p>
      </section>
      <dl>
        <InspectorRow
          icon={Cable}
          label="Connections"
          value={`${preview.connectionCount} 个绑定`}
        />
        <InspectorRow
          icon={Database}
          label="Knowledge"
          value={preview.knowledge}
        />
        <InspectorRow
          icon={Wrench}
          label="Tools"
          value={preview.tools.join(", ") || "未授权"}
        />
        <InspectorRow
          icon={ShieldCheck}
          label="Permissions"
          value={`${preview.riskClass} risk`}
        />
        <InspectorRow
          icon={FileJson2}
          label="Final schema"
          value={schemaLabel(preview.outputSchema)}
        />
      </dl>
      <footer>
        模型只能从已配置的 Connection、Knowledge 和当前 ExecutionContext
        中选择能力；发布后版本与权限会被冻结。
      </footer>
    </aside>
  );
}

function InspectorRow({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Cable;
  label: string;
  value: string;
}) {
  return (
    <div>
      <dt>
        <Icon aria-hidden="true" size={14} /> {label}
      </dt>
      <dd>{value}</dd>
    </div>
  );
}

function schemaLabel(value: string): string {
  try {
    const schema = JSON.parse(value) as { type?: string; properties?: object };
    const propertyCount = Object.keys(schema.properties ?? {}).length;
    return `${schema.type ?? "schema"}${propertyCount ? ` · ${propertyCount} fields` : ""}`;
  } catch {
    return "Schema 待修正";
  }
}

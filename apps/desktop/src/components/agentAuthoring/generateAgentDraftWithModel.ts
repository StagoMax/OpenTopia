import type { ApiClient } from "../../api/client";
import type { AgentTemplateVersionView, AppSettings } from "../../types";

type GenerateAgentDraftInput = {
  client: ApiClient;
  threadId: string;
  requirement: string;
  existingTemplates: AgentTemplateVersionView[];
  settings: AppSettings | null;
};

/**
 * Lets the existing Agent Loop author one draft through the same governed tools
 * available to a normal Flow-mode conversation. The UI only coordinates the
 * turn and observes the resulting persisted draft.
 */
export async function generateAgentDraftWithModel({
  client,
  threadId,
  requirement,
  existingTemplates,
  settings,
}: GenerateAgentDraftInput): Promise<AgentTemplateVersionView> {
  const baseline = new Set(existingTemplates.map(templateKey));
  const startedAt = Date.now();
  const [connections, sagSources, libraryProviders] = await Promise.all([
    client.listConnections(),
    client.listSagSources().catch(() => []),
    client.listLibraryProviders().catch(() => []),
  ]);
  const connectionCatalog = await Promise.all(
    connections
      .filter(
        (connection) =>
          connection.enabled &&
          connection.status === "ready" &&
          connection.activeCapabilityRevision,
      )
      .map(async (connection) => {
        const revision = await client.getConnectionCapabilityRevision(
          connection.id,
          connection.activeCapabilityRevision!,
        );
        return {
          connectionId: connection.id,
          name: connection.name,
          capabilityRevision: revision.revision,
          operations: revision.capabilities.map((capability) => ({
            operationId: capability.capabilityId,
            name: capability.name,
            description: capability.description,
          })),
        };
      }),
  );
  const namespaces = [...new Set(sagSources.map((source) => source.namespace))];
  const activeProvider = settings?.providers.find(
    (provider) => provider.id === settings.activeProviderId,
  );
  const response = await client.sendMessage(
    threadId,
    [
      "为当前用户创建一个单 Agent 配置。先调用 agent_search 避免重复，然后调用 agent_create 恰好一次创建草稿；不要创建 Flow，不要发布，也不要创建多 Agent。",
      "根据需求生成完整 instructions：明确角色、目标、任意来源参数的处理方式、Connection 工具使用策略、@Flow.input/@Trigger.input 引用策略和期望 Final JSON。只能选择下面已配置好的 Connection、知识库 provider、SAG namespace 和当前模型；不能扩大权限。选择 knowledgeProvider 后，系统会自动派生 library_search 权限，不要要求用户去 Flow Revision 重复配置。",
      `用户需求：\n${requirement.trim()}`,
      `可用 Connection：\n${JSON.stringify(connectionCatalog, null, 2)}`,
      `可用知识库 provider：\n${JSON.stringify(libraryProviders, null, 2)}`,
      `可用 SAG namespaces：\n${JSON.stringify(namespaces)}`,
      `当前模型：${activeProvider ? `${activeProvider.id}:${activeProvider.model}` : "未指定；modelPolicy 保持 deny-all"}`,
      "创建完成后用一句话说明草稿名称和仍需用户审核的配置。",
    ].join("\n\n"),
  );

  let assistantNote = "";
  for (let attempt = 0; attempt < 120; attempt += 1) {
    await wait(1_000);
    const nextTemplates = await client.listAgentTemplates();
    const generated =
      nextTemplates
        .filter(
          (view) =>
            !baseline.has(templateKey(view)) &&
            new Date(view.template.createdAt).getTime() >= startedAt - 2_000,
        )
        .sort((left, right) =>
          right.template.createdAt.localeCompare(left.template.createdAt),
        )[0] ?? null;
    if (generated) return generated;

    const status = await client.getTurnStatus(threadId);
    const isRequestedTurn =
      !response.turnId || status?.turnId === response.turnId;
    if (
      isRequestedTurn &&
      status &&
      ["succeeded", "failed", "cancelled", "interrupted"].includes(
        status.status,
      )
    ) {
      const messages = await client.listMessages(threadId, undefined, {
        limit: 12,
      });
      assistantNote = latestAssistantText(messages);
      break;
    }
  }

  throw new Error(
    assistantNote ||
      "模型没有创建 Agent 草稿。请查看当前 Flow 会话中的说明或审批请求。",
  );
}

function templateKey(view: AgentTemplateVersionView): string {
  return `${view.template.templateId}@${view.template.version}`;
}

function latestAssistantText(
  messages: Array<{
    role: string;
    parts: Array<{ type: string; text?: string; message?: string }>;
  }>,
): string {
  const message = messages.findLast((item) => item.role === "assistant");
  return (
    message?.parts
      .map((part) => part.text ?? part.message ?? "")
      .filter(Boolean)
      .join("\n") ?? ""
  );
}

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}

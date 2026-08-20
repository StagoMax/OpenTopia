import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import type { TokenEstimateBreakdown, TokenEstimateDetail } from "../types";
import { legacyTokenBreakdownDetails } from "../usageTokenBreakdown";
import type { ModelBreakdownGroup } from "../usageTokenBreakdownModels";
import { Badge } from "./ui";

type VisibleDetail = {
  detail: TokenEstimateDetail;
  depth: number;
  path: string;
};

const DEFAULT_EXPANDED = new Set([
  "base_instructions",
  "developer_instructions",
  "repository_instructions",
  "conversation",
  "conversation/system_messages",
  "conversation/developer_messages",
  "conversation/user_messages",
  "conversation/assistant_messages",
  "conversation/tool_messages",
  "current_user",
  "tool_calls",
  "tool_results",
  "tool_schemas",
  "tool_schemas/direct_tool_schemas",
  "tool_schemas/deferred_tool_catalog",
  "tool_schemas/loaded_tool_schemas",
  "turn_assistant_state",
  "provider_state",
]);

const LABELS: Record<string, string> = {
  base_instructions: "基础提示词",
  developer_instructions: "开发者提示词",
  repository_instructions: "仓库指令",
  runtime_context: "运行时上下文",
  skill_instructions: "Skills 指令",
  summaries: "上下文摘要",
  checkpoints: "检查点",
  conversation: "会话历史",
  current_user: "当前用户输入",
  tool_calls: "工具调用",
  tool_results: "工具结果",
  tool_schemas: "工具能力面（请求 tools 字段）",
  direct_tool_schemas: "直接加载的工具 Schema",
  deferred_tool_catalog: "延迟工具目录",
  loaded_tool_schemas: "Tool Search 后加载的 Schema",
  output_schema: "结构化输出 Schema",
  structured_output_schema: "响应格式定义",
  turn_assistant_state: "当前 Turn 助手回放",
  provider_state: "Provider 不透明续接状态",
  other: "其他",
  identity_and_objective: "身份与目标",
  instruction_hierarchy: "指令优先级",
  request_interpretation: "请求理解",
  workspace_discipline: "工作区纪律",
  codebase_exploration: "代码库探索",
  git_safety: "Git 安全",
  skills: "Skills 加载规则",
  tool_loop: "工具调用循环",
  validation: "验证要求",
  communication: "沟通方式",
  completion: "完成条件",
  system_messages: "System 历史消息",
  developer_messages: "Developer 历史消息",
  user_messages: "用户历史消息",
  assistant_messages: "助手历史输出",
  tool_messages: "工具历史消息",
  message_text: "消息文本",
  attached_content: "附件与结构化内容",
  typed_content: "附件与结构化内容",
  content_text: "附加文本内容",
  content_json: "JSON 内容",
  content_image: "图像输入",
  content_resource: "资源引用",
  historical_tool_calls: "历史助手工具调用",
  historical_tool_results: "历史工具结果",
  call_identity: "调用 ID 与工具名",
  call_arguments: "调用参数",
  result_output: "结果正文",
  result_metadata: "结果元数据与协议字段",
  protocol_framing: "协议封装",
  message: "Assistant message item",
  reasoning: "加密推理续接项",
  compaction: "Provider 压缩项",
  openai_chat_assistant_state: "Chat 助手推理与调用关联",
  legacy_unattributed: "旧日志中无法继续拆分",
  legacy_unclassified_provider_state: "旧日志未记录 Provider 子类型",
};

const ROOT_SCOPES: Record<string, string> = {
  base_instructions: "本次请求 · 每个模型请求均发送",
  developer_instructions: "本次请求 · 每个模型请求均发送",
  repository_instructions: "本次请求 · 每个模型请求均发送",
  runtime_context: "本次请求 · 按生命周期装配",
  skill_instructions: "本次请求 · 仅已加载 Skills",
  summaries: "当前历史快照 · 每个请求读取",
  checkpoints: "当前历史快照 · 每个请求读取",
  conversation: "当前 Turn 之前的历史 · 每个请求重复读取",
  current_user: "当前 Turn 发起消息 · 每个 Round 重复读取",
  tool_calls: "当前 Turn · 截至本请求发送前的累计调用",
  tool_results: "当前 Turn · 截至本请求发送前的累计结果",
  tool_schemas: "本次请求的 tools 字段",
  output_schema: "本次请求的响应格式字段",
  turn_assistant_state: "当前 Turn · Provider 原生助手输出回放",
  provider_state: "本次请求 · 仅不可由普通历史重建的续接项",
  other: "本次请求 · 未归入已知模块",
};

export function UsageTokenBreakdownModelCard({
  group,
}: {
  group: ModelBreakdownGroup;
}) {
  const usageIsComplete = group.reportedCallCount === group.calls.length;
  const difference = usageIsComplete
    ? group.actualInputTokens - group.breakdown.total
    : null;

  return (
    <article className="usage-model-breakdown-card">
      <header className="usage-model-breakdown-header">
        <div className="usage-model-breakdown-title">
          <span>模型</span>
          <strong title={group.model}>{group.model}</strong>
        </div>
        <div className="usage-model-breakdown-badges">
          {group.providerId ? (
            <Badge variant="neutral">{group.providerId}</Badge>
          ) : null}
          <Badge variant="neutral">{group.calls.length} 次请求</Badge>
        </div>
      </header>

      <dl className="usage-model-breakdown-metrics">
        <div>
          <dt>Provider 实际输入</dt>
          <dd>
            {group.reportedCallCount > 0
              ? formatInteger(group.actualInputTokens)
              : "—"}
          </dd>
          <small>
            usage 覆盖 {group.reportedCallCount} / {group.calls.length}
          </small>
        </div>
        <div>
          <dt>本地构成归因</dt>
          <dd>{formatInteger(group.breakdown.total)}</dd>
          <small>该模型的请求累计</small>
        </div>
        <div>
          <dt>完整覆盖时差值</dt>
          <dd>{difference === null ? "—" : formatSigned(difference)}</dd>
          <small>Provider 实际值 − 本地估算</small>
        </div>
      </dl>

      <details className="usage-model-breakdown-details" open>
        <summary>查看该模型的模块构成</summary>
        <UsageTokenBreakdownTable breakdown={group.breakdown} />
      </details>
    </article>
  );
}

export function UsageTokenBreakdownTable({
  breakdown,
}: {
  breakdown: TokenEstimateBreakdown;
}) {
  const [expanded, setExpanded] = useState(() => new Set(DEFAULT_EXPANDED));
  const roots = useMemo(() => detailsForBreakdown(breakdown), [breakdown]);
  const rows = useMemo(
    () => flattenVisibleDetails(roots, expanded),
    [expanded, roots],
  );

  if (rows.length === 0) {
    return (
      <div className="usage-table-state">
        <span>新请求完成后会记录可审计的模块级 Token 构成。</span>
      </div>
    );
  }

  return (
    <div className="usage-table-wrap">
      <table className="usage-token-breakdown-table">
        <thead>
          <tr>
            <th scope="col">输入内容与时间范围</th>
            <th scope="col" className="usage-number-cell">
              本地估算
            </th>
            <th scope="col" className="usage-number-cell">
              占比
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ detail, depth, path }) => {
            const hasChildren = (detail.children?.length ?? 0) > 0;
            const isExpanded = expanded.has(path);
            return (
              <tr key={path}>
                <th scope="row">
                  <div
                    className={`usage-token-tree-cell usage-token-tree-depth-${Math.min(depth, 3)}`}
                  >
                    {hasChildren ? (
                      <button
                        type="button"
                        className="usage-token-tree-toggle"
                        aria-label={`${isExpanded ? "收起" : "展开"}${detailLabel(detail)}`}
                        aria-expanded={isExpanded}
                        onClick={() =>
                          setExpanded((current) =>
                            toggleExpanded(current, path),
                          )
                        }
                      >
                        {isExpanded ? (
                          <ChevronDown size={14} aria-hidden="true" />
                        ) : (
                          <ChevronRight size={14} aria-hidden="true" />
                        )}
                      </button>
                    ) : (
                      <span
                        className="usage-token-tree-spacer"
                        aria-hidden="true"
                      />
                    )}
                    <span className="usage-token-tree-label">
                      <span>{detailLabel(detail)}</span>
                      {depth === 0 && ROOT_SCOPES[detail.id] ? (
                        <small>{ROOT_SCOPES[detail.id]}</small>
                      ) : null}
                    </span>
                  </div>
                </th>
                <td className="usage-number-cell">
                  {formatInteger(detail.tokens)}
                </td>
                <td className="usage-number-cell">
                  {formatPercent(
                    breakdown.total > 0
                      ? detail.tokens / breakdown.total
                      : null,
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
        <tfoot>
          <tr>
            <th scope="row">本地归因合计</th>
            <td className="usage-number-cell">
              {formatInteger(breakdown.total)}
            </td>
            <td className="usage-number-cell">
              {breakdown.total > 0 ? "100%" : "—"}
            </td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

function detailsForBreakdown(
  breakdown: TokenEstimateBreakdown,
): TokenEstimateDetail[] {
  const roots =
    breakdown.details && breakdown.details.length > 0
      ? breakdown.details
      : legacyTokenBreakdownDetails(breakdown);
  const visible = roots.filter(
    (detail) => detail.tokens > 0 || detail.id === "tool_schemas",
  );
  if (!visible.some((detail) => detail.id === "tool_schemas")) {
    visible.push({
      id: "tool_schemas",
      label: "Tool surface",
      tokens: breakdown.toolSchemas,
      children: [],
    });
  }
  return visible;
}

function flattenVisibleDetails(
  roots: TokenEstimateDetail[],
  expanded: Set<string>,
): VisibleDetail[] {
  const rows: VisibleDetail[] = [];
  const visit = (
    details: TokenEstimateDetail[],
    depth: number,
    parentPath: string,
  ) => {
    for (const item of details) {
      const path = parentPath ? `${parentPath}/${item.id}` : item.id;
      rows.push({ detail: item, depth, path });
      if (expanded.has(path) && item.children?.length) {
        visit(item.children, depth + 1, path);
      }
    }
  };
  visit(roots, 0, "");
  return rows;
}

function toggleExpanded(current: Set<string>, path: string): Set<string> {
  const next = new Set(current);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  return next;
}

function detailLabel(detail: TokenEstimateDetail): string {
  return LABELS[detail.id] ?? detail.label;
}

function formatInteger(value: number | null): string {
  return value === null ? "—" : Math.round(value).toLocaleString("zh-CN");
}

function formatPercent(value: number | null): string {
  return value === null
    ? "—"
    : new Intl.NumberFormat("zh-CN", {
        style: "percent",
        maximumFractionDigits: 1,
      }).format(value);
}

function formatSigned(value: number): string {
  if (value === 0) return "0";
  return `${value > 0 ? "+" : "−"}${formatInteger(Math.abs(value))}`;
}

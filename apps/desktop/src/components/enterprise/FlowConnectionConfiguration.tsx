import { ArrowRight, GitBranch, Repeat2 } from "lucide-react";
import { Badge, NumberField, SelectField, TextField } from "../ui";
import type { WorkflowConnection } from "./workflowGraphOperations";
import {
  workflowNodeLabel,
  type WorkflowEdgeConfiguration,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import type { AgentTemplateVersionView, DataClassification } from "../../types";

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
          <strong>连线</strong>
          <Badge variant={connection.loopPolicy ? "warning" : "neutral"}>
            {connection.loopPolicy ? "feedback" : "edge"}
          </Badge>
        </header>
        <div className="flow-connection-route" aria-label="连线路径">
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
            <strong>路由条件</strong>
            <small>基于上游节点输出决定是否进入下游</small>
          </span>
          <GitBranch aria-hidden="true" size={15} />
        </header>
        <TextField
          error={conditionError(configuration.condition)}
          hint="留空表示始终通过；支持 path、!path、==、!="
          label="Condition（可选）"
          onChange={(event) => change({ condition: event.target.value })}
          placeholder="passed == true"
          value={configuration.condition}
        />
        <TextField
          hint="用逗号分隔；留空传递完整输出"
          label="允许传递的字段"
          onChange={(event) =>
            change({ allowedFields: commaSeparatedValues(event.target.value) })
          }
          placeholder="value, score"
          value={configuration.allowedFields.join(", ")}
        />
        <SelectField<DataClassification>
          label="数据级别"
          onChange={(dataClassification) => change({ dataClassification })}
          options={[
            { value: "public", label: "Public" },
            { value: "internal", label: "Internal" },
            { value: "confidential", label: "Confidential" },
            { value: "restricted", label: "Restricted" },
          ]}
          value={configuration.dataClassification}
        />
        <SelectField
          hint="上游执行失败时跳转的节点；留空表示按默认错误策略终止"
          label="错误路由"
          onChange={(onError) => change({ onError: onError || null })}
          options={[
            { value: "", label: "无 / 默认终止" },
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
              <strong>反馈循环</strong>
              <small>回连线必须有终止条件和次数上限</small>
            </span>
            <Repeat2 aria-hidden="true" size={15} />
          </header>
          <div className="flow-loop-policy">
            <label>
              <span>最大迭代次数</span>
              <NumberField
                label="最大迭代次数"
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
              error={conditionError(configuration.loopPolicy.continueCondition)}
              hint="条件为 true 且尚未到达上限时，沿此反馈边继续"
              label="继续条件"
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
              label="达到上限后"
              onChange={(onExhausted) =>
                change({
                  loopPolicy: { ...configuration.loopPolicy!, onExhausted },
                })
              }
              options={[
                { value: "require_human", label: "暂停并要求人工处理" },
                { value: "return_partial", label: "返回已有部分结果" },
                { value: "fail", label: "标记 Flow 失败" },
              ]}
              value={configuration.loopPolicy.onExhausted}
            />
            <p className="flow-editor-inspector__note">
              <Repeat2 aria-hidden="true" size={13} /> 当前 Flow
              的全局循环预算为 4 次。删除这条回连线即可移除循环。
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

function conditionError(value: string) {
  if (!value.trim()) return undefined;
  if (value.length > 512) return "条件表达式最长 512 个字符";
  if (
    [";", "{", "}", "=>", "function", "import", "eval", "exec("].some((token) =>
      value.includes(token),
    )
  ) {
    return "仅支持受限字段比较，不能包含代码或函数调用";
  }
  return undefined;
}

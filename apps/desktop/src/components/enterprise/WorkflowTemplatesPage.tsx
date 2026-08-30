import { Copy, PauseCircle, PlayCircle, Workflow } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { ActiveFlow, FlowDraftView } from "../../types";
import { Button } from "../ui";
import { FlowEditorInspector } from "./FlowEditorInspector";
import { FlowEditorToolbar } from "./FlowEditorToolbar";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import { guidedWorkflowSpec } from "./model";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "./pageHeader";
import { useEnterpriseStore } from "./store";
import {
  FlowInspectorPortal,
  useFlowAgentSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { FlowTriggerConfigPage } from "./FlowTriggerConfigPage";
import { activationLabel, templateKey } from "./flowActivation";
import { WorkflowGraphEditor } from "./WorkflowGraphEditor";
import {
  createDefaultWorkflowNodes,
  removeWorkflowNode,
  workflowNodeLabel,
  workflowNodesFromGraph,
  workflowNodesFromSpec,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import "./workflow-editor.css";

export function WorkflowTemplatesPage({
  client,
  onPageHeaderChange,
  threadId,
}: {
  client: ApiClient;
  onPageHeaderChange?: EnterprisePageHeaderChange;
  threadId: string | null;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const selection = useFlowAgentSelection();
  const publishedTemplates = useMemo(
    () =>
      snapshot.templates.filter((item) => item.template.status === "published"),
    [snapshot.templates],
  );
  const [flowId, setFlowId] = useState(() =>
    selection?.creatingFlow
      ? `flow-${crypto.randomUUID().slice(0, 8)}`
      : "guided-workflow",
  );
  const [name, setName] = useState(() =>
    selection?.creatingFlow ? "Untitled Flow / 未命名 Flow" : "Guided workflow",
  );
  const [owner, setOwner] = useState("local_operator");
  const [outcome, setOutcome] = useState("");
  const [nodes, setNodes] = useState<WorkflowNodeSelection[]>([]);
  const [draft, setDraft] = useState<FlowDraftView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [creating, setCreating] = useState(Boolean(selection?.creatingFlow));
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [editorLayoutId, setEditorLayoutId] = useState(
    () => `draft:${crypto.randomUUID()}`,
  );
  const [detailPage, setDetailPage] = useState<{
    kind: "trigger";
    nodeId: string;
  } | null>(null);
  const handledCreateRequest = useRef(selection?.createFlowRequest ?? 0);

  const selectedFlowId = selection?.creatingFlow
    ? undefined
    : snapshot.flows.some((flow) => flow.flowId === selection?.selectedFlowId)
      ? selection?.selectedFlowId
      : snapshot.flows[0]?.flowId;
  const selectedFlow = selectedFlowId
    ? (snapshot.flows.find((flow) => flow.flowId === selectedFlowId) ?? null)
    : null;

  useEffect(() => {
    const request = selection?.createFlowRequest ?? 0;
    if (request > handledCreateRequest.current) {
      handledCreateRequest.current = request;
      const suffix = crypto.randomUUID().slice(0, 8);
      setFlowId(`flow-${suffix}`);
      setName("Untitled Flow / 未命名 Flow");
      setOwner("local_operator");
      setOutcome("");
      setNodes(createDefaultWorkflowNodes(publishedTemplates[0]));
      setDraft(null);
      setError(null);
      setNotice(null);
      setSelectedNodeId(null);
      setEditorLayoutId(`draft:${crypto.randomUUID()}`);
      setDetailPage(null);
      setCreating(true);
    }
  }, [publishedTemplates, selection?.createFlowRequest]);

  useEffect(() => {
    if (selection?.selectedFlowId) {
      setCreating(false);
      setDetailPage(null);
    }
  }, [selection?.selectedFlowId]);

  useEffect(() => {
    if (
      selection?.selectedFlowId &&
      !snapshot.flows.some((flow) => flow.flowId === selection.selectedFlowId)
    ) {
      selection.setSelectedFlowId(null);
    }
  }, [selection, snapshot.flows]);

  const detailNode = detailPage
    ? (nodes.find((item) => item.id === detailPage.nodeId) ?? null)
    : null;
  useFlowWorkspaceTitle(creating ? name : selectedFlow?.name);
  useEnterpriseSubpageHeader(onPageHeaderChange, Boolean(detailPage), {
    title: `Flows / ${name} / 配置 Trigger`,
    backLabel: "返回 Flow 图",
    onBack: () => {
      setDetailPage(null);
    },
  });

  useEffect(() => {
    const available = new Set(publishedTemplates.map(templateKey));
    setNodes((current) => {
      if (current.length === 0) {
        return createDefaultWorkflowNodes(publishedTemplates[0]);
      }
      const next = current
        .filter(
          (node) => node.kind === "agent" && !available.has(node.templateKey),
        )
        .reduce((result, node) => removeWorkflowNode(result, node.id), current);
      if (
        creating &&
        publishedTemplates[0] &&
        !next.some((node) => node.kind === "agent") &&
        next.length === 1 &&
        next[0]?.kind === "output"
      ) {
        return createDefaultWorkflowNodes(publishedTemplates[0]);
      }
      return next;
    });
  }, [creating, publishedTemplates]);

  const agentNodes = nodes.filter((node) => node.kind === "agent");
  const allAgentsAvailable = agentNodes.every((node) =>
    publishedTemplates.some(
      (template) => templateKey(template) === node.templateKey,
    ),
  );
  const passedDryRun = Boolean(
    draft?.trials.some(
      (trial) =>
        trial.draftRevision === draft.draft.revision &&
        trial.status === "passed",
    ),
  );
  const successfulTestRun = Boolean(
    draft?.testRuns.some(
      (run) =>
        run.testDraftRevision === draft.draft.revision &&
        run.definitionContentHash === draft.draft.contentHash &&
        run.status === "succeeded",
    ),
  );
  const activeTestRun = draft?.testRuns.find(
    (run) =>
      run.testDraftRevision === draft.draft.revision && !isTerminal(run.status),
  );

  useEffect(() => {
    if (!activeTestRun) return;
    const timer = window.setInterval(() => {
      void client
        .getFlowRun(activeTestRun.id)
        .then((run) => {
          setDraft((current) =>
            current
              ? {
                  ...current,
                  testRuns: [
                    run,
                    ...current.testRuns.filter((item) => item.id !== run.id),
                  ],
                }
              : current,
          );
        })
        .catch((pollError: unknown) => {
          setError(readableError(pollError));
        });
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [activeTestRun, client]);

  const activeSelections = selectedFlow
    ? workflowNodesFromGraph(selectedFlow.activeRevision.compiledWorkflow.graph)
    : [];

  async function execute(name: string, action: () => Promise<void>) {
    if (busy) return;
    setBusy(name);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (actionError) {
      setError(readableError(actionError));
    } finally {
      setBusy(null);
    }
  }

  function createDraft() {
    if (
      !threadId ||
      agentNodes.length === 0 ||
      !allAgentsAvailable ||
      !outcome.trim()
    )
      return;
    void execute("create", async () => {
      const created = await client.createFlowDraft(
        threadId,
        guidedWorkflowSpec({
          flowId: flowId.trim(),
          name: name.trim(),
          owner: owner.trim(),
          outcome: outcome.trim(),
          nodes,
          templates: publishedTemplates,
        }),
      );
      setDraft(created);
      setNotice(
        "Workflow 草稿已创建；Node 与 Agent 模板版本已固定。下一步先验证。 ",
      );
    });
  }

  function changeNodes(next: WorkflowNodeSelection[]) {
    const hadDraft = Boolean(draft);
    setNodes(next);
    setDraft(null);
    setSelectedNodeId((current) =>
      current && next.some((node) => node.id === current) ? current : null,
    );
    if (hadDraft) setNotice("节点配置已修改，请重新创建草稿并验证。");
  }

  function changeFlowConfiguration(
    change: Partial<{
      flowId: string;
      name: string;
      owner: string;
      outcome: string;
    }>,
  ) {
    const hadDraft = Boolean(draft);
    if (change.flowId !== undefined) setFlowId(change.flowId);
    if (change.name !== undefined) setName(change.name);
    if (change.owner !== undefined) setOwner(change.owner);
    if (change.outcome !== undefined) setOutcome(change.outcome);
    setDraft(null);
    if (hadDraft) setNotice("Flow 配置已修改，请重新创建草稿并验证。");
  }

  function changeNode(next: WorkflowNodeSelection) {
    changeNodes(nodes.map((node) => (node.id === next.id ? next : node)));
  }

  function validateDraft() {
    if (!draft) return;
    void execute("validate", async () => {
      setDraft(await client.validateFlowDraft(draft.draft.id));
      setNotice("静态验证完成，可以进行执行计划 Dry Run。 ");
    });
  }

  function dryRunDraft() {
    if (!draft || !threadId) return;
    void execute("simulate", async () => {
      await client.simulateFlowDraft(draft.draft.id, {});
      setDraft(
        (await client.listFlowDrafts(threadId)).find(
          (item) => item.draft.id === draft.draft.id,
        ) ?? draft,
      );
      setNotice("Dry Run 已通过；下一步执行真实 Test Run。 ");
    });
  }

  function startTestRun() {
    if (!draft) return;
    void execute("test-run", async () => {
      const run = await client.startFlowTestRun(
        draft.draft.id,
        {},
        owner.trim(),
      );
      setDraft((current) =>
        current
          ? { ...current, testRuns: [run, ...current.testRuns] }
          : current,
      );
      setNotice(
        "真实 Test Run 已启动；Agent、工具和 Connection 会按冻结权限执行。 ",
      );
    });
  }

  function refreshTestRun() {
    if (!draft || !threadId) return;
    void execute("refresh-test", async () => {
      setDraft(
        (await client.listFlowDrafts(threadId)).find(
          (item) => item.draft.id === draft.draft.id,
        ) ?? draft,
      );
    });
  }

  function activateDraft() {
    if (!draft) return;
    void execute("activate", async () => {
      const activatedFlowId = draft.draft.spec.flowId;
      const active = snapshot.flows.find(
        (flow) => flow.flowId === activatedFlowId,
      );
      await client.activateFlowDraft(draft.draft.id, {
        activatedBy: owner.trim(),
        expectedFlowRevision: active?.revision,
      });
      await store.load(true);
      selection?.setSelectedFlowId(activatedFlowId);
      setNotice(
        "Flow 已激活；Trigger 现在会直接创建 Case，并按入口策略进入待处理或立即运行。 ",
      );
      setCreating(false);
    });
  }

  function copySelectedFlow() {
    if (!selectedFlow) return;
    void execute(`copy:${selectedFlow.flowId}`, async () => {
      const suffix = crypto.randomUUID().slice(0, 8);
      const copyId = `${selectedFlow.flowId}-copy-${suffix}`;
      const copyName = `${selectedFlow.name} Copy`;
      const copied = await client.copyFlow(selectedFlow.flowId, {
        flowId: copyId,
        name: copyName,
        owner,
      });
      setFlowId(copyId);
      setName(copyName);
      setOwner(copied.draft.spec.owner);
      setOutcome(copied.draft.spec.description);
      setNodes(workflowNodesFromSpec(copied.draft.spec));
      setDraft(copied);
      setSelectedNodeId(null);
      setEditorLayoutId(`draft:${crypto.randomUUID()}`);
      selection?.beginFlowDraft();
      setCreating(true);
      setNotice(
        "已创建副本草稿；自动 Trigger 已改为人工确认，请复核后再激活。",
      );
    });
  }

  if (!creating) {
    if (!selectedFlow) {
      return (
        <div className="enterprise-agent-prompt-empty" role="status">
          <Workflow aria-hidden="true" size={20} />
          <strong>尚未创建 Flow</strong>
          <p>使用左侧新建按钮创建一个 Flow。</p>
        </div>
      );
    }
    const selectedActiveNode =
      activeSelections.find((node) => node.id === selectedNodeId) ?? null;
    return (
      <>
        <FlowInspectorPortal>
          <FlowInspectorPanel
            actions={
              <>
                <Button
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void execute(`status:${selectedFlow.flowId}`, async () => {
                      if (selectedFlow.status === "active") {
                        await client.pauseFlow(
                          selectedFlow.flowId,
                          selectedFlow.revision,
                        );
                      } else {
                        await client.resumeFlow(
                          selectedFlow.flowId,
                          selectedFlow.revision,
                        );
                      }
                      await store.load(true);
                    })
                  }
                  size="compact"
                  variant="primary"
                >
                  {selectedFlow.status === "active" ? (
                    <PauseCircle aria-hidden="true" size={14} />
                  ) : (
                    <PlayCircle aria-hidden="true" size={14} />
                  )}
                  {selectedFlow.status === "active" ? "暂停" : "恢复"}
                </Button>
                <Button
                  aria-label={`复制 ${selectedFlow.name}`}
                  disabled={Boolean(busy)}
                  onClick={copySelectedFlow}
                  size="compact"
                  variant="quiet"
                >
                  <Copy aria-hidden="true" size={14} />
                </Button>
              </>
            }
            status={selectedActiveNode?.kind ?? selectedFlow.status}
            statusVariant={
              selectedActiveNode?.kind === "approval"
                ? "warning"
                : selectedActiveNode?.kind === "output" ||
                    (!selectedActiveNode && selectedFlow.status === "active")
                  ? "success"
                  : selectedActiveNode
                    ? "info"
                    : "neutral"
            }
            subtitle={
              selectedActiveNode
                ? workflowNodeLabel(selectedActiveNode, publishedTemplates)
                : `${selectedFlow.flowId}@${selectedFlow.activeRevision.compiledWorkflow.flowVersion}`
            }
            title={selectedActiveNode ? "Node 配置" : "Flow 配置"}
          >
            {selectedActiveNode ? (
              <FlowInspectorSection title="Node">
                <dl className="enterprise-facts flow-inspector-facts">
                  <div>
                    <dt>名称</dt>
                    <dd>
                      {workflowNodeLabel(
                        selectedActiveNode,
                        publishedTemplates,
                      )}
                    </dd>
                  </div>
                  <div>
                    <dt>Node ID</dt>
                    <dd>{selectedActiveNode.id}</dd>
                  </div>
                  <div>
                    <dt>Kind</dt>
                    <dd>{selectedActiveNode.kind}</dd>
                  </div>
                  <div>
                    <dt>Input</dt>
                    <dd>
                      {activationLabel(
                        selectedActiveNode.activation,
                        activeSelections,
                        publishedTemplates,
                      )}
                    </dd>
                  </div>
                </dl>
              </FlowInspectorSection>
            ) : (
              <FlowInspectorSection title="Revision">
                <dl className="enterprise-facts flow-inspector-facts">
                  <div>
                    <dt>所有者</dt>
                    <dd>{selectedFlow.createdBy}</dd>
                  </div>
                  <div>
                    <dt>触发方式</dt>
                    <dd>{triggerLabel(selectedFlow.activeRevision.trigger)}</dd>
                  </div>
                  <div>
                    <dt>入口策略</dt>
                    <dd>
                      {selectedFlow.activeRevision.ingressPolicy === "immediate"
                        ? "自动执行"
                        : "人工确认"}
                    </dd>
                  </div>
                  <div>
                    <dt>输出</dt>
                    <dd>{outputLabel(selectedFlow.activeRevision.output)}</dd>
                  </div>
                </dl>
              </FlowInspectorSection>
            )}
            {error ? (
              <p className="enterprise-page__message is-error" role="alert">
                {error}
              </p>
            ) : null}
          </FlowInspectorPanel>
        </FlowInspectorPortal>
        <div className="enterprise-page enterprise-workflow-editor-page">
          <section className="workflow-editor workflow-editor--canvas-only">
            <div className="workflow-editor__body">
              <WorkflowGraphEditor
                layoutId={`active:${selectedFlow.flowId}@${selectedFlow.activeRevision.compiledWorkflow.flowVersion}`}
                onSelectNode={setSelectedNodeId}
                readOnly
                selections={activeSelections}
                selectedNodeId={selectedNodeId}
                templates={publishedTemplates}
              />
            </div>
          </section>
        </div>
      </>
    );
  }

  return (
    <>
      <FlowInspectorPortal>
        <section
          className="flow-workspace-inspector workflow-editor-inspector-shell"
          aria-label="Flow 配置"
        >
          <FlowEditorToolbar
            activeTestRun={Boolean(activeTestRun)}
            busy={busy}
            canActivate={Boolean(
              draft?.draft.lastValidation?.valid &&
              passedDryRun &&
              successfulTestRun,
            )}
            canCreateDraft={Boolean(
              threadId &&
              agentNodes.length > 0 &&
              allAgentsAvailable &&
              outcome.trim(),
            )}
            canDryRun={Boolean(draft?.draft.lastValidation?.valid)}
            canTestRun={Boolean(
              draft?.draft.lastValidation?.valid && passedDryRun,
            )}
            draftExists={Boolean(draft)}
            flowId={flowId}
            name={name}
            nodeCount={nodes.length}
            onActivate={activateDraft}
            onCreateDraft={createDraft}
            onDryRun={dryRunDraft}
            onRefresh={refreshTestRun}
            onTestRun={startTestRun}
            onValidate={validateDraft}
            passedDryRun={passedDryRun}
            successfulTestRun={successfulTestRun}
            threadReady={Boolean(threadId)}
            validated={Boolean(draft?.draft.lastValidation?.valid)}
          />
          {detailPage?.kind === "trigger" && detailNode ? (
            <FlowTriggerConfigPage
              node={detailNode}
              onChange={(activation) => {
                setDraft(null);
                setNodes((current) =>
                  current.map((item) =>
                    item.id === detailNode.id ? { ...item, activation } : item,
                  ),
                );
              }}
              selections={nodes}
              templates={publishedTemplates}
            />
          ) : (
            <FlowEditorInspector
              draft={draft}
              error={error}
              flowId={flowId}
              name={name}
              nodes={nodes}
              notice={notice}
              onChangeFlow={changeFlowConfiguration}
              onChangeNode={changeNode}
              onEditTrigger={(nodeId) =>
                setDetailPage({ kind: "trigger", nodeId })
              }
              onSelectNode={setSelectedNodeId}
              outcome={outcome}
              owner={owner}
              passedDryRun={passedDryRun}
              selectedNodeId={selectedNodeId}
              successfulTestRun={successfulTestRun}
              templates={publishedTemplates}
            />
          )}
        </section>
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-workflow-editor-page">
        <section
          className="workflow-editor workflow-editor--canvas-only"
          aria-label="Flow 编辑器"
        >
          <div className="workflow-editor__body">
            <WorkflowGraphEditor
              disabled={Boolean(busy)}
              layoutId={editorLayoutId}
              onChange={changeNodes}
              onEditTrigger={(nodeId) =>
                setDetailPage({ kind: "trigger", nodeId })
              }
              onSelectNode={setSelectedNodeId}
              selections={nodes}
              selectedNodeId={selectedNodeId}
              templates={publishedTemplates}
            />
          </div>
        </section>
      </div>
    </>
  );
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isTerminal(status: string): boolean {
  return ["succeeded", "failed", "cancelled"].includes(status);
}

function triggerLabel(
  trigger: ActiveFlow["activeRevision"]["trigger"],
): string {
  if (trigger.kind === "manual") return "手动触发";
  if (trigger.kind === "webhook") return `API / Webhook · ${trigger.triggerId}`;
  if (trigger.kind === "schedule") {
    return `定时触发 · 每 ${trigger.intervalSeconds} 秒`;
  }
  return `连接事件 · ${trigger.source} / ${trigger.eventType}`;
}

function outputLabel(output: ActiveFlow["activeRevision"]["output"]): string {
  if (output.kind === "inbox") return "Inbox";
  if (output.kind === "webhook") return `Webhook · ${output.endpoint}`;
  if (output.kind === "human_task") return `人工任务 · ${output.title}`;
  return `Connection · ${output.operation.operationId}`;
}

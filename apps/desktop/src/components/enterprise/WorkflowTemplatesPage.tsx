import { Copy, PauseCircle, PlayCircle, Workflow } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { ActiveFlow, FlowDraftView } from "../../types";
import { Button } from "../ui";
import {
  FlowEditorInspector,
  type FlowRuntimeConfiguration,
} from "./FlowEditorInspector";
import { FlowEditorToolbar } from "./FlowEditorToolbar";
import { FlowCreateDialog, type FlowCreateValues } from "./FlowCreateDialog";
import { FlowTestRunDialog } from "./FlowTestRunDialog";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import {
  DEFAULT_GUIDED_FLOW_BUDGET,
  DEFAULT_GUIDED_FLOW_RISK_CLASS,
  guidedWorkflowSpec,
} from "./model";
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
import { templateKey } from "./flowActivation";
import { WorkflowGraphEditor } from "./WorkflowGraphEditor";
import {
  configureWorkflowConnection,
  workflowConnections,
  type WorkflowConnection,
} from "./workflowGraphOperations";
import { workflowGraphNodeInputLabel } from "./workflowCanvasModel";
import {
  createDefaultWorkflowNodes,
  removeWorkflowNode,
  workflowNodeLabel,
  workflowNodesFromSpec,
  type WorkflowNodeSelection,
  type WorkflowEdgeConfiguration,
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
  const [runtimeConfiguration, setRuntimeConfiguration] =
    useState<FlowRuntimeConfiguration>(() => defaultRuntimeConfiguration());
  const [nodes, setNodes] = useState<WorkflowNodeSelection[]>([]);
  const [draft, setDraft] = useState<FlowDraftView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [createDialogOpen, setCreateDialogOpen] = useState(
    Boolean(selection?.creatingFlow),
  );
  const [createValues, setCreateValues] = useState<FlowCreateValues>(() =>
    defaultCreateValues(),
  );
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [selectedConnection, setSelectedConnection] =
    useState<WorkflowConnection | null>(null);
  const [testInputText, setTestInputText] = useState("{}");
  const [testRunDialogOpen, setTestRunDialogOpen] = useState(false);
  const [editorLayoutId, setEditorLayoutId] = useState(
    () => `draft:${crypto.randomUUID()}`,
  );
  const [detailPage, setDetailPage] = useState<{
    kind: "trigger";
    nodeId: string;
  } | null>(null);
  const handledCreateRequest = useRef(selection?.createFlowRequest ?? 0);
  const handledFailedTestRun = useRef<string | null>(null);

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
      setCreateValues(defaultCreateValues());
      setRuntimeConfiguration(defaultRuntimeConfiguration());
      setCreating(false);
      setCreateDialogOpen(true);
      setDraft(null);
      setError(null);
      setNotice(null);
      setSelectedNodeId(null);
      setSelectedConnection(null);
      setEditorLayoutId(`draft:${crypto.randomUUID()}`);
      setDetailPage(null);
      setTestInputText("{}");
      setTestRunDialogOpen(false);
    }
  }, [selection?.createFlowRequest]);

  useEffect(() => {
    if (selection?.selectedFlowId) {
      setCreating(false);
      setCreateDialogOpen(false);
      setDetailPage(null);
      setSelectedNodeId(null);
      setSelectedConnection(null);
      setTestRunDialogOpen(false);
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
  useFlowWorkspaceTitle(
    createDialogOpen ? "创建 Flow" : creating ? name : selectedFlow?.name,
  );
  useEnterpriseSubpageHeader(onPageHeaderChange, Boolean(detailPage), {
    title: `Flows / ${name} / 配置 Trigger`,
    backLabel: "返回 Flow 图",
    onBack: () => {
      setDetailPage(null);
    },
  });

  useEffect(() => {
    if (!creating) return;
    const available = new Set(publishedTemplates.map(templateKey));
    setNodes((current) => {
      return current
        .filter(
          (node) =>
            node.kind === "agent" &&
            Boolean(node.templateKey) &&
            !available.has(node.templateKey),
        )
        .reduce((result, node) => removeWorkflowNode(result, node.id), current);
    });
  }, [creating, publishedTemplates]);

  const agentNodes = nodes.filter((node) => node.kind === "agent");
  const allAgentsAvailable = agentNodes.every((node) =>
    publishedTemplates.some(
      (template) => templateKey(template) === node.templateKey,
    ),
  );
  const graphReady =
    nodes.some((node) => node.kind !== "output") &&
    nodes.every(nodeConfigurationReady);
  const currentTestRuns =
    draft?.testRuns.filter(
      (run) =>
        run.testDraftRevision === draft.draft.revision &&
        run.definitionContentHash === draft.draft.contentHash,
    ) ?? [];
  const successfulTestRun = Boolean(
    currentTestRuns.some((run) => run.status === "succeeded"),
  );
  const activeTestRun = currentTestRuns.find((run) => !isTerminal(run.status));
  const currentTestRun = activeTestRun ?? currentTestRuns[0] ?? null;
  const testExecutionSteps = useMemo(
    () =>
      nodes
        .filter((node) => node.kind === "agent" || node.kind === "tool")
        .map(
          (node) =>
            `${node.kind === "agent" ? "Agent" : "Action"}：${workflowNodeLabel(node, publishedTemplates)}`,
        ),
    [nodes, publishedTemplates],
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

  useEffect(() => {
    if (
      !currentTestRun ||
      currentTestRun.status !== "failed" ||
      handledFailedTestRun.current === currentTestRun.id
    )
      return;
    handledFailedTestRun.current = currentTestRun.id;
    const failedNode = [...currentTestRun.nodeRuns]
      .reverse()
      .find((nodeRun) => nodeRun.status === "failed");
    if (!failedNode) return;
    setDetailPage(null);
    setSelectedConnection(null);
    setSelectedNodeId(failedNode.nodeId);
    setNotice("Test Run 失败，已定位到出错节点。右侧可查看输入、输出和错误。");
  }, [currentTestRun]);

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

  function selectNode(nodeId: string | null) {
    setDetailPage(null);
    if (nodeId) setSelectedConnection(null);
    setSelectedNodeId(nodeId);
  }

  function selectConnection(connection: WorkflowConnection | null) {
    setDetailPage(null);
    if (connection) setSelectedNodeId(null);
    setSelectedConnection(connection);
  }

  function confirmCreateFlow() {
    const nextNodes = createDefaultWorkflowNodes();
    setFlowId(`flow-${crypto.randomUUID().slice(0, 8)}`);
    setName(createValues.name);
    setOwner("local_operator");
    setOutcome(createValues.outcome);
    setNodes(nextNodes);
    setRuntimeConfiguration(defaultRuntimeConfiguration());
    setDraft(null);
    setError(null);
    setNotice("Flow 已创建。请从空画布添加第一个节点。");
    setSelectedNodeId(null);
    setSelectedConnection(null);
    setEditorLayoutId(`draft:${crypto.randomUUID()}`);
    setDetailPage(null);
    setTestInputText("{}");
    setTestRunDialogOpen(false);
    setCreateDialogOpen(false);
    selection?.beginFlowDraft();
    setCreating(true);
  }

  function cancelCreateFlow() {
    setCreateDialogOpen(false);
    setCreating(false);
    selection?.cancelCreateFlow();
  }

  function createDraft() {
    if (!threadId || !graphReady || !allAgentsAvailable || !outcome.trim())
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
          budget: runtimeConfiguration.budget,
          riskClass: runtimeConfiguration.riskClass,
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
    const addedOutput =
      !nodes.some((node) => node.kind === "output") &&
      next.some((node) => node.kind === "output");
    const addedBusinessNode = next.some(
      (node) =>
        node.kind !== "output" &&
        !nodes.some((current) => current.id === node.id),
    );
    setNodes(next);
    setDraft(null);
    setTestRunDialogOpen(false);
    setSelectedNodeId((current) =>
      current && next.some((node) => node.id === current) ? current : null,
    );
    setSelectedConnection((current) =>
      current &&
      workflowConnections(next).some(
        (edge) =>
          edge.sourceId === current.sourceId &&
          edge.targetId === current.targetId,
      )
        ? current
        : null,
    );
    if (hadDraft) setNotice("节点配置已修改，请重新创建草稿并验证。");
    else if (addedOutput && addedBusinessNode)
      setNotice("已添加第一个步骤，并自动创建 Flow 的固定 Output 终点。");
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

  function changeRuntimeConfiguration(next: FlowRuntimeConfiguration) {
    const hadDraft = Boolean(draft);
    setRuntimeConfiguration(next);
    setDraft(null);
    if (hadDraft) setNotice("运行设置已修改，请重新创建草稿并验证。");
  }

  function changeConnection(
    connection: WorkflowConnection,
    configuration: WorkflowEdgeConfiguration,
  ) {
    const next = configureWorkflowConnection(
      nodes,
      connection.sourceId,
      connection.targetId,
      configuration,
    );
    changeNodes(next);
    setSelectedConnection(
      workflowConnections(next).find(
        (edge) =>
          edge.sourceId === connection.sourceId &&
          edge.targetId === connection.targetId,
      ) ?? null,
    );
  }

  function validateDraft() {
    if (!draft) return;
    void execute("validate", async () => {
      setDraft(await client.validateFlowDraft(draft.draft.id));
      setNotice("验证完成。下一步使用一份测试输入执行 Test Run。 ");
    });
  }

  function startTestRun(input: unknown) {
    if (!draft) return;
    void execute("test-run", async () => {
      const run = await client.startFlowTestRun(
        draft.draft.id,
        input,
        owner.trim(),
      );
      setDraft((current) =>
        current
          ? { ...current, testRuns: [run, ...current.testRuns] }
          : current,
      );
      setTestRunDialogOpen(false);
      setNotice(
        "Test Run 已启动；可在画布查看实际执行路径，并点击节点检查输入和输出。 ",
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
      setRuntimeConfiguration({
        budget: { ...copied.draft.spec.budget },
        riskClass: copied.draft.spec.riskClass,
      });
      setNodes(workflowNodesFromSpec(copied.draft.spec));
      setDraft(copied);
      setSelectedNodeId(null);
      setSelectedConnection(null);
      setEditorLayoutId(`draft:${crypto.randomUUID()}`);
      setTestInputText("{}");
      setTestRunDialogOpen(false);
      setCreateDialogOpen(false);
      selection?.beginFlowDraft();
      setCreating(true);
      setNotice(
        "已创建副本草稿；自动 Trigger 已改为人工确认，请复核后再激活。",
      );
    });
  }

  const createDialog = (
    <FlowCreateDialog
      onCancel={cancelCreateFlow}
      onChange={setCreateValues}
      onSubmit={confirmCreateFlow}
      open={createDialogOpen}
      values={createValues}
    />
  );
  const testRunDialog = (
    <FlowTestRunDialog
      busy={busy === "test-run"}
      executionSteps={testExecutionSteps}
      externalError={testRunDialogOpen ? error : null}
      inputSchema={draft?.draft.spec.inputSchema}
      inputText={testInputText}
      onCancel={() => setTestRunDialogOpen(false)}
      onChangeInput={setTestInputText}
      onSubmit={startTestRun}
      open={testRunDialogOpen}
    />
  );

  if (!creating) {
    if (!selectedFlow) {
      return (
        <>
          {createDialog}
          <div className="enterprise-agent-prompt-empty" role="status">
            <Workflow aria-hidden="true" size={20} />
            <strong>尚未创建 Flow</strong>
            <p>使用左侧新建按钮创建一个 Flow。</p>
          </div>
        </>
      );
    }
    const activeGraph = selectedFlow.activeRevision.compiledWorkflow.graph;
    const selectedActiveNode =
      activeGraph.nodes.find((node) => node.id === selectedNodeId) ?? null;
    return (
      <>
        {createDialog}
        <FlowInspectorPortal>
          <FlowInspectorPanel
            key={selectedActiveNode?.id ?? selectedFlow.flowId}
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
                ? selectedActiveNode.label
                : `${selectedFlow.flowId}@${selectedFlow.activeRevision.compiledWorkflow.flowVersion}`
            }
            title={selectedActiveNode ? "Node 配置" : "Flow 配置"}
          >
            {selectedActiveNode ? (
              <FlowInspectorSection title="Node">
                <dl className="enterprise-facts flow-inspector-facts">
                  <div>
                    <dt>名称</dt>
                    <dd>{selectedActiveNode.label}</dd>
                  </div>
                  <div>
                    <dt>Kind</dt>
                    <dd>{selectedActiveNode.kind}</dd>
                  </div>
                  <div>
                    <dt>Input</dt>
                    <dd>
                      {workflowGraphNodeInputLabel(
                        selectedActiveNode,
                        activeGraph,
                      )}
                    </dd>
                  </div>
                </dl>
                <details className="flow-editor-inspector__advanced">
                  <summary>节点高级信息</summary>
                  <dl className="enterprise-facts flow-inspector-facts">
                    <div>
                      <dt>Node ID</dt>
                      <dd>{selectedActiveNode.id}</dd>
                    </div>
                  </dl>
                </details>
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
                compiledGraph={activeGraph}
                layoutId={`active-compiled-v2:${selectedFlow.flowId}@${selectedFlow.activeRevision.compiledWorkflow.flowVersion}`}
                onSelectNode={selectNode}
                readOnly
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
      {createDialog}
      {testRunDialog}
      <FlowInspectorPortal>
        <section
          className="flow-workspace-inspector workflow-editor-inspector-shell"
          aria-label="Flow 配置"
        >
          <FlowEditorToolbar
            activeTestRun={Boolean(activeTestRun)}
            busy={busy}
            canActivate={Boolean(
              draft?.draft.lastValidation?.valid && successfulTestRun,
            )}
            canCreateDraft={Boolean(
              threadId && graphReady && allAgentsAvailable && outcome.trim(),
            )}
            canTestRun={Boolean(draft?.draft.lastValidation?.valid)}
            draftExists={Boolean(draft)}
            flowId={flowId}
            name={name}
            nodeCount={nodes.filter((node) => node.kind !== "output").length}
            onActivate={activateDraft}
            onCreateDraft={createDraft}
            onRefresh={refreshTestRun}
            onTestRun={() => {
              setError(null);
              setTestRunDialogOpen(true);
            }}
            onValidate={validateDraft}
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
              onChangeConnection={changeConnection}
              onChangeNode={changeNode}
              onChangeRuntimeConfiguration={changeRuntimeConfiguration}
              onEditTrigger={(nodeId) =>
                setDetailPage({ kind: "trigger", nodeId })
              }
              onSelectNode={selectNode}
              onSelectConnection={selectConnection}
              outcome={outcome}
              owner={owner}
              runtimeConfiguration={runtimeConfiguration}
              selectedNodeId={selectedNodeId}
              selectedConnection={selectedConnection}
              successfulTestRun={successfulTestRun}
              testRun={currentTestRun}
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
              onSelectNode={selectNode}
              onSelectConnection={selectConnection}
              selections={nodes}
              selectedConnection={selectedConnection}
              selectedNodeId={selectedNodeId}
              testRun={currentTestRun}
              templates={publishedTemplates}
            />
          </div>
        </section>
      </div>
    </>
  );
}

function defaultCreateValues(): FlowCreateValues {
  return {
    name: "",
    outcome: "",
  };
}

function defaultRuntimeConfiguration(): FlowRuntimeConfiguration {
  return {
    budget: { ...DEFAULT_GUIDED_FLOW_BUDGET },
    riskClass: DEFAULT_GUIDED_FLOW_RISK_CLASS,
  };
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isTerminal(status: string): boolean {
  return ["succeeded", "failed", "cancelled"].includes(status);
}

function nodeConfigurationReady(node: WorkflowNodeSelection) {
  if (node.kind === "skill" || node.kind === "tool") {
    return Boolean(node.reference.trim());
  }
  if (node.kind === "condition") return Boolean(node.expression.trim());
  return true;
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

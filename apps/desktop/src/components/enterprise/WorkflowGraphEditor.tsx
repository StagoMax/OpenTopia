import {
  Background,
  BackgroundVariant,
  ReactFlow,
  SelectionMode,
  type Connection,
  type Edge,
  type FinalConnectionState,
  type NodeChange,
  type NodeTypes,
  type ReactFlowInstance,
  type XYPosition,
} from "@xyflow/react";
import "@xyflow/react/dist/base.css";
import { Bot } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentTemplateVersionView, FlowRun, FlowSpec } from "../../types";
import { Button } from "../ui";
import { createFinalActivation } from "./flowActivation";
import {
  canConnectWorkflowNodes,
  connectWorkflowNodes,
  disconnectWorkflowNodes,
  workflowConnections,
  type WorkflowConnection,
} from "./workflowGraphOperations";
import {
  readWorkflowCanvasLayout,
  reconcileWorkflowPositions,
  writeWorkflowCanvasLayout,
  type WorkflowCanvasLayout,
} from "./workflowCanvasLayout";
import {
  isWorkflowCanvasSpaceKey,
  isWorkflowCanvasTemporaryPanKey,
  workflowCanvasCommand,
} from "./workflowCanvasShortcuts";
import {
  WorkflowCanvasQuickCreate,
  WorkflowCanvasToolbar,
  type CanvasTool,
  type QuickCreateState,
} from "./WorkflowCanvasControls";
import {
  WorkflowCanvasNode,
  type WorkflowCanvasRunStatus,
  type WorkflowCanvasNodeType,
} from "./WorkflowCanvasNode";
import {
  compiledWorkflowCanvasModel,
  editableWorkflowCanvasModel,
} from "./workflowCanvasModel";
import {
  applyWorkflowNodePositions,
  committedWorkflowNodePositionChanges,
  reconcileWorkflowCanvasNodes,
} from "./workflowCanvasNodeState";
import {
  addWorkflowNode,
  removeWorkflowNode,
  type AddableWorkflowNodeKind,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection";
import "./workflow-graph.css";

const nodeTypes: NodeTypes = { workflowNode: WorkflowCanvasNode };
const HISTORY_LIMIT = 50;
const DEFAULT_VIEWPORT = { x: 0, y: 0, zoom: 0.9 };
const EMPTY_SELECTIONS: WorkflowNodeSelection[] = [];

type CanvasSnapshot = {
  layout: WorkflowCanvasLayout;
  selections: WorkflowNodeSelection[];
};

export function WorkflowGraphEditor({
  compiledGraph,
  disabled = false,
  layoutId,
  onChange,
  onEditTrigger,
  onSelectNode,
  onSelectConnection,
  readOnly = false,
  selections = EMPTY_SELECTIONS,
  selectedNodeId,
  selectedConnection,
  testRun = null,
  templates,
}: {
  compiledGraph?: FlowSpec["graph"];
  disabled?: boolean;
  layoutId: string;
  onChange?(selections: WorkflowNodeSelection[]): void;
  onEditTrigger?(nodeId: string): void;
  onSelectNode(nodeId: string | null): void;
  onSelectConnection?(connection: WorkflowConnection | null): void;
  readOnly?: boolean;
  selections?: WorkflowNodeSelection[];
  selectedNodeId: string | null;
  selectedConnection?: WorkflowConnection | null;
  testRun?: FlowRun | null;
  templates: AgentTemplateVersionView[];
}) {
  const canvasReadOnly = readOnly || Boolean(compiledGraph);
  const canvasModel = useMemo(
    () =>
      compiledGraph
        ? compiledWorkflowCanvasModel(compiledGraph)
        : editableWorkflowCanvasModel(selections, templates),
    [compiledGraph, selections, templates],
  );
  const canvasRef = useRef<HTMLElement | null>(null);
  const instanceRef = useRef<ReactFlowInstance<WorkflowCanvasNodeType> | null>(
    null,
  );
  const dragStartPositions = useRef<Record<string, XYPosition> | null>(null);
  const syncedPositionsRef = useRef<WorkflowCanvasLayout["positions"] | null>(
    null,
  );
  const history = useRef<{
    future: CanvasSnapshot[];
    past: CanvasSnapshot[];
  }>({ future: [], past: [] });
  const keyDownHandlerRef = useRef<(event: KeyboardEvent) => void>(() => {});
  const [, setHistoryVersion] = useState(0);
  const [layout, setLayout] = useState(() =>
    readWorkflowCanvasLayout(
      layoutId,
      canvasModel.nodes,
      canvasModel.connections,
    ),
  );
  const [nodePickerOpen, setNodePickerOpen] = useState(false);
  const [quickCreate, setQuickCreate] = useState<QuickCreateState | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [tool, setTool] = useState<CanvasTool>("select");
  const [spacePanning, setSpacePanning] = useState(false);
  const activeTool: CanvasTool = spacePanning ? "pan" : tool;
  const runStatuses = useMemo(() => workflowRunStatuses(testRun), [testRun]);
  const executedNodeIds = useMemo(
    () => new Set(testRun?.nodeRuns.map((nodeRun) => nodeRun.nodeId) ?? []),
    [testRun],
  );

  useEffect(() => {
    if (!selectedConnection) return;
    const match = canvasModel.connections.find(
      (edge) =>
        edge.sourceId === selectedConnection.sourceId &&
        edge.targetId === selectedConnection.targetId,
    );
    setSelectedEdgeId(
      match ? (match.id ?? edgeId(match.sourceId, match.targetId)) : null,
    );
  }, [canvasModel.connections, selectedConnection]);

  useEffect(() => {
    const next = readWorkflowCanvasLayout(
      layoutId,
      canvasModel.nodes,
      canvasModel.connections,
    );
    setLayout(next);
    setNodePickerOpen(false);
    setQuickCreate(null);
    setSelectedEdgeId(null);
    syncedPositionsRef.current = null;
    history.current = { future: [], past: [] };
    setHistoryVersion((value) => value + 1);
    const frame = window.requestAnimationFrame(() => {
      const instance = instanceRef.current;
      if (!instance) return;
      if (next.viewport) void instance.setViewport(next.viewport);
      else void instance.fitView({ maxZoom: 1, minZoom: 0.3, padding: 0.18 });
    });
    return () => window.cancelAnimationFrame(frame);
    // The layout identity is the reset boundary. Node changes are reconciled
    // separately so adding a node does not reset the viewport.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layoutId]);

  useEffect(() => {
    setLayout((current) => ({
      ...current,
      positions: reconcileWorkflowPositions(
        canvasModel.nodes,
        canvasModel.connections,
        current.positions,
      ),
    }));
  }, [canvasModel]);

  useEffect(() => {
    const timer = window.setTimeout(
      () => writeWorkflowCanvasLayout(layoutId, layout),
      180,
    );
    return () => window.clearTimeout(timer);
  }, [layout, layoutId]);

  const flowNodes: WorkflowCanvasNodeType[] = canvasModel.nodes.map((node) => {
    return {
      id: node.id,
      type: "workflowNode",
      position: layout.positions[node.id] ?? { x: 0, y: 0 },
      selected: selectedNodeId === node.id,
      // Canvas layout is local presentation state. Keep it adjustable even
      // when the published Flow itself is structurally read-only.
      draggable: activeTool === "select",
      connectable: !canvasReadOnly && !disabled,
      deletable: !canvasReadOnly && node.kind !== "output",
      focusable: true,
      ariaLabel: `${node.label}，${node.kind} node`,
      data: {
        activationText: node.inputText,
        disabled,
        kind: node.kind,
        label: node.label,
        onEditTrigger: (nodeId) => onEditTrigger?.(nodeId),
        onRemove: removeNode,
        onSelect: (nodeId) => {
          setSelectedEdgeId(null);
          onSelectConnection?.(null);
          onSelectNode(nodeId);
        },
        readOnly: canvasReadOnly,
        runStatus: runStatuses.get(node.id),
        subtitle: node.subtitle,
      },
    };
  });
  const initialFlowNodesRef = useRef(flowNodes);

  useEffect(() => {
    const instance = instanceRef.current;
    if (!instance) return;
    const syncPositions = syncedPositionsRef.current !== layout.positions;
    instance.setNodes((current) =>
      reconcileWorkflowCanvasNodes(current, flowNodes, syncPositions),
    );
    syncedPositionsRef.current = layout.positions;
  }, [flowNodes, layout.positions]);

  const edges: Edge[] = canvasModel.connections.map((edge) => {
    const tested = Boolean(
      executedNodeIds.has(edge.sourceId) && executedNodeIds.has(edge.targetId),
    );
    return {
      id: edge.id ?? edgeId(edge.sourceId, edge.targetId),
      source: edge.sourceId,
      sourceHandle: "final",
      target: edge.targetId,
      targetHandle: "input",
      type: "smoothstep",
      className: `workflow-canvas__edge${tested ? " is-tested" : ""}`,
      label: edge.loopPolicy
        ? edge.condition.trim()
          ? `Loop · ${edge.condition.trim()}`
          : "Loop"
        : edge.condition.trim() || undefined,
      labelBgStyle: { fill: "var(--surface)" },
      labelStyle: { fill: "var(--text-secondary)" },
      selected:
        selectedEdgeId === (edge.id ?? edgeId(edge.sourceId, edge.targetId)),
      selectable: true,
      deletable: !canvasReadOnly && !disabled,
      reconnectable: !canvasReadOnly && !disabled,
    };
  });

  function snapshot(
    snapshotLayout: WorkflowCanvasLayout = layout,
  ): CanvasSnapshot {
    return {
      layout: {
        ...snapshotLayout,
        positions: { ...snapshotLayout.positions },
      },
      selections: structuredClone(selections) as WorkflowNodeSelection[],
    };
  }

  function pushPast(value: CanvasSnapshot) {
    history.current.past = [...history.current.past, value].slice(
      -HISTORY_LIMIT,
    );
    history.current.future = [];
    setHistoryVersion((current) => current + 1);
  }

  function commitSemanticChange(
    nextSelections: WorkflowNodeSelection[],
    nextPositions = reconcileWorkflowPositions(
      nextSelections,
      workflowConnections(nextSelections),
      layout.positions,
    ),
  ) {
    if (!onChange || canvasReadOnly || disabled) return;
    pushPast(snapshot());
    setLayout((current) => ({ ...current, positions: nextPositions }));
    onChange(nextSelections);
    setSelectedEdgeId(null);
    onSelectConnection?.(null);
  }

  function addNode(
    kind: AddableWorkflowNodeKind,
    preferredPosition?: XYPosition,
    sourceId?: string,
  ) {
    const next = addWorkflowNode(selections, kind);
    const added = next.find(
      (candidate) => !selections.some((node) => node.id === candidate.id),
    );
    if (!added) return;
    const position = preferredPosition ?? viewportCenter();
    const withSource = sourceId
      ? next.map((node) =>
          node.id === added.id
            ? { ...node, activation: createFinalActivation(sourceId) }
            : node,
        )
      : next;
    commitSemanticChange(withSource, {
      ...reconcileWorkflowPositions(
        withSource,
        workflowConnections(withSource),
        layout.positions,
      ),
      [added.id]: position,
    });
    onSelectNode(added.id);
    onSelectConnection?.(null);
    setQuickCreate(null);
  }

  function removeNode(nodeId: string) {
    const next = removeWorkflowNode(selections, nodeId);
    if (next.length === selections.length) return;
    commitSemanticChange(next);
    onSelectNode(null);
    onSelectConnection?.(null);
  }

  function connect(connection: Connection) {
    if (!connection.source || !connection.target) return;
    const next = connectWorkflowNodes(
      selections,
      connection.source,
      connection.target,
    );
    if (sameConnections(selections, next)) return;
    const addedConnection = workflowConnections(next).find(
      (candidate) =>
        candidate.sourceId === connection.source &&
        candidate.targetId === connection.target,
    );
    commitSemanticChange(next);
    setSelectedEdgeId(edgeId(connection.source, connection.target));
    onSelectNode(null);
    onSelectConnection?.(addedConnection ?? null);
  }

  function reconnect(oldEdge: Edge, connection: Connection) {
    if (!connection.source || !connection.target) return;
    const disconnected = disconnectWorkflowNodes(
      selections,
      oldEdge.source,
      oldEdge.target,
    );
    if (
      !canConnectWorkflowNodes(
        disconnected,
        connection.source,
        connection.target,
      )
    ) {
      return;
    }
    const reconnectedNodes = connectWorkflowNodes(
      disconnected,
      connection.source,
      connection.target,
    );
    const reconnected = workflowConnections(reconnectedNodes).find(
      (candidate) =>
        candidate.sourceId === connection.source &&
        candidate.targetId === connection.target,
    );
    commitSemanticChange(reconnectedNodes);
    setSelectedEdgeId(edgeId(connection.source, connection.target));
    onSelectNode(null);
    onSelectConnection?.(reconnected ?? null);
  }

  function disconnectSelectedEdge() {
    if (!selectedEdgeId) return false;
    const edge = edges.find((candidate) => candidate.id === selectedEdgeId);
    if (!edge) return false;
    commitSemanticChange(
      disconnectWorkflowNodes(selections, edge.source, edge.target),
    );
    setSelectedEdgeId(null);
    onSelectConnection?.(null);
    return true;
  }

  function undo() {
    const previous = history.current.past.pop();
    if (!previous) return;
    history.current.future = [snapshot(), ...history.current.future].slice(
      0,
      HISTORY_LIMIT,
    );
    applySnapshot(previous);
  }

  function redo() {
    const next = history.current.future.shift();
    if (!next) return;
    history.current.past = [...history.current.past, snapshot()].slice(
      -HISTORY_LIMIT,
    );
    applySnapshot(next);
  }

  function applySnapshot(next: CanvasSnapshot) {
    setLayout(next.layout);
    if (!sameSelections(selections, next.selections))
      onChange?.(next.selections);
    setQuickCreate(null);
    setSelectedEdgeId(null);
    onSelectConnection?.(null);
    onSelectNode(null);
    setHistoryVersion((current) => current + 1);
  }

  function viewportCenter(): XYPosition {
    const bounds = canvasRef.current?.getBoundingClientRect();
    const instance = instanceRef.current;
    if (!bounds || !instance) return { x: 80, y: 80 };
    return instance.screenToFlowPosition({
      x: bounds.left + bounds.width / 2,
      y: bounds.top + bounds.height / 2,
    });
  }

  function handleNodesChange(changes: NodeChange<WorkflowCanvasNodeType>[]) {
    // Pointer drags are committed once by onNodeDragStop, together with their
    // history entry. React Flow still applies every frame to its internal store.
    if (dragStartPositions.current) return;
    const committedPositionChanges =
      committedWorkflowNodePositionChanges(changes);
    if (committedPositionChanges.length === 0) return;
    setLayout((current) => {
      const positions = applyWorkflowNodePositions(
        current.positions,
        committedPositionChanges,
      );
      return positions === current.positions
        ? current
        : { ...current, positions };
    });
  }

  function handleConnectEnd(
    event: MouseEvent | TouchEvent,
    connectionState: FinalConnectionState,
  ) {
    if (
      canvasReadOnly ||
      disabled ||
      connectionState.isValid ||
      connectionState.toNode ||
      !connectionState.fromNode ||
      connectionState.fromHandle?.type !== "source" ||
      connectionState.fromNode.id === "output"
    ) {
      return;
    }
    const point = pointerPosition(event);
    const bounds = canvasRef.current?.getBoundingClientRect();
    const instance = instanceRef.current;
    if (!point || !bounds || !instance) return;
    const styles = getComputedStyle(document.documentElement);
    const margin = cssNumber(styles, "--space-4", 8);
    const menuWidth = cssNumber(styles, "--popover-width-compact", 260);
    const menuHeight = cssNumber(styles, "--control-height-lg", 36) * 11;
    const left = Math.min(
      Math.max(margin, point.x - bounds.left),
      Math.max(margin, bounds.width - menuWidth - margin),
    );
    const top = Math.min(
      Math.max(margin, point.y - bounds.top),
      Math.max(margin, bounds.height - menuHeight - margin),
    );
    setQuickCreate({
      canvasPosition: instance.screenToFlowPosition({ x: point.x, y: point.y }),
      left,
      sourceId: connectionState.fromNode.id,
      top,
    });
  }

  function handleKeyDown(event: KeyboardEvent) {
    const command = workflowCanvasCommand(event, {
      disabled,
      readOnly: canvasReadOnly,
    });
    if (!command) return;

    event.preventDefault();
    if (command === "undo") return undo();
    if (command === "redo") return redo();
    if (command === "selectTool") return setTool("select");
    if (command === "panTool") return setTool("pan");
    if (command === "openNodePicker") {
      setQuickCreate(null);
      return setNodePickerOpen(true);
    }
    if (command === "fitView") return fitCanvas();
    if (command === "zoomIn") return void instanceRef.current?.zoomIn();
    if (command === "zoomOut") return void instanceRef.current?.zoomOut();
    if (command === "deselect") {
      setNodePickerOpen(false);
      setQuickCreate(null);
      setSelectedEdgeId(null);
      onSelectConnection?.(null);
      onSelectNode(null);
      return;
    }
    if (command === "deleteSelection") {
      if (disconnectSelectedEdge()) {
        return;
      }
      if (selectedNodeId) {
        const node = selections.find((item) => item.id === selectedNodeId);
        if (node && node.kind !== "output") {
          removeNode(node.id);
        }
      }
    }
  }

  keyDownHandlerRef.current = handleKeyDown;

  useEffect(() => {
    function handleWindowKeyDown(event: KeyboardEvent) {
      if (isWorkflowCanvasTextEditingTarget(event.target)) return;
      if (isWorkflowCanvasTemporaryPanKey(event)) {
        if (isSpaceActivatedControl(event.target)) return;
        event.preventDefault();
        setSpacePanning(true);
        return;
      }
      if (event.defaultPrevented) return;
      keyDownHandlerRef.current(event);
    }

    function stopTemporaryPan(event: KeyboardEvent) {
      if (isWorkflowCanvasSpaceKey(event)) setSpacePanning(false);
    }

    function stopTemporaryPanOnBlur() {
      setSpacePanning(false);
    }

    window.addEventListener("keydown", handleWindowKeyDown);
    window.addEventListener("keyup", stopTemporaryPan);
    window.addEventListener("blur", stopTemporaryPanOnBlur);
    return () => {
      window.removeEventListener("keydown", handleWindowKeyDown);
      window.removeEventListener("keyup", stopTemporaryPan);
      window.removeEventListener("blur", stopTemporaryPanOnBlur);
    };
  }, []);

  function fitCanvas() {
    void instanceRef.current?.fitView({
      duration: 180,
      padding: 0.18,
    });
  }

  return (
    <section
      aria-label="Flow 交互画布"
      className={`workflow-graph workflow-graph--${activeTool}`}
      ref={canvasRef}
      tabIndex={0}
    >
      <ReactFlow<WorkflowCanvasNodeType>
        autoPanOnConnect
        autoPanOnNodeDrag={false}
        connectOnClick
        deleteKeyCode={null}
        edges={edges}
        elementsSelectable
        defaultViewport={layout.viewport ?? DEFAULT_VIEWPORT}
        isValidConnection={(connection) =>
          canConnectWorkflowNodes(
            selections,
            connection.source,
            connection.target,
          )
        }
        maxZoom={1.8}
        minZoom={0.3}
        nodeTypes={nodeTypes}
        defaultNodes={initialFlowNodesRef.current}
        nodesConnectable={!canvasReadOnly && !disabled}
        nodesDraggable={activeTool === "select"}
        onConnect={connect}
        onConnectEnd={handleConnectEnd}
        onEdgeClick={(event, edge) => {
          event.stopPropagation();
          setQuickCreate(null);
          setSelectedEdgeId(edge.id);
          onSelectNode(null);
          onSelectConnection?.(
            canvasModel.connections.find(
              (candidate) =>
                (candidate.id ??
                  edgeId(candidate.sourceId, candidate.targetId)) === edge.id,
            ) ?? null,
          );
        }}
        onInit={(instance) => {
          instanceRef.current = instance;
          instance.setNodes((current) =>
            reconcileWorkflowCanvasNodes(current, flowNodes, true),
          );
          syncedPositionsRef.current = layout.positions;
        }}
        onMoveEnd={(_event, viewport) =>
          setLayout((current) => ({ ...current, viewport }))
        }
        onNodeClick={(_event, node) => {
          setQuickCreate(null);
          setSelectedEdgeId(null);
          onSelectConnection?.(null);
          onSelectNode(node.id);
        }}
        onNodeDragStart={(_event, node) => {
          dragStartPositions.current = { ...layout.positions };
          setSelectedEdgeId(null);
          onSelectConnection?.(null);
          onSelectNode(node.id);
        }}
        onNodeDragStop={(_event, node) => {
          const before = dragStartPositions.current;
          dragStartPositions.current = null;
          setLayout((current) => ({
            ...current,
            positions: { ...current.positions, [node.id]: node.position },
          }));
          if (
            before &&
            (before[node.id]?.x !== node.position.x ||
              before[node.id]?.y !== node.position.y)
          ) {
            pushPast(snapshot({ ...layout, positions: before }));
          }
        }}
        onNodesChange={handleNodesChange}
        onPaneClick={() => {
          canvasRef.current?.focus({ preventScroll: true });
          setNodePickerOpen(false);
          setQuickCreate(null);
          setSelectedEdgeId(null);
          onSelectConnection?.(null);
          onSelectNode(null);
        }}
        onReconnect={reconnect}
        panActivationKeyCode="Space"
        panOnDrag={activeTool === "pan" ? true : [1]}
        panOnScroll
        selectionMode={SelectionMode.Partial}
        selectionOnDrag={false}
        zoomActivationKeyCode={["Meta", "Control"]}
        zoomOnDoubleClick={false}
        zoomOnScroll={false}
      >
        <Background
          color="var(--border)"
          gap={24}
          size={1}
          variant={BackgroundVariant.Dots}
        />
        <WorkflowCanvasToolbar
          canRedo={history.current.future.length > 0}
          canUndo={history.current.past.length > 0}
          disabled={disabled}
          disableAgent={false}
          nodePickerOpen={nodePickerOpen}
          onAdd={addNode}
          onFitView={fitCanvas}
          onNodePickerOpenChange={setNodePickerOpen}
          onRedo={redo}
          onToolChange={setTool}
          onUndo={undo}
          onZoomIn={() => void instanceRef.current?.zoomIn()}
          onZoomOut={() => void instanceRef.current?.zoomOut()}
          readOnly={canvasReadOnly}
          tool={activeTool}
        />
      </ReactFlow>

      {!canvasReadOnly && selections.length === 0 ? (
        <div className="workflow-graph__empty" role="status">
          <span className="workflow-graph__empty-icon">
            <Bot aria-hidden="true" size={18} />
          </span>
          <strong>从第一个节点开始</strong>
          <p>添加 Agent 节点后，再在右侧选择具体 Agent 和版本。</p>
          <Button
            disabled={disabled}
            onClick={() => addNode("agent")}
            size="compact"
            variant="primary"
          >
            <Bot aria-hidden="true" size={14} /> 添加 Agent 节点
          </Button>
        </div>
      ) : null}

      {quickCreate ? (
        <WorkflowCanvasQuickCreate
          disableAgent={false}
          onAdd={addNode}
          state={quickCreate}
        />
      ) : null}
    </section>
  );
}

function isWorkflowCanvasTextEditingTarget(target: EventTarget | null) {
  return (
    target instanceof Element &&
    Boolean(
      target.closest(
        'input, textarea, select, [contenteditable]:not([contenteditable="false"])',
      ),
    )
  );
}

function isSpaceActivatedControl(target: EventTarget | null) {
  return (
    target instanceof Element &&
    Boolean(target.closest('button, a[href], summary, [role="button"]'))
  );
}

function edgeId(sourceId: string, targetId: string) {
  return `${sourceId}:${targetId}`;
}

function sameConnections(
  current: readonly WorkflowNodeSelection[],
  next: readonly WorkflowNodeSelection[],
) {
  return (
    JSON.stringify(workflowConnections(current)) ===
    JSON.stringify(workflowConnections(next))
  );
}

function sameSelections(
  current: readonly WorkflowNodeSelection[],
  next: readonly WorkflowNodeSelection[],
) {
  return JSON.stringify(current) === JSON.stringify(next);
}

function pointerPosition(event: MouseEvent | TouchEvent) {
  if (event instanceof MouseEvent)
    return { x: event.clientX, y: event.clientY };
  const touch = event.changedTouches[0];
  return touch ? { x: touch.clientX, y: touch.clientY } : null;
}

function cssNumber(
  styles: CSSStyleDeclaration,
  name: string,
  fallback: number,
) {
  const value = Number.parseFloat(styles.getPropertyValue(name));
  return Number.isFinite(value) ? value : fallback;
}

function workflowRunStatuses(testRun: FlowRun | null) {
  const statuses = new Map<string, WorkflowCanvasRunStatus>();
  const attempts = new Map<string, number>();
  if (!testRun) return statuses;
  for (const nodeRun of testRun.nodeRuns) {
    if ((attempts.get(nodeRun.nodeId) ?? -1) > nodeRun.attempt) continue;
    attempts.set(nodeRun.nodeId, nodeRun.attempt);
    statuses.set(nodeRun.nodeId, nodeRun.status);
  }
  for (const nodeId of testRun.readyNodes) {
    if (!statuses.has(nodeId)) statuses.set(nodeId, "ready");
  }
  return statuses;
}

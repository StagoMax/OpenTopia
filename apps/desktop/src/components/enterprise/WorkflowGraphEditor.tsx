import {
  Background,
  BackgroundVariant,
  ReactFlow,
  SelectionMode,
  applyNodeChanges,
  type Connection,
  type Edge,
  type FinalConnectionState,
  type NodeChange,
  type NodeTypes,
  type ReactFlowInstance,
  type XYPosition,
} from "@xyflow/react";
import "@xyflow/react/dist/base.css";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import type { AgentTemplateVersionView, FlowSpec } from "../../types";
import { createFinalActivation } from "./flowActivation";
import {
  canConnectWorkflowNodes,
  connectWorkflowNodes,
  disconnectWorkflowNodes,
  workflowConnections,
} from "./workflowGraphOperations";
import {
  readWorkflowCanvasLayout,
  reconcileWorkflowPositions,
  writeWorkflowCanvasLayout,
  type WorkflowCanvasLayout,
} from "./workflowCanvasLayout";
import { workflowCanvasCommand } from "./workflowCanvasShortcuts";
import {
  WorkflowCanvasQuickCreate,
  WorkflowCanvasToolbar,
  type CanvasTool,
  type QuickCreateState,
} from "./WorkflowCanvasControls";
import {
  WorkflowCanvasNode,
  type WorkflowCanvasNodeType,
} from "./WorkflowCanvasNode";
import {
  compiledWorkflowCanvasModel,
  editableWorkflowCanvasModel,
} from "./workflowCanvasModel";
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
  readOnly = false,
  selections = EMPTY_SELECTIONS,
  selectedNodeId,
  templates,
}: {
  compiledGraph?: FlowSpec["graph"];
  disabled?: boolean;
  layoutId: string;
  onChange?(selections: WorkflowNodeSelection[]): void;
  onEditTrigger?(nodeId: string): void;
  onSelectNode(nodeId: string | null): void;
  readOnly?: boolean;
  selections?: WorkflowNodeSelection[];
  selectedNodeId: string | null;
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
  const history = useRef<{
    future: CanvasSnapshot[];
    past: CanvasSnapshot[];
  }>({ future: [], past: [] });
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
  const [tool, setTool] = useState<CanvasTool>(
    canvasReadOnly ? "pan" : "select",
  );

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
    history.current = { future: [], past: [] };
    setHistoryVersion((value) => value + 1);
    const frame = window.requestAnimationFrame(() => {
      const instance = instanceRef.current;
      if (!instance) return;
      if (next.viewport) void instance.setViewport(next.viewport);
      else if (compiledGraph)
        void instance.fitView({ maxZoom: 1, minZoom: 0.3, padding: 0.12 });
      else void instance.setViewport(DEFAULT_VIEWPORT);
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
      draggable: !canvasReadOnly && !disabled && tool === "select",
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
          onSelectNode(nodeId);
        },
        readOnly: canvasReadOnly,
        subtitle: node.subtitle,
      },
    };
  });

  const edges: Edge[] = canvasModel.connections.map((edge) => ({
    id: edge.id ?? edgeId(edge.sourceId, edge.targetId),
    source: edge.sourceId,
    sourceHandle: "final",
    target: edge.targetId,
    targetHandle: "input",
    type: "smoothstep",
    className: "workflow-canvas__edge",
    selected:
      selectedEdgeId === (edge.id ?? edgeId(edge.sourceId, edge.targetId)),
    selectable: true,
    deletable: !canvasReadOnly && !disabled,
    reconnectable: !canvasReadOnly && !disabled,
  }));

  function snapshot(
    snapshotLayout: WorkflowCanvasLayout = layout,
  ): CanvasSnapshot {
    return {
      layout: {
        ...snapshotLayout,
        positions: { ...snapshotLayout.positions },
      },
      selections: selections.map((node) => ({ ...node })),
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
  }

  function addNode(
    kind: AddableWorkflowNodeKind,
    preferredPosition?: XYPosition,
    sourceId?: string,
  ) {
    const next = addWorkflowNode(selections, kind, templates[0]);
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
    setQuickCreate(null);
  }

  function removeNode(nodeId: string) {
    const next = removeWorkflowNode(selections, nodeId);
    if (next.length === selections.length) return;
    commitSemanticChange(next);
    onSelectNode(null);
  }

  function connect(connection: Connection) {
    if (!connection.source || !connection.target) return;
    const next = connectWorkflowNodes(
      selections,
      connection.source,
      connection.target,
    );
    if (sameConnections(selections, next)) return;
    commitSemanticChange(next);
    onSelectNode(connection.target);
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
    commitSemanticChange(
      connectWorkflowNodes(disconnected, connection.source, connection.target),
    );
    onSelectNode(connection.target);
  }

  function disconnectSelectedEdge() {
    if (!selectedEdgeId) return false;
    const edge = edges.find((candidate) => candidate.id === selectedEdgeId);
    if (!edge) return false;
    commitSemanticChange(
      disconnectWorkflowNodes(selections, edge.source, edge.target),
    );
    setSelectedEdgeId(null);
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
    const positionChanges = changes.filter(
      (change) => change.type === "position",
    );
    if (positionChanges.length === 0) return;
    const moved = applyNodeChanges(positionChanges, flowNodes);
    setLayout((current) => ({
      ...current,
      positions: {
        ...current.positions,
        ...Object.fromEntries(moved.map((node) => [node.id, node.position])),
      },
    }));
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
    const menuHeight = cssNumber(styles, "--control-height-lg", 36) * 5;
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

  function handleKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    const target = event.target as HTMLElement;
    const isEditing =
      target.matches("input, textarea, select") || target.isContentEditable;
    if (isEditing) return;

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

  function fitCanvas() {
    void instanceRef.current?.fitView({
      duration: 180,
      padding: 0.18,
    });
  }

  return (
    <section
      aria-label="Flow 交互画布"
      className={`workflow-graph workflow-graph--${tool}`}
      onKeyDownCapture={handleKeyDown}
      ref={canvasRef}
      tabIndex={0}
    >
      <ReactFlow<WorkflowCanvasNodeType>
        autoPanOnConnect
        autoPanOnNodeDrag
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
        nodes={flowNodes}
        nodesConnectable={!canvasReadOnly && !disabled}
        nodesDraggable={!canvasReadOnly && !disabled && tool === "select"}
        onConnect={connect}
        onConnectEnd={handleConnectEnd}
        onEdgeClick={(event, edge) => {
          event.stopPropagation();
          setQuickCreate(null);
          setSelectedEdgeId(edge.id);
          onSelectNode(null);
        }}
        onInit={(instance) => {
          instanceRef.current = instance;
        }}
        onMoveEnd={(_event, viewport) =>
          setLayout((current) => ({ ...current, viewport }))
        }
        onNodeClick={(_event, node) => {
          setQuickCreate(null);
          setSelectedEdgeId(null);
          onSelectNode(node.id);
        }}
        onNodeDragStart={(_event, node) => {
          dragStartPositions.current = { ...layout.positions };
          setSelectedEdgeId(null);
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
          onSelectNode(null);
        }}
        onReconnect={reconnect}
        panActivationKeyCode="Space"
        panOnDrag={tool === "pan" ? true : [1]}
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
          disableAgent={templates.length === 0}
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
          tool={tool}
        />
      </ReactFlow>

      {quickCreate ? (
        <WorkflowCanvasQuickCreate
          disableAgent={templates.length === 0}
          onAdd={addNode}
          state={quickCreate}
        />
      ) : null}
    </section>
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

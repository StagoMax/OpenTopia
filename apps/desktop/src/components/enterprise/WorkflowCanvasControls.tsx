import { Panel, type XYPosition } from "@xyflow/react";
import {
  Bot,
  Hand,
  Minus,
  MousePointer2,
  Plus,
  Redo2,
  Scan,
  ShieldCheck,
  Undo2,
} from "lucide-react";
import type { ReactNode } from "react";
import {
  Button,
  IconButton,
  Popover,
  Tooltip,
  type TooltipPlacement,
  type TooltipTriggerProps,
} from "../ui";
import {
  workflowCanvasAriaShortcuts,
  workflowCanvasShortcutLabels,
} from "./workflowCanvasShortcuts";
import type { AddableWorkflowNodeKind } from "./workflowNodeSelection";

export type CanvasTool = "select" | "pan";

export type QuickCreateState = {
  canvasPosition: XYPosition;
  left: number;
  sourceId: string;
  top: number;
};

export function WorkflowCanvasToolbar({
  canRedo,
  canUndo,
  disabled,
  disableAgent,
  onAdd,
  onFitView,
  onNodePickerOpenChange,
  onRedo,
  onToolChange,
  onUndo,
  onZoomIn,
  onZoomOut,
  nodePickerOpen,
  readOnly,
  tool,
}: {
  canRedo: boolean;
  canUndo: boolean;
  disabled: boolean;
  disableAgent: boolean;
  onAdd(kind: AddableWorkflowNodeKind): void;
  onFitView(): void;
  onNodePickerOpenChange(open: boolean): void;
  onRedo(): void;
  onToolChange(tool: CanvasTool): void;
  onUndo(): void;
  onZoomIn(): void;
  onZoomOut(): void;
  nodePickerOpen: boolean;
  readOnly: boolean;
  tool: CanvasTool;
}) {
  return (
    <>
      <Panel className="workflow-canvas-toolbar nopan" position="top-left">
        <div aria-label="画布工具" role="toolbar">
          <CanvasActionTooltip
            label="选择工具"
            shortcut={workflowCanvasShortcutLabels.selectTool}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.selectTool}
                aria-label="选择工具"
                aria-pressed={tool === "select"}
                onClick={() => onToolChange("select")}
                size="compact"
                variant={tool === "select" ? "secondary" : "quiet"}
              >
                <MousePointer2 aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          <CanvasActionTooltip
            label="抓手工具（按住 Space 临时平移）"
            shortcut={workflowCanvasShortcutLabels.panTool}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.panTool}
                aria-label="抓手工具"
                aria-pressed={tool === "pan"}
                onClick={() => onToolChange("pan")}
                size="compact"
                variant={tool === "pan" ? "secondary" : "quiet"}
              >
                <Hand aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          {!readOnly ? (
            <CanvasActionTooltip
              label="新建节点"
              placement="top"
              shortcut={workflowCanvasShortcutLabels.openNodePicker}
            >
              {(tooltipProps) => (
                <Popover
                  align="start"
                  autoFocus
                  label="选择要添加的节点类型"
                  onOpenChange={onNodePickerOpenChange}
                  open={nodePickerOpen}
                  placement="bottom"
                  trigger={(popoverProps) => (
                    <Button
                      {...tooltipProps}
                      {...popoverProps}
                      aria-keyshortcuts={
                        workflowCanvasAriaShortcuts.openNodePicker
                      }
                      aria-label="新建节点"
                      className="workflow-canvas-toolbar__add"
                      disabled={disabled}
                      ref={(node) => {
                        tooltipProps.ref(node);
                        popoverProps.ref(node);
                      }}
                      size="compact"
                      variant="secondary"
                    >
                      <Plus aria-hidden="true" size={14} /> 节点
                    </Button>
                  )}
                >
                  {({ close }) => (
                    <WorkflowNodePicker
                      disableAgent={disableAgent}
                      onAdd={(kind) => {
                        onAdd(kind);
                        close();
                      }}
                    />
                  )}
                </Popover>
              )}
            </CanvasActionTooltip>
          ) : null}
          {!readOnly ? (
            <>
              <span
                aria-hidden="true"
                className="workflow-canvas-toolbar__divider"
              />
              <CanvasActionTooltip
                label="撤销"
                shortcut={workflowCanvasShortcutLabels.undo}
              >
                {(props) => (
                  <IconButton
                    {...props}
                    aria-keyshortcuts={workflowCanvasAriaShortcuts.undo}
                    aria-label="撤销"
                    disabled={!canUndo}
                    onClick={onUndo}
                    size="compact"
                    variant="quiet"
                  >
                    <Undo2 aria-hidden="true" size={15} />
                  </IconButton>
                )}
              </CanvasActionTooltip>
              <CanvasActionTooltip
                label="重做"
                shortcut={workflowCanvasShortcutLabels.redo}
              >
                {(props) => (
                  <IconButton
                    {...props}
                    aria-keyshortcuts={workflowCanvasAriaShortcuts.redo}
                    aria-label="重做"
                    disabled={!canRedo}
                    onClick={onRedo}
                    size="compact"
                    variant="quiet"
                  >
                    <Redo2 aria-hidden="true" size={15} />
                  </IconButton>
                )}
              </CanvasActionTooltip>
            </>
          ) : null}
        </div>
      </Panel>
      <Panel className="workflow-canvas-controls nopan" position="bottom-left">
        <div aria-label="画布缩放" role="toolbar">
          <CanvasActionTooltip
            label="缩小画布"
            placement="top"
            shortcut={workflowCanvasShortcutLabels.zoomOut}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.zoomOut}
                aria-label="缩小画布"
                onClick={onZoomOut}
                size="compact"
                variant="quiet"
              >
                <Minus aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          <CanvasActionTooltip
            label="适应全部节点"
            placement="top"
            shortcut={workflowCanvasShortcutLabels.fitView}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.fitView}
                aria-label="适应全部节点"
                onClick={onFitView}
                size="compact"
                variant="quiet"
              >
                <Scan aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          <CanvasActionTooltip
            label="放大画布"
            placement="top"
            shortcut={workflowCanvasShortcutLabels.zoomIn}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.zoomIn}
                aria-label="放大画布"
                onClick={onZoomIn}
                size="compact"
                variant="quiet"
              >
                <Plus aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
        </div>
      </Panel>
    </>
  );
}

function CanvasActionTooltip({
  children,
  label,
  placement = "bottom",
  shortcut,
}: {
  children(props: TooltipTriggerProps): ReactNode;
  label: string;
  placement?: TooltipPlacement;
  shortcut: string;
}) {
  return (
    <Tooltip
      content={
        <span className="workflow-canvas-tooltip">
          <span>{label}</span>
          <kbd>{shortcut}</kbd>
        </span>
      }
      placement={placement}
    >
      {children}
    </Tooltip>
  );
}

export function WorkflowCanvasQuickCreate({
  disableAgent,
  onAdd,
  state,
}: {
  disableAgent: boolean;
  onAdd(
    kind: AddableWorkflowNodeKind,
    position: XYPosition,
    sourceId: string,
  ): void;
  state: QuickCreateState;
}) {
  return (
    <div
      aria-label="连接到新节点"
      className="workflow-canvas-quick-create nodrag nopan"
      role="dialog"
      style={{ left: state.left, top: state.top }}
    >
      <header>
        <strong>连接到新节点</strong>
        <small>创建后会自动完成连线</small>
      </header>
      <WorkflowNodePicker
        disableAgent={disableAgent}
        onAdd={(kind) => onAdd(kind, state.canvasPosition, state.sourceId)}
      />
    </div>
  );
}

function WorkflowNodePicker({
  disableAgent,
  onAdd,
}: {
  disableAgent: boolean;
  onAdd(kind: AddableWorkflowNodeKind): void;
}) {
  return (
    <div className="workflow-node-picker">
      <button
        disabled={disableAgent}
        onClick={() => onAdd("agent")}
        type="button"
      >
        <Bot aria-hidden="true" size={16} />
        <span>
          <strong>Agent</strong>
          <small>运行一个已发布的 Agent 模板</small>
        </span>
      </button>
      <button onClick={() => onAdd("approval")} type="button">
        <ShieldCheck aria-hidden="true" size={16} />
        <span>
          <strong>Approval</strong>
          <small>暂停流程并等待人工审批</small>
        </span>
      </button>
    </div>
  );
}

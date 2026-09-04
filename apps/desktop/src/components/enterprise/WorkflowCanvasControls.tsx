import { Panel, type XYPosition } from "@xyflow/react";
import {
  BadgeCheck,
  Bot,
  Hand,
  Merge,
  Minus,
  MousePointer2,
  Plus,
  Redo2,
  Scan,
  Settings2,
  ShieldCheck,
  Undo2,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import {
  Button,
  DisclosureSummary,
  IconButton,
  Tooltip,
  type TooltipPlacement,
  type TooltipTriggerProps,
} from "../ui";
import {
  workflowCanvasAriaShortcuts,
  workflowCanvasShortcutLabels,
} from "./workflowCanvasShortcuts";
import type { AddableWorkflowNodeKind } from "./workflowNodeSelection";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

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
  const { t } = useApplicationLanguage();
  return (
    <>
      <Panel className="workflow-canvas-toolbar nopan" position="top-left">
        <div aria-label={t("flow.canvas.tools")} role="toolbar">
          <CanvasActionTooltip
            label={t("flow.canvas.selectTool")}
            shortcut={workflowCanvasShortcutLabels.selectTool}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.selectTool}
                aria-label={t("flow.canvas.selectTool")}
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
            label={t("flow.canvas.panTool")}
            shortcut={workflowCanvasShortcutLabels.panTool}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.panTool}
                aria-label={t("flow.canvas.panToolAria")}
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
              label={t("flow.canvas.newNode")}
              placement="top"
              shortcut={workflowCanvasShortcutLabels.openNodePicker}
            >
              {(tooltipProps) => (
                <Button
                  {...tooltipProps}
                  aria-controls="workflow-canvas-node-menu"
                  aria-expanded={nodePickerOpen}
                  aria-haspopup="dialog"
                  aria-keyshortcuts={workflowCanvasAriaShortcuts.openNodePicker}
                  aria-label={t("flow.canvas.newNode")}
                  className="workflow-canvas-toolbar__add"
                  disabled={disabled}
                  onClick={() => onNodePickerOpenChange(!nodePickerOpen)}
                  size="compact"
                  variant="secondary"
                >
                  <Plus aria-hidden="true" size={14} />
                  {t("flow.canvas.node")}
                </Button>
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
                label={t("flow.canvas.undo")}
                shortcut={workflowCanvasShortcutLabels.undo}
              >
                {(props) => (
                  <IconButton
                    {...props}
                    aria-keyshortcuts={workflowCanvasAriaShortcuts.undo}
                    aria-label={t("flow.canvas.undo")}
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
                label={t("flow.canvas.redo")}
                shortcut={workflowCanvasShortcutLabels.redo}
              >
                {(props) => (
                  <IconButton
                    {...props}
                    aria-keyshortcuts={workflowCanvasAriaShortcuts.redo}
                    aria-label={t("flow.canvas.redo")}
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
      {nodePickerOpen && !readOnly ? (
        <Panel className="workflow-canvas-node-menu nopan" position="top-left">
          <div
            aria-label={t("flow.canvas.chooseNode")}
            id="workflow-canvas-node-menu"
            role="dialog"
          >
            <WorkflowNodePicker
              autoFocus
              disableAgent={disableAgent}
              onAdd={(kind) => {
                onAdd(kind);
                onNodePickerOpenChange(false);
              }}
            />
          </div>
        </Panel>
      ) : null}
      <Panel className="workflow-canvas-controls nopan" position="bottom-left">
        <div aria-label={t("flow.canvas.zoomTools")} role="toolbar">
          <CanvasActionTooltip
            label={t("flow.canvas.zoomOut")}
            placement="top"
            shortcut={workflowCanvasShortcutLabels.zoomOut}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.zoomOut}
                aria-label={t("flow.canvas.zoomOut")}
                onClick={onZoomOut}
                size="compact"
                variant="quiet"
              >
                <Minus aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          <CanvasActionTooltip
            label={t("flow.canvas.fit")}
            placement="top"
            shortcut={workflowCanvasShortcutLabels.fitView}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.fitView}
                aria-label={t("flow.canvas.fit")}
                onClick={onFitView}
                size="compact"
                variant="quiet"
              >
                <Scan aria-hidden="true" size={16} />
              </IconButton>
            )}
          </CanvasActionTooltip>
          <CanvasActionTooltip
            label={t("flow.canvas.zoomIn")}
            placement="top"
            shortcut={workflowCanvasShortcutLabels.zoomIn}
          >
            {(props) => (
              <IconButton
                {...props}
                aria-keyshortcuts={workflowCanvasAriaShortcuts.zoomIn}
                aria-label={t("flow.canvas.zoomIn")}
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
  const { t } = useApplicationLanguage();
  return (
    <div
      aria-label={t("flow.canvas.connectNew")}
      className="workflow-canvas-quick-create nodrag nopan"
      role="dialog"
      style={{ left: state.left, top: state.top }}
    >
      <header>
        <strong>{t("flow.canvas.connectNew")}</strong>
        <small>{t("flow.canvas.connectNewHint")}</small>
      </header>
      <WorkflowNodePicker
        disableAgent={disableAgent}
        onAdd={(kind) => onAdd(kind, state.canvasPosition, state.sourceId)}
      />
    </div>
  );
}

function WorkflowNodePicker({
  autoFocus = false,
  disableAgent,
  onAdd,
}: {
  autoFocus?: boolean;
  disableAgent: boolean;
  onAdd(kind: AddableWorkflowNodeKind): void;
}) {
  const { t } = useApplicationLanguage();
  const primaryItems: WorkflowNodePickerItem[] = [
    {
      kind: "agent",
      label: t("flow.node.kind.agent"),
      description: t("flow.canvas.agentDescription"),
      icon: Bot,
      disabled: disableAgent,
    },
    {
      kind: "tool",
      label: t("flow.node.kind.action"),
      description: t("flow.canvas.actionDescription"),
      icon: Wrench,
    },
    {
      kind: "approval",
      label: t("flow.canvas.approval"),
      description: t("flow.canvas.approvalDescription"),
      icon: ShieldCheck,
    },
  ];
  const advancedItems: WorkflowNodePickerItem[] = [
    {
      kind: "validator",
      label: t("flow.canvas.validator"),
      description: t("flow.canvas.validatorDescription"),
      icon: BadgeCheck,
    },
    {
      kind: "join",
      label: t("flow.canvas.join"),
      description: t("flow.canvas.joinDescription"),
      icon: Merge,
    },
  ];
  return (
    <div className="workflow-node-picker">
      <section>
        <small className="workflow-node-picker__group">
          {t("flow.canvas.primaryNodes")}
        </small>
        {primaryItems.map((item) => (
          <WorkflowNodePickerButton
            autoFocus={
              autoFocus &&
              ((item.kind === "agent" && !disableAgent) ||
                (item.kind === "tool" && disableAgent))
            }
            item={item}
            key={item.kind}
            onAdd={onAdd}
          />
        ))}
      </section>
      <details className="workflow-node-picker__advanced">
        <DisclosureSummary icon={<Settings2 aria-hidden="true" size={14} />}>
          {t("flow.canvas.advancedControls")}
        </DisclosureSummary>
        <section>
          {advancedItems.map((item) => (
            <WorkflowNodePickerButton
              item={item}
              key={item.kind}
              onAdd={onAdd}
            />
          ))}
        </section>
      </details>
      <p className="workflow-node-picker__hint">
        {t("flow.canvas.pickerHint")}
      </p>
    </div>
  );
}

type WorkflowNodePickerItem = {
  description: string;
  disabled?: boolean;
  icon: LucideIcon;
  kind: AddableWorkflowNodeKind;
  label: string;
};

function WorkflowNodePickerButton({
  autoFocus,
  item,
  onAdd,
}: {
  autoFocus?: boolean;
  item: WorkflowNodePickerItem;
  onAdd(kind: AddableWorkflowNodeKind): void;
}) {
  const Icon = item.icon;
  return (
    <button
      autoFocus={autoFocus}
      className="workflow-node-picker__item"
      disabled={item.disabled}
      onClick={() => onAdd(item.kind)}
      type="button"
    >
      <Icon aria-hidden="true" size={16} />
      <span>
        <strong>{item.label}</strong>
        <small>{item.description}</small>
      </span>
    </button>
  );
}

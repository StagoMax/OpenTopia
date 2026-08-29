import { Workflow, X } from "lucide-react";
import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { Button, IconButton, TextField } from "../ui";

export type FlowCreateValues = {
  flowId: string;
  name: string;
  owner: string;
  outcome: string;
};

export function FlowCreateDialog({
  onCancel,
  onChange,
  onSubmit,
  open,
  values,
}: {
  onCancel(): void;
  onChange(values: FlowCreateValues): void;
  onSubmit(): void;
  open: boolean;
  values: FlowCreateValues;
}) {
  const onCancelRef = useRef(onCancel);
  const dialogRef = useRef<HTMLElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const formId = useId();
  const titleId = useId();
  onCancelRef.current = onCancel;

  useEffect(() => {
    if (!open) return undefined;
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancelRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'input, textarea, button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable?.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previousFocusRef.current?.focus();
    };
  }, [open]);

  if (!open) return null;
  const complete =
    values.flowId.trim() &&
    values.name.trim() &&
    values.owner.trim() &&
    values.outcome.trim();

  return createPortal(
    <div
      className="flow-create-dialog__backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
      role="presentation"
    >
      <section
        aria-labelledby={titleId}
        aria-modal="true"
        className="flow-create-dialog"
        ref={dialogRef}
        role="dialog"
      >
        <header>
          <span className="flow-create-dialog__heading">
            <span className="flow-create-dialog__icon">
              <Workflow aria-hidden="true" size={18} />
            </span>
            <span>
              <strong id={titleId}>创建 Flow</strong>
              <small>先定义 Flow 的基本信息，随后进入节点画布。</small>
            </span>
          </span>
          <IconButton
            aria-label="关闭创建 Flow 窗口"
            onClick={onCancel}
            size="compact"
            variant="quiet"
          >
            <X aria-hidden="true" size={15} />
          </IconButton>
        </header>
        <form
          id={formId}
          onSubmit={(event) => {
            event.preventDefault();
            if (complete) onSubmit();
          }}
        >
          <div className="flow-create-dialog__grid">
            <TextField
              autoFocus
              label="Workflow ID"
              onChange={(event) =>
                onChange({ ...values, flowId: event.target.value })
              }
              required
              value={values.flowId}
            />
            <TextField
              label="名称"
              onChange={(event) =>
                onChange({ ...values, name: event.target.value })
              }
              required
              value={values.name}
            />
            <TextField
              label="所有者"
              onChange={(event) =>
                onChange({ ...values, owner: event.target.value })
              }
              required
              value={values.owner}
              wrapperClassName="flow-create-dialog__wide"
            />
            <label className="flow-create-dialog__outcome">
              <span>希望完成的业务结果</span>
              <textarea
                onChange={(event) =>
                  onChange({ ...values, outcome: event.target.value })
                }
                placeholder="例如：读取新客户资料，检查必填字段并生成一份可供销售复核的摘要。"
                required
                rows={4}
                value={values.outcome}
              />
              <small>
                使用自然语言描述目标，节点细节可以进入画布后继续配置。
              </small>
            </label>
          </div>
        </form>
        <footer>
          <Button onClick={onCancel} variant="quiet">
            取消
          </Button>
          <Button
            disabled={!complete}
            form={formId}
            type="submit"
            variant="primary"
          >
            创建并进入画布
          </Button>
        </footer>
      </section>
    </div>,
    document.body,
  );
}

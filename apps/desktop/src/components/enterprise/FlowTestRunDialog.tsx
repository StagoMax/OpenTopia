import { AlertTriangle, Braces, FlaskConical, X } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button, IconButton } from "../ui";
import { parseFlowTestInput } from "./flowTestInput";
import "./flow-test-run-dialog.css";

export function FlowTestRunDialog({
  busy,
  executionSteps,
  externalError,
  inputSchema,
  inputText,
  onCancel,
  onChangeInput,
  onSubmit,
  open,
}: {
  busy: boolean;
  executionSteps: string[];
  externalError: string | null;
  inputSchema: Record<string, unknown> | undefined;
  inputText: string;
  onCancel(): void;
  onChangeInput(value: string): void;
  onSubmit(input: unknown): void;
  open: boolean;
}) {
  const [localError, setLocalError] = useState<string | null>(null);
  const busyRef = useRef(busy);
  const dialogRef = useRef<HTMLElement>(null);
  const onCancelRef = useRef(onCancel);
  const submitRef = useRef<() => void>(() => {});
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const titleId = useId();
  const inputId = useId();
  const errorId = useId();
  busyRef.current = busy;
  onCancelRef.current = onCancel;

  useEffect(() => {
    if (!open) return undefined;
    setLocalError(null);
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const frame = window.requestAnimationFrame(() =>
      textareaRef.current?.focus(),
    );

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !busyRef.current) {
        event.preventDefault();
        onCancelRef.current();
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        submitRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'textarea, button:not([disabled]), [tabindex]:not([tabindex="-1"])',
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
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
      previousFocusRef.current?.focus();
    };
  }, [open]);

  if (!open) return null;

  function submit() {
    if (busy) return;
    const parsed = parseFlowTestInput(inputText, inputSchema);
    if (!parsed.ok) {
      setLocalError(parsed.error);
      textareaRef.current?.focus();
      return;
    }
    setLocalError(null);
    onChangeInput(parsed.formatted);
    onSubmit(parsed.input);
  }
  submitRef.current = submit;

  function formatInput() {
    const parsed = parseFlowTestInput(inputText);
    if (!parsed.ok) {
      setLocalError(parsed.error);
      return;
    }
    setLocalError(null);
    onChangeInput(parsed.formatted);
  }

  const error = localError ?? externalError;

  return createPortal(
    <div
      className="flow-test-dialog__backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
      role="presentation"
    >
      <section
        aria-labelledby={titleId}
        aria-modal="true"
        className="flow-test-dialog"
        ref={dialogRef}
        role="dialog"
      >
        <header>
          <span className="flow-test-dialog__heading">
            <span className="flow-test-dialog__icon">
              <FlaskConical aria-hidden="true" size={18} />
            </span>
            <span>
              <strong id={titleId}>Test Run</strong>
              <small>用一份明确的输入真实执行当前 Flow 草稿。</small>
            </span>
          </span>
          <IconButton
            aria-label="关闭 Test Run"
            disabled={busy}
            onClick={onCancel}
            size="compact"
            variant="quiet"
          >
            <X aria-hidden="true" size={15} />
          </IconButton>
        </header>

        <label className="flow-test-dialog__input" htmlFor={inputId}>
          <span>
            <strong>测试输入</strong>
            <small>该 JSON 会作为本次运行的 @Flow.input。</small>
          </span>
          <textarea
            aria-describedby={error ? errorId : undefined}
            aria-invalid={Boolean(error)}
            id={inputId}
            onChange={(event) => {
              setLocalError(null);
              onChangeInput(event.target.value);
            }}
            ref={textareaRef}
            rows={12}
            spellCheck={false}
            value={inputText}
          />
          {error ? (
            <small
              className="flow-test-dialog__error"
              id={errorId}
              role="alert"
            >
              {error}
            </small>
          ) : (
            <small>
              可粘贴 Manual、Webhook 或 Connection Event 的样本参数。
            </small>
          )}
        </label>

        <aside className="flow-test-dialog__warning">
          <AlertTriangle aria-hidden="true" size={16} />
          <span>
            <strong>这是真实测试</strong>
            <small>
              Agent、Tool 和 Connection 会实际执行，可能访问或修改外部数据。
            </small>
          </span>
        </aside>

        {executionSteps.length > 0 ? (
          <details className="flow-test-dialog__steps">
            <summary>
              <Braces aria-hidden="true" size={14} /> 可能执行的 Agent /
              Action（
              {executionSteps.length}）
            </summary>
            <ul>
              {executionSteps.map((step, index) => (
                <li key={`${index}:${step}`}>{step}</li>
              ))}
            </ul>
          </details>
        ) : null}

        <footer>
          <Button disabled={busy} onClick={formatInput} variant="quiet">
            格式化 JSON
          </Button>
          <span>
            <Button disabled={busy} onClick={onCancel} variant="quiet">
              取消
            </Button>
            <Button disabled={busy} onClick={submit} variant="primary">
              <FlaskConical aria-hidden="true" size={14} />
              {busy ? "启动中…" : "开始 Test Run"}
            </Button>
          </span>
        </footer>
      </section>
    </div>,
    document.body,
  );
}

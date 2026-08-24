import { useEffect, useRef, useState } from "react";
import {
  AlertCircle,
  Loader2,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  X,
} from "lucide-react";
import { Button } from "../../components/ui";
import type { TurnUndoPreview, WindowsSandboxSetupStatus } from "../../types";

export type TurnUndoDialogState = {
  turnId: string;
  preview: TurnUndoPreview | null;
  loading: boolean;
  applying: boolean;
  error: string | null;
};

export type RenameTarget = {
  kind: "project" | "thread";
  id: string;
  name: string;
};

export function TurnUndoDialog({
  state,
  onConfirm,
  onClose,
}: {
  state: TurnUndoDialogState;
  onConfirm(): void;
  onClose(): void;
}) {
  const { preview } = state;
  const files = preview?.changeSet.files ?? [];

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !state.applying) onClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose, state.applying]);

  return (
    <div
      className="modal-backdrop project-modal-backdrop"
      role="presentation"
      onClick={() => {
        if (!state.applying) onClose();
      }}
    >
      <section
        className="turn-undo-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="turn-undo-dialog-title"
        aria-describedby="turn-undo-dialog-description"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <h2 id="turn-undo-dialog-title">撤销本轮修改</h2>
            <p id="turn-undo-dialog-description">
              使用当前工作区与该轮修改前后的快照进行三方合并。
            </p>
          </div>
          <button
            className="icon-button small"
            type="button"
            autoFocus
            aria-label="关闭撤销对话框"
            disabled={state.applying}
            onClick={onClose}
          >
            <X size={14} />
          </button>
        </header>

        {state.loading ? (
          <div className="turn-undo-loading" role="status">
            <Loader2 className="spin" size={16} />
            <span>正在检查当前文件与历史快照…</span>
          </div>
        ) : state.error ? (
          <div className="turn-undo-alert" role="alert">
            <AlertCircle size={16} />
            <span>{state.error}</span>
          </div>
        ) : preview ? (
          <>
            <div className="turn-undo-overview">
              <strong>{preview.changeSet.files.length} 个文件</strong>
              <span className="file-change-additions">
                +{preview.changeSet.additions}
              </span>
              <span className="file-change-deletions">
                -{preview.changeSet.deletions}
              </span>
            </div>

            {preview.conflicts.length > 0 ? (
              <div className="turn-undo-conflicts" role="alert">
                <strong>无法自动撤销</strong>
                <p>以下内容与该轮之后的修改发生冲突，工作区尚未更改。</p>
                <ul>
                  {preview.conflicts.map((conflict, index) => (
                    <li key={`${conflict.path ?? conflict.kind}-${index}`}>
                      <span>{conflict.path ?? "工作区"}</span>
                      <small>{conflict.reason}</small>
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <div className="turn-undo-file-list" aria-label="将撤销的文件">
                {files.map((file, index) => {
                  const path = file.newPath ?? file.oldPath ?? "未知文件";
                  return (
                    <div key={`${file.kind}-${path}-${index}`}>
                      <span className="turn-undo-file-kind">
                        {turnFileChangeLabel(file.kind)}
                      </span>
                      <span title={path}>{path}</span>
                      <small>
                        <span className="file-change-additions">
                          +{file.additions ?? 0}
                        </span>{" "}
                        <span className="file-change-deletions">
                          -{file.deletions ?? 0}
                        </span>
                      </small>
                    </div>
                  );
                })}
              </div>
            )}
          </>
        ) : null}

        <footer>
          <button
            className="secondary-button"
            type="button"
            disabled={state.applying}
            onClick={onClose}
          >
            取消
          </button>
          {preview?.canUndo ? (
            <button
              className="turn-undo-confirm"
              type="button"
              disabled={state.applying}
              onClick={onConfirm}
            >
              {state.applying ? (
                <Loader2 className="spin" size={14} />
              ) : (
                <RotateCcw size={14} />
              )}
              {state.applying ? "正在撤销" : "确认撤销"}
            </button>
          ) : null}
        </footer>
      </section>
    </div>
  );
}

function turnFileChangeLabel(kind: string) {
  if (kind === "added") return "新增";
  if (kind === "deleted") return "删除";
  if (kind === "renamed") return "重命名";
  return "修改";
}

export function RenameDialog({
  target,
  onSubmit,
  onClose,
}: {
  target: RenameTarget;
  onSubmit(name: string): Promise<boolean>;
  onClose(): void;
}) {
  const [name, setName] = useState(target.name);
  const [isSaving, setIsSaving] = useState(false);
  const label = target.kind === "project" ? "项目" : "任务";

  return (
    <div
      className="modal-backdrop project-modal-backdrop"
      role="presentation"
      onClick={onClose}
    >
      <form
        className="project-name-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="rename-dialog-title"
        onClick={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (!name.trim() || isSaving) return;
          setIsSaving(true);
          void onSubmit(name).finally(() => setIsSaving(false));
        }}
      >
        <header>
          <div>
            <h2 id="rename-dialog-title">重命名{label}</h2>
            <p>名称将在所有项目视图中同步更新。</p>
          </div>
          <button
            className="icon-button small"
            type="button"
            aria-label="关闭重命名弹窗"
            onClick={onClose}
          >
            <X size={14} />
          </button>
        </header>
        <input
          autoFocus
          aria-label={`${label}名称`}
          value={name}
          onChange={(event) => setName(event.target.value)}
          onFocus={(event) => event.currentTarget.select()}
        />
        <footer>
          <button className="secondary-button" type="button" onClick={onClose}>
            取消
          </button>
          <button
            className="primary-button"
            type="submit"
            disabled={!name.trim() || isSaving}
          >
            {isSaving ? "保存中..." : "保存"}
          </button>
        </footer>
      </form>
    </div>
  );
}

export function KeyboardShortcutsDialog({ onClose }: { onClose(): void }) {
  useEscapeToClose(onClose);
  const shortcuts = [
    ["新聊天", "Ctrl+N"],
    ["打开文件夹", "Ctrl+O"],
    ["关闭窗口", "Ctrl+W"],
    ["退出 ChatGPT", "Ctrl+Q"],
    ["搜索任务", "Ctrl+K"],
    ["切换侧栏", "Ctrl+B"],
    ["设置", "Ctrl+,"],
    ["打开终端", "Ctrl+`"],
    ["新建浏览器", "Ctrl+T"],
    ["打开文件", "Ctrl+P"],
    ["侧边任务", "Ctrl+Alt+S"],
    ["切换文件树", "Ctrl+Shift+E"],
  ];

  return (
    <div
      className="modal-backdrop chrome-dialog-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="chrome-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="keyboard-shortcuts-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <h2 id="keyboard-shortcuts-title">键盘快捷键</h2>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭键盘快捷键"
            title="关闭"
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </header>
        <dl className="chrome-shortcuts-list">
          {shortcuts.map(([label, shortcut]) => (
            <div key={shortcut}>
              <dt>{label}</dt>
              <dd>
                <kbd>{shortcut}</kbd>
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}

export function AboutDialog({ onClose }: { onClose(): void }) {
  useEscapeToClose(onClose);
  return (
    <div
      className="modal-backdrop chrome-dialog-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="chrome-dialog chrome-about-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="about-opentopia-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <h2 id="about-opentopia-title">OpenTopia</h2>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭关于 OpenTopia"
            title="关闭"
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </header>
        <p>本地优先的 AI 编码与工作代理。</p>
      </section>
    </div>
  );
}

function useEscapeToClose(onClose: () => void) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);
}

export function WindowsSandboxSetupDialog({
  status,
  busy,
  error,
  onSetup,
  onOpenSettings,
  onLater,
}: {
  status: WindowsSandboxSetupStatus;
  busy: boolean;
  error: string | null;
  onSetup(): void;
  onOpenSettings(): void;
  onLater(): void;
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const laterActionRef = useRef<HTMLButtonElement>(null);
  const busyRef = useRef(busy);
  const onLaterRef = useRef(onLater);

  useEffect(() => {
    busyRef.current = busy;
    onLaterRef.current = onLater;
  }, [busy, onLater]);

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const primary = primaryActionRef.current;
    if (primary && !primary.disabled) primary.focus();
    else laterActionRef.current?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busyRef.current) {
        event.preventDefault();
        onLaterRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]):not([tabindex="-1"]), [href], [tabindex]:not([tabindex="-1"])',
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
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, []);

  const unavailable = status.state === "unavailable";
  const degraded = status.state === "degraded";

  return (
    <div className="modal-backdrop chrome-dialog-backdrop" role="presentation">
      <section
        ref={dialogRef}
        className="chrome-dialog chrome-about-dialog sandbox-setup-dialog"
        role="dialog"
        aria-modal="true"
        aria-busy={busy}
        aria-labelledby="windows-sandbox-setup-title"
        aria-describedby="windows-sandbox-setup-description"
      >
        <header>
          <h2 id="windows-sandbox-setup-title">
            {degraded ? "修复 Windows 安全沙箱" : "安装 Windows 安全沙箱"}
          </h2>
          <ShieldCheck size={20} aria-hidden="true" />
        </header>
        <p id="windows-sandbox-setup-description">
          {unavailable
            ? "OpenTopia 默认使用强制沙箱，但当前安装中没有可用的 Windows 沙箱组件。"
            : degraded
              ? "检测到专用账户、凭据或离线网络规则不完整，需要修复后才能安全运行工具。"
              : "未检测到 OpenTopia Windows 安全沙箱。安装后，工具会在两个隔离的普通用户中运行，并由离线网络规则保护。"}
        </p>
        {!unavailable ? (
          <p>
            点击继续后，Windows 会弹出标准 UAC
            窗口；普通任务运行时不会重复弹出。完成配置前，需要强制沙箱的工具会保持禁用。
          </p>
        ) : null}
        {error ? (
          <div className="settings-danger-notice" role="alert">
            <ShieldAlert size={16} />
            <span>{error}</span>
          </div>
        ) : null}
        {!error && status.issues.length > 0 ? (
          <div className="settings-warning-notice" role="status">
            <ShieldAlert size={16} />
            <span>{status.issues.join("；")}</span>
          </div>
        ) : null}
        <div className="sandbox-setup-dialog__actions">
          <Button
            ref={laterActionRef}
            variant="quiet"
            disabled={busy}
            onClick={onLater}
          >
            稍后
          </Button>
          {unavailable ? (
            <Button
              ref={primaryActionRef}
              variant="primary"
              onClick={onOpenSettings}
            >
              打开权限设置
            </Button>
          ) : (
            <Button
              ref={primaryActionRef}
              variant="primary"
              disabled={busy || !status.helperAvailable}
              onClick={onSetup}
            >
              {busy ? (
                <Loader2 className="spin" size={16} aria-hidden="true" />
              ) : null}
              {busy
                ? "正在等待 Windows 授权…"
                : error
                  ? degraded
                    ? "重试修复"
                    : "重试安装"
                  : degraded
                    ? "修复配置"
                    : "安装沙箱"}
            </Button>
          )}
        </div>
      </section>
    </div>
  );
}

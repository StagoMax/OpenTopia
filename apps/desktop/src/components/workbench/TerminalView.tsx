import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronDown,
  Loader2,
  Plus,
  Square,
  TerminalSquare,
  Trash2,
} from "lucide-react";
import type {
  AgentEvent,
  TerminalEvent,
  TerminalSession,
  Thread,
} from "../../types";
import { writePendingTerminalEvents } from "../../terminalEventReplay";
import { XtermTerminal, type XtermTerminalHandle } from "../XtermTerminal";
import { Badge, Button, IconButton } from "../ui";
import {
  ArtifactReferenceList,
  buildCombinedTerminalRows,
} from "./terminalModel";

export function TerminalView({
  thread,
  events,
  terminalEvents,
  terminalSession,
  onEnsureSession,
  onWriteSession,
  onResizeSession,
  onCloseSession,
  onOpenArtifact,
}: {
  thread: Thread | null;
  events: AgentEvent[];
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  onEnsureSession(threadId: string): Promise<TerminalSession>;
  onWriteSession(threadId: string, sessionId: string, data: string): void;
  onResizeSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): void;
  onCloseSession(threadId: string, sessionId: string): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
}) {
  const xtermRef = useRef<XtermTerminalHandle | null>(null);
  const readyTerminalRef = useRef<XtermTerminalHandle | null>(null);
  const writtenTerminalEventsRef = useRef<Set<string>>(new Set());
  const lastThreadIdRef = useRef<string | null>(null);
  const inputBufferRef = useRef("");
  const inputTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const terminalRows = useMemo(
    () => buildCombinedTerminalRows(events, terminalEvents),
    [events, terminalEvents],
  );
  const inputDisabled = !thread || !terminalSession || isStartingSession;

  useEffect(() => {
    const threadId = thread?.id ?? null;
    if (lastThreadIdRef.current === threadId) return;
    lastThreadIdRef.current = threadId;
    writtenTerminalEventsRef.current = new Set();
    readyTerminalRef.current?.clear();
  }, [thread?.id]);

  useEffect(() => {
    writePendingTerminalEvents(
      terminalEvents,
      readyTerminalRef.current,
      writtenTerminalEventsRef.current,
      writeTerminalEventToXterm,
    );
  }, [terminalEvents]);

  const handleTerminalReady = useCallback(
    (terminal: XtermTerminalHandle | null) => {
      readyTerminalRef.current = terminal;
      if (!terminal) return;
      const written = new Set<string>();
      writtenTerminalEventsRef.current = written;
      terminal.clear();
      writePendingTerminalEvents(
        terminalEvents,
        terminal,
        written,
        writeTerminalEventToXterm,
      );
    },
    [terminalEvents],
  );

  const handleData = useCallback(
    (data: string) => {
      if (!thread || !terminalSession) return;
      inputBufferRef.current += data;
      if (inputTimerRef.current) return;
      inputTimerRef.current = setTimeout(() => {
        inputTimerRef.current = null;
        const pending = inputBufferRef.current;
        inputBufferRef.current = "";
        if (pending) {
          onWriteSession(thread.id, terminalSession.sessionId, pending);
        }
      }, 12);
    },
    [onWriteSession, terminalSession, thread],
  );

  const handleResize = useCallback(
    (cols: number, rows: number) => {
      if (!thread || !terminalSession) return;
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        resizeTimerRef.current = null;
        onResizeSession(thread.id, terminalSession.sessionId, cols, rows);
      }, 80);
    },
    [onResizeSession, terminalSession, thread],
  );

  useEffect(
    () => () => {
      if (inputTimerRef.current) clearTimeout(inputTimerRef.current);
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
    },
    [],
  );

  const handleRestart = useCallback(() => {
    if (!thread || isStartingSession) return;
    setIsStartingSession(true);
    void onEnsureSession(thread.id)
      .then(() => xtermRef.current?.focus())
      .finally(() => setIsStartingSession(false));
  }, [isStartingSession, onEnsureSession, thread]);

  return (
    <div className="terminal-view">
      <div className="terminal-toolbar" role="toolbar" aria-label="终端控制">
        <span
          className="terminal-session-label"
          aria-label={
            terminalSession
              ? `${terminalSession.shell}，正在运行`
              : "终端未启动"
          }
          title={terminalSession?.shell}
        >
          <TerminalSquare size={14} aria-hidden="true" />
          <strong>
            {terminalSession
              ? terminalShellName(terminalSession.shell)
              : "终端未启动"}
          </strong>
          {terminalSession && <Badge variant="success">正在运行</Badge>}
        </span>
        {terminalSession && (
          <span className="terminal-session-cwd" title={terminalSession.cwd}>
            {terminalSession.cwd}
          </span>
        )}
        <span className="terminal-toolbar-spacer" />
        {thread && terminalSession ? (
          <IconButton
            size="compact"
            variant="quiet"
            title="终止终端"
            aria-label="终止终端"
            onClick={() => onCloseSession(thread.id, terminalSession.sessionId)}
          >
            <Square size={14} aria-hidden="true" />
          </IconButton>
        ) : (
          <Button
            size="compact"
            variant="quiet"
            disabled={!thread || isStartingSession}
            onClick={handleRestart}
          >
            {isStartingSession ? (
              <Loader2 className="spin" size={14} aria-hidden="true" />
            ) : (
              <Plus size={14} aria-hidden="true" />
            )}
            {isStartingSession ? "启动中" : "新建终端"}
          </Button>
        )}
        <IconButton
          size="compact"
          variant="quiet"
          title="清空终端"
          aria-label="清空终端"
          onClick={() => xtermRef.current?.clear()}
        >
          <Trash2 size={14} aria-hidden="true" />
        </IconButton>
      </div>
      <div className="xterm-wrapper">
        <XtermTerminal
          ref={xtermRef}
          disabled={inputDisabled}
          onData={handleData}
          onReady={handleTerminalReady}
          onResize={handleResize}
        />
      </div>
      <details className="terminal-history">
        <summary>
          命令历史（{terminalRows.length}）
          <ChevronDown size={12} />
        </summary>
        <div className="terminal-screen" role="log" aria-live="polite">
          {terminalRows.length ? (
            terminalRows.map((row) => (
              <div className={`terminal-row ${row.kind}`} key={row.id}>
                <div className="terminal-row-meta">
                  <span>{row.time}</span>
                  <strong>{row.label}</strong>
                </div>
                {row.body && <pre>{row.body}</pre>}
                {thread && row.artifacts.length > 0 && (
                  <ArtifactReferenceList
                    artifacts={row.artifacts}
                    threadId={thread.id}
                    onOpenArtifact={onOpenArtifact}
                  />
                )}
              </div>
            ))
          ) : (
            <span className="muted">暂无命令历史。</span>
          )}
        </div>
      </details>
    </div>
  );
}

export function terminalShellName(shell: string): string {
  const executable = shell.split(/[\\/]/).at(-1) ?? shell;
  const name = executable.replace(/\.exe$/i, "");
  switch (name.toLowerCase()) {
    case "cmd":
      return "命令提示符";
    case "powershell":
      return "Windows PowerShell";
    case "pwsh":
      return "PowerShell";
    default:
      return name;
  }
}

function writeTerminalEventToXterm(
  event: TerminalEvent,
  terminal: XtermTerminalHandle | null,
) {
  if (!terminal) return;

  switch (event.type) {
    case "started":
      if (event.command && !event.command.startsWith("interactive ")) {
        terminal.write(`$ ${event.command}\r\n`);
      }
      return;
    case "stdout":
      terminal.write(toXtermText(event.data ?? ""));
      return;
    case "stderr":
      terminal.write(`\x1b[31m${toXtermText(event.data ?? "")}\x1b[0m`);
      return;
    case "finished":
      if (event.message) {
        terminal.write(`\r\n\x1b[31m${event.message}\x1b[0m`);
      }
      terminal.write("\r\n");
      return;
    case "cancelled":
      terminal.write(
        `\r\n\x1b[33m${event.message ?? "command cancelled"}\x1b[0m\r\n`,
      );
      return;
    case "error":
      terminal.write(
        `\r\n\x1b[31m${event.message ?? "terminal error"}\x1b[0m\r\n`,
      );
      return;
  }
}

function toXtermText(value: string): string {
  return value.replace(/\r?\n/g, "\r\n");
}

function isTerminalEndEvent(type: TerminalEvent["type"]): boolean {
  return type === "finished" || type === "cancelled" || type === "error";
}

function getRunningTerminalCommandId(events: TerminalEvent[]): string | null {
  const running = new Set<string>();
  for (const event of events) {
    if (event.type === "started") {
      running.add(event.commandId);
    } else if (isTerminalEndEvent(event.type)) {
      running.delete(event.commandId);
    }
  }
  return Array.from(running).at(-1) ?? null;
}

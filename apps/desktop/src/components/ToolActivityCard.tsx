import { useEffect, useState } from "react";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  FilePen,
  FileText,
  FolderTree,
  GitCompare,
  Globe2,
  ListChecks,
  Monitor,
  Plug,
  Search,
  SquareTerminal,
  Sparkles,
  Table2,
  Wrench,
} from "lucide-react";
import type { ToolCall, ToolResult } from "../types";
import {
  buildToolActivity,
  type ToolActivityBody,
  type ToolActivityKind,
  type ToolActivityView,
} from "../toolActivity";
import "./ToolActivityCard.css";

export type ToolSandboxState = {
  label: string;
  detail: string;
  unsafe: boolean;
};

/**
 * One tool call rendered as a single line plus one panel. The line is built
 * from the call input and the result metadata — the model never writes it —
 * and the panel shows the call and its output together instead of splitting
 * "arguments" and "result" into two boxes.
 */
export function ToolActivityCard({
  call,
  result,
  timing,
  sandbox,
  streaming = false,
  defaultExpanded = false,
}: {
  call: ToolCall;
  result?: ToolResult;
  timing?: string;
  sandbox?: ToolSandboxState | null;
  streaming?: boolean;
  defaultExpanded?: boolean;
}) {
  const view = buildToolActivity(call, result);
  const running = !result;
  const [expanded, setExpanded] = useState(defaultExpanded);

  useEffect(() => {
    if (streaming) setExpanded(false);
  }, [streaming, call.id]);

  return (
    <div
      className="tool-activity"
      data-state={view.failed ? "error" : running ? "running" : "complete"}
      data-kind={view.kind}
    >
      <button
        className="tool-activity-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="tool-activity-icon" aria-hidden="true">
          {toolActivityIcon(view.kind)}
        </span>
        <span
          className="tool-activity-title"
          data-flow={streaming || undefined}
          title={view.detail ? `${view.title} · ${view.detail}` : view.title}
        >
          {view.title}
        </span>
        <span className="tool-activity-meta">
          {view.detail && (
            <span className="tool-activity-detail">{view.detail}</span>
          )}
          {view.chips.map((chip) => (
            <span
              key={chip.label}
              className="tool-activity-chip"
              data-tone={chip.tone ?? "neutral"}
              title={chip.title}
            >
              {chip.label}
            </span>
          ))}
          {sandbox && (
            <span
              className="tool-activity-chip"
              data-tone={sandbox.unsafe ? "warning" : "neutral"}
              title={sandbox.detail}
            >
              {sandbox.label}
            </span>
          )}
          {timing && <span className="tool-activity-timing">{timing}</span>}
        </span>
        <span className="tool-activity-chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
      </button>
      {expanded && (
        <div className="tool-activity-body">
          <ToolActivityBodyView body={view.body} view={view} />
        </div>
      )}
    </div>
  );
}

export function toolActivityIcon(kind: ToolActivityKind, size = 14) {
  if (kind === "shell") return <SquareTerminal size={size} />;
  if (kind === "read") return <FileText size={size} />;
  if (kind === "list") return <FolderTree size={size} />;
  if (kind === "search") return <Search size={size} />;
  if (kind === "edit") return <FilePen size={size} />;
  if (kind === "diff") return <GitCompare size={size} />;
  if (kind === "browser") return <Globe2 size={size} />;
  if (kind === "computer") return <Monitor size={size} />;
  if (kind === "spreadsheet") return <Table2 size={size} />;
  if (kind === "agent") return <Bot size={size} />;
  if (kind === "plan") return <ListChecks size={size} />;
  if (kind === "skill") return <Sparkles size={size} />;
  if (kind === "mcp") return <Plug size={size} />;
  return <Wrench size={size} />;
}

function ToolActivityBodyView({
  body,
  view,
}: {
  body: ToolActivityBody;
  view: ToolActivityView;
}) {
  if (body.type === "pending") {
    return (
      <div className="tool-panel is-pending" role="status">
        等待工具返回…
      </div>
    );
  }

  if (body.type === "terminal") {
    const { streams } = body;
    const status = view.failed ? "失败" : "成功";
    return (
      <div className="tool-panel" data-panel="terminal">
        <div className="tool-panel-head">
          <span className="tool-panel-prompt" aria-hidden="true">
            $
          </span>
          <code className="tool-panel-command">{streams.command}</code>
          {status && (
            <span
              className="tool-panel-status"
              data-tone={view.failed ? "danger" : "success"}
            >
              {status}
            </span>
          )}
        </div>
        <div className="tool-panel-scroll">
          {streams.stdout.trim() && (
            <pre className="tool-panel-stream" data-stream="stdout">
              {streams.stdout}
            </pre>
          )}
          {streams.stderr.trim() && (
            <pre className="tool-panel-stream" data-stream="stderr">
              {streams.stderr}
            </pre>
          )}
          {!streams.stdout.trim() && !streams.stderr.trim() && (
            <p className="tool-panel-empty">命令没有输出。</p>
          )}
        </div>
      </div>
    );
  }

  if (body.type === "patch") {
    const hasOld = body.lines.some((line) => line.oldLine !== null);
    const hasNew = body.lines.some((line) => line.newLine !== null);
    const numberColumns = (hasOld ? 1 : 0) + (hasNew ? 1 : 0);
    return (
      <div className="tool-panel" data-panel="patch">
        <div className="tool-panel-head">
          <span className="tool-panel-label">代码差异</span>
          <span className="tool-panel-status" data-tone="neutral">
            <span className="file-change-additions">+{body.additions}</span>{" "}
            <span className="file-change-deletions">-{body.deletions}</span>
          </span>
        </div>
        <div
          className="tool-panel-scroll tool-panel-diff"
          data-number-columns={numberColumns}
          tabIndex={0}
        >
          {body.lines.map((line, index) => (
            <div
              className="tool-panel-diff-line"
              data-kind={line.kind}
              key={`${index}:${line.oldLine ?? ""}:${line.newLine ?? ""}`}
            >
              {line.kind === "file" ? (
                <code className="tool-panel-diff-file">{line.text}</code>
              ) : (
                <>
                  {hasOld && (
                    <span className="tool-panel-diff-number">
                      {line.oldLine ?? ""}
                    </span>
                  )}
                  {hasNew && (
                    <span className="tool-panel-diff-number">
                      {line.newLine ?? ""}
                    </span>
                  )}
                  <code>{line.text || " "}</code>
                </>
              )}
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (body.type === "file") {
    return (
      <div className="tool-panel" data-panel="file">
        <div className="tool-panel-head">
          <code className="tool-panel-command" title={body.path}>
            {body.path || "文件"}
          </code>
        </div>
        <div className="tool-panel-scroll">
          <pre className="tool-panel-stream">{body.text || "（空文件）"}</pre>
        </div>
      </div>
    );
  }

  if (body.type === "entries") {
    return (
      <div className="tool-panel" data-panel="entries">
        <div className="tool-panel-scroll tool-panel-entries">
          {body.entries.map((entry, index) => (
            <code key={`${entry}-${index}`} title={entry}>
              {entry}
            </code>
          ))}
          {body.entries.length === 0 && (
            <p className="tool-panel-empty">目录为空。</p>
          )}
        </div>
        {body.total !== undefined && body.total > body.entries.length && (
          <footer className="tool-panel-foot">
            共 {body.total} 项，已显示前 {body.entries.length} 项
          </footer>
        )}
      </div>
    );
  }

  if (body.type === "matches") {
    return (
      <div className="tool-panel" data-panel="matches">
        <div className="tool-panel-scroll tool-panel-matches">
          {body.groups.map((group) => (
            <div className="tool-panel-match-group" key={group.path}>
              <code className="tool-panel-match-path" title={group.path}>
                {group.path}
              </code>
              {group.hits.map((hit, index) => (
                <div className="tool-panel-match" key={`${hit.line}-${index}`}>
                  <span className="tool-panel-diff-number">
                    {hit.line ?? ""}
                  </span>
                  <code>{hit.text || " "}</code>
                </div>
              ))}
            </div>
          ))}
        </div>
        <footer className="tool-panel-foot">共 {body.total} 处匹配</footer>
      </div>
    );
  }

  if (body.type === "fields") {
    return (
      <div className="tool-panel" data-panel="fields">
        <dl className="tool-panel-fields">
          {body.fields.map((field) => (
            <div key={field.label}>
              <dt>{field.label}</dt>
              <dd data-mono={field.mono || undefined}>{field.value}</dd>
            </div>
          ))}
        </dl>
        {body.text && (
          <div className="tool-panel-scroll">
            <pre className="tool-panel-stream">{body.text}</pre>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="tool-panel" data-panel="text">
      <div className="tool-panel-scroll">
        <pre className="tool-panel-stream">
          {body.text || `${view.title} 没有返回文本输出。`}
        </pre>
      </div>
    </div>
  );
}

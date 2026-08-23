import { memo, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  FileArchive,
  FileCode,
  FilePen,
  FileSpreadsheet,
  FileText,
  FolderTree,
  GitCompare,
  Globe2,
  Image as ImageIcon,
  ListChecks,
  Monitor,
  Paperclip,
  Plug,
  Search,
  SquareTerminal,
  Sparkles,
  Wrench,
} from "lucide-react";
import type { ToolCall, ToolResult } from "../types";
import {
  buildToolActivity,
  type ToolActivityBody,
  type ToolActivityChip,
  type ToolActivityIconKind,
  type ToolActivityView,
} from "../toolActivity";
import { FileTypeIcon } from "./FileTypeIcon";
import { ShimmerText } from "./ui";
import "./ToolActivityCard.css";

export type ToolSandboxState = {
  label: string;
  detail: string;
  unsafe: boolean;
};

export function ActivityCallCard({
  identity,
  state,
  kind,
  icon,
  title,
  detail,
  chips = [],
  timing,
  streaming = false,
  defaultExpanded = false,
  children,
}: {
  identity: string;
  state: "running" | "complete" | "error";
  kind: string;
  icon: ReactNode;
  title: string;
  detail?: string;
  chips?: ToolActivityChip[];
  timing?: string;
  streaming?: boolean;
  defaultExpanded?: boolean;
  children: ReactNode;
}) {
  const running = state === "running";
  const [expanded, setExpanded] = useState(defaultExpanded);

  useEffect(() => {
    if (streaming) setExpanded(false);
  }, [streaming, identity]);

  return (
    <div className="tool-activity" data-state={state} data-kind={kind}>
      <button
        className="tool-activity-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="tool-activity-icon" aria-hidden="true">
          {icon}
        </span>
        {running ? (
          <ShimmerText
            className="tool-activity-title"
            title={detail ? `${title} · ${detail}` : title}
          >
            {title}
          </ShimmerText>
        ) : (
          <span
            className="tool-activity-title"
            title={detail ? `${title} · ${detail}` : title}
          >
            {title}
          </span>
        )}
        <span className="tool-activity-meta">
          {detail && <span className="tool-activity-detail">{detail}</span>}
          {chips.map((chip) => (
            <span
              key={`${chip.label}:${chip.title ?? ""}`}
              className="tool-activity-chip"
              data-tone={chip.tone ?? "neutral"}
              title={chip.title}
            >
              {chip.label}
            </span>
          ))}
          {timing && <span className="tool-activity-timing">{timing}</span>}
        </span>
        <span className="tool-activity-chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
      </button>
      {expanded && <div className="tool-activity-body">{children}</div>}
    </div>
  );
}

/**
 * One tool call rendered as a single line plus one panel. The line is built
 * from the call input and the result metadata — the model never writes it —
 * and the panel shows the call and its output together instead of splitting
 * "arguments" and "result" into two boxes.
 */
type ToolActivityCardProps = {
  call: ToolCall;
  result?: ToolResult;
  timing?: string;
  sandbox?: ToolSandboxState | null;
  streaming?: boolean;
  defaultExpanded?: boolean;
};

export const ToolActivityCard = memo(function ToolActivityCard({
  call,
  result,
  timing,
  sandbox,
  streaming = false,
  defaultExpanded = false,
}: ToolActivityCardProps) {
  const view = useMemo(() => buildToolActivity(call, result), [call, result]);
  const running = !result;
  const chips = useMemo<ToolActivityChip[]>(
    () =>
      sandbox
        ? [
            ...view.chips,
            {
              label: sandbox.label,
              tone: sandbox.unsafe ? "warning" : "neutral",
              title: sandbox.detail,
            },
          ]
        : view.chips,
    [sandbox, view.chips],
  );

  return (
    <ActivityCallCard
      identity={call.id}
      state={view.failed ? "error" : running ? "running" : "complete"}
      kind={view.kind}
      icon={
        view.detail &&
        (view.kind === "attachment" || view.kind === "spreadsheet") ? (
          <FileTypeIcon name={view.detail} size={14} />
        ) : (
          toolActivityIcon(view.iconKind ?? view.kind)
        )
      }
      title={view.title}
      detail={view.detail}
      chips={chips}
      timing={timing}
      streaming={streaming}
      defaultExpanded={defaultExpanded}
    >
      <ToolActivityBodyView body={view.body} view={view} />
    </ActivityCallCard>
  );
}, toolActivityCardPropsEqual);

function toolActivityCardPropsEqual(
  previous: ToolActivityCardProps,
  next: ToolActivityCardProps,
): boolean {
  return (
    previous.call === next.call &&
    previous.result === next.result &&
    previous.timing === next.timing &&
    previous.streaming === next.streaming &&
    previous.defaultExpanded === next.defaultExpanded &&
    toolSandboxEqual(previous.sandbox, next.sandbox)
  );
}

function toolSandboxEqual(
  previous: ToolSandboxState | null | undefined,
  next: ToolSandboxState | null | undefined,
): boolean {
  return (
    previous === next ||
    (previous?.label === next?.label &&
      previous?.detail === next?.detail &&
      previous?.unsafe === next?.unsafe)
  );
}

export function toolActivityIcon(kind: ToolActivityIconKind, size = 14) {
  if (kind === "shell") return <SquareTerminal size={size} />;
  if (kind === "read") return <FileText size={size} />;
  if (kind === "list") return <FolderTree size={size} />;
  if (kind === "search") return <Search size={size} />;
  if (kind === "edit") return <FilePen size={size} />;
  if (kind === "diff") return <GitCompare size={size} />;
  if (kind === "browser") return <Globe2 size={size} />;
  if (kind === "computer") return <Monitor size={size} />;
  if (kind === "image") return <ImageIcon size={size} />;
  if (kind === "document") return <FileText size={size} />;
  if (kind === "code") return <FileCode size={size} />;
  if (kind === "archive") return <FileArchive size={size} />;
  if (kind === "attachment") return <Paperclip size={size} />;
  if (kind === "spreadsheet") return <FileSpreadsheet size={size} />;
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

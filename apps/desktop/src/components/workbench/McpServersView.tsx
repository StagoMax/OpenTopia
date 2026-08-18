import { useState, type FormEvent } from "react";
import { Pencil, Plus, RefreshCw, Save, Trash2, X } from "lucide-react";
import type {
  McpServerInput,
  McpServerView,
  ThreadMcpServerView,
} from "../../types";

type McpEditorState = {
  serverId: string | null;
  name: string;
  command: string;
  args: string;
  cwd: string;
  envKeys: string;
  timeoutMs: string;
  enabled: boolean;
};

function emptyMcpEditor(): McpEditorState {
  return {
    serverId: null,
    name: "",
    command: "",
    args: "",
    cwd: "",
    envKeys: "",
    timeoutMs: "30000",
    enabled: true,
  };
}

function mcpEditorFor(view: McpServerView): McpEditorState {
  return {
    serverId: view.server.serverId,
    name: view.server.name,
    command: view.server.command,
    args: view.server.args.join("\n"),
    cwd: view.server.cwd ?? "",
    envKeys: view.server.envKeys.join("\n"),
    timeoutMs: String(view.server.timeoutMs),
    enabled: view.server.enabled,
  };
}

function lines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

export function McpServersView({
  hasThread,
  mcpServers,
  threadMcpServers,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
}: {
  hasThread: boolean;
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
}) {
  const [editor, setEditor] = useState<McpEditorState | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bindings = new Map(
    threadMcpServers.map((item) => [item.server.serverId, item]),
  );
  const enabledCount = threadMcpServers.filter((item) => item.enabled).length;

  async function run(key: string, action: () => Promise<void>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
      throw caught;
    } finally {
      setBusyKey(null);
    }
  }

  async function submitEditor(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!editor) return;
    const name = editor.name.trim();
    const command = editor.command.trim();
    if (!name || !command) {
      setError("Name and command are required.");
      return;
    }
    const input: McpServerInput = {
      name,
      command,
      args: lines(editor.args),
      cwd: editor.cwd.trim() || undefined,
      envKeys: lines(editor.envKeys),
      timeoutMs: Number(editor.timeoutMs) || 30_000,
      enabled: editor.enabled,
    };
    const key = editor.serverId ? `save:${editor.serverId}` : "create";
    try {
      await run(key, () =>
        editor.serverId
          ? onUpdateMcpServer(editor.serverId, input)
          : onCreateMcpServer(input),
      );
      setEditor(null);
    } catch {
      // The inline error keeps the form open for correction.
    }
  }

  async function restart(serverId: string) {
    try {
      await run(`restart:${serverId}`, () => onRestartMcpServer(serverId));
    } catch {
      // The server row keeps its last known state and exposes the error below.
    }
  }

  async function remove(view: McpServerView) {
    if (!window.confirm(`Delete MCP server "${view.server.name}"?`)) return;
    try {
      await run(`delete:${view.server.serverId}`, () =>
        onDeleteMcpServer(view.server.serverId),
      );
      if (editor?.serverId === view.server.serverId) setEditor(null);
    } catch {
      // The inline error provides the recovery path.
    }
  }

  return (
    <div className="extensions-view">
      <div className="extensions-toolbar">
        <div className="diff-summary-row">
          <span>{mcpServers.length} available</span>
          <span>{enabledCount} enabled</span>
        </div>
        <button
          className="icon-button"
          type="button"
          title="Add MCP server"
          aria-label="Add MCP server"
          onClick={() => {
            setError(null);
            setEditor(emptyMcpEditor());
          }}
        >
          <Plus size={15} />
        </button>
      </div>

      {editor && (
        <form className="mcp-editor" onSubmit={submitEditor}>
          <div className="mcp-editor-header">
            <strong>
              {editor.serverId ? "Edit MCP server" : "New MCP server"}
            </strong>
            <button
              className="icon-button"
              type="button"
              title="Close editor"
              aria-label="Close MCP editor"
              onClick={() => setEditor(null)}
            >
              <X size={15} />
            </button>
          </div>
          <div className="mcp-editor-grid">
            <label>
              <span>Name</span>
              <input
                autoFocus
                value={editor.name}
                onChange={(event) =>
                  setEditor({ ...editor, name: event.target.value })
                }
              />
            </label>
            <label>
              <span>Command</span>
              <input
                value={editor.command}
                onChange={(event) =>
                  setEditor({ ...editor, command: event.target.value })
                }
              />
            </label>
            <label>
              <span>Arguments</span>
              <textarea
                rows={3}
                title="One argument per line"
                value={editor.args}
                onChange={(event) =>
                  setEditor({ ...editor, args: event.target.value })
                }
              />
            </label>
            <label>
              <span>Environment keys</span>
              <textarea
                rows={3}
                title="One inherited environment variable name per line"
                value={editor.envKeys}
                onChange={(event) =>
                  setEditor({ ...editor, envKeys: event.target.value })
                }
              />
            </label>
            <label>
              <span>Working directory</span>
              <input
                value={editor.cwd}
                onChange={(event) =>
                  setEditor({ ...editor, cwd: event.target.value })
                }
              />
            </label>
            <label>
              <span>Timeout (ms)</span>
              <input
                type="number"
                min={1000}
                max={300000}
                step={1000}
                value={editor.timeoutMs}
                onChange={(event) =>
                  setEditor({ ...editor, timeoutMs: event.target.value })
                }
              />
            </label>
          </div>
          <div className="mcp-editor-actions">
            <label className="mcp-enabled-toggle">
              <input
                type="checkbox"
                checked={editor.enabled}
                onChange={(event) =>
                  setEditor({ ...editor, enabled: event.target.checked })
                }
              />
              <span>Enabled</span>
            </label>
            <button
              className="primary-button"
              type="submit"
              disabled={busyKey !== null}
            >
              <Save size={14} />
              {busyKey?.startsWith("save:") || busyKey === "create"
                ? "Saving"
                : "Save"}
            </button>
          </div>
        </form>
      )}

      {error && (
        <p className="workspace-error" role="alert">
          {error}
        </p>
      )}

      <div className="extension-list">
        {mcpServers.length ? (
          mcpServers.map((view) => {
            const serverId = view.server.serverId;
            const binding = bindings.get(serverId);
            const rowBusy = busyKey?.endsWith(serverId) ?? false;
            return (
              <div className="extension-row" key={serverId}>
                <input
                  type="checkbox"
                  aria-label={`Enable ${view.server.name} for this thread`}
                  checked={binding?.enabled ?? false}
                  disabled={!hasThread || !view.server.enabled || rowBusy}
                  onChange={(event) =>
                    onToggleThreadMcp(serverId, event.target.checked)
                  }
                />
                <div className="extension-main">
                  <span>{view.server.name}</span>
                  <small title={view.server.command}>
                    {view.server.command}
                  </small>
                </div>
                <em
                  className={`mcp-status is-${view.status.status}`}
                  title={view.status.message}
                >
                  {view.status.status}
                  {view.status.toolsCount ? ` · ${view.status.toolsCount}` : ""}
                </em>
                <div className="extension-actions">
                  <button
                    className="icon-button"
                    type="button"
                    title="Edit MCP server"
                    aria-label={`Edit ${view.server.name}`}
                    disabled={busyKey !== null}
                    onClick={() => {
                      setError(null);
                      setEditor(mcpEditorFor(view));
                    }}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    className="icon-button"
                    type="button"
                    title="Restart MCP server"
                    aria-label={`Restart ${view.server.name}`}
                    disabled={busyKey !== null || !view.server.enabled}
                    onClick={() => void restart(serverId)}
                  >
                    <RefreshCw
                      className={
                        busyKey === `restart:${serverId}` ? "spinning" : ""
                      }
                      size={14}
                    />
                  </button>
                  <button
                    className="icon-button danger"
                    type="button"
                    title="Delete MCP server"
                    aria-label={`Delete ${view.server.name}`}
                    disabled={busyKey !== null}
                    onClick={() => void remove(view)}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })
        ) : (
          <div className="workbench-empty-state">
            No MCP servers configured.
          </div>
        )}
      </div>
    </div>
  );
}

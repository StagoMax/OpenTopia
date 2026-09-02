import { useEffect, useMemo, useState } from "react";
import {
  Check,
  FolderOpen,
  PackagePlus,
  Plus,
  Puzzle,
  Search,
  Settings2,
  ShieldAlert,
  Trash2,
  Workflow,
  Wrench,
  X,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import type {
  McpServerInput,
  McpServerView,
  PluginView,
  ThreadMcpServerView,
} from "../../types";
import { PluginControlPanel } from "../PluginControlPanel";
import { McpServersView } from "./McpServersView";

export function ExtensionsView({
  client,
  hasThread,
  threadId,
  workspaceRoot,
  plugins,
  selectedSkillIds,
  mcpServers,
  threadMcpServers,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
  onInstallPlugin,
  onUninstallPlugin,
  onTogglePlugin,
  onUsePluginSkills,
  onOpenPath,
}: {
  client: ApiClient | null;
  hasThread: boolean;
  threadId: string | null;
  workspaceRoot: string | null;
  plugins: PluginView[];
  selectedSkillIds: string[];
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
  onInstallPlugin(): Promise<void>;
  onUninstallPlugin(pluginId: string): Promise<void>;
  onTogglePlugin(pluginId: string, enabled: boolean): Promise<void>;
  onUsePluginSkills(pluginId: string, enabled: boolean): Promise<void>;
  onOpenPath(targetPath: string): void;
}) {
  const [view, setView] = useState<"plugins" | "mcp">("plugins");
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<"all" | PluginView["plugin"]["source"]>(
    "all",
  );
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedPluginId, setSelectedPluginId] = useState<string | null>(null);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filteredPlugins = useMemo(
    () =>
      plugins.filter(({ plugin }) => {
        if (source !== "all" && plugin.source !== source) return false;
        if (!normalizedQuery) return true;
        return `${plugin.displayName} ${plugin.name} ${plugin.description} ${plugin.author} ${plugin.category}`
          .toLocaleLowerCase()
          .includes(normalizedQuery);
      }),
    [normalizedQuery, plugins, source],
  );
  const activeCount = plugins.filter(
    (plugin) => plugin.effectiveEnabled,
  ).length;
  const selectedPlugin = plugins.find(
    (item) => item.plugin.id === selectedPluginId,
  );

  useEffect(() => {
    if (selectedPluginId && !selectedPlugin) setSelectedPluginId(null);
  }, [selectedPlugin, selectedPluginId]);

  async function run(key: string, action: () => Promise<void>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function removePlugin(view: PluginView) {
    if (
      !window.confirm(
        `Remove "${view.plugin.displayName}" from OpenTopia? The original source folder is not changed.`,
      )
    ) {
      return;
    }
    await run(`remove:${view.plugin.id}`, () =>
      onUninstallPlugin(view.plugin.id),
    );
  }

  if (selectedPlugin) {
    return (
      <PluginControlPanel
        client={client}
        pluginView={selectedPlugin}
        threadId={threadId}
        workspaceRoot={workspaceRoot}
        onBack={() => setSelectedPluginId(null)}
      />
    );
  }

  return (
    <div className="extensions-view plugins-browser">
      <div className="plugin-browser-header">
        <div
          className="plugin-view-switch"
          role="tablist"
          aria-label="Plugin view"
        >
          <button
            className={view === "plugins" ? "active" : ""}
            type="button"
            role="tab"
            aria-selected={view === "plugins"}
            onClick={() => setView("plugins")}
          >
            <Puzzle size={14} />
            Plugins
          </button>
          <button
            className={view === "mcp" ? "active" : ""}
            type="button"
            role="tab"
            aria-selected={view === "mcp"}
            onClick={() => setView("mcp")}
          >
            <Settings2 size={14} />
            MCP servers
          </button>
        </div>
        {view === "plugins" && (
          <button
            className="secondary-button compact plugin-install-button"
            type="button"
            disabled={busyKey !== null}
            onClick={() => void run("install", onInstallPlugin)}
          >
            <PackagePlus size={14} />
            {busyKey === "install" ? "Installing" : "Add local"}
          </button>
        )}
      </div>

      {view === "plugins" ? (
        <>
          <div className="plugin-directory-controls">
            <label className="plugin-search">
              <span className="sr-only">Search plugins</span>
              <Search size={14} />
              <input
                value={query}
                placeholder="Search installed plugins"
                onChange={(event) => setQuery(event.target.value)}
              />
              {query && (
                <button
                  type="button"
                  title="Clear search"
                  aria-label="Clear plugin search"
                  onClick={() => setQuery("")}
                >
                  <X size={13} />
                </button>
              )}
            </label>
            <label className="plugin-scope-filter">
              <span className="sr-only">Plugin source</span>
              <select
                value={source}
                onChange={(event) =>
                  setSource(event.target.value as typeof source)
                }
              >
                <option value="all">All sources</option>
                <option value="bundled">Bundled</option>
                <option value="workspace">Project</option>
                <option value="user">OpenTopia</option>
                <option value="codex">Codex</option>
              </select>
            </label>
          </div>
          <div className="plugin-summary">
            <span>{plugins.length} installed</span>
            <span>{activeCount} enabled</span>
            <span>{filteredPlugins.length} shown</span>
          </div>
          {error && (
            <p className="workspace-error" role="alert">
              {error}
            </p>
          )}
          <div className="plugin-directory" aria-live="polite">
            {filteredPlugins.length ? (
              filteredPlugins.map((item) => {
                const plugin = item.plugin;
                const skillsSelected =
                  item.skillIds.length > 0 &&
                  item.skillIds.every((id) => selectedSkillIds.includes(id));
                const busy = busyKey?.endsWith(plugin.id) ?? false;
                return (
                  <article
                    className={`plugin-entry ${item.compatible ? "" : "is-incompatible"}`}
                    key={plugin.id}
                    style={{ borderLeftColor: plugin.brandColor ?? undefined }}
                  >
                    <div className="plugin-entry-heading">
                      <span className="plugin-monogram" aria-hidden="true">
                        {plugin.displayName.slice(0, 1).toLocaleUpperCase()}
                      </span>
                      <div className="plugin-entry-title">
                        <strong>{plugin.displayName}</strong>
                        <span>{plugin.description || plugin.name}</span>
                      </div>
                      <span className={`plugin-source is-${plugin.source}`}>
                        {plugin.source === "bundled"
                          ? "Bundled"
                          : plugin.source === "workspace"
                            ? "Project"
                            : plugin.source === "codex"
                              ? "Codex"
                              : "OpenTopia"}
                      </span>
                    </div>
                    <div
                      className="plugin-capabilities"
                      aria-label="Capabilities"
                    >
                      {plugin.skillCount > 0 && (
                        <span>
                          <Workflow size={12} /> {plugin.skillCount} Skills
                        </span>
                      )}
                      {plugin.mcpServerCount > 0 && (
                        <span>
                          <Wrench size={12} /> {plugin.supportedMcpServerCount}/
                          {plugin.mcpServerCount} MCP
                        </span>
                      )}
                      {plugin.nativeCapabilities.length > 0 && (
                        <span>
                          <Wrench size={12} />{" "}
                          {plugin.nativeCapabilities.length} native
                        </span>
                      )}
                      {plugin.source === "bundled" && (
                        <span>
                          {plugin.trust === "trusted_driver"
                            ? "Trusted driver"
                            : plugin.trust === "privileged"
                              ? "Privileged"
                              : "Official"}
                        </span>
                      )}
                      {plugin.hasApps && <span>App</span>}
                      {plugin.version && <span>v{plugin.version}</span>}
                      {plugin.category && <span>{plugin.category}</span>}
                    </div>
                    {plugin.issues.length > 0 && (
                      <details className="plugin-issues">
                        <summary>
                          <ShieldAlert size={13} />
                          {item.compatible
                            ? "Limited support"
                            : "Not available"}
                        </summary>
                        <ul>
                          {plugin.issues.map((issue) => (
                            <li key={issue}>{issue}</li>
                          ))}
                        </ul>
                      </details>
                    )}
                    <div className="plugin-entry-actions">
                      <div className="plugin-primary-actions">
                        {item.skillIds.length > 0 && (
                          <button
                            className={`secondary-button compact ${skillsSelected ? "is-selected" : ""}`}
                            type="button"
                            aria-pressed={skillsSelected}
                            disabled={busy}
                            onClick={() =>
                              void run(`skills:${plugin.id}`, () =>
                                onUsePluginSkills(plugin.id, !skillsSelected),
                              )
                            }
                          >
                            {skillsSelected ? (
                              <Check size={13} />
                            ) : (
                              <Plus size={13} />
                            )}
                            {skillsSelected ? "Skills added" : "Use Skills"}
                          </button>
                        )}
                        <label
                          className="plugin-task-toggle"
                          title={
                            workspaceRoot
                              ? "Enable this plugin for the current project"
                              : "Enable this plugin globally"
                          }
                        >
                          <input
                            type="checkbox"
                            checked={item.effectiveEnabled}
                            disabled={!item.compatible || busy}
                            onChange={(event) =>
                              void run(`toggle:${plugin.id}`, () =>
                                onTogglePlugin(plugin.id, event.target.checked),
                              )
                            }
                          />
                          <span>Enabled</span>
                        </label>
                      </div>
                      <div className="plugin-secondary-actions">
                        <button
                          className="icon-button"
                          type="button"
                          title="Configure plugin"
                          aria-label={`Configure ${plugin.displayName}`}
                          onClick={() => setSelectedPluginId(plugin.id)}
                        >
                          <Settings2 size={14} />
                        </button>
                        <button
                          className="icon-button"
                          type="button"
                          title="Open plugin folder"
                          aria-label={`Open ${plugin.displayName} folder`}
                          onClick={() => onOpenPath(plugin.path)}
                        >
                          <FolderOpen size={14} />
                        </button>
                        {plugin.managed && (
                          <button
                            className="icon-button danger"
                            type="button"
                            title="Remove plugin"
                            aria-label={`Remove ${plugin.displayName}`}
                            disabled={busyKey !== null}
                            onClick={() => void removePlugin(item)}
                          >
                            <Trash2 size={14} />
                          </button>
                        )}
                      </div>
                    </div>
                  </article>
                );
              })
            ) : (
              <div className="workbench-empty-state plugin-empty-state">
                <Puzzle size={22} />
                <strong>
                  {plugins.length ? "No plugins match" : "No plugins installed"}
                </strong>
                <span>
                  {plugins.length
                    ? "Try another search or source."
                    : "Add a local Codex-compatible plugin folder."}
                </span>
              </div>
            )}
          </div>
        </>
      ) : (
        <McpServersView
          hasThread={hasThread}
          mcpServers={mcpServers}
          threadMcpServers={threadMcpServers}
          onToggleThreadMcp={onToggleThreadMcp}
          onCreateMcpServer={onCreateMcpServer}
          onUpdateMcpServer={onUpdateMcpServer}
          onRestartMcpServer={onRestartMcpServer}
          onDeleteMcpServer={onDeleteMcpServer}
        />
      )}
    </div>
  );
}

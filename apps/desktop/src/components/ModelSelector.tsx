import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronRight,
  RotateCcw,
  Search,
  Settings,
} from "lucide-react";

import "./ModelSelector.css";
import {
  buildConnectionModelGroups,
  formatModelDisplayName,
  reconcileReasoningEffort,
  resolveDefaultModelId,
  resolveReasoningOptions,
  REASONING_EFFORT_DETAILS,
} from "../modelCatalog";
import { providerDisplayName } from "../providerSettings";
import type {
  ProviderSettings,
  ReasoningEffort,
  ThreadModelSelection,
} from "../types";

export type ModelSelectorProps = {
  connections: ProviderSettings[];
  /** Connection used when the thread has not pinned one yet. */
  activeConnectionId: string;
  selection: ThreadModelSelection | null;
  onChange: (selection: ThreadModelSelection) => void;
  onOpenSettings: () => void;
  disabled?: boolean;
};

type ModelOption = {
  connection: ProviderSettings;
  modelId: string;
  displayName: string;
  latest: boolean;
  preview: boolean;
  familyLabel: string;
};

type OpenSubmenu = "model" | "effort" | null;

/** Grace period that covers the gap between the panel and an open submenu. */
const CLOSE_DELAY_MS = 200;

/**
 * Composer-level model picker. It reads as a quiet status label — the model in
 * use and its reasoning effort — and only becomes a menu on hover, so the
 * composer toolbar is not carrying two dropdowns for a setting most turns leave
 * alone. Choosing stays two-step by design: the valid efforts depend on the
 * model, so effort lives in its own submenu.
 */
export function ModelSelector({
  connections,
  activeConnectionId,
  selection,
  onChange,
  onOpenSettings,
  disabled = false,
}: ModelSelectorProps) {
  const resolved = useResolvedSelection(
    connections,
    activeConnectionId,
    selection,
  );

  const [open, setOpen] = useState(false);
  const [submenu, setSubmenu] = useState<OpenSubmenu>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const closeTimer = useRef<number | null>(null);

  const cancelClose = useCallback(() => {
    if (closeTimer.current === null) return;
    window.clearTimeout(closeTimer.current);
    closeTimer.current = null;
  }, []);

  const closeAll = useCallback(() => {
    cancelClose();
    setOpen(false);
    setSubmenu(null);
  }, [cancelClose]);

  // Pointer exits are forgiving: a keyboard user typing in the model search
  // must not lose the panel just because the pointer drifted off it.
  const scheduleClose = useCallback(() => {
    cancelClose();
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      if (rootRef.current?.contains(document.activeElement)) return;
      setOpen(false);
      setSubmenu(null);
    }, CLOSE_DELAY_MS);
  }, [cancelClose]);

  useEffect(() => cancelClose, [cancelClose]);

  useEffect(() => {
    if (disabled) closeAll();
  }, [closeAll, disabled]);

  useEffect(() => {
    if (!open) return undefined;

    function onPointerDown(event: PointerEvent) {
      if (rootRef.current?.contains(event.target as Node)) return;
      closeAll();
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      if (submenu) {
        setSubmenu(null);
        return;
      }
      closeAll();
      triggerRef.current?.focus();
    }

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [closeAll, open, submenu]);

  if (!resolved) return null;

  const { connection, modelId, reasoningEffort } = resolved;
  const capability = resolveReasoningOptions(connection.kind, modelId);
  const modelLabel = formatModelDisplayName(modelId);
  const effortLabel = reasoningEffort
    ? REASONING_EFFORT_DETAILS[reasoningEffort].label
    : null;
  const supportsEffort = capability.supportedEfforts.length > 0;

  const fallback = defaultSelectionFor(
    connections.find((item) => item.id === activeConnectionId) ??
      connections[0],
  );
  const isDefault =
    connection.id === fallback.connectionId &&
    modelId === fallback.modelId &&
    reasoningEffort === fallback.reasoningEffort;

  return (
    <div
      className="model-selector"
      onMouseEnter={() => {
        if (disabled) return;
        cancelClose();
        setOpen(true);
      }}
      onMouseLeave={scheduleClose}
      ref={rootRef}
    >
      <button
        aria-expanded={open}
        aria-haspopup="menu"
        className="model-selector-trigger"
        disabled={disabled}
        onClick={() => {
          if (open) closeAll();
          else setOpen(true);
        }}
        ref={triggerRef}
        title={modelId}
        type="button"
      >
        <span className="model-selector-trigger-model">{modelLabel}</span>
        {effortLabel ? (
          <span className="model-selector-trigger-effort">{effortLabel}</span>
        ) : null}
      </button>

      {open ? (
        <div aria-label="模型设置" className="model-selector-menu" role="menu">
          <div className="model-selector-row-wrap">
            <button
              aria-expanded={submenu === "model"}
              aria-haspopup="menu"
              className="model-selector-row"
              onClick={() =>
                setSubmenu((current) => (current === "model" ? null : "model"))
              }
              onMouseEnter={() => setSubmenu("model")}
              role="menuitem"
              type="button"
            >
              <span className="model-selector-row-label">模型</span>
              <span className="model-selector-row-value">{modelLabel}</span>
              <ChevronRight aria-hidden="true" size={14} />
            </button>
            {submenu === "model" ? (
              <div className="model-selector-submenu">
                <ModelMenu
                  connections={connections}
                  onOpenSettings={() => {
                    closeAll();
                    onOpenSettings();
                  }}
                  onSelect={(option) => {
                    onChange({
                      connectionId: option.connection.id,
                      modelId: option.modelId,
                      reasoningEffort: reconcileReasoningEffort(
                        option.connection.kind,
                        option.modelId,
                        reasoningEffort,
                      ),
                    });
                    setSubmenu(null);
                  }}
                  selectedConnectionId={connection.id}
                  selectedModelId={modelId}
                />
              </div>
            ) : null}
          </div>

          {supportsEffort ? (
            <div className="model-selector-row-wrap">
              <button
                aria-expanded={submenu === "effort"}
                aria-haspopup="menu"
                className="model-selector-row"
                onClick={() =>
                  setSubmenu((current) =>
                    current === "effort" ? null : "effort",
                  )
                }
                onMouseEnter={() => setSubmenu("effort")}
                role="menuitem"
                type="button"
              >
                <span className="model-selector-row-label">推理强度</span>
                <span className="model-selector-row-value">
                  {effortLabel ?? "默认"}
                </span>
                <ChevronRight aria-hidden="true" size={14} />
              </button>
              {submenu === "effort" ? (
                <div className="model-selector-submenu model-selector-submenu--effort">
                  <ul className="model-menu-list" role="listbox">
                    {capability.supportedEfforts.map((effort) => (
                      <li key={effort}>
                        <button
                          aria-selected={effort === reasoningEffort}
                          className="model-menu-option"
                          onClick={() => {
                            onChange({
                              connectionId: connection.id,
                              modelId,
                              reasoningEffort: effort,
                            });
                            setSubmenu(null);
                          }}
                          role="option"
                          type="button"
                        >
                          <span className="model-menu-option-main">
                            <span className="model-menu-option-name">
                              {REASONING_EFFORT_DETAILS[effort].label}
                            </span>
                          </span>
                          {effort === reasoningEffort ? (
                            <Check aria-hidden="true" size={14} />
                          ) : null}
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </div>
          ) : null}

          <div className="model-selector-menu-separator" />

          <button
            className="model-selector-row model-selector-row--reset"
            disabled={isDefault}
            onClick={() => {
              onChange(fallback);
              closeAll();
            }}
            onMouseEnter={() => setSubmenu(null)}
            role="menuitem"
            type="button"
          >
            <span className="model-selector-row-label">重置为默认设置</span>
            <RotateCcw aria-hidden="true" size={14} />
          </button>
        </div>
      ) : null}
    </div>
  );
}

function ModelMenu({
  connections,
  onOpenSettings,
  onSelect,
  selectedConnectionId,
  selectedModelId,
}: {
  connections: ProviderSettings[];
  onOpenSettings: () => void;
  onSelect: (option: ModelOption) => void;
  selectedConnectionId: string;
  selectedModelId: string;
}) {
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());

  const sections = useMemo(() => buildSections(connections), [connections]);

  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filtered = useMemo(() => {
    if (!normalizedQuery) return sections;
    return sections
      .map((section) => ({
        ...section,
        groups: section.groups
          .map((group) => ({
            ...group,
            options: group.options.filter(
              (option) =>
                option.modelId.toLocaleLowerCase().includes(normalizedQuery) ||
                option.displayName
                  .toLocaleLowerCase()
                  .includes(normalizedQuery),
            ),
          }))
          .filter((group) => group.options.length > 0),
      }))
      .filter((section) => section.groups.length > 0);
  }, [normalizedQuery, sections]);

  // Only one connection means the connection heading is noise.
  const showConnectionHeadings = sections.length > 1;

  return (
    <div className="model-menu">
      <div className="model-menu-search">
        <Search aria-hidden="true" size={14} />
        <input
          aria-label="搜索模型"
          className="model-menu-search-input"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="搜索模型…"
          type="search"
          value={query}
        />
      </div>

      {filtered.length === 0 ? (
        <p className="model-menu-empty">
          没有匹配的模型。请在设置中同步模型列表或启用更多系列。
        </p>
      ) : (
        <div className="model-menu-scroll">
          {filtered.map((section) => (
            <section key={section.connection.id}>
              {showConnectionHeadings ? (
                <h3 className="model-menu-connection">
                  {providerDisplayName(section.connection)}
                </h3>
              ) : null}
              {section.groups.map((group) => {
                const key = `${section.connection.id}:${group.familyLabel}`;
                // Searching should never hide results behind a collapsed group.
                const isCollapsed = !normalizedQuery && collapsed.has(key);
                return (
                  <div className="model-menu-family" key={key}>
                    <button
                      aria-expanded={!isCollapsed}
                      className="model-menu-family-toggle"
                      onClick={() =>
                        setCollapsed((current) => {
                          const next = new Set(current);
                          if (next.has(key)) next.delete(key);
                          else next.add(key);
                          return next;
                        })
                      }
                      type="button"
                    >
                      {isCollapsed ? (
                        <ChevronRight aria-hidden="true" size={14} />
                      ) : (
                        <ChevronDown aria-hidden="true" size={14} />
                      )}
                      <span>{group.familyLabel}</span>
                      <span className="model-menu-family-count">
                        {group.options.length}
                      </span>
                    </button>
                    {isCollapsed ? null : (
                      <ul className="model-menu-list" role="listbox">
                        {group.options.map((option) => {
                          const selected =
                            option.modelId === selectedModelId &&
                            option.connection.id === selectedConnectionId;
                          return (
                            <li
                              key={`${option.connection.id}:${option.modelId}`}
                            >
                              <button
                                aria-selected={selected}
                                className="model-menu-option"
                                onClick={() => onSelect(option)}
                                role="option"
                                type="button"
                              >
                                <span className="model-menu-option-main">
                                  <span className="model-menu-option-name">
                                    {option.displayName}
                                    {option.latest ? (
                                      <span className="model-menu-tag">
                                        最新
                                      </span>
                                    ) : null}
                                    {option.preview ? (
                                      <span className="model-menu-tag">
                                        预览
                                      </span>
                                    ) : null}
                                  </span>
                                  <span className="model-menu-option-meta">
                                    {option.modelId}
                                  </span>
                                </span>
                                {selected ? (
                                  <Check aria-hidden="true" size={14} />
                                ) : null}
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </div>
                );
              })}
            </section>
          ))}
        </div>
      )}

      <button
        className="model-menu-footer"
        onClick={onOpenSettings}
        type="button"
      >
        <Settings aria-hidden="true" size={14} />
        <span>管理模型和 API</span>
      </button>
    </div>
  );
}

type MenuSection = {
  connection: ProviderSettings;
  groups: { familyLabel: string; options: ModelOption[] }[];
};

function buildSections(connections: ProviderSettings[]): MenuSection[] {
  return connections
    .map((connection) => {
      const groups = buildConnectionModelGroups(
        connectionModelIds(connection),
        connection.enabledFamilies ?? [],
      ).map((group) => ({
        familyLabel: group.family.label,
        options: group.models.map((model) => ({
          connection,
          modelId: model.id,
          displayName: model.displayName,
          latest: model.latest,
          preview: model.preview,
          familyLabel: group.family.label,
        })),
      }));
      return { connection, groups };
    })
    .filter((section) => section.groups.length > 0);
}

/**
 * Synced ids plus the connection's own default, so a hand-typed model stays
 * selectable even when the endpoint has no `/v1/models` route.
 */
function connectionModelIds(connection: ProviderSettings): string[] {
  const synced = connection.syncedModels ?? [];
  const configured = connection.model?.trim();
  if (configured && !synced.includes(configured)) {
    return [configured, ...synced];
  }
  return synced.length > 0 ? synced : configured ? [configured] : [];
}

/**
 * What "重置为默认设置" restores: the newest stable model of the active
 * connection, matching how a brand-new task picks its model.
 */
function defaultSelectionFor(
  connection: ProviderSettings,
): ThreadModelSelection {
  const modelIds = connectionModelIds(connection);
  const modelId = resolveDefaultModelId(
    modelIds,
    connection.enabledFamilies ?? [],
    connection.model,
  );
  return {
    connectionId: connection.id,
    modelId,
    reasoningEffort: reconcileReasoningEffort(
      connection.kind,
      modelId,
      connection.reasoningEffort ?? null,
    ),
  };
}

/**
 * Falls back to the active connection's default model so threads created
 * before per-thread models still render a selection.
 */
function useResolvedSelection(
  connections: ProviderSettings[],
  activeConnectionId: string,
  selection: ThreadModelSelection | null,
): {
  connection: ProviderSettings;
  modelId: string;
  reasoningEffort: ReasoningEffort | null;
} | null {
  return useMemo(() => {
    if (connections.length === 0) return null;
    const active =
      connections.find((item) => item.id === activeConnectionId) ??
      connections[0];
    if (!selection) {
      return {
        connection: active,
        modelId: active.model,
        reasoningEffort: active.reasoningEffort ?? null,
      };
    }
    const connection =
      connections.find((item) => item.id === selection.connectionId) ?? active;
    return {
      connection,
      modelId: selection.modelId || connection.model,
      reasoningEffort: selection.reasoningEffort ?? null,
    };
  }, [activeConnectionId, connections, selection]);
}

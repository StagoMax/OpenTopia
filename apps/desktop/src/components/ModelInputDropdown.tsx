import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, RefreshCw, Search, X } from "lucide-react";

import {
  buildConnectionModelGroups,
  formatModelDisplayName,
} from "../modelCatalog";
import type { ProviderSettings } from "../types";

export type ModelInputDropdownProps = {
  connection: ProviderSettings;
  value: string;
  onChange: (modelId: string) => void;
  onSync(): void;
  syncing: boolean;
  disabled?: boolean;
};

/**
 * Input + dropdown button for model selection. The dropdown shows synced
 * models grouped by family. Users can also type a model ID directly.
 */
export function ModelInputDropdown({
  connection,
  value,
  onChange,
  onSync,
  syncing,
  disabled = false,
}: ModelInputDropdownProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const modelIds = useMemo(() => {
    const ids = [...connection.syncedModels];
    const fallback = connection.model.trim();
    if (fallback && !ids.includes(fallback)) ids.push(fallback);
    return ids;
  }, [connection.syncedModels, connection.model]);

  const groups = useMemo(
    () => buildConnectionModelGroups(modelIds, connection.enabledFamilies ?? []),
    [modelIds, connection.enabledFamilies],
  );

  const hasSyncedModels = connection.syncedModels.length > 0;

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (rootRef.current?.contains(event.target as Node)) return;
      setOpen(false);
      setQuery("");
    }
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
  }, []);

  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filtered = useMemo(() => {
    if (!normalizedQuery) return groups;
    return groups
      .map((group) => ({
        ...group,
        models: group.models.filter(
          (m) =>
            m.id.toLocaleLowerCase().includes(normalizedQuery) ||
            m.displayName.toLocaleLowerCase().includes(normalizedQuery),
        ),
      }))
      .filter((group) => group.models.length > 0);
  }, [normalizedQuery, groups]);

  const totalFiltered = useMemo(
    () => filtered.reduce((sum, g) => sum + g.models.length, 0),
    [filtered],
  );

  return (
    <div className="model-input-dropdown" ref={rootRef}>
      <div className="model-input-dropdown-row">
        <input
          ref={inputRef}
          type="text"
          className="model-input-dropdown-input"
          value={value}
          spellCheck={false}
          placeholder="输入模型 ID 或从列表选择"
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          onFocus={() => {
            if (hasSyncedModels && !disabled) setOpen(true);
          }}
        />
        {hasSyncedModels && !disabled ? (
          <button
            type="button"
            className={`model-input-dropdown-trigger ${open ? "open" : ""}`}
            aria-label="展开模型列表"
            title="选择模型"
            onClick={() => {
              if (open) {
                close();
              } else {
                setOpen(true);
                inputRef.current?.focus();
              }
            }}
          >
            <ChevronDown size={14} />
          </button>
        ) : null}
        <button
          type="button"
          className="model-input-dropdown-sync"
          disabled={syncing || disabled}
          title="从 API 同步模型列表"
          onClick={onSync}
        >
          <RefreshCw
            size={13}
            className={syncing ? "spin" : ""}
            aria-hidden="true"
          />
        </button>
      </div>

      {open ? (
        <div className="model-input-dropdown-menu" role="listbox">
          <div className="model-input-dropdown-search">
            <Search size={13} aria-hidden="true" />
            <input
              type="search"
              className="model-input-dropdown-search-input"
              placeholder="搜索模型…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              autoFocus
            />
            {query ? (
              <button
                type="button"
                className="model-input-dropdown-search-clear"
                onClick={() => setQuery("")}
              >
                <X size={12} />
              </button>
            ) : null}
          </div>

          <div className="model-input-dropdown-scroll">
            {totalFiltered === 0 ? (
              <p className="model-input-dropdown-empty">
                {modelIds.length === 0
                  ? "没有模型。请先点击右侧同步按钮。"
                  : "没有匹配的模型。"}
              </p>
            ) : (
              filtered.map((group) => (
                <div key={group.family.id} className="model-input-dropdown-group">
                  <div className="model-input-dropdown-group-label">
                    <span>{group.family.label}</span>
                    <span className="model-input-dropdown-group-count">
                      {group.models.length}
                    </span>
                  </div>
                  {group.models.map((model) => (
                    <button
                      key={model.id}
                      type="button"
                      className={`model-input-dropdown-option ${
                        model.id === value ? "selected" : ""
                      }`}
                      role="option"
                      aria-selected={model.id === value}
                      onClick={() => {
                        onChange(model.id);
                        close();
                      }}
                    >
                      <span className="model-input-dropdown-option-name">
                        {model.displayName}
                        {model.latest ? (
                          <span className="model-input-dropdown-tag">最新</span>
                        ) : null}
                        {model.preview ? (
                          <span className="model-input-dropdown-tag preview">
                            预览
                          </span>
                        ) : null}
                      </span>
                      <span className="model-input-dropdown-option-id">
                        {model.id}
                      </span>
                    </button>
                  ))}
                </div>
              ))
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}

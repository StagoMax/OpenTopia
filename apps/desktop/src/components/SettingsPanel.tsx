import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import {
  Bell,
  BellRing,
  Check,
  Eye,
  EyeOff,
  FileJson,
  FileText,
  Import,
  KeyRound,
  Plus,
  Search,
  Server,
  Shield,
  SlidersHorizontal,
  Trash2,
  X,
} from "lucide-react";
import {
  PROVIDER_IMPORT_PRESETS,
  createProviderDraftFromPreset,
  parseProviderImport,
  type ProviderImportDraft,
} from "../providerImport";
import type { TaskNotificationPreferences } from "../taskNotifications";
import type {
  AppSettings,
  KeyringMetadata,
  PlatformInfo,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderSettings,
  SecretSources,
  WebSearchKeyringMetadata,
} from "../types";

type SettingsTab = "general" | "providers" | "permissions" | "advanced";

export type SettingsSaveInput = {
  providers?: ProviderSettings[];
  activeProviderId?: string;
  permissionMode?: "chat" | "read_only" | "auto" | "approve" | "full_access";
  sandbox?: AppSettings["sandbox"];
  webSearch?: AppSettings["webSearch"];
};

type SettingsPanelProps = {
  platform: PlatformInfo | null;
  settings: AppSettings | null;
  providerHealth: ProviderHealth[];
  providerTest: {
    providerId: string;
    status: "testing" | "complete";
    result?: ProviderHealthCheckResult;
  } | null;
  secretSources: SecretSources | null;
  notificationPreferences: TaskNotificationPreferences;
  isSaving: boolean;
  isSavingSecret: boolean;
  onSave(input: SettingsSaveInput): Promise<void>;
  onTestProvider(providerId: string, providers: ProviderSettings[]): void;
  onStoreProviderApiKey(
    providerId: string,
    value: string,
  ): Promise<KeyringMetadata | null>;
  onDeleteProviderApiKey(providerId: string): Promise<KeyringMetadata | null>;
  onStoreWebSearchApiKey(
    value: string,
  ): Promise<WebSearchKeyringMetadata | null>;
  onDeleteWebSearchApiKey(): Promise<WebSearchKeyringMetadata | null>;
  onNotificationPreferencesChange(
    preferences: TaskNotificationPreferences,
  ): void;
  onTestNotification(): void;
  onOpenLogs(): void;
  onClose(): void;
};

const settingsTabs: Array<{
  id: SettingsTab;
  label: string;
  description: string;
  keywords: string;
  icon: typeof Bell;
}> = [
  {
    id: "general",
    label: "常规",
    description: "通知与应用信息",
    keywords: "通知 提示 音效 系统 弹窗 日志 平台 backend",
    icon: Bell,
  },
  {
    id: "providers",
    label: "模型与 API",
    description: "供应商、模型和密钥",
    keywords: "api 模型 provider 供应商 导入 密钥 key url ollama openai",
    icon: Server,
  },
  {
    id: "permissions",
    label: "权限",
    description: "审批、沙箱和网络",
    keywords: "权限 审批 沙箱 网络 文件 sandbox permission",
    icon: Shield,
  },
  {
    id: "advanced",
    label: "高级",
    description: "连接状态与诊断",
    keywords: "高级 状态 诊断 健康 测试 connection health logs",
    icon: SlidersHorizontal,
  },
];

export function SettingsPanel({
  platform,
  settings,
  providerHealth,
  providerTest,
  secretSources,
  notificationPreferences,
  isSaving,
  isSavingSecret,
  onSave,
  onTestProvider,
  onStoreProviderApiKey,
  onDeleteProviderApiKey,
  onStoreWebSearchApiKey,
  onDeleteWebSearchApiKey,
  onNotificationPreferencesChange,
  onTestNotification,
  onOpenLogs,
  onClose,
}: SettingsPanelProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [searchQuery, setSearchQuery] = useState("");
  const [providers, setProviders] = useState<ProviderSettings[]>(
    settings?.providers ?? [],
  );
  const [activeProviderId, setActiveProviderId] = useState(
    settings?.activeProviderId ?? settings?.providers[0]?.id ?? "default",
  );
  const [editingProviderId, setEditingProviderId] = useState(
    settings?.activeProviderId ?? settings?.providers[0]?.id ?? null,
  );
  const [permissionMode, setPermissionMode] = useState<
    "chat" | "read_only" | "auto" | "approve" | "full_access"
  >(settings?.permissionMode ?? "auto");
  const [sandboxSettings, setSandboxSettings] = useState<
    AppSettings["sandbox"]
  >(
    settings?.sandbox ?? {
      sandboxMode: "workspace-write",
      enforcement: "enforce",
      network: "deny",
      writableRoots: [],
      readPaths: [],
    },
  );
  const [webSearch, setWebSearch] = useState<AppSettings["webSearch"]>(
    settings?.webSearch ?? {
      mode: "disabled",
      endpoint: "",
      apiKeySource: "OPENTOPIA_WEB_SEARCH_API_KEY",
      apiKeyConfigured: false,
      maxResults: 5,
    },
  );
  const [webSearchApiKey, setWebSearchApiKey] = useState("");
  const [pendingApiKeys, setPendingApiKeys] = useState<Record<string, string>>(
    {},
  );
  const [showApiKey, setShowApiKey] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [importText, setImportText] = useState("");
  const [importDraft, setImportDraft] = useState<ProviderImportDraft | null>(
    null,
  );
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [isApplyingSave, setIsApplyingSave] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const baselineRef = useRef("");

  const editingProvider =
    providers.find((provider) => provider.id === editingProviderId) ??
    providers[0] ??
    null;

  useEffect(() => {
    if (!settings) return;
    setProviders(settings.providers);
    setActiveProviderId(settings.activeProviderId);
    setEditingProviderId((current) =>
      settings.providers.some((provider) => provider.id === current)
        ? current
        : settings.activeProviderId,
    );
    setPermissionMode(settings.permissionMode);
    setSandboxSettings(settings.sandbox);
    setWebSearch(settings.webSearch);
    baselineRef.current = settingsSnapshot(
      settings.providers,
      settings.activeProviderId,
      settings.permissionMode,
      settings.sandbox,
      settings.webSearch,
    );
  }, [settings]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const currentSnapshot = settingsSnapshot(
    providers,
    activeProviderId,
    permissionMode,
    sandboxSettings,
    webSearch,
  );
  const isDirty =
    Object.values(pendingApiKeys).some(Boolean) ||
    (Boolean(baselineRef.current) && currentSnapshot !== baselineRef.current);

  const closeSafely = () => {
    if (isDirty && !window.confirm("设置尚未保存。确定要放弃这些更改吗？")) {
      return;
    }
    onClose();
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      if (importOpen) {
        setImportOpen(false);
      } else {
        closeSafely();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [importOpen, isDirty]);

  const matchingTabs = useMemo(() => {
    const query = searchQuery.trim().toLocaleLowerCase();
    if (!query) return settingsTabs;
    return settingsTabs.filter((tab) =>
      `${tab.label} ${tab.description} ${tab.keywords}`
        .toLocaleLowerCase()
        .includes(query),
    );
  }, [searchQuery]);

  useEffect(() => {
    if (
      searchQuery.trim() &&
      matchingTabs.length > 0 &&
      !matchingTabs.some((tab) => tab.id === activeTab)
    ) {
      setActiveTab(matchingTabs[0].id);
    }
  }, [activeTab, matchingTabs, searchQuery]);

  function updateProvider<K extends keyof ProviderSettings>(
    id: string,
    field: K,
    value: ProviderSettings[K],
  ) {
    setProviders((current) =>
      current.map((provider) =>
        provider.id === id ? { ...provider, [field]: value } : provider,
      ),
    );
  }

  function addProvider() {
    const id = uniqueProviderId("custom-provider", providers);
    setProviders((current) => [...current, createProviderSettings(id)]);
    setEditingProviderId(id);
    setActiveProviderId(id);
    setActiveTab("providers");
  }

  function applyImportedProvider(draft: ProviderImportDraft) {
    const id = uniqueProviderId(draft.id, providers);
    const provider = createProviderSettings(id, {
      kind: draft.kind,
      baseUrl: draft.baseUrl,
      model: draft.model,
    });
    setProviders((current) => [...current, provider]);
    setEditingProviderId(id);
    setActiveProviderId(id);
    if (draft.apiKey) {
      setPendingApiKeys((current) => ({ ...current, [id]: draft.apiKey! }));
    }
    setImportOpen(false);
    setImportText("");
    setImportDraft(null);
    setStatusMessage(
      draft.apiKey
        ? `已导入 ${draft.name}，保存时会加密写入 API 密钥。`
        : `已导入 ${draft.name}，请检查模型与密钥后保存。`,
    );
  }

  function removeProvider(id: string) {
    if (providers.length <= 1) return;
    if (!window.confirm(`确定移除供应商“${id}”吗？`)) return;
    const next = providers.filter((provider) => provider.id !== id);
    setProviders(next);
    setPendingApiKeys((current) => {
      const copy = { ...current };
      delete copy[id];
      return copy;
    });
    if (activeProviderId === id) setActiveProviderId(next[0].id);
    if (editingProviderId === id) setEditingProviderId(next[0].id);
  }

  async function submitSettings(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (isSaving || isApplyingSave) return;
    setStatusMessage(null);
    setIsApplyingSave(true);
    try {
      let nextProviders = providers;
      let nextWebSearch = webSearch;
      for (const [providerId, apiKey] of Object.entries(pendingApiKeys)) {
        if (!apiKey.trim()) continue;
        const metadata = await onStoreProviderApiKey(providerId, apiKey);
        if (!metadata) {
          setStatusMessage(
            `无法安全保存 ${providerId} 的密钥，请检查系统密钥存储。`,
          );
          return;
        }
        nextProviders = nextProviders.map((provider) =>
          provider.id === providerId
            ? {
                ...provider,
                apiKeySource: metadata.envTarget,
                apiKeyConfigured: true,
              }
            : provider,
        );
      }
      if (webSearchApiKey.trim()) {
        const metadata = await onStoreWebSearchApiKey(webSearchApiKey);
        if (!metadata) {
          setStatusMessage("无法安全保存网页搜索密钥，请检查系统密钥存储。");
          return;
        }
        nextWebSearch = {
          ...webSearch,
          apiKeySource: metadata.envTarget,
          apiKeyConfigured: true,
        };
        setWebSearch(nextWebSearch);
      }
      setProviders(nextProviders);
      await onSave({
        providers: nextProviders,
        activeProviderId,
        permissionMode,
        sandbox: sandboxSettings,
        webSearch: nextWebSearch,
      });
      setPendingApiKeys({});
      setWebSearchApiKey("");
      baselineRef.current = settingsSnapshot(
        nextProviders,
        activeProviderId,
        permissionMode,
        sandboxSettings,
        nextWebSearch,
      );
      setStatusMessage("设置已保存。");
    } finally {
      setIsApplyingSave(false);
    }
  }

  const saving = isSaving || isApplyingSave || isSavingSecret;

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={closeSafely}
    >
      <section
        className="settings-panel settings-panel-redesigned"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="settings-header">
          <div>
            <h2 id="settings-title">设置</h2>
            <p>管理 OpenTopia 的本机体验与运行配置</p>
          </div>
          <button
            type="button"
            className="icon-button"
            aria-label="关闭设置"
            title="关闭"
            onClick={closeSafely}
          >
            <X size={17} />
          </button>
        </header>

        <form className="settings-layout" onSubmit={submitSettings}>
          <aside className="settings-sidebar" aria-label="设置分类">
            <label className="settings-search">
              <Search size={15} aria-hidden="true" />
              <span className="sr-only">搜索设置</span>
              <input
                ref={searchRef}
                type="search"
                value={searchQuery}
                placeholder="搜索设置"
                onChange={(event) => setSearchQuery(event.target.value)}
              />
              {searchQuery ? (
                <button
                  type="button"
                  aria-label="清除搜索"
                  title="清除搜索"
                  onClick={() => setSearchQuery("")}
                >
                  <X size={14} />
                </button>
              ) : null}
            </label>
            <nav>
              {matchingTabs.map((tab) => {
                const Icon = tab.icon;
                return (
                  <button
                    key={tab.id}
                    type="button"
                    className={activeTab === tab.id ? "active" : ""}
                    aria-current={activeTab === tab.id ? "page" : undefined}
                    onClick={() => setActiveTab(tab.id)}
                  >
                    <Icon size={17} aria-hidden="true" />
                    <span>
                      <strong>{tab.label}</strong>
                      <small>{tab.description}</small>
                    </span>
                  </button>
                );
              })}
            </nav>
            {matchingTabs.length === 0 ? (
              <p className="settings-search-empty">没有匹配的设置</p>
            ) : null}
          </aside>

          <div className="settings-content">
            {activeTab === "general" ? (
              <GeneralSettings
                platform={platform}
                preferences={notificationPreferences}
                onChange={onNotificationPreferencesChange}
                onTestNotification={onTestNotification}
                onOpenLogs={onOpenLogs}
              />
            ) : null}
            {activeTab === "providers" ? (
              <ProviderSettingsView
                platform={platform}
                providers={providers}
                editingProvider={editingProvider}
                activeProviderId={activeProviderId}
                providerHealth={providerHealth}
                providerTest={providerTest}
                secretSources={secretSources}
                pendingApiKey={
                  editingProvider
                    ? (pendingApiKeys[editingProvider.id] ?? "")
                    : ""
                }
                showApiKey={showApiKey}
                saving={saving}
                onSelectProvider={setEditingProviderId}
                onSetActiveProvider={setActiveProviderId}
                onUpdateProvider={updateProvider}
                onAddProvider={addProvider}
                onRemoveProvider={removeProvider}
                onOpenImport={() => setImportOpen(true)}
                onPendingApiKeyChange={(providerId, apiKey) =>
                  setPendingApiKeys((current) => ({
                    ...current,
                    [providerId]: apiKey,
                  }))
                }
                onToggleApiKeyVisibility={() =>
                  setShowApiKey((value) => !value)
                }
                onDeleteProviderApiKey={async (providerId) => {
                  const metadata = await onDeleteProviderApiKey(providerId);
                  if (!metadata) return;
                  updateProvider(providerId, "apiKeyConfigured", false);
                  setPendingApiKeys((current) => ({
                    ...current,
                    [providerId]: "",
                  }));
                  setStatusMessage(`已移除 ${providerId} 的密钥。`);
                }}
                onTestProvider={onTestProvider}
              />
            ) : null}
            {activeTab === "permissions" ? (
              <PermissionSettings
                permissionMode={permissionMode}
                sandbox={sandboxSettings}
                onPermissionModeChange={(nextMode) => {
                  setPermissionMode(nextMode);
                  setSandboxSettings((current) =>
                    nextMode === "full_access"
                      ? {
                          ...current,
                          sandboxMode: "danger-full-access",
                          enforcement: "disabled",
                          network: "allow",
                        }
                      : controlledSandboxSettings(current),
                  );
                }}
                onSandboxChange={setSandboxSettings}
              />
            ) : null}
            {activeTab === "advanced" ? (
              <AdvancedSettings
                providers={providers}
                activeProviderId={activeProviderId}
                providerHealth={providerHealth}
                providerTest={providerTest}
                webSearch={webSearch}
                webSearchApiKey={webSearchApiKey}
                secretSources={secretSources}
                saving={saving}
                onWebSearchChange={setWebSearch}
                onWebSearchApiKeyChange={setWebSearchApiKey}
                onDeleteWebSearchApiKey={async () => {
                  const metadata = await onDeleteWebSearchApiKey();
                  if (!metadata) return;
                  setWebSearch((current) => ({
                    ...current,
                    apiKeyConfigured: false,
                  }));
                  setWebSearchApiKey("");
                  setStatusMessage("已移除网页搜索密钥。");
                }}
                onTestProvider={onTestProvider}
                onOpenLogs={onOpenLogs}
              />
            ) : null}
          </div>

          <footer className="settings-footer">
            <div className="settings-save-status" aria-live="polite">
              {statusMessage ?? (isDirty ? "有未保存的更改" : "所有更改已保存")}
            </div>
            <button
              type="button"
              className="secondary-button"
              onClick={closeSafely}
            >
              取消
            </button>
            <button className="primary-button" disabled={saving || !settings}>
              {saving ? "保存中…" : "保存设置"}
            </button>
          </footer>
        </form>

        {importOpen ? (
          <ProviderImportDialog
            text={importText}
            draft={importDraft}
            onTextChange={(value) => {
              setImportText(value);
              setImportDraft(null);
            }}
            onParse={() => setImportDraft(parseProviderImport(importText))}
            onApply={applyImportedProvider}
            onClose={() => setImportOpen(false)}
          />
        ) : null}
      </section>
    </div>
  );
}

function GeneralSettings({
  platform,
  preferences,
  onChange,
  onTestNotification,
  onOpenLogs,
}: {
  platform: PlatformInfo | null;
  preferences: TaskNotificationPreferences;
  onChange(preferences: TaskNotificationPreferences): void;
  onTestNotification(): void;
  onOpenLogs(): void;
}) {
  const update = <K extends keyof TaskNotificationPreferences>(
    key: K,
    value: TaskNotificationPreferences[K],
  ) => onChange({ ...preferences, [key]: value });

  return (
    <SettingsPage title="常规" description="本机通知和应用信息。">
      <SettingsGroup title="任务通知">
        <SettingsRow
          title="完成提醒"
          description="任务完成时发送提醒。"
          control={
            <Switch
              label="完成提醒"
              checked={preferences.enabled}
              onChange={(checked) => update("enabled", checked)}
            />
          }
        />
        <SettingsRow
          title="系统通知"
          description="使用 Windows、macOS 或 Linux 的原生通知。"
          disabled={!preferences.enabled}
          control={
            <Switch
              label="系统通知"
              checked={preferences.systemNotification}
              disabled={!preferences.enabled}
              onChange={(checked) => update("systemNotification", checked)}
            />
          }
        />
        <SettingsRow
          title="完成提示音"
          description="任务结束时播放简短提示音。"
          disabled={!preferences.enabled}
          control={
            <Switch
              label="完成提示音"
              checked={preferences.completionSound}
              disabled={!preferences.enabled}
              onChange={(checked) => update("completionSound", checked)}
            />
          }
        />
        <SettingsRow
          title="仅在后台提醒"
          description="窗口处于前台时不打断当前工作。"
          disabled={!preferences.enabled}
          control={
            <Switch
              label="仅在后台提醒"
              checked={preferences.onlyWhenUnfocused}
              disabled={!preferences.enabled}
              onChange={(checked) => update("onlyWhenUnfocused", checked)}
            />
          }
        />
        <div className="settings-group-actions">
          <button
            type="button"
            className="secondary-button"
            disabled={
              !preferences.enabled ||
              (!preferences.systemNotification && !preferences.completionSound)
            }
            onClick={onTestNotification}
          >
            <BellRing size={15} />
            测试提醒
          </button>
        </div>
      </SettingsGroup>

      <SettingsGroup title="应用">
        <SettingsRow
          title="运行平台"
          description={platform?.platform === "desktop" ? "桌面应用" : "浏览器"}
          control={<code>{platform?.os ?? "browser"}</code>}
        />
        <SettingsRow
          title="服务地址"
          description="OpenTopia 本地后端"
          control={
            <code>{platform?.backendUrl ?? "http://127.0.0.1:8787"}</code>
          }
        />
        <SettingsRow
          title="诊断日志"
          description="查看启动、服务与错误日志。"
          control={
            <button
              type="button"
              className="secondary-button"
              onClick={onOpenLogs}
            >
              <FileText size={15} />
              查看日志
            </button>
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

function ProviderSettingsView({
  platform,
  providers,
  editingProvider,
  activeProviderId,
  providerHealth,
  providerTest,
  secretSources,
  pendingApiKey,
  showApiKey,
  saving,
  onSelectProvider,
  onSetActiveProvider,
  onUpdateProvider,
  onAddProvider,
  onRemoveProvider,
  onOpenImport,
  onPendingApiKeyChange,
  onToggleApiKeyVisibility,
  onDeleteProviderApiKey,
  onTestProvider,
}: {
  platform: PlatformInfo | null;
  providers: ProviderSettings[];
  editingProvider: ProviderSettings | null;
  activeProviderId: string;
  providerHealth: ProviderHealth[];
  providerTest: SettingsPanelProps["providerTest"];
  secretSources: SecretSources | null;
  pendingApiKey: string;
  showApiKey: boolean;
  saving: boolean;
  onSelectProvider(id: string): void;
  onSetActiveProvider(id: string): void;
  onUpdateProvider<K extends keyof ProviderSettings>(
    id: string,
    field: K,
    value: ProviderSettings[K],
  ): void;
  onAddProvider(): void;
  onRemoveProvider(id: string): void;
  onOpenImport(): void;
  onPendingApiKeyChange(providerId: string, apiKey: string): void;
  onToggleApiKeyVisibility(): void;
  onDeleteProviderApiKey(providerId: string): Promise<void>;
  onTestProvider(providerId: string, providers: ProviderSettings[]): void;
}) {
  return (
    <SettingsPage
      title="模型与 API"
      description="管理模型供应商、连接地址与加密凭据。"
      actions={
        <>
          <button
            type="button"
            className="secondary-button"
            onClick={onOpenImport}
          >
            <Import size={15} />
            导入配置
          </button>
          <button
            type="button"
            className="secondary-button"
            onClick={onAddProvider}
          >
            <Plus size={15} />
            新建
          </button>
        </>
      }
    >
      <div className="settings-provider-workspace">
        <div className="settings-provider-list" role="list" aria-label="供应商">
          {providers.map((provider) => {
            const health = providerHealth.find(
              (item) => item.id === provider.id,
            );
            return (
              <div
                key={provider.id}
                className={`settings-provider-item ${
                  editingProvider?.id === provider.id ? "editing" : ""
                }`}
              >
                <button
                  type="button"
                  className="settings-provider-select"
                  onClick={() => onSelectProvider(provider.id)}
                >
                  <span className="settings-provider-name">
                    {provider.id === activeProviderId ? (
                      <Check size={13} />
                    ) : null}
                    {provider.id}
                  </span>
                  <small>{health?.status ?? "未检测"}</small>
                </button>
                <button
                  type="button"
                  className="icon-button small danger"
                  disabled={providers.length <= 1}
                  aria-label={`移除 ${provider.id}`}
                  title="移除供应商"
                  onClick={() => onRemoveProvider(provider.id)}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            );
          })}
        </div>

        {editingProvider ? (
          <div className="settings-provider-editor">
            <div className="settings-editor-heading">
              <div>
                <h3>{editingProvider.id}</h3>
                <span>{providerKindLabel(editingProvider.kind)}</span>
              </div>
              {editingProvider.id === activeProviderId ? (
                <span className="settings-active-badge">
                  <Check size={13} /> 默认模型
                </span>
              ) : (
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => onSetActiveProvider(editingProvider.id)}
                >
                  设为默认
                </button>
              )}
            </div>

            <div className="settings-form-grid">
              <label>
                <span>供应商类型</span>
                <select
                  value={editingProvider.kind}
                  onChange={(event) =>
                    onUpdateProvider(
                      editingProvider.id,
                      "kind",
                      event.target.value as ProviderKind,
                    )
                  }
                >
                  <option value="openai_compatible">OpenAI Compatible</option>
                  <option value="openai_responses">OpenAI Responses</option>
                  <option value="mock">Mock</option>
                </select>
              </label>
              <label>
                <span>模型</span>
                <input
                  value={editingProvider.model}
                  required
                  onChange={(event) =>
                    onUpdateProvider(
                      editingProvider.id,
                      "model",
                      event.target.value,
                    )
                  }
                />
              </label>
              <label className="settings-field-wide">
                <span>Base URL</span>
                <input
                  type="url"
                  value={editingProvider.baseUrl}
                  required
                  spellCheck={false}
                  onChange={(event) =>
                    onUpdateProvider(
                      editingProvider.id,
                      "baseUrl",
                      event.target.value,
                    )
                  }
                />
              </label>
              <label className="settings-field-wide">
                <span>API 密钥</span>
                <div className="settings-secret-input">
                  <KeyRound size={15} aria-hidden="true" />
                  <input
                    type={showApiKey ? "text" : "password"}
                    autoComplete="off"
                    value={pendingApiKey}
                    disabled={
                      platform?.platform === "desktop" &&
                      secretSources?.keyring &&
                      !secretSources.keyring.encryptionAvailable
                    }
                    placeholder={
                      editingProvider.apiKeyConfigured
                        ? "已加密保存，输入新密钥可替换"
                        : "输入密钥，保存时写入系统安全存储"
                    }
                    onChange={(event) =>
                      onPendingApiKeyChange(
                        editingProvider.id,
                        event.target.value,
                      )
                    }
                  />
                  <button
                    type="button"
                    aria-label={showApiKey ? "隐藏 API 密钥" : "显示 API 密钥"}
                    title={showApiKey ? "隐藏密钥" : "显示密钥"}
                    onClick={onToggleApiKeyVisibility}
                  >
                    {showApiKey ? <EyeOff size={15} /> : <Eye size={15} />}
                  </button>
                </div>
                <small>
                  {editingProvider.apiKeyConfigured
                    ? "密钥已加密保存；界面不会回显原文。"
                    : "密钥不会写入普通设置文件。"}
                </small>
              </label>
            </div>

            <details className="settings-advanced-fields">
              <summary>模型高级参数</summary>
              <div className="settings-form-grid">
                <label>
                  <span>Temperature</span>
                  <input
                    type="number"
                    min="0"
                    max="2"
                    step="0.1"
                    value={editingProvider.temperature}
                    onChange={(event) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "temperature",
                        Number(event.target.value),
                      )
                    }
                  />
                </label>
                <label>
                  <span>最大输出 Token</span>
                  <input
                    type="number"
                    min="1"
                    value={editingProvider.maxOutputTokens ?? ""}
                    placeholder="跟随供应商"
                    onChange={(event) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "maxOutputTokens",
                        event.target.value ? Number(event.target.value) : null,
                      )
                    }
                  />
                </label>
                <label>
                  <span>上下文窗口</span>
                  <input
                    type="number"
                    min="4096"
                    step="1024"
                    value={editingProvider.contextWindowTokens}
                    onChange={(event) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "contextWindowTokens",
                        Number(event.target.value),
                      )
                    }
                  />
                </label>
                <label>
                  <span>推理强度</span>
                  <select
                    value={editingProvider.reasoningEffort ?? ""}
                    onChange={(event) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "reasoningEffort",
                        (event.target.value ||
                          null) as ProviderSettings["reasoningEffort"],
                      )
                    }
                  >
                    <option value="">跟随供应商</option>
                    <option value="none">None</option>
                    <option value="minimal">Minimal</option>
                    <option value="low">Low</option>
                    <option value="medium">Medium</option>
                    <option value="high">High</option>
                    <option value="xhigh">Extra high</option>
                    <option value="max">Max</option>
                  </select>
                </label>
                <label className="settings-field-wide">
                  <span>Prompt cache key</span>
                  <input
                    value={editingProvider.promptCacheKey ?? ""}
                    placeholder="按工作区自动生成"
                    onChange={(event) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "promptCacheKey",
                        event.target.value || null,
                      )
                    }
                  />
                </label>
                {editingProvider.kind === "openai_responses" ? (
                  <>
                    <label>
                      <span>缓存策略</span>
                      <select
                        value={editingProvider.promptCachePolicy ?? ""}
                        onChange={(event) =>
                          onUpdateProvider(
                            editingProvider.id,
                            "promptCachePolicy",
                            (event.target.value ||
                              null) as ProviderSettings["promptCachePolicy"],
                          )
                        }
                      >
                        <option value="">自动</option>
                        <option value="explicit_30m">
                          显式断点（30 分钟）
                        </option>
                        <option value="legacy_in_memory">旧版内存缓存</option>
                        <option value="legacy_24h">旧版 24 小时缓存</option>
                      </select>
                    </label>
                    <label>
                      <span>原生压缩阈值</span>
                      <input
                        type="number"
                        min="4096"
                        step="1024"
                        value={
                          editingProvider.responsesCompactionThresholdTokens ??
                          ""
                        }
                        placeholder="关闭"
                        onChange={(event) =>
                          onUpdateProvider(
                            editingProvider.id,
                            "responsesCompactionThresholdTokens",
                            event.target.value
                              ? Number(event.target.value)
                              : null,
                          )
                        }
                      />
                    </label>
                  </>
                ) : null}
              </div>
              <div className="settings-toggle-stack">
                <SettingsRow
                  title="并行工具调用"
                  description="允许模型在同一轮并行请求多个工具。"
                  control={
                    <Switch
                      label="并行工具调用"
                      checked={editingProvider.parallelToolCalls}
                      onChange={(checked) =>
                        onUpdateProvider(
                          editingProvider.id,
                          "parallelToolCalls",
                          checked,
                        )
                      }
                    />
                  }
                />
                {editingProvider.kind === "openai_responses" ? (
                  <SettingsRow
                    title="延续 Responses 状态"
                    description="在多轮请求间保留供应商响应状态。"
                    control={
                      <Switch
                        label="延续 Responses 状态"
                        checked={editingProvider.storeResponses}
                        onChange={(checked) =>
                          onUpdateProvider(
                            editingProvider.id,
                            "storeResponses",
                            checked,
                          )
                        }
                      />
                    }
                  />
                ) : null}
              </div>
            </details>

            <div className="settings-provider-footer">
              <div className="settings-provider-health-status">
                {providerStatusChips(editingProvider, providerHealth)}
              </div>
              <div className="settings-provider-actions">
                {editingProvider.apiKeyConfigured ? (
                  <button
                    type="button"
                    className="secondary-button danger-text"
                    disabled={saving}
                    onClick={() =>
                      void onDeleteProviderApiKey(editingProvider.id)
                    }
                  >
                    移除密钥
                  </button>
                ) : null}
                <button
                  type="button"
                  className="secondary-button"
                  disabled={
                    saving ||
                    providerTest?.status === "testing" ||
                    Boolean(pendingApiKey)
                  }
                  title={pendingApiKey ? "先保存密钥，再测试连接" : undefined}
                  onClick={() => onTestProvider(editingProvider.id, providers)}
                >
                  {providerTest?.providerId === editingProvider.id &&
                  providerTest.status === "testing"
                    ? "测试中…"
                    : "测试连接"}
                </button>
              </div>
            </div>
            {providerTest?.providerId === editingProvider.id &&
            providerTest.status === "complete" ? (
              <ProviderTestResult result={providerTest.result} />
            ) : null}
          </div>
        ) : (
          <div className="settings-empty-state">没有可编辑的供应商。</div>
        )}
      </div>
    </SettingsPage>
  );
}

function PermissionSettings({
  permissionMode,
  sandbox,
  onPermissionModeChange,
  onSandboxChange,
}: {
  permissionMode: "chat" | "read_only" | "auto" | "approve" | "full_access";
  sandbox: AppSettings["sandbox"];
  onPermissionModeChange(mode: "auto" | "approve" | "full_access"): void;
  onSandboxChange(settings: AppSettings["sandbox"]): void;
}) {
  return (
    <SettingsPage title="权限" description="控制工具调用的审批与系统访问范围。">
      <SettingsGroup title="审批策略">
        <div className="settings-permission-options">
          {(
            [
              ["approve", "请求批准", "每次高风险操作前等待确认。"],
              ["auto", "自动审批", "按策略自动处理常规权限请求。"],
              ["full_access", "完全访问", "关闭系统沙箱并允许网络访问。"],
            ] as const
          ).map(([id, title, description]) => (
            <button
              key={id}
              type="button"
              className={permissionMode === id ? "active" : ""}
              aria-pressed={permissionMode === id}
              onClick={() => {
                if (
                  id === "full_access" &&
                  !window.confirm(
                    "完全访问会允许命令访问当前用户可用的文件和网络。确定继续吗？",
                  )
                ) {
                  return;
                }
                onPermissionModeChange(id);
              }}
            >
              <span>{permissionMode === id ? <Check size={15} /> : null}</span>
              <strong>{title}</strong>
              <small>{description}</small>
            </button>
          ))}
        </div>
      </SettingsGroup>

      <SettingsGroup title="沙箱">
        <div className="settings-form-grid settings-sandbox-grid">
          <label>
            <span>访问模式</span>
            <select
              value={sandbox.sandboxMode}
              onChange={(event) => {
                const sandboxMode = event.target
                  .value as AppSettings["sandbox"]["sandboxMode"];
                const danger = sandboxMode === "danger-full-access";
                onSandboxChange({
                  ...sandbox,
                  sandboxMode,
                  enforcement: danger
                    ? "disabled"
                    : sandbox.enforcement === "disabled"
                      ? "enforce"
                      : sandbox.enforcement,
                  network: danger ? "allow" : sandbox.network,
                });
              }}
            >
              <option value="read-only">只读</option>
              <option value="workspace-write">工作区可写</option>
              <option value="danger-full-access">完整系统访问</option>
            </select>
          </label>
          <label>
            <span>系统隔离</span>
            <select
              value={sandbox.enforcement}
              disabled={sandbox.sandboxMode === "danger-full-access"}
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  enforcement: event.target
                    .value as AppSettings["sandbox"]["enforcement"],
                })
              }
            >
              <option value="enforce">强制</option>
              <option value="best-effort">尽力执行</option>
              <option value="disabled">关闭</option>
            </select>
          </label>
          <label>
            <span>网络</span>
            <select
              value={sandbox.network}
              disabled={sandbox.sandboxMode === "danger-full-access"}
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  network: event.target
                    .value as AppSettings["sandbox"]["network"],
                })
              }
            >
              <option value="deny">拒绝</option>
              <option value="inherit">继承</option>
              <option value="allow">允许</option>
            </select>
          </label>
          <label className="settings-field-wide">
            <span>额外可写目录</span>
            <textarea
              rows={3}
              value={sandbox.writableRoots.join("\n")}
              placeholder="每行一个绝对路径"
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  writableRoots: parsePathList(event.target.value),
                })
              }
            />
          </label>
          <label className="settings-field-wide">
            <span>额外可读路径</span>
            <textarea
              rows={3}
              value={sandbox.readPaths.join("\n")}
              placeholder="每行一个绝对路径"
              onChange={(event) =>
                onSandboxChange({
                  ...sandbox,
                  readPaths: parsePathList(event.target.value),
                })
              }
            />
          </label>
        </div>
        {sandbox.sandboxMode === "danger-full-access" ||
        sandbox.enforcement === "disabled" ? (
          <div className="settings-danger-notice" role="status">
            <Shield size={16} />
            系统沙箱已关闭，命令可访问当前用户有权访问的文件与网络。
          </div>
        ) : sandbox.enforcement === "best-effort" ? (
          <div className="settings-warning-notice" role="status">
            <Shield size={16} />
            尽力执行模式在隔离后端不可用时可能降级运行。
          </div>
        ) : null}
      </SettingsGroup>
    </SettingsPage>
  );
}

function AdvancedSettings({
  providers,
  activeProviderId,
  providerHealth,
  providerTest,
  webSearch,
  webSearchApiKey,
  secretSources,
  saving,
  onWebSearchChange,
  onWebSearchApiKeyChange,
  onDeleteWebSearchApiKey,
  onTestProvider,
  onOpenLogs,
}: {
  providers: ProviderSettings[];
  activeProviderId: string;
  providerHealth: ProviderHealth[];
  providerTest: SettingsPanelProps["providerTest"];
  webSearch: AppSettings["webSearch"];
  webSearchApiKey: string;
  secretSources: SecretSources | null;
  saving: boolean;
  onWebSearchChange(settings: AppSettings["webSearch"]): void;
  onWebSearchApiKeyChange(apiKey: string): void;
  onDeleteWebSearchApiKey(): Promise<void>;
  onTestProvider(providerId: string, providers: ProviderSettings[]): void;
  onOpenLogs(): void;
}) {
  const activeProvider =
    providers.find((provider) => provider.id === activeProviderId) ?? null;
  return (
    <SettingsPage title="高级" description="检查模型连接状态并打开诊断信息。">
      <SettingsGroup title="网页搜索">
        <div
          className="settings-web-search-modes"
          role="radiogroup"
          aria-label="网页搜索模式"
        >
          {(
            [
              ["disabled", "关闭", "不提供网页搜索工具。"],
              ["provider_native", "供应商原生", "使用 Responses 网页搜索。"],
              ["custom_api", "自定义 API", "连接兼容的搜索端点。"],
            ] as const
          ).map(([mode, label, description]) => {
            const disabled =
              mode === "provider_native" &&
              !providers.some(
                (provider) => provider.kind === "openai_responses",
              );
            return (
              <label
                key={mode}
                className={webSearch.mode === mode ? "active" : ""}
                title={
                  disabled ? "需要先配置 OpenAI Responses 供应商" : undefined
                }
              >
                <input
                  type="radio"
                  name="web-search-mode"
                  value={mode}
                  checked={webSearch.mode === mode}
                  disabled={disabled}
                  onChange={() => onWebSearchChange({ ...webSearch, mode })}
                />
                <span>
                  <strong>{label}</strong>
                  <small>{description}</small>
                </span>
              </label>
            );
          })}
        </div>
        {webSearch.mode === "custom_api" ? (
          <div className="settings-web-search-custom">
            <div className="settings-form-grid">
              <label className="settings-field-wide">
                <span>搜索端点</span>
                <input
                  type="url"
                  required
                  value={webSearch.endpoint}
                  placeholder="https://search.example.com/v1/search"
                  onChange={(event) =>
                    onWebSearchChange({
                      ...webSearch,
                      endpoint: event.target.value,
                    })
                  }
                />
              </label>
              <label>
                <span>最大结果数</span>
                <input
                  type="number"
                  min="1"
                  max="10"
                  value={webSearch.maxResults}
                  onChange={(event) =>
                    onWebSearchChange({
                      ...webSearch,
                      maxResults: Math.min(
                        10,
                        Math.max(1, Number(event.target.value) || 1),
                      ),
                    })
                  }
                />
              </label>
              <label className="settings-field-wide">
                <span>搜索 API 密钥</span>
                <div className="settings-secret-input">
                  <KeyRound size={15} aria-hidden="true" />
                  <input
                    type="password"
                    autoComplete="off"
                    value={webSearchApiKey}
                    disabled={
                      Boolean(secretSources?.webSearchKeyring) &&
                      !secretSources?.webSearchKeyring?.encryptionAvailable
                    }
                    placeholder={
                      webSearch.apiKeyConfigured
                        ? "已加密保存，输入新密钥可替换"
                        : "保存时写入系统安全存储"
                    }
                    onChange={(event) =>
                      onWebSearchApiKeyChange(event.target.value)
                    }
                  />
                </div>
              </label>
            </div>
            {webSearch.apiKeyConfigured ? (
              <div className="settings-group-actions">
                <button
                  type="button"
                  className="secondary-button danger-text"
                  disabled={saving}
                  onClick={() => void onDeleteWebSearchApiKey()}
                >
                  移除搜索密钥
                </button>
              </div>
            ) : null}
          </div>
        ) : null}
        {webSearch.mode === "provider_native" &&
        activeProvider?.kind !== "openai_responses" ? (
          <div className="settings-warning-notice" role="status">
            <Shield size={16} />
            请将一个 OpenAI Responses 供应商设为默认模型。
          </div>
        ) : null}
      </SettingsGroup>
      <SettingsGroup title="供应商连接">
        {providers.map((provider) => {
          const health = providerHealth.find((item) => item.id === provider.id);
          const isTesting =
            providerTest?.providerId === provider.id &&
            providerTest.status === "testing";
          return (
            <SettingsRow
              key={provider.id}
              title={provider.id}
              description={`${provider.model} · ${health?.status ?? "未检测"}`}
              control={
                <button
                  type="button"
                  className="secondary-button"
                  disabled={providerTest?.status === "testing"}
                  onClick={() => onTestProvider(provider.id, providers)}
                >
                  {isTesting ? "测试中…" : "测试"}
                </button>
              }
            />
          );
        })}
      </SettingsGroup>
      <SettingsGroup title="诊断">
        <SettingsRow
          title="应用日志"
          description="查看主进程、服务与崩溃日志。"
          control={
            <button
              type="button"
              className="secondary-button"
              onClick={onOpenLogs}
            >
              <FileText size={15} />
              打开日志
            </button>
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

function ProviderImportDialog({
  text,
  draft,
  onTextChange,
  onParse,
  onApply,
  onClose,
}: {
  text: string;
  draft: ProviderImportDraft | null;
  onTextChange(value: string): void;
  onParse(): void;
  onApply(draft: ProviderImportDraft): void;
  onClose(): void;
}) {
  return (
    <div
      className="settings-import-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="settings-import-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-import-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <h3 id="provider-import-title">导入 API 配置</h3>
            <p>选择预设，或粘贴 JSON、环境变量与 curl 命令。</p>
          </div>
          <button
            type="button"
            className="icon-button"
            aria-label="关闭导入"
            title="关闭"
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </header>

        <div className="settings-import-presets">
          {PROVIDER_IMPORT_PRESETS.map((preset) => (
            <button
              key={preset.id}
              type="button"
              onClick={() => onApply(createProviderDraftFromPreset(preset.id))}
            >
              <Server size={17} />
              <span>
                <strong>{preset.name}</strong>
                <small>{preset.description}</small>
              </span>
            </button>
          ))}
        </div>

        <div className="settings-import-divider">
          <span>或粘贴配置</span>
        </div>
        <label className="settings-import-input">
          <span>配置内容</span>
          <textarea
            autoFocus
            rows={8}
            value={text}
            spellCheck={false}
            placeholder={
              "OPENAI_BASE_URL=https://example.com/v1\nOPENAI_API_KEY=...\nOPENAI_MODEL=..."
            }
            onChange={(event) => onTextChange(event.target.value)}
          />
        </label>

        {draft ? (
          <div className="settings-import-preview" aria-live="polite">
            <div className="settings-import-preview-title">
              <FileJson size={17} />
              <strong>解析结果</strong>
              <span>{formatImportFormat(draft.detectedFormat)}</span>
            </div>
            <dl>
              <div>
                <dt>供应商</dt>
                <dd>{draft.name}</dd>
              </div>
              <div>
                <dt>Base URL</dt>
                <dd>{draft.baseUrl}</dd>
              </div>
              <div>
                <dt>模型</dt>
                <dd>{draft.model}</dd>
              </div>
              <div>
                <dt>密钥</dt>
                <dd>{draft.apiKey ? "已检测，将加密保存" : "未检测"}</dd>
              </div>
            </dl>
            {draft.warnings.length > 0 ? (
              <ul>
                {draft.warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            ) : null}
          </div>
        ) : null}

        <footer>
          <button type="button" className="secondary-button" onClick={onClose}>
            取消
          </button>
          {draft ? (
            <button
              type="button"
              className="primary-button"
              onClick={() => onApply(draft)}
            >
              应用配置
            </button>
          ) : (
            <button
              type="button"
              className="primary-button"
              disabled={!text.trim()}
              onClick={onParse}
            >
              解析配置
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}

function SettingsPage({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="settings-page" aria-labelledby={`settings-${title}`}>
      <div className="settings-page-heading">
        <div>
          <h3 id={`settings-${title}`}>{title}</h3>
          <p>{description}</p>
        </div>
        {actions ? (
          <div className="settings-page-actions">{actions}</div>
        ) : null}
      </div>
      {children}
    </section>
  );
}

function SettingsGroup({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="settings-group">
      <h4>{title}</h4>
      <div className="settings-group-body">{children}</div>
    </section>
  );
}

function SettingsRow({
  title,
  description,
  control,
  disabled = false,
}: {
  title: string;
  description: string;
  control: React.ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`settings-row ${disabled ? "disabled" : ""}`}>
      <div>
        <strong>{title}</strong>
        <span>{description}</span>
      </div>
      <div className="settings-row-control">{control}</div>
    </div>
  );
}

function Switch({
  label,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange(checked: boolean): void;
}) {
  return (
    <button
      type="button"
      className="settings-switch"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

function ProviderTestResult({
  result,
}: {
  result?: ProviderHealthCheckResult;
}) {
  const success = Boolean(result?.reachable && result.modelAvailable);
  return (
    <div
      className={`settings-test-result ${success ? "success" : "error"}`}
      role="status"
    >
      {success
        ? `连接成功${result?.latencyMs ? ` · ${result.latencyMs} ms` : ""}`
        : (result?.error ?? "连接失败，请检查地址、模型和密钥。")}
    </div>
  );
}

function createProviderSettings(
  id: string,
  overrides: Partial<ProviderSettings> = {},
): ProviderSettings {
  return {
    id,
    kind: "openai_compatible",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    temperature: 0.2,
    maxOutputTokens: null,
    contextWindowTokens: 128000,
    reasoningEffort: null,
    storeResponses: false,
    parallelToolCalls: false,
    promptCacheKey: null,
    promptCachePolicy: null,
    responsesCompactionThresholdTokens: null,
    rolloutBudget: null,
    apiKeySource: "OPENTOPIA_API_KEY",
    apiKeyConfigured: false,
    healthStatus: null,
    ...overrides,
  };
}

function uniqueProviderId(
  suggestedId: string,
  providers: ProviderSettings[],
): string {
  const base =
    suggestedId
      .trim()
      .toLocaleLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "") || "custom-provider";
  const ids = new Set(providers.map((provider) => provider.id));
  if (!ids.has(base)) return base;
  let suffix = 2;
  while (ids.has(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

function settingsSnapshot(
  providers: ProviderSettings[],
  activeProviderId: string,
  permissionMode: AppSettings["permissionMode"],
  sandbox: AppSettings["sandbox"],
  webSearch: AppSettings["webSearch"],
): string {
  return JSON.stringify({
    providers,
    activeProviderId,
    permissionMode,
    sandbox,
    webSearch,
  });
}

function controlledSandboxSettings(
  sandbox: AppSettings["sandbox"],
): AppSettings["sandbox"] {
  return {
    ...sandbox,
    sandboxMode:
      sandbox.sandboxMode === "danger-full-access"
        ? "workspace-write"
        : sandbox.sandboxMode,
    enforcement:
      sandbox.enforcement === "disabled" ? "enforce" : sandbox.enforcement,
    network: sandbox.network === "allow" ? "deny" : sandbox.network,
  };
}

function parsePathList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/\r?\n/)
        .map((path) => path.trim())
        .filter(Boolean),
    ),
  ];
}

function providerKindLabel(kind: ProviderKind): string {
  if (kind === "openai_responses") return "OpenAI Responses";
  if (kind === "openai_compatible") return "OpenAI Compatible";
  return "Mock";
}

function providerStatusChips(
  provider: ProviderSettings,
  health: ProviderHealth[],
): React.ReactNode {
  const providerHealth = health.find((item) => item.id === provider.id);
  return (
    <>
      <span>{providerHealth?.status ?? "状态未知"}</span>
      <span>{provider.apiKeyConfigured ? "密钥已配置" : "未配置密钥"}</span>
      <span>{providerHealth?.usingMock ? "Mock" : "远程模型"}</span>
    </>
  );
}

function formatImportFormat(
  format: ProviderImportDraft["detectedFormat"],
): string {
  if (format === "env") return "环境变量";
  if (format === "curl") return "curl";
  if (format === "json") return "JSON";
  return "预设";
}

import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import {
  Bell,
  BellRing,
  Bot,
  Check,
  Eye,
  EyeOff,
  ExternalLink,
  FileJson,
  FileText,
  Import,
  KeyRound,
  Plus,
  Search,
  Server,
  Settings,
  Shield,
  SlidersHorizontal,
  Smile,
  Sun,
  Trash2,
  X,
} from "lucide-react";
import {
  PROVIDER_IMPORT_PRESETS,
  createProviderDraftFromPreset,
  parseProviderImport,
  type ProviderImportDraft,
} from "../providerImport";
import { openExternal } from "../platform";
import {
  availableFamiliesForModels,
  classifyModelFamily,
} from "../modelCatalog";
import { ModelInputDropdown } from "./ModelInputDropdown";
import { Badge, Button, Panel, SegmentedControl, Select, Switch } from "./ui";
import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsLayout";
import { AppearanceSettingsView } from "./AppearanceSettings";
import { PersonalizationSettingsView } from "./PersonalizationSettings";
import type { AppearanceSettings, ResolvedTheme } from "../appearance";
import type { PersonalizationSettings } from "../personalization";
import type { EditorPreferences } from "../editorPreferences";
import {
  MAX_PROVIDER_NAME_LENGTH,
  OPENAI_MODEL_CATALOG_VERIFIED_AT,
  REASONING_EFFORT_DETAILS,
  findOfficialModelPreset,
  normalizeProviderNames,
  normalizeProviderReasoningEffort,
  normalizeReasoningEffortForModel,
  providerDisplayName,
  resolveModelReasoningCapability,
} from "../providerSettings";
import type { TaskNotificationPreferences } from "../taskNotifications";
import type {
  AgentRuntimeSettings,
  AppSettings,
  CodexAccountStatus,
  CodexLoginStart,
  PlatformInfo,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderModelSyncResult,
  ProviderSecretOutcome,
  ProviderSettings,
  SecretSources,
} from "../types";

type SettingsTab =
  | "general"
  | "appearance"
  | "personalization"
  | "agent"
  | "providers"
  | "permissions"
  | "advanced";

/** Nav grouping shown in the sidebar, in render order. */
type SettingsSection = "personal" | "coding";

const settingsSectionLabels: Record<SettingsSection, string> = {
  personal: "个人",
  coding: "编码",
};

export type SettingsSaveInput = {
  providers?: ProviderSettings[];
  activeProviderId?: string;
  permissionMode?: "chat" | "read_only" | "auto" | "approve" | "full_access";
  agentRuntime?: AgentRuntimeSettings;
  sandbox?: AppSettings["sandbox"];
};

type SettingsPanelProps = {
  platform: PlatformInfo | null;
  settings: AppSettings | null;
  providerHealth: ProviderHealth[];
  codexAccount: CodexAccountStatus | null;
  codexAccountLoading: boolean;
  codexAccountError: string | null;
  providerTest: {
    providerId: string;
    status: "testing" | "complete";
    result?: ProviderHealthCheckResult;
  } | null;
  secretSources: SecretSources | null;
  notificationPreferences: TaskNotificationPreferences;
  appearance: AppearanceSettings;
  resolvedTheme: ResolvedTheme;
  personalization: PersonalizationSettings;
  editorPreferences: EditorPreferences;
  isSaving: boolean;
  isSavingSecret: boolean;
  onAppearanceChange(value: AppearanceSettings): void;
  onPersonalizationChange(value: PersonalizationSettings): void;
  onEditorPreferencesChange(value: EditorPreferences): void;
  onSave(input: SettingsSaveInput): Promise<boolean>;
  onTestProvider(providerId: string, providers: ProviderSettings[]): void;
  // Pulls the connection's model list so families can be picked from what the
  // endpoint actually serves. Includes any context limits it advertises.
  onSyncProviderModels(
    providerId: string,
  ): Promise<ProviderModelSyncResult | null>;
  onStoreProviderApiKey(
    providerId: string,
    value: string,
  ): Promise<ProviderSecretOutcome>;
  onDeleteProviderApiKey(providerId: string): Promise<ProviderSecretOutcome>;
  onRefreshCodexAccount(): void;
  onStartCodexLogin(): Promise<CodexLoginStart | null>;
  onCancelCodexLogin(): Promise<void>;
  onLogoutCodexAccount(): Promise<void>;
  onNotificationPreferencesChange(
    preferences: TaskNotificationPreferences,
  ): void;
  onTestNotification(): void;
  onOpenLogs(): void;
  onClose(): void;
};

const settingsTabs: Array<{
  id: SettingsTab;
  section: SettingsSection;
  label: string;
  description: string;
  keywords: string;
  icon: typeof Bell;
}> = [
  {
    id: "general",
    section: "personal",
    label: "常规",
    description: "编辑器、通知与应用信息",
    keywords:
      "通知 提示 音效 系统 弹窗 日志 平台 backend 编辑器 发送 快捷键 上下文",
    icon: Settings,
  },
  {
    id: "appearance",
    section: "personal",
    label: "外观",
    description: "主题、字体与密度",
    keywords:
      "外观 主题 深色 浅色 dark light 字体 字号 颜色 强调色 对比度 动画 差异 diff appearance theme",
    icon: Sun,
  },
  {
    id: "personalization",
    section: "personal",
    label: "个性化",
    description: "语气与自定义指令",
    keywords:
      "个性化 语气 个性 personality 自定义指令 instructions 提示 personalization",
    icon: Smile,
  },
  {
    id: "agent",
    section: "coding",
    label: "智能体",
    description: "风格、自治与协作",
    keywords:
      "智能体 agent 提示词 风格 自治 进度 多 agent 委派 personality autonomy",
    icon: Bot,
  },
  {
    id: "providers",
    section: "coding",
    label: "模型与 API",
    description: "供应商、模型和密钥",
    keywords: "api 模型 provider 供应商 导入 密钥 key url ollama openai",
    icon: Server,
  },
  {
    id: "permissions",
    section: "coding",
    label: "权限",
    description: "审批、沙箱和网络",
    keywords: "权限 审批 沙箱 网络 文件 sandbox permission",
    icon: Shield,
  },
  {
    id: "advanced",
    section: "coding",
    label: "高级",
    description: "连接状态与诊断",
    keywords: "高级 状态 诊断 健康 测试 connection health logs",
    icon: SlidersHorizontal,
  },
];

const settingsSectionOrder: SettingsSection[] = ["personal", "coding"];

const defaultAgentRuntimeSettings: AgentRuntimeSettings = {
  personality: "professional",
  autonomy: "balanced",
  multiAgent: "explicit",
  progressUpdates: "balanced",
};

export function SettingsPanel({
  platform,
  settings,
  providerHealth,
  codexAccount,
  codexAccountLoading,
  codexAccountError,
  providerTest,
  secretSources,
  notificationPreferences,
  appearance,
  resolvedTheme,
  personalization,
  editorPreferences,
  isSaving,
  isSavingSecret,
  onAppearanceChange,
  onPersonalizationChange,
  onEditorPreferencesChange,
  onSave,
  onTestProvider,
  onSyncProviderModels,
  onStoreProviderApiKey,
  onDeleteProviderApiKey,
  onRefreshCodexAccount,
  onStartCodexLogin,
  onCancelCodexLogin,
  onLogoutCodexAccount,
  onNotificationPreferencesChange,
  onTestNotification,
  onOpenLogs,
  onClose,
}: SettingsPanelProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [searchQuery, setSearchQuery] = useState("");
  const [providers, setProviders] = useState<ProviderSettings[]>(
    normalizeProviderNames(settings?.providers ?? []),
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
  const [agentRuntime, setAgentRuntime] = useState<AgentRuntimeSettings>(
    settings?.agentRuntime ?? defaultAgentRuntimeSettings,
  );
  const [sandboxSettings, setSandboxSettings] = useState<
    AppSettings["sandbox"]
  >(
    settings?.sandbox ?? {
      sandboxMode: "workspace-write",
      enforcement: "enforce",
      network: "allow",
      writableRoots: [],
      readPaths: [],
    },
  );
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
    const normalizedProviders = normalizeProviderNames(settings.providers);
    setProviders(normalizedProviders);
    setActiveProviderId(settings.activeProviderId);
    setEditingProviderId((current) =>
      settings.providers.some((provider) => provider.id === current)
        ? current
        : settings.activeProviderId,
    );
    setPermissionMode(settings.permissionMode);
    setAgentRuntime(settings.agentRuntime ?? defaultAgentRuntimeSettings);
    setSandboxSettings(settings.sandbox);
    baselineRef.current = settingsSnapshot(
      normalizedProviders,
      settings.activeProviderId,
      settings.permissionMode,
      settings.agentRuntime ?? defaultAgentRuntimeSettings,
      settings.sandbox,
    );
  }, [settings]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const currentSnapshot = settingsSnapshot(
    providers,
    activeProviderId,
    permissionMode,
    agentRuntime,
    sandboxSettings,
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
        provider.id === id
          ? {
              ...provider,
              [field]: value,
              ...(["baseUrl", "model", "kind"].includes(field as string)
                ? { openaiCompatibility: null }
                : {}),
            }
          : provider,
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
      name: draft.name,
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
    const provider = providers.find((item) => item.id === id);
    if (
      !window.confirm(
        `确定移除供应商“${provider ? providerDisplayName(provider) : id}”吗？`,
      )
    )
      return;
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
      let nextProviders = providers.map((provider) =>
        normalizeProviderReasoningEffort({
          ...provider,
          name: provider.name.trim(),
        }),
      );
      const invalidProvider = nextProviders.find((provider) => !provider.name);
      if (invalidProvider) {
        setEditingProviderId(invalidProvider.id);
        setActiveTab("providers");
        setStatusMessage("供应商名称不能为空。");
        return;
      }
      // A backend that fails to restart is not a failed save: the key is
      // already in the keyring. Collect those warnings and keep going so the
      // user never loses the connection they just filled in.
      const backendWarnings: string[] = [];
      for (const [providerId, apiKey] of Object.entries(pendingApiKeys)) {
        if (!apiKey.trim()) continue;
        const outcome = await onStoreProviderApiKey(providerId, apiKey);
        if (!outcome.stored) {
          setEditingProviderId(providerId);
          setActiveTab("providers");
          setStatusMessage(`无法保存 ${providerId} 的密钥：${outcome.error}`);
          return;
        }
        const { metadata } = outcome;
        if (metadata.backendRestart && !metadata.backendRestart.restarted) {
          backendWarnings.push(
            metadata.backendRestart.error ?? "本地后端未能重启。",
          );
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
      setProviders(nextProviders);
      const didSave = await onSave({
        providers: nextProviders,
        activeProviderId,
        permissionMode,
        agentRuntime,
        sandbox: sandboxSettings,
      });
      if (!didSave) {
        setStatusMessage("保存设置失败，请检查连接后重试。");
        return;
      }
      const providerToDiscover = nextProviders.find(
        (provider) => provider.id === editingProviderId,
      );
      let discovery: ProviderModelSyncResult | null = null;
      if (
        providerToDiscover &&
        providerToDiscover.apiKeyConfigured &&
        providerToDiscover.kind !== "mock" &&
        providerToDiscover.kind !== "codex_app_server"
      ) {
        const synced = await onSyncProviderModels(providerToDiscover.id);
        discovery = synced;
        if (synced) {
          nextProviders = nextProviders.map((provider) =>
            provider.id === providerToDiscover.id
              ? {
                  ...provider,
                  syncedModels: synced.models,
                  modelContextWindows: synced.modelContextWindows,
                  modelsSyncedAt: synced.syncedAt,
                }
              : provider,
          );
          setProviders(nextProviders);
        }
      }
      setPendingApiKeys({});
      baselineRef.current = settingsSnapshot(
        nextProviders,
        activeProviderId,
        permissionMode,
        agentRuntime,
        sandboxSettings,
      );
      setStatusMessage(
        backendWarnings.length > 0
          ? `设置已保存，密钥也已写入系统密钥库；但本地后端未能重启：${backendWarnings[0]} 请重启应用后再发起对话。`
          : discovery
            ? `设置已保存，已同步 ${discovery.models.length} 个模型的能力信息。`
            : "设置已保存。如果服务端不提供 /models，可手动同步模型列表。",
      );
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
            <nav aria-label="设置分类导航">
              {settingsSectionOrder.map((section) => {
                const tabs = matchingTabs.filter(
                  (tab) => tab.section === section,
                );
                if (tabs.length === 0) return null;
                return (
                  <div key={section} className="settings-nav-section">
                    <h3 className="settings-nav-section__label">
                      {settingsSectionLabels[section]}
                    </h3>
                    {tabs.map((tab) => {
                      const Icon = tab.icon;
                      return (
                        <button
                          key={tab.id}
                          type="button"
                          className={activeTab === tab.id ? "active" : ""}
                          aria-current={
                            activeTab === tab.id ? "page" : undefined
                          }
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
                  </div>
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
                editorPreferences={editorPreferences}
                onEditorPreferencesChange={onEditorPreferencesChange}
                onChange={onNotificationPreferencesChange}
                onTestNotification={onTestNotification}
                onOpenLogs={onOpenLogs}
              />
            ) : null}
            {activeTab === "appearance" ? (
              <AppearanceSettingsView
                value={appearance}
                resolvedTheme={resolvedTheme}
                onChange={onAppearanceChange}
              />
            ) : null}
            {activeTab === "personalization" ? (
              <PersonalizationSettingsView
                agentRuntime={agentRuntime}
                personalization={personalization}
                onAgentRuntimeChange={setAgentRuntime}
                onPersonalizationChange={onPersonalizationChange}
              />
            ) : null}
            {activeTab === "agent" ? (
              <AgentRuntimeSettingsView
                value={agentRuntime}
                onChange={setAgentRuntime}
              />
            ) : null}
            {activeTab === "providers" ? (
              <ProviderSettingsView
                platform={platform}
                providers={providers}
                editingProvider={editingProvider}
                activeProviderId={activeProviderId}
                providerHealth={providerHealth}
                codexAccount={codexAccount}
                codexAccountLoading={codexAccountLoading}
                codexAccountError={codexAccountError}
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
                  const outcome = await onDeleteProviderApiKey(providerId);
                  if (!outcome.stored) {
                    setStatusMessage(
                      `无法移除 ${providerId} 的密钥：${outcome.error}`,
                    );
                    return;
                  }
                  updateProvider(providerId, "apiKeyConfigured", false);
                  setPendingApiKeys((current) => ({
                    ...current,
                    [providerId]: "",
                  }));
                  const restart = outcome.metadata.backendRestart;
                  setStatusMessage(
                    restart && !restart.restarted
                      ? `已移除 ${providerId} 的密钥，但本地后端未能重启：${restart.error ?? "未知原因"}`
                      : `已移除 ${providerId} 的密钥。`,
                  );
                }}
                onTestProvider={onTestProvider}
                onSyncProviderModels={onSyncProviderModels}
                onRefreshCodexAccount={onRefreshCodexAccount}
                onStartCodexLogin={onStartCodexLogin}
                onCancelCodexLogin={onCancelCodexLogin}
                onLogoutCodexAccount={onLogoutCodexAccount}
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
                providerHealth={providerHealth}
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
  editorPreferences,
  onEditorPreferencesChange,
  onChange,
  onTestNotification,
  onOpenLogs,
}: {
  platform: PlatformInfo | null;
  preferences: TaskNotificationPreferences;
  editorPreferences: EditorPreferences;
  onEditorPreferencesChange(value: EditorPreferences): void;
  onChange(preferences: TaskNotificationPreferences): void;
  onTestNotification(): void;
  onOpenLogs(): void;
}) {
  const update = <K extends keyof TaskNotificationPreferences>(
    key: K,
    value: TaskNotificationPreferences[K],
  ) => onChange({ ...preferences, [key]: value });

  const updateEditor = <K extends keyof EditorPreferences>(
    key: K,
    value: EditorPreferences[K],
  ) => onEditorPreferencesChange({ ...editorPreferences, [key]: value });

  return (
    <SettingsPage title="常规" description="编辑器、通知和应用信息。">
      <SettingsGroup title="编辑器">
        <SettingsRow
          title="显示上下文窗口使用情况"
          description="在对话框附近显示当前轮次占用的上下文比例。"
          control={
            <Switch
              label="显示上下文窗口使用情况"
              checked={editorPreferences.showContextWindowUsage}
              onChange={(checked) =>
                updateEditor("showContextWindowUsage", checked)
              }
            />
          }
        />
        <SettingsRow
          title="发送快捷键"
          description="选择按 Enter 时是发送提示还是插入新行。"
          control={
            <Select
              label="发送快捷键"
              value={editorPreferences.sendShortcut}
              options={[
                { value: "enter", label: "按 Enter 键" },
                { value: "mod-enter", label: "按 Ctrl/Cmd + Enter" },
              ]}
              onChange={(next) => updateEditor("sendShortcut", next)}
            />
          }
        />
        <SettingsRow
          title="跟进行为"
          description="在运行时将后续指令加入队列，或引导当前运行。"
          control={
            <SegmentedControl
              label="跟进行为"
              value={editorPreferences.followUpBehavior}
              options={[
                { value: "queue", label: "排队" },
                { value: "steer", label: "引导" },
              ]}
              onChange={(next) => updateEditor("followUpBehavior", next)}
            />
          }
        />
        <SettingsRow
          title="底部面板"
          description="在应用标题栏中显示底部面板控件。"
          control={
            <Switch
              label="底部面板"
              checked={editorPreferences.showBottomPanel}
              onChange={(checked) => updateEditor("showBottomPanel", checked)}
            />
          }
        />
        <SettingsRow
          title="默认设为无项目任务"
          description="无需选择项目即可开始新任务。"
          control={
            <Switch
              label="默认设为无项目任务"
              checked={editorPreferences.allowProjectlessTasks}
              onChange={(checked) =>
                updateEditor("allowProjectlessTasks", checked)
              }
            />
          }
        />
      </SettingsGroup>

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
          control={
            <span className="settings-readonly-value">
              {platform?.os ?? "browser"}
            </span>
          }
        />
        <SettingsRow
          title="服务地址"
          description="OpenTopia 本地后端"
          control={
            <span className="settings-readonly-value">
              {platform?.backendUrl ?? "http://127.0.0.1:8787"}
            </span>
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

function AgentRuntimeSettingsView({
  value,
  onChange,
}: {
  value: AgentRuntimeSettings;
  onChange(value: AgentRuntimeSettings): void;
}) {
  return (
    <SettingsPage title="智能体" description="配置每轮任务装配的协作策略。">
      <SettingsGroup title="行为策略">
        <RuntimeChoiceGroup
          label="沟通风格"
          description="控制表达密度与协作语气。"
          value={value.personality}
          options={[
            ["focused", "专注", "直接、紧凑，以结果和关键证据为主。"],
            ["professional", "专业", "清晰说明重要判断与取舍。"],
            ["warm", "自然", "更有引导性，同时保持准确克制。"],
          ]}
          onChange={(personality) => onChange({ ...value, personality })}
        />
        <RuntimeChoiceGroup
          label="自治程度"
          description="控制已授权范围内的推进方式。"
          value={value.autonomy}
          options={[
            ["guided", "引导", "遇到重要设计选择时先与你确认。"],
            ["balanced", "平衡", "处理常规细节，只确认关键决策。"],
            ["proactive", "主动", "自主完成可逆选择并推进到验证。"],
          ]}
          onChange={(autonomy) => onChange({ ...value, autonomy })}
        />
        <RuntimeChoiceGroup
          label="多 Agent"
          description="控制内部任务委派及相关工具是否可用。"
          value={value.multiAgent}
          options={[
            ["off", "关闭", "隐藏委派工具，由当前智能体独立完成。"],
            ["explicit", "显式", "仅在你或项目规则明确要求时委派。"],
            ["adaptive", "自适应", "有明确并行收益时按边界主动委派。"],
          ]}
          onChange={(multiAgent) => onChange({ ...value, multiAgent })}
        />
        <RuntimeChoiceGroup
          label="进度更新"
          description="控制长任务中的状态同步频率。"
          value={value.progressUpdates}
          options={[
            ["milestones", "里程碑", "仅阶段完成、变化或阻塞时更新。"],
            ["balanced", "适中", "报告重要发现、决策和验证结果。"],
            ["frequent", "频繁", "在每个有意义的工作转换点更新。"],
          ]}
          onChange={(progressUpdates) =>
            onChange({ ...value, progressUpdates })
          }
        />
        <div className="settings-runtime-boundary" role="note">
          <Shield size={16} aria-hidden="true" />
          行为策略不会扩大工具权限、系统沙箱或网络范围。
        </div>
      </SettingsGroup>

      <SettingsGroup title="提示词装配">
        <div className="settings-runtime-layers">
          <div>
            <span className="settings-layer-badge fixed">固定</span>
            <strong>核心契约</strong>
            <small>安全边界、完成条件、验证纪律、上下文与 Skill 协议。</small>
          </div>
          <div>
            <span className="settings-layer-badge conditional">条件</span>
            <strong>运行时策略</strong>
            <small>当前页面选择的风格、自治、进度和多 Agent 模块。</small>
          </div>
          <div>
            <span className="settings-layer-badge dynamic">动态</span>
            <strong>每轮状态</strong>
            <small>工作区、项目规则、权限、工具、Skills 与当前环境快照。</small>
          </div>
        </div>
      </SettingsGroup>
    </SettingsPage>
  );
}

function RuntimeChoiceGroup<T extends string>({
  label,
  description,
  value,
  options,
  onChange,
}: {
  label: string;
  description: string;
  value: T;
  options: ReadonlyArray<readonly [T, string, string]>;
  onChange(value: T): void;
}) {
  return (
    <div className="settings-runtime-section">
      <div className="settings-runtime-heading">
        <strong>{label}</strong>
        <span>{description}</span>
      </div>
      <div className="settings-runtime-options" role="group" aria-label={label}>
        {options.map(([id, title, detail]) => {
          const selected = value === id;
          return (
            <button
              key={id}
              type="button"
              className={selected ? "active" : ""}
              aria-pressed={selected}
              onClick={() => onChange(id)}
            >
              <span className="settings-runtime-check">
                {selected ? <Check size={14} aria-hidden="true" /> : null}
              </span>
              <strong>{title}</strong>
              <small>{detail}</small>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ProviderSettingsView({
  platform,
  providers,
  editingProvider,
  activeProviderId,
  providerHealth,
  codexAccount,
  codexAccountLoading,
  codexAccountError,
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
  onSyncProviderModels,
  onRefreshCodexAccount,
  onStartCodexLogin,
  onCancelCodexLogin,
  onLogoutCodexAccount,
}: {
  platform: PlatformInfo | null;
  providers: ProviderSettings[];
  editingProvider: ProviderSettings | null;
  activeProviderId: string;
  providerHealth: ProviderHealth[];
  codexAccount: CodexAccountStatus | null;
  codexAccountLoading: boolean;
  codexAccountError: string | null;
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
  onSyncProviderModels(
    providerId: string,
  ): Promise<ProviderModelSyncResult | null>;
  onRefreshCodexAccount(): void;
  onStartCodexLogin(): Promise<CodexLoginStart | null>;
  onCancelCodexLogin(): Promise<void>;
  onLogoutCodexAccount(): Promise<void>;
}) {
  const usesCodexAppServer = editingProvider?.kind === "codex_app_server";
  const [manualModelProviderId, setManualModelProviderId] = useState<
    string | null
  >(null);
  const [modelSyncing, setModelSyncing] = useState(false);
  const selectedModelPreset = editingProvider
    ? findOfficialModelPreset(editingProvider.model)
    : null;
  const usesManualModel = Boolean(
    editingProvider &&
    (manualModelProviderId === editingProvider.id || !selectedModelPreset),
  );
  const reasoningCapability = editingProvider
    ? resolveModelReasoningCapability(
        editingProvider.kind,
        editingProvider.model,
      )
    : null;
  const selectedReasoningEffort = editingProvider
    ? normalizeReasoningEffortForModel(
        editingProvider.kind,
        editingProvider.model,
        editingProvider.reasoningEffort,
      )
    : null;
  const reportedContextWindow = editingProvider
    ? editingProvider.modelContextWindows?.[editingProvider.model.trim()]
    : undefined;
  const providerProtocolHint = editingProvider
    ? providerProtocolDescription(editingProvider.kind)
    : "";
  const modelSourceUrl =
    selectedModelPreset?.sourceUrl ?? reasoningCapability?.sourceUrl ?? null;
  const modelDescription = selectedModelPreset
    ? selectedModelPreset.description
    : reasoningCapability?.official
      ? reasoningCapability.status === "supported"
        ? `已识别官方能力，支持 ${reasoningCapability.supportedEfforts.length} 个推理档位。`
        : "已识别官方模型，不提供推理强度参数。"
      : "自定义模型；可使用供应商兼容档位，保存前建议完成连接测试。";

  function updateModel(model: string) {
    if (!editingProvider) return;
    onUpdateProvider(editingProvider.id, "model", model);
    const reasoningEffort = normalizeReasoningEffortForModel(
      editingProvider.kind,
      model,
      editingProvider.reasoningEffort,
    );
    if (reasoningEffort !== (editingProvider.reasoningEffort ?? null)) {
      onUpdateProvider(editingProvider.id, "reasoningEffort", reasoningEffort);
    }
  }

  async function syncModels() {
    if (!editingProvider) return;
    setModelSyncing(true);
    try {
      const result = await onSyncProviderModels(editingProvider.id);
      if (result) {
        onUpdateProvider(editingProvider.id, "syncedModels", result.models);
        onUpdateProvider(
          editingProvider.id,
          "modelContextWindows",
          result.modelContextWindows,
        );
      }
    } finally {
      setModelSyncing(false);
    }
  }

  function updateProviderKind(kind: ProviderKind | "openai") {
    if (!editingProvider) return;
    const resolvedKind: ProviderKind =
      kind === "openai"
        ? isOpenAiProviderKind(editingProvider.kind)
          ? editingProvider.kind
          : "openai_compatible"
        : kind;
    const model =
      resolvedKind === "codex_app_server"
        ? ""
        : editingProvider.model.trim() || "gpt-4.1-mini";
    onUpdateProvider(editingProvider.id, "kind", resolvedKind);
    if (model !== editingProvider.model) {
      onUpdateProvider(editingProvider.id, "model", model);
    }
    const reasoningEffort = normalizeReasoningEffortForModel(
      resolvedKind,
      model,
      editingProvider.reasoningEffort,
    );
    if (reasoningEffort !== (editingProvider.reasoningEffort ?? null)) {
      onUpdateProvider(editingProvider.id, "reasoningEffort", reasoningEffort);
    }
    if (resolvedKind === "codex_app_server") setManualModelProviderId(null);
  }

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
            const displayName = providerDisplayName(provider);
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
                  <span className="settings-provider-name">{displayName}</span>
                  <small>{health?.status ?? "未检测"}</small>
                </button>
                <button
                  type="button"
                  className="icon-button small danger"
                  disabled={providers.length <= 1}
                  aria-label={`移除 ${displayName}`}
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
                <h3>{providerDisplayName(editingProvider)}</h3>
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
              <label className="settings-field-wide">
                <span>名称</span>
                <input
                  value={editingProvider.name}
                  required
                  maxLength={MAX_PROVIDER_NAME_LENGTH}
                  aria-invalid={!editingProvider.name.trim()}
                  placeholder="例如：Kimi K3"
                  onChange={(event) =>
                    onUpdateProvider(
                      editingProvider.id,
                      "name",
                      event.target.value,
                    )
                  }
                />
                {!editingProvider.name.trim() ? (
                  <small className="settings-field-error" role="alert">
                    请输入供应商名称。
                  </small>
                ) : null}
              </label>
              <label>
                <span>供应商类型</span>
                <select
                  value={providerTypeSelection(editingProvider.kind)}
                  onChange={(event) =>
                    updateProviderKind(
                      event.target.value as ProviderKind | "openai",
                    )
                  }
                >
                  <option value="openai">
                    OpenAI Compatible（自动识别）
                  </option>
                  <option value="anthropic">Anthropic Messages</option>
                  <option value="codex_app_server">
                    Codex App Server (local)
                  </option>
                  <option value="mock">Mock</option>
                </select>
                <small>{providerProtocolHint}</small>
              </label>
              {!usesCodexAppServer ? (
                <div className="settings-field-wide">
                  <label>
                    <span>模型</span>
                    <ModelInputDropdown
                      connection={editingProvider}
                      value={editingProvider.model}
                      onChange={updateModel}
                      onSync={() => void syncModels()}
                      syncing={modelSyncing}
                    />
                  </label>
                  <div className="settings-model-meta">
                    <span>{modelDescription}</span>
                    {modelSourceUrl ? (
                      <button
                        type="button"
                        className="settings-source-link"
                        title={`官方资料，${OPENAI_MODEL_CATALOG_VERIFIED_AT} 核对`}
                        onClick={() => void openExternal(modelSourceUrl)}
                      >
                        官方资料
                        <ExternalLink size={12} aria-hidden="true" />
                      </button>
                    ) : null}
                  </div>
                </div>
              ) : null}
              {usesCodexAppServer ? (
                <div
                  className="settings-field-wide settings-provider-local-note"
                >
                  <CodexAccountSettings
                    account={codexAccount}
                    loading={codexAccountLoading}
                    error={codexAccountError}
                    onRefresh={onRefreshCodexAccount}
                    onStartLogin={onStartCodexLogin}
                    onCancelLogin={onCancelCodexLogin}
                    onLogout={onLogoutCodexAccount}
                  />
                  使用本机已安装的 Codex 及其模型配置处理本地附件；不需要 Base
                  URL、API 密钥、模型名或图片服务器。
                </div>
              ) : (
                <>
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
                        aria-label={
                          showApiKey ? "隐藏 API 密钥" : "显示 API 密钥"
                        }
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
                </>
              )}
            </div>

            <div className="settings-toggle-stack">
              <SettingsRow
                title="支持视觉输入"
                description="关闭后，带图片的请求会在发送前明确拒绝。"
                control={
                  <Switch
                    label="支持视觉输入"
                    checked={editingProvider.supportsVision}
                    onChange={(checked) =>
                      onUpdateProvider(
                        editingProvider.id,
                        "supportsVision",
                        checked,
                      )
                    }
                  />
                }
              />
            </div>

            {!usesCodexAppServer ? (
              <ModelFamilySection
                connection={editingProvider}
                onUpdateProvider={onUpdateProvider}
              />
            ) : null}

            {!usesCodexAppServer ? (
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
                      value={editingProvider.temperature ?? ""}
                      placeholder="默认（跟随模型）"
                      title="留空则不发送 temperature 参数，使用模型供应商的默认值"
                      onChange={(event) =>
                        onUpdateProvider(
                          editingProvider.id,
                          "temperature",
                          event.target.value
                            ? Number(event.target.value)
                            : null,
                        )
                      }
                    />
                    <small>
                      留空则不发送此参数，使用模型默认值。推理模型（o 系列、GPT-5）不支持自定义温度。
                    </small>
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
                          event.target.value
                            ? Number(event.target.value)
                            : null,
                        )
                      }
                    />
                  </label>
                  <div className="settings-field-wide">
                    <label>
                      <span>上下文窗口覆盖值</span>
                      <input
                        type="number"
                        min="4096"
                        step="1024"
                        value={editingProvider.contextWindowTokens ?? ""}
                        placeholder="自动识别"
                        title="仅在特殊网关或套餐需要时手动填写；留空则自动使用 API 报告、内置表或 128K 兜底"
                        onChange={(event) =>
                          onUpdateProvider(
                            editingProvider.id,
                            "contextWindowTokens",
                            event.target.value
                              ? Number(event.target.value)
                              : null,
                          )
                        }
                      />
                      <small role="status">
                        {editingProvider.contextWindowTokens !== null
                          ? `正在使用手动覆盖：${editingProvider.contextWindowTokens.toLocaleString()} tokens。它会压过 API 探测与内置模型表。`
                          : reportedContextWindow
                            ? `API /models 已报告此模型为 ${reportedContextWindow.toLocaleString()} tokens，将作为上下文上限。`
                            : "未从 API 获得此模型的上限；会依次使用内置模型表、未知模型 128K 兜底。"}
                      </small>
                    </label>
                    {editingProvider.contextWindowTokens !== null ? (
                      <Button
                        size="compact"
                        variant="quiet"
                        onClick={() =>
                          onUpdateProvider(
                            editingProvider.id,
                            "contextWindowTokens",
                            null,
                          )
                        }
                      >
                        改为自动识别
                      </Button>
                    ) : null}
                  </div>
                  {reasoningCapability?.status === "unsupported" ? (
                    <div
                      className="settings-field-wide settings-reasoning-unavailable"
                      role="status"
                    >
                      <span>推理强度</span>
                      <strong>
                        {reasoningCapability.official
                          ? "当前模型不提供推理强度"
                          : "当前供应商类型不使用此参数"}
                      </strong>
                    </div>
                  ) : (
                    <label className="settings-field-wide settings-reasoning-field">
                      <span>推理强度</span>
                      <select
                        value={selectedReasoningEffort ?? ""}
                        onChange={(event) =>
                          onUpdateProvider(
                            editingProvider.id,
                            "reasoningEffort",
                            (event.target.value ||
                              null) as ProviderSettings["reasoningEffort"],
                          )
                        }
                      >
                        <option value="">
                          {reasoningCapability?.official &&
                          reasoningCapability.defaultEffort
                            ? `自动 · 官方默认（${REASONING_EFFORT_DETAILS[reasoningCapability.defaultEffort].label}）`
                            : "自动 · 跟随供应商"}
                        </option>
                        {reasoningCapability?.supportedEfforts.map((effort) => (
                          <option key={effort} value={effort}>
                            {REASONING_EFFORT_DETAILS[effort].label}
                          </option>
                        ))}
                      </select>
                      <small>
                        {reasoningCapability?.official
                          ? `已按官方能力显示 ${reasoningCapability.supportedEfforts.length} 个可用档位。`
                          : "模型能力未知，保留兼容供应商支持的全部档位。"}
                      </small>
                    </label>
                  )}
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
            ) : null}

            <div className="settings-provider-footer">
              <div className="settings-provider-health-status">
                {providerStatusChips(editingProvider, providerHealth)}
              </div>
              <div className="settings-provider-actions">
                {!usesCodexAppServer && editingProvider.apiKeyConfigured ? (
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
            ) : editingProvider.openaiCompatibility ? (
              <OpenAiCompatibilityDetails
                report={editingProvider.openaiCompatibility}
                stored
              />
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

function CodexAccountSettings({
  account,
  loading,
  error,
  onRefresh,
  onStartLogin,
  onCancelLogin,
  onLogout,
}: {
  account: CodexAccountStatus | null;
  loading: boolean;
  error: string | null;
  onRefresh(): void;
  onStartLogin(): Promise<CodexLoginStart | null>;
  onCancelLogin(): Promise<void>;
  onLogout(): Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const loginUrl = account?.verificationUrl ?? account?.authUrl ?? null;

  async function startLogin() {
    setBusy(true);
    try {
      const login = await onStartLogin();
      const url = login?.verificationUrl ?? login?.authUrl;
      if (url) void openExternal(url);
    } finally {
      setBusy(false);
    }
  }

  async function cancelLogin() {
    setBusy(true);
    try {
      await onCancelLogin();
    } finally {
      setBusy(false);
    }
  }

  async function logout() {
    setBusy(true);
    try {
      await onLogout();
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel
      className="settings-codex-account"
      title="ChatGPT / Codex 账号"
      actions={
        <Button
          size="compact"
          variant="quiet"
          disabled={loading || busy}
          onClick={onRefresh}
        >
          刷新
        </Button>
      }
    >
      <div className="settings-codex-account-status">
        <Badge variant={account?.loggedIn ? "success" : "warning"}>
          {account?.loggedIn ? "已登录" : "未登录"}
        </Badge>
        {account?.planType ? <span>{account.planType}</span> : null}
        {account?.email ? <span>{account.email}</span> : null}
        {account?.rateLimits ? <span>额度已同步</span> : null}
        {account?.usage ? <span>用量已同步</span> : null}
      </div>

      {account?.loginPending ? (
        <div className="settings-codex-login-instructions">
          <strong>请完成 ChatGPT 登录</strong>
          {account.userCode ? <code>{account.userCode}</code> : null}
          {loginUrl ? (
            <Button
              size="compact"
              variant="primary"
              disabled={busy}
              onClick={() => void openExternal(loginUrl)}
            >
              打开登录页面
              <ExternalLink size={14} aria-hidden="true" />
            </Button>
          ) : null}
          <Button
            size="compact"
            variant="quiet"
            disabled={busy}
            onClick={() => void cancelLogin()}
          >
            取消
          </Button>
        </div>
      ) : account?.loggedIn ? (
        <Button
          size="compact"
          variant="danger"
          disabled={busy}
          onClick={() => void logout()}
        >
          退出登录
        </Button>
      ) : (
        <Button
          size="compact"
          variant="primary"
          disabled={busy || loading}
          onClick={() => void startLogin()}
        >
          登录 ChatGPT 账号
        </Button>
      )}

      {error ? (
        <p className="settings-codex-account-error" role="alert">
          {error}
        </p>
      ) : null}
      <small className="settings-codex-account-hint">
        登录后，Codex Provider 会使用该账号可用的 ChatGPT/Codex 额度；API
        Key 模式仍使用 API 平台额度。
      </small>
    </Panel>
  );
}

function AdvancedSettings({
  providers,
  providerHealth,
  onOpenLogs,
}: {
  providers: ProviderSettings[];
  providerHealth: ProviderHealth[];
  onOpenLogs(): void;
}) {
  return (
    <SettingsPage title="高级" description="检查模型连接状态并打开诊断信息。">
      <SettingsGroup title="供应商连接">
        {providers.map((provider) => {
          const displayName = providerDisplayName(provider);
          const health = providerHealth.find((item) => item.id === provider.id);
          return (
            <SettingsRow
              key={provider.id}
              title={displayName}
              description={`${provider.model} · ${health?.status ?? "未检测"}`}
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

/**
 * Per-connection model scope. Users pick whole families rather than individual
 * model ids, because one API key on an aggregator or relay already grants the
 * whole vendor lineup — enumerating every id by hand is busywork that goes
 * stale on every vendor release.
 */
function ModelFamilySection({
  connection,
  onUpdateProvider,
}: {
  connection: ProviderSettings;
  onUpdateProvider<K extends keyof ProviderSettings>(
    id: string,
    field: K,
    value: ProviderSettings[K],
  ): void;
}) {
  // The hand-configured default belongs in the catalog even when the endpoint
  // has no /v1/models, so those connections still get a usable picker.
  const modelIds = useMemo(() => {
    const ids = [...connection.syncedModels];
    const fallback = connection.model.trim();
    if (fallback && !ids.includes(fallback)) ids.push(fallback);
    return ids;
  }, [connection.syncedModels, connection.model]);

  const families = useMemo(
    () => availableFamiliesForModels(modelIds),
    [modelIds],
  );
  const modelCountByFamily = useMemo(() => {
    const counts = new Map<string, number>();
    for (const id of modelIds) {
      const familyId = classifyModelFamily(id);
      counts.set(familyId, (counts.get(familyId) ?? 0) + 1);
    }
    return counts;
  }, [modelIds]);

  // An empty allow-list means "not narrowed yet", which shows everything.
  const enabled = connection.enabledFamilies;
  const allowAll = enabled.length === 0;
  const effectivelyEnabled = allowAll
    ? families.map((family) => family.id)
    : enabled;

  function toggleFamily(familyId: string, checked: boolean) {
    const next = checked
      ? Array.from(new Set([...effectivelyEnabled, familyId]))
      : effectivelyEnabled.filter((id) => id !== familyId);
    onUpdateProvider(connection.id, "enabledFamilies", next);
  }

  return (
    <section className="settings-model-families">
      <header>
        <div>
          <strong>模型系列</strong>
          <span>
            启用后，这些系列的模型会出现在对话框的模型选择里。
            {connection.modelsSyncedAt
              ? ` 上次同步：${new Date(connection.modelsSyncedAt).toLocaleString()}。`
              : ""}
          </span>
        </div>
      </header>
      {families.length === 0 ? (
        <p className="settings-model-families-note">
          还没有模型列表。请在上方模型字段旁点击同步按钮拉取。
        </p>
      ) : (
        <div className="settings-toggle-stack">
          {families.map((family) => (
            <SettingsRow
              key={family.id}
              title={family.label}
              description={`${family.vendor} · ${modelCountByFamily.get(family.id) ?? 0} 个模型`}
              control={
                <Switch
                  label={`启用 ${family.label}`}
                  checked={effectivelyEnabled.includes(family.id)}
                  // Turning off the last family would empty the list, which the
                  // "empty means all" convention would read as re-enabling
                  // everything. Keep at least one on instead.
                  disabled={
                    effectivelyEnabled.length <= 1 &&
                    effectivelyEnabled.includes(family.id)
                  }
                  onChange={(checked) => toggleFamily(family.id, checked)}
                />
              }
            />
          ))}
        </div>
      )}
    </section>
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
      <div>
        {success
          ? `连接成功${result?.latencyMs ? ` · ${result.latencyMs} ms` : ""}`
          : (result?.error ?? "连接失败，请检查地址、模型和密钥。")}
      </div>
      {result?.openaiCompatibility ? (
        <OpenAiCompatibilityDetails report={result.openaiCompatibility} />
      ) : null}
    </div>
  );
}

function OpenAiCompatibilityDetails({
  report,
  stored = false,
}: {
  report: NonNullable<ProviderSettings["openaiCompatibility"]>;
  stored?: boolean;
}) {
  return (
    <div
      className={`settings-compatibility-report${stored ? " stored" : ""}`}
      role={stored ? "status" : undefined}
    >
      <div className="settings-compatibility-heading">
        {stored ? "已缓存的兼容性检测" : "兼容性检测结果"}
        <span>{new Date(report.checkedAt).toLocaleString()}</span>
      </div>
      <div className="settings-compatibility-grid">
        <CompatibilityItem
          label="采用协议"
          value={
            report.selectedProtocol === "responses"
              ? "Responses"
              : "Chat Completions"
          }
          state="supported"
        />
        <CompatibilityItem
          label="Chat Completions"
          value={providerFeatureSupportLabel(report.chatCompletions)}
          state={report.chatCompletions}
        />
        <CompatibilityItem
          label="Chat function tools"
          value={providerFeatureSupportLabel(report.chatFunctionTools)}
          state={report.chatFunctionTools}
        />
        <CompatibilityItem
          label="Responses"
          value={providerFeatureSupportLabel(report.responses)}
          state={report.responses}
        />
        <CompatibilityItem
          label="Responses native tools"
          value={providerFeatureSupportLabel(report.responsesNativeTools)}
          state={report.responsesNativeTools}
        />
        <CompatibilityItem
          label="developer 角色"
          value={providerFeatureSupportLabel(report.developerMessages)}
          state={report.developerMessages}
        />
      </div>
      <div
        className={`settings-compatibility-mode ${
          report.messageCompatibility ? "compatibility" : "native"
        }`}
      >
        {report.selectedProtocol === "responses"
          ? "已选择 Responses 适配器；系统与开发者指令会通过 Responses 原生 instructions 发送。"
          : report.messageCompatibility
            ? "已启用兼容模式：运行时会直接合并 developer 指令并扁平化结构化工具历史，避免每轮先触发 400。"
            : "使用原生 Chat 消息格式；若运行时首次遇到同类 400，会自动学习并缓存兼容模式。"}
      </div>
    </div>
  );
}

function CompatibilityItem({
  label,
  value,
  state,
}: {
  label: string;
  value: string;
  state: "supported" | "unsupported" | "unknown";
}) {
  return (
    <div className="settings-compatibility-item">
      <span>{label}</span>
      <strong className={state}>{value}</strong>
    </div>
  );
}

function createProviderSettings(
  id: string,
  overrides: Partial<ProviderSettings> = {},
): ProviderSettings {
  return {
    id,
    name: id,
    kind: "openai_compatible",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    enabledFamilies: [],
    syncedModels: [],
    modelsSyncedAt: null,
    temperature: null,
    maxOutputTokens: null,
    contextWindowTokens: null,
    reasoningEffort: null,
    storeResponses: false,
    parallelToolCalls: false,
    promptCacheKey: null,
    promptCachePolicy: null,
    responsesCompactionThresholdTokens: null,
    rolloutBudget: null,
    supportsVision: true,
    openaiCompatibility: null,
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
  agentRuntime: AgentRuntimeSettings,
  sandbox: AppSettings["sandbox"],
): string {
  return JSON.stringify({
    providers,
    activeProviderId,
    permissionMode,
    agentRuntime,
    sandbox,
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
    network: sandbox.network,
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
  if (kind === "codex_app_server") return "Codex App Server";
  if (kind === "anthropic") return "Anthropic Messages";
  if (isOpenAiProviderKind(kind)) return "OpenAI Compatible";
  return "Mock";
}

function providerProtocolDescription(kind: ProviderKind): string {
  if (isOpenAiProviderKind(kind)) {
    return "填写统一的 /v1 Base URL；连接测试会自动识别 Chat Completions、Responses 与消息格式兼容性。";
  }
  if (kind === "anthropic") {
    return "Anthropic Messages：请求会发到 Base URL + /v1/messages，并使用 x-api-key 认证。";
  }
  if (kind === "codex_app_server") {
    return "使用本机 Codex App Server，不需要 URL 或 API 密钥。";
  }
  return "仅用于本地模拟，不会调用远程 API。";
}

function isOpenAiProviderKind(kind: ProviderKind): boolean {
  return kind === "openai_compatible" || kind === "openai_responses";
}

function providerTypeSelection(kind: ProviderKind): ProviderKind | "openai" {
  return isOpenAiProviderKind(kind) ? "openai" : kind;
}

function providerFeatureSupportLabel(
  support: "supported" | "unsupported" | "unknown",
): string {
  if (support === "supported") return "支持";
  if (support === "unsupported") return "不支持";
  return "未确认";
}

function providerStatusChips(
  provider: ProviderSettings,
  health: ProviderHealth[],
): React.ReactNode {
  const providerHealth = health.find((item) => item.id === provider.id);
  return (
    <>
      <span>{providerHealth?.status ?? "状态未知"}</span>
      <span>
        {provider.kind === "codex_app_server"
          ? "本地 Codex"
          : provider.apiKeyConfigured
            ? "密钥已配置"
            : "未配置密钥"}
      </span>
      <span>{providerHealth?.usingMock ? "Mock" : "模型"}</span>
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

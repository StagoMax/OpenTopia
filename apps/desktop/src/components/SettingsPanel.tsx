import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  ArrowLeft,
  Bell,
  BellRing,
  Bot,
  Check,
  FileText,
  Search,
  Server,
  Settings,
  Shield,
  SlidersHorizontal,
  Smile,
  Sun,
  X,
} from "lucide-react";
import {
  parseProviderImport,
  type ProviderImportDraft,
} from "../providerImport";
import { Button, SegmentedControl, Select, Switch } from "./ui";
import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsLayout";
import {
  AdvancedSettings,
  PermissionSettings,
} from "./settings/RuntimeSettingsPages";
import { ProviderImportDialog } from "./settings/ProviderImportDialog";
import { ProviderSettingsView } from "./settings/ProviderSettingsView";
import {
  controlledSandboxSettings,
  createProviderSettings,
  providerAllowedAdapters,
  providerDiscoverySignature,
  providerEffectiveAuth,
  providerEffectiveTransport,
  providerSettingsSnapshot,
  uniqueProviderId,
  type ModelDiscoveryState,
} from "./settings/providerSettingsModel";
import { AppearanceSettingsView } from "./AppearanceSettings";
import { PersonalizationSettingsView } from "./PersonalizationSettings";
import type { AppearanceSettings, ResolvedTheme } from "../appearance";
import type { PersonalizationSettings } from "../personalization";
import type { EditorPreferences } from "../editorPreferences";
import {
  normalizeProviderNames,
  normalizeProviderReasoningEffort,
  providerDisplayName,
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
  ProviderModelSyncResult,
  ProviderSecretOutcome,
  ProviderSettings,
  SecretSources,
  WindowsSandboxSetupStatus,
} from "../types";

export type SettingsTab =
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

type SettingsSidebarResize = {
  width: number;
  minWidth: number;
  maxWidth: number;
  isResizing: boolean;
  onPointerDown(event: ReactPointerEvent<HTMLDivElement>): void;
  onPointerMove(event: ReactPointerEvent<HTMLDivElement>): void;
  onPointerUp(event: ReactPointerEvent<HTMLDivElement>): void;
  onPointerCancel(event: ReactPointerEvent<HTMLDivElement>): void;
  onLostPointerCapture(event: ReactPointerEvent<HTMLDivElement>): void;
  onDoubleClick(): void;
  onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>): void;
};

type SettingsPanelProps = {
  initialTab: SettingsTab;
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
  sidebarResize: SettingsSidebarResize;
  onAppearanceChange(value: AppearanceSettings): void;
  onPersonalizationChange(value: PersonalizationSettings): void;
  onEditorPreferencesChange(value: EditorPreferences): void;
  onSave(input: SettingsSaveInput): Promise<boolean>;
  onTestProvider(providerId: string, providers?: ProviderSettings[]): void;
  // Pulls the connection's model list so families can be picked from what the
  // endpoint actually serves. Includes any context limits it advertises.
  onSyncProviderModels(providerId: string): Promise<ProviderModelSyncResult>;
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
  windowsSandboxSetup: WindowsSandboxSetupStatus | null;
  windowsSandboxSetupBusy: boolean;
  windowsSandboxSetupError: string | null;
  onSetupWindowsSandbox(): Promise<WindowsSandboxSetupStatus>;
  onRemoveWindowsSandbox(): Promise<WindowsSandboxSetupStatus>;
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
  initialTab,
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
  sidebarResize,
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
  windowsSandboxSetup,
  windowsSandboxSetupBusy,
  windowsSandboxSetupError,
  onSetupWindowsSandbox,
  onRemoveWindowsSandbox,
  onOpenLogs,
  onClose,
}: SettingsPanelProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);
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
  const initializedSettingsRef = useRef(false);
  const providersRef = useRef(providers);
  const pendingApiKeysRef = useRef(pendingApiKeys);
  const activeProviderIdRef = useRef(activeProviderId);
  const modelDiscoveryAttemptsRef = useRef<Record<string, string>>({});
  const modelDiscoveryInFlightRef = useRef(new Set<string>());
  const [modelDiscoveryStates, setModelDiscoveryStates] = useState<
    Record<string, ModelDiscoveryState>
  >({});
  const automaticSaveTimerRef = useRef<number | null>(null);
  const pendingAutomaticSaveRef = useRef<SettingsSaveInput | null>(null);
  const automaticSaveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const [autoSaveError, setAutoSaveError] = useState<string | null>(null);

  const editingProvider =
    providers.find((provider) => provider.id === editingProviderId) ??
    providers[0] ??
    null;

  const providerSnapshot = providerSettingsSnapshot(
    providers,
    activeProviderId,
  );
  const isProviderDirty =
    Object.values(pendingApiKeys).some(Boolean) ||
    (Boolean(baselineRef.current) && providerSnapshot !== baselineRef.current);

  useEffect(() => {
    providersRef.current = providers;
  }, [providers]);

  useEffect(() => {
    pendingApiKeysRef.current = pendingApiKeys;
  }, [pendingApiKeys]);

  useEffect(() => {
    activeProviderIdRef.current = activeProviderId;
  }, [activeProviderId]);

  useEffect(() => {
    if (!settings) return;
    const normalizedProviders = normalizeProviderNames(settings.providers);
    const serverProviderSnapshot = providerSettingsSnapshot(
      normalizedProviders,
      settings.activeProviderId,
    );
    if (!initializedSettingsRef.current) {
      initializedSettingsRef.current = true;
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
      baselineRef.current = serverProviderSnapshot;
      return;
    }

    // Automatic preference updates must not overwrite an unsaved API draft.
    if (providerSnapshot === baselineRef.current) {
      setProviders(normalizedProviders);
      setActiveProviderId(settings.activeProviderId);
      setEditingProviderId((current) =>
        settings.providers.some((provider) => provider.id === current)
          ? current
          : settings.activeProviderId,
      );
      baselineRef.current = serverProviderSnapshot;
    }
  }, [settings]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  function flushAutomaticSave(): Promise<void> {
    if (automaticSaveTimerRef.current !== null) {
      window.clearTimeout(automaticSaveTimerRef.current);
      automaticSaveTimerRef.current = null;
    }
    const input = pendingAutomaticSaveRef.current;
    if (!input) return automaticSaveQueueRef.current;
    pendingAutomaticSaveRef.current = null;
    const save = async () => {
      const didSave = await onSave(input);
      setAutoSaveError(didSave ? null : "自动保存失败，请检查连接后重试。");
    };
    automaticSaveQueueRef.current = automaticSaveQueueRef.current.then(
      save,
      save,
    );
    return automaticSaveQueueRef.current;
  }

  function scheduleAutomaticSave(input: SettingsSaveInput) {
    if (!settings) return;
    pendingAutomaticSaveRef.current = {
      ...pendingAutomaticSaveRef.current,
      ...input,
    };
    if (automaticSaveTimerRef.current !== null) {
      window.clearTimeout(automaticSaveTimerRef.current);
    }
    automaticSaveTimerRef.current = window.setTimeout(() => {
      automaticSaveTimerRef.current = null;
      void flushAutomaticSave();
    }, 250);
  }

  useEffect(
    () => () => {
      if (automaticSaveTimerRef.current !== null) {
        window.clearTimeout(automaticSaveTimerRef.current);
      }
    },
    [],
  );

  function discardProviderDraft() {
    if (!settings) return;
    const savedProviders = normalizeProviderNames(settings.providers);
    const savedActiveProviderId = settings.activeProviderId;
    setProviders(savedProviders);
    providersRef.current = savedProviders;
    setActiveProviderId(savedActiveProviderId);
    activeProviderIdRef.current = savedActiveProviderId;
    setEditingProviderId((current) =>
      savedProviders.some((provider) => provider.id === current)
        ? current
        : savedActiveProviderId,
    );
    setPendingApiKeys({});
    pendingApiKeysRef.current = {};
    setModelDiscoveryStates({});
    modelDiscoveryAttemptsRef.current = {};
    baselineRef.current = providerSettingsSnapshot(
      savedProviders,
      savedActiveProviderId,
    );
  }

  const closeSafely = () => {
    if (isProviderDirty) {
      const discard = window.confirm(
        "API 配置尚未保存。确定要放弃这些更改吗？",
      );
      if (!discard) return;
      discardProviderDraft();
    }
    void flushAutomaticSave();
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
  }, [importOpen, isProviderDirty]);

  function updateAgentRuntime(next: AgentRuntimeSettings) {
    setAgentRuntime(next);
    scheduleAutomaticSave({ agentRuntime: next });
  }

  function updatePermissionMode(nextMode: "auto" | "approve" | "full_access") {
    const nextSandbox =
      nextMode === "full_access"
        ? {
            ...sandboxSettings,
            sandboxMode: "danger-full-access" as const,
            enforcement: "disabled" as const,
            network: "allow" as const,
          }
        : controlledSandboxSettings(sandboxSettings);
    setPermissionMode(nextMode);
    setSandboxSettings(nextSandbox);
    scheduleAutomaticSave({
      permissionMode: nextMode,
      sandbox: nextSandbox,
    });
  }

  function updateSandbox(nextSandbox: AppSettings["sandbox"]) {
    setSandboxSettings(nextSandbox);
    scheduleAutomaticSave({ sandbox: nextSandbox });
  }

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
    setProviders((current) => {
      const next = current.map((provider) =>
        provider.id === id
          ? {
              ...provider,
              [field]: value,
              ...(["baseUrl", "model", "kind"].includes(field as string)
                ? { openaiCompatibility: null }
                : {}),
            }
          : provider,
      );
      providersRef.current = next;
      return next;
    });
  }

  function updatePendingApiKey(providerId: string, apiKey: string) {
    setPendingApiKeys((current) => {
      const next = { ...current, [providerId]: apiKey };
      pendingApiKeysRef.current = next;
      return next;
    });
  }

  function setModelDiscoveryState(
    providerId: string,
    state: ModelDiscoveryState,
  ) {
    setModelDiscoveryStates((current) => ({ ...current, [providerId]: state }));
  }

  async function discoverProviderModels(providerId: string): Promise<void> {
    if (modelDiscoveryInFlightRef.current.has(providerId)) return;

    const initialProvider = providersRef.current.find(
      (provider) => provider.id === providerId,
    );
    if (!initialProvider) return;

    const initialApiKey =
      providerEffectiveAuth(initialProvider) === "none"
        ? ""
        : (pendingApiKeysRef.current[providerId]?.trim() ?? "");
    const signature = providerDiscoverySignature(
      initialProvider,
      initialApiKey,
    );
    if (!signature) {
      const needsApiKey = providerEffectiveAuth(initialProvider) !== "none";
      setModelDiscoveryState(providerId, {
        status: "error",
        message: needsApiKey
          ? "请先填写有效的 Base URL 和 API 密钥。"
          : "请先填写有效的 Base URL。",
      });
      return;
    }

    modelDiscoveryInFlightRef.current.add(providerId);
    modelDiscoveryAttemptsRef.current[providerId] = signature;
    setModelDiscoveryState(providerId, { status: "discovering" });

    try {
      let nextProviders = providersRef.current;
      if (initialApiKey) {
        const outcome = await onStoreProviderApiKey(providerId, initialApiKey);
        if (!outcome.stored) {
          setModelDiscoveryState(providerId, {
            status: "error",
            message: `无法保存 API 密钥：${outcome.error}`,
          });
          return;
        }

        nextProviders = providersRef.current.map((provider) =>
          provider.id === providerId
            ? {
                ...provider,
                apiKeySource: outcome.metadata.envTarget,
                apiKeyConfigured: true,
              }
            : provider,
        );
        providersRef.current = nextProviders;
        setProviders(nextProviders);
      }

      const didSave = await onSave({
        providers: nextProviders,
        activeProviderId: activeProviderIdRef.current,
      });
      if (!didSave) {
        setModelDiscoveryState(providerId, {
          status: "error",
          message: "连接配置保存失败，无法识别模型。",
        });
        return;
      }

      if (
        initialApiKey &&
        pendingApiKeysRef.current[providerId]?.trim() === initialApiKey
      ) {
        const nextPendingApiKeys = {
          ...pendingApiKeysRef.current,
          [providerId]: "",
        };
        pendingApiKeysRef.current = nextPendingApiKeys;
        setPendingApiKeys(nextPendingApiKeys);
      }

      const result = await onSyncProviderModels(providerId);
      nextProviders = providersRef.current.map((provider) =>
        provider.id === providerId ? result.provider : provider,
      );
      providersRef.current = nextProviders;
      setProviders(nextProviders);
      baselineRef.current = providerSettingsSnapshot(
        nextProviders,
        activeProviderIdRef.current,
      );

      const completedProvider = nextProviders.find(
        (provider) => provider.id === providerId,
      );
      const currentApiKey = pendingApiKeysRef.current[providerId]?.trim() ?? "";
      if (completedProvider && !currentApiKey) {
        const completedSignature = providerDiscoverySignature(
          completedProvider,
          currentApiKey,
        );
        if (completedSignature) {
          modelDiscoveryAttemptsRef.current[providerId] = completedSignature;
        }
      }
      setModelDiscoveryState(providerId, {
        status: "success",
        modelCount: result.models.length,
      });
    } catch (error) {
      setModelDiscoveryState(providerId, {
        status: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      modelDiscoveryInFlightRef.current.delete(providerId);
      // A URL or key may have changed while the request was in flight. Trigger
      // the effect once more so that newer input is discovered next.
      setModelDiscoveryStates((current) => ({ ...current }));
    }
  }

  const editingProviderBaseUrl = editingProvider?.baseUrl ?? "";
  const editingProviderKind = editingProvider?.kind ?? null;
  const editingProviderApiKey = editingProvider
    ? (pendingApiKeys[editingProvider.id] ?? "")
    : "";
  const editingProviderApiKeyConfigured =
    editingProvider?.apiKeyConfigured ?? false;

  useEffect(() => {
    if (!editingProvider) return;
    const signature = providerDiscoverySignature(
      editingProvider,
      editingProviderApiKey.trim(),
    );
    if (
      !signature ||
      modelDiscoveryAttemptsRef.current[editingProvider.id] === signature ||
      modelDiscoveryInFlightRef.current.has(editingProvider.id)
    ) {
      return;
    }

    const timer = window.setTimeout(() => {
      void discoverProviderModels(editingProvider.id);
    }, 600);
    return () => window.clearTimeout(timer);
  }, [
    editingProvider,
    editingProviderApiKey,
    editingProviderApiKeyConfigured,
    editingProviderBaseUrl,
    editingProviderKind,
    modelDiscoveryStates,
  ]);

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
    await flushAutomaticSave();
    setStatusMessage(null);
    setIsApplyingSave(true);
    try {
      let nextProviders = providers.map((provider) =>
        normalizeProviderReasoningEffort({
          ...provider,
          name: provider.name.trim(),
        }),
      );
      const invalidProvider = nextProviders.find(
        (provider) =>
          providerEffectiveTransport(provider) === "http" &&
          !provider.baseUrl.trim(),
      );
      if (invalidProvider) {
        setEditingProviderId(invalidProvider.id);
        setActiveTab("providers");
        setStatusMessage("请先填写 Base URL。");
        return;
      }
      // A backend that fails to restart is not a failed save: the key is
      // already in the keyring. Collect those warnings and keep going so the
      // user never loses the connection they just filled in.
      const backendWarnings: string[] = [];
      for (const [providerId, apiKey] of Object.entries(pendingApiKeys)) {
        if (!apiKey.trim()) continue;
        const provider = nextProviders.find(
          (candidate) => candidate.id === providerId,
        );
        const auth = provider ? providerEffectiveAuth(provider) : null;
        if (!provider || (auth !== "bearer" && auth !== "x_api_key")) continue;
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
      providersRef.current = nextProviders;
      setProviders(nextProviders);
      const didSave = await onSave({
        providers: nextProviders,
        activeProviderId,
      });
      if (!didSave) {
        setStatusMessage("保存设置失败，请检查连接后重试。");
        return;
      }
      setPendingApiKeys({});
      pendingApiKeysRef.current = {};
      baselineRef.current = providerSettingsSnapshot(
        nextProviders,
        activeProviderId,
      );
      setStatusMessage(
        backendWarnings.length > 0
          ? `设置已保存，密钥也已写入系统密钥库；但本地后端未能重启：${backendWarnings[0]} 请重启应用后再发起对话。`
          : "设置已保存。填写连接信息后会自动识别模型。",
      );
    } finally {
      setIsApplyingSave(false);
    }
  }

  const saving = isSaving || isApplyingSave || isSavingSecret;
  const layoutStyle = {
    "--settings-sidebar-width": `${sidebarResize.width}px`,
  } as CSSProperties;

  return (
    <section
      className={`settings-panel settings-panel-redesigned${
        sidebarResize.isResizing ? " is-resizing" : ""
      }`}
      style={layoutStyle}
    >
      <form className="settings-layout" onSubmit={submitSettings}>
        <aside
          id="settings-sidebar"
          className="settings-sidebar"
          aria-label="设置分类"
        >
          <header className="settings-sidebar-header">
            <Button
              className="settings-return-button"
              variant="quiet"
              onClick={closeSafely}
            >
              <ArrowLeft size={16} aria-hidden="true" focusable="false" />
              返回应用
            </Button>
          </header>
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
                        aria-current={activeTab === tab.id ? "page" : undefined}
                        onClick={() => setActiveTab(tab.id)}
                      >
                        <Icon size={16} aria-hidden="true" />
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

        <div
          className={`workspace-resizer settings-resizer ${
            sidebarResize.isResizing ? "active" : ""
          }`}
          role="separator"
          tabIndex={0}
          aria-label="调整设置侧栏宽度"
          aria-controls="settings-sidebar"
          aria-orientation="vertical"
          aria-valuemin={sidebarResize.minWidth}
          aria-valuemax={sidebarResize.maxWidth}
          aria-valuenow={sidebarResize.width}
          aria-valuetext={`${sidebarResize.width} 像素`}
          onPointerDown={sidebarResize.onPointerDown}
          onPointerMove={sidebarResize.onPointerMove}
          onPointerUp={sidebarResize.onPointerUp}
          onPointerCancel={sidebarResize.onPointerCancel}
          onLostPointerCapture={sidebarResize.onLostPointerCapture}
          onDoubleClick={sidebarResize.onDoubleClick}
          onKeyDown={sidebarResize.onKeyDown}
        />

        <div className="settings-workspace">
          <h2 id="settings-title" className="ot-sr-only">
            设置
          </h2>
          <div className="settings-content">
            {autoSaveError ? (
              <p className="settings-auto-save-error" role="alert">
                {autoSaveError}
              </p>
            ) : null}
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
                onAgentRuntimeChange={updateAgentRuntime}
                onPersonalizationChange={onPersonalizationChange}
              />
            ) : null}
            {activeTab === "agent" ? (
              <AgentRuntimeSettingsView
                value={agentRuntime}
                onChange={updateAgentRuntime}
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
                hasUnsavedChanges={isProviderDirty}
                statusMessage={statusMessage}
                onSelectProvider={setEditingProviderId}
                onSetActiveProvider={setActiveProviderId}
                onUpdateProvider={updateProvider}
                onAddProvider={addProvider}
                onRemoveProvider={removeProvider}
                onOpenImport={() => setImportOpen(true)}
                onPendingApiKeyChange={updatePendingApiKey}
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
                  updatePendingApiKey(providerId, "");
                  const restart = outcome.metadata.backendRestart;
                  setStatusMessage(
                    restart && !restart.restarted
                      ? `已移除 ${providerId} 的密钥，但本地后端未能重启：${restart.error ?? "未知原因"}`
                      : `已移除 ${providerId} 的密钥。`,
                  );
                }}
                onTestProvider={(providerId) =>
                  onTestProvider(providerId, undefined)
                }
                modelDiscovery={
                  editingProvider
                    ? (modelDiscoveryStates[editingProvider.id] ?? {
                        status: "idle",
                      })
                    : { status: "idle" }
                }
                onDiscoverProvider={discoverProviderModels}
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
                isWindows={
                  platform?.os === "win32" || platform?.os === "windows"
                }
                onPermissionModeChange={updatePermissionMode}
                onSandboxChange={updateSandbox}
                windowsSetup={windowsSandboxSetup}
                windowsSetupBusy={windowsSandboxSetupBusy}
                windowsSetupError={windowsSandboxSetupError}
                onSetupWindowsSandbox={onSetupWindowsSandbox}
                onRemoveWindowsSandbox={onRemoveWindowsSandbox}
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
        </div>
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
            <small>工作区、项目规则、权限、选定 Skills 与当前环境状态。</small>
          </div>
          <div>
            <span className="settings-layer-badge conditional">独立</span>
            <strong>Provider 工具面</strong>
            <small>
              直接工具的完整 Schema 通过 tools / dynamicTools
              传输；延迟工具先暴露目录，Tool Search 后再加载所选
              Schema。两者都不拼入文字提示词。
            </small>
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

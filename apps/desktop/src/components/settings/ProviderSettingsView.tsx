import { useState, type ReactNode } from "react";
import {
  Check,
  Eye,
  EyeOff,
  ExternalLink,
  Import,
  KeyRound,
  Plus,
  Trash2,
} from "lucide-react";
import { openExternal } from "../../platform";
import {
  MAX_PROVIDER_NAME_LENGTH,
  OPENAI_MODEL_CATALOG_VERIFIED_AT,
  findOfficialModelPreset,
  normalizeReasoningEffortForModel,
  providerDisplayName,
  resolveModelReasoningCapability,
} from "../../providerSettings";
import type {
  CodexAccountStatus,
  CodexLoginStart,
  PlatformInfo,
  ProviderAdapterKind,
  ProviderAuthKind,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderSettings,
  ProviderTransportKind,
  SecretSources,
} from "../../types";
import { ModelInputDropdown } from "../ModelInputDropdown";
import { InputDropdown } from "../ui";
import { SettingsPage } from "../SettingsLayout";
import { CodexAccountSettings } from "./RuntimeSettingsPages";
import { ProviderAdvancedDefaults } from "./ProviderAdvancedDefaults";
import {
  ModelConfigurationSection,
  ModelDiscoveryStatus,
  ModelFamilySection,
} from "./ProviderModels";
import {
  isOpenAiProviderKind,
  providerAllowedAdapters,
  providerAxesFromKind,
  providerBaseUrlPresetOptions,
  providerBaseUrlPresets,
  providerEffectiveAuth,
  providerEffectiveTransport,
  providerHealthStatusLabel,
  providerKindLabel,
  providerProtocolDescription,
  providerProtocolSelection,
  type ModelDiscoveryState,
  type ProviderProtocolSelection,
} from "./providerSettingsModel";
import {
  OpenAiCompatibilityDetails,
  ProviderTestResult,
} from "./ProviderTestResult";

export function ProviderSettingsView({
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
  hasUnsavedChanges,
  statusMessage,
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
  modelDiscovery,
  onDiscoverProvider,
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
  providerTest: {
    providerId: string;
    status: "testing" | "complete";
    result?: ProviderHealthCheckResult;
  } | null;
  secretSources: SecretSources | null;
  pendingApiKey: string;
  showApiKey: boolean;
  saving: boolean;
  hasUnsavedChanges: boolean;
  statusMessage: string | null;
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
  onTestProvider(providerId: string): void;
  modelDiscovery: ModelDiscoveryState;
  onDiscoverProvider(providerId: string): Promise<void>;
  onRefreshCodexAccount(): void;
  onStartCodexLogin(): Promise<CodexLoginStart | null>;
  onCancelCodexLogin(): Promise<void>;
  onLogoutCodexAccount(): Promise<void>;
}) {
  const usesHttpTransport = editingProvider
    ? providerEffectiveTransport(editingProvider) === "http"
    : false;
  const usesCodexAppServer = editingProvider
    ? providerEffectiveTransport(editingProvider) === "codex_app_server"
    : false;
  const usesResponsesAdapter = editingProvider
    ? providerAllowedAdapters(editingProvider).includes("open_ai_responses")
    : false;
  // Existing connection-wide values remain readable for migrated settings, but
  // new setup is intentionally configured at the individual-model level.
  const showLegacyConnectionDefaults = false;
  const [manualModelProviderId, setManualModelProviderId] = useState<
    string | null
  >(null);
  const modelSyncing = modelDiscovery.status === "discovering";
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
  const detectedContextWindowText =
    reportedContextWindow == null
      ? null
      : `当前模型上下文窗口自动识别为 ${reportedContextWindow.toLocaleString()} tokens。`;
  const providerProtocolHint = editingProvider
    ? providerProtocolDescription(editingProvider)
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
    await onDiscoverProvider(editingProvider.id);
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
      resolvedKind === "codex_app_server" ? "" : editingProvider.model.trim();
    const axes = providerAxesFromKind(resolvedKind);
    onUpdateProvider(editingProvider.id, "kind", resolvedKind);
    onUpdateProvider(editingProvider.id, "transport", axes.transport);
    onUpdateProvider(editingProvider.id, "auth", axes.auth);
    onUpdateProvider(
      editingProvider.id,
      "allowedAdapters",
      axes.allowedAdapters,
    );
    onUpdateProvider(
      editingProvider.id,
      "preferredAdapter",
      axes.preferredAdapter,
    );
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

  function updateTransport(transport: ProviderTransportKind) {
    if (transport === "codex_app_server") {
      updateProviderKind("codex_app_server");
    } else if (transport === "mock") {
      updateProviderKind("mock");
    } else {
      updateProviderKind(
        editingProvider?.kind === "anthropic" ? "anthropic" : "openai",
      );
    }
  }

  function updateProtocol(selection: ProviderProtocolSelection) {
    if (!editingProvider) return;
    const config =
      selection === "anthropic_messages"
        ? {
            kind: "anthropic" as const,
            allowed: ["anthropic_messages"] as ProviderAdapterKind[],
            preferred: "anthropic_messages" as ProviderAdapterKind,
          }
        : selection === "open_ai_responses"
          ? {
              kind: "openai_responses" as const,
              allowed: ["open_ai_responses"] as ProviderAdapterKind[],
              preferred: "open_ai_responses" as ProviderAdapterKind,
            }
          : selection === "open_ai_chat"
            ? {
                kind: "openai_compatible" as const,
                allowed: ["open_ai_chat"] as ProviderAdapterKind[],
                preferred: "open_ai_chat" as ProviderAdapterKind,
              }
            : {
                kind: "openai_compatible" as const,
                allowed: [
                  "open_ai_responses",
                  "open_ai_chat",
                ] as ProviderAdapterKind[],
                preferred: null,
              };
    onUpdateProvider(editingProvider.id, "kind", config.kind);
    onUpdateProvider(editingProvider.id, "allowedAdapters", config.allowed);
    onUpdateProvider(editingProvider.id, "preferredAdapter", config.preferred);
  }

  const selectedBaseUrlPresetId = editingProvider
    ? (providerBaseUrlPresets.find(
        (preset) => preset.baseUrl === editingProvider.baseUrl.trim(),
      )?.id ?? "")
    : "";

  function selectBaseUrlPreset(presetId: string) {
    if (!editingProvider) return;
    const preset = providerBaseUrlPresets.find((item) => item.id === presetId);
    if (!preset) return;
    onUpdateProvider(editingProvider.id, "baseUrl", preset.baseUrl);
    if (preset.kind !== editingProvider.kind) {
      updateProviderKind(preset.kind);
    }
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
                  <small>{providerHealthStatusLabel(health?.status)}</small>
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
                  maxLength={MAX_PROVIDER_NAME_LENGTH}
                  placeholder="例如：Kimi K3"
                  onChange={(event) =>
                    onUpdateProvider(
                      editingProvider.id,
                      "name",
                      event.target.value,
                    )
                  }
                />
              </label>
              <label>
                <span>连接方式</span>
                <select
                  value={providerEffectiveTransport(editingProvider)}
                  onChange={(event) =>
                    updateTransport(event.target.value as ProviderTransportKind)
                  }
                >
                  <option value="http">HTTP API</option>
                  <option value="codex_app_server">
                    Codex App Server (local)
                  </option>
                  <option value="mock">Mock</option>
                </select>
              </label>
              {providerEffectiveTransport(editingProvider) === "http" ? (
                <>
                  <label>
                    <span>认证方式</span>
                    <select
                      value={providerEffectiveAuth(editingProvider)}
                      onChange={(event) =>
                        onUpdateProvider(
                          editingProvider.id,
                          "auth",
                          event.target.value as ProviderAuthKind,
                        )
                      }
                    >
                      <option value="bearer">Bearer Token</option>
                      <option value="x_api_key">x-api-key</option>
                      <option value="none">无需认证</option>
                    </select>
                  </label>
                  <label>
                    <span>协议适配器</span>
                    <select
                      value={providerProtocolSelection(editingProvider)}
                      onChange={(event) =>
                        updateProtocol(
                          event.target.value as ProviderProtocolSelection,
                        )
                      }
                    >
                      <option value="openai_auto">
                        OpenAI Chat + Responses（自动）
                      </option>
                      <option value="open_ai_chat">
                        OpenAI Chat Completions
                      </option>
                      <option value="open_ai_responses">
                        OpenAI Responses
                      </option>
                      <option value="anthropic_messages">
                        Anthropic Messages
                      </option>
                    </select>
                    <small>{providerProtocolHint}</small>
                  </label>
                </>
              ) : null}
              {usesCodexAppServer ? (
                <div className="settings-field-wide settings-provider-local-note">
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
              ) : usesHttpTransport ? (
                <>
                  <label className="settings-field-wide">
                    <span>Base URL</span>
                    <InputDropdown
                      value={editingProvider.baseUrl}
                      options={providerBaseUrlPresetOptions}
                      selectedOptionValue={selectedBaseUrlPresetId}
                      inputProps={{
                        type: "url",
                        spellCheck: false,
                        placeholder: "https://api.example.com/v1",
                      }}
                      label="选择常用 Base URL"
                      menuLabel="常用 Base URL"
                      onValueChange={(value) =>
                        onUpdateProvider(editingProvider.id, "baseUrl", value)
                      }
                      onOptionSelect={selectBaseUrlPreset}
                    />
                  </label>
                  {providerEffectiveAuth(editingProvider) !== "none" ? (
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
                          {showApiKey ? (
                            <EyeOff size={15} />
                          ) : (
                            <Eye size={15} />
                          )}
                        </button>
                      </div>
                      <small>
                        {editingProvider.apiKeyConfigured
                          ? "密钥已加密保存；界面不会回显原文。"
                          : "密钥不会写入普通设置文件。"}
                      </small>
                    </label>
                  ) : null}
                  <div className="settings-field-wide">
                    <label>
                      <span>默认模型</span>
                      <ModelInputDropdown
                        connection={editingProvider}
                        value={editingProvider.model}
                        onChange={updateModel}
                        onSync={() => void syncModels()}
                        syncing={modelSyncing}
                        disabled={modelSyncing}
                      />
                    </label>
                    <div className="settings-model-meta">
                      <div>
                        <span>
                          {editingProvider.syncedModels.length > 0
                            ? modelDescription
                            : providerEffectiveAuth(editingProvider) === "none"
                              ? "填写 Base URL 后会自动识别此 API 的所有模型，无需手动填写模型名称。"
                              : "填写 Base URL 和 API 密钥后会自动识别此 API 的所有模型，无需手动填写模型名称。"}
                        </span>
                        {detectedContextWindowText ? (
                          <small role="status">{detectedContextWindowText}</small>
                        ) : null}
                      </div>
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
                    <ModelDiscoveryStatus
                      state={modelDiscovery}
                      onRetry={() => void syncModels()}
                    />
                  </div>
                </>
              ) : (
                <div className="settings-field-wide settings-provider-local-note">
                  Mock 连接只用于本地模拟，不需要 Base URL、API 密钥或模型目录。
                </div>
              )}
            </div>

            {usesHttpTransport ? (
              <ModelFamilySection
                connection={editingProvider}
                onUpdateProvider={onUpdateProvider}
              />
            ) : null}

            {usesHttpTransport ? (
              <ModelConfigurationSection
                connection={editingProvider}
                onUpdateProvider={onUpdateProvider}
              />
            ) : null}

            {showLegacyConnectionDefaults && usesHttpTransport ? (
              <ProviderAdvancedDefaults
                connection={editingProvider}
                onUpdateProvider={onUpdateProvider}
              />
            ) : null}

            <div className="settings-provider-footer">
              <div className="settings-provider-health-status">
                {providerStatusChips(editingProvider, providerHealth)}
                {statusMessage ? (
                  <span className="settings-inline-status" role="status">
                    {statusMessage}
                  </span>
                ) : null}
              </div>
              <div className="settings-provider-actions">
                {usesHttpTransport &&
                providerEffectiveAuth(editingProvider) !== "none" &&
                editingProvider.apiKeyConfigured ? (
                  <button
                    type="button"
                    className="secondary-button danger-text"
                    disabled={saving || modelSyncing}
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
                    modelSyncing ||
                    providerTest?.status === "testing" ||
                    hasUnsavedChanges
                  }
                  title={
                    hasUnsavedChanges
                      ? "请先保存 API 配置，再测试连接"
                      : undefined
                  }
                  onClick={() => onTestProvider(editingProvider.id)}
                >
                  {providerTest?.providerId === editingProvider.id &&
                  providerTest.status === "testing"
                    ? "测试中…"
                    : "测试连接"}
                </button>
                <button
                  type="submit"
                  className="primary-button"
                  disabled={saving || modelSyncing || !hasUnsavedChanges}
                >
                  {saving ? "保存中…" : "保存 API 配置"}
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

function providerStatusChips(
  provider: ProviderSettings,
  health: ProviderHealth[],
): ReactNode {
  const providerHealth = health.find((item) => item.id === provider.id);
  return (
    <>
      <span>{providerHealthStatusLabel(providerHealth?.status)}</span>
      <span>
        {providerEffectiveTransport(provider) === "codex_app_server"
          ? "本地 Codex"
          : providerEffectiveAuth(provider) === "none"
            ? "无需认证"
            : provider.apiKeyConfigured
              ? "密钥已配置"
              : "未配置密钥"}
      </span>
      <span>{providerHealth?.usingMock ? "Mock" : "模型"}</span>
    </>
  );
}

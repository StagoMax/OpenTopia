import { useMemo, useState } from "react";
import {
  availableFamiliesForModels,
  classifyModelFamily,
  formatModelDisplayName,
} from "../../modelCatalog";
import {
  resolveModelContextWindow,
  resolveModelVisionSupport,
} from "../../modelCapabilities";
import {
  REASONING_EFFORT_DETAILS,
  resolveModelReasoningCapability,
} from "../../providerSettings";
import type { ProviderModelSettings, ProviderSettings } from "../../types";
import { Button, InputDropdown, Select, Switch } from "../ui";
import { SettingsRow } from "../SettingsLayout";
import {
  contextWindowPresetOptions,
  contextWindowPresets,
  type ModelDiscoveryState,
} from "./providerSettingsModel";

export function ModelDiscoveryStatus({
  state,
  onRetry,
}: {
  state: ModelDiscoveryState;
  onRetry(): void;
}) {
  if (state.status === "idle") return null;

  const message =
    state.status === "discovering"
      ? "正在识别模型与能力…"
      : state.status === "success"
        ? `已识别 ${state.modelCount} 个模型，模型能力和参数已更新。`
        : `识别失败：${state.message}`;

  return (
    <div
      className={`settings-model-discovery settings-model-discovery--${state.status}`}
      role={state.status === "error" ? "alert" : "status"}
      aria-live="polite"
    >
      <span>{message}</span>
      {state.status === "error" ? (
        <Button size="compact" variant="quiet" onClick={onRetry}>
          重试
        </Button>
      ) : null}
    </div>
  );
}

export function ModelConfigurationSection({
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
  const modelIds = useMemo(
    () => Array.from(new Set(connection.syncedModels)),
    [connection.syncedModels],
  );
  const [configuredModelId, setConfiguredModelId] = useState("");
  const modelId =
    modelIds.find((id) => id === configuredModelId) ??
    modelIds.find((id) => id === connection.model.trim()) ??
    modelIds[0] ??
    "";
  const modelSettings = connection.modelSettings?.[modelId];
  const reasoningCapability = resolveModelReasoningCapability(
    connection.kind,
    modelId,
  );
  const contextWindowResolution = resolveModelContextWindow(
    connection,
    modelId,
  );
  const visionResolution = resolveModelVisionSupport(connection, modelId);
  const supportsVision = visionResolution.supportsVision;
  const visionPreference =
    modelSettings?.supportsVision === undefined
      ? "automatic"
      : modelSettings.supportsVision
        ? "supported"
        : "unsupported";
  const automaticVisionDescription =
    visionResolution.automaticSource === "detected"
      ? `API 识别结果：${visionResolution.automaticSupportsVision ? "支持" : "不支持"}`
      : visionResolution.automaticSource === "official"
        ? `官方能力表：${visionResolution.automaticSupportsVision ? "支持" : "不支持"}`
        : "API 与官方能力表均无记录；自动模式按不支持处理";
  const selectedContextWindowPreset =
    modelSettings?.contextWindowTokens == null
      ? "auto"
      : (contextWindowPresets
          .find((preset) => preset.tokens === modelSettings.contextWindowTokens)
          ?.tokens.toString() ?? null);

  function updateModelSettings(patch: Partial<ProviderModelSettings>) {
    if (!modelId) return;
    onUpdateProvider(connection.id, "modelSettings", {
      ...(connection.modelSettings ?? {}),
      [modelId]: {
        ...(connection.modelSettings?.[modelId] ?? {}),
        ...patch,
      },
    });
  }

  function resetModelSetting<K extends keyof ProviderModelSettings>(field: K) {
    if (!modelId) return;
    const nextSettings = { ...(connection.modelSettings ?? {}) };
    const nextModelSettings = { ...(nextSettings[modelId] ?? {}) };
    delete nextModelSettings[field];
    if (Object.keys(nextModelSettings).length === 0) {
      delete nextSettings[modelId];
    } else {
      nextSettings[modelId] = nextModelSettings;
    }
    onUpdateProvider(connection.id, "modelSettings", nextSettings);
  }

  function selectContextWindowPreset(value: string) {
    if (value === "auto") {
      resetModelSetting("contextWindowTokens");
      return;
    }
    if (!value) return;
    updateModelSettings({ contextWindowTokens: Number(value) });
  }

  if (modelIds.length === 0) return null;

  return (
    <section className="settings-model-families">
      <header>
        <div>
          <strong>模型配置</strong>
          <span>
            参数只应用到选中的模型；上下文窗口与多模态能力会自动解析并显示。
          </span>
        </div>
      </header>
      <div className="settings-form-grid settings-model-parameter-grid">
        <label className="settings-field-wide">
          <span>配置模型</span>
          <select
            value={modelId}
            onChange={(event) => setConfiguredModelId(event.target.value)}
          >
            {modelIds.map((id) => (
              <option key={id} value={id}>
                {formatModelDisplayName(id)} ({id})
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Temperature</span>
          <input
            type="number"
            min="0"
            max="2"
            step="0.1"
            value={modelSettings?.temperature ?? ""}
            placeholder={
              connection.temperature === null ||
              connection.temperature === undefined
                ? "供应商默认"
                : `继承连接默认值 ${connection.temperature}`
            }
            onChange={(event) =>
              event.target.value
                ? updateModelSettings({
                    temperature: Number(event.target.value),
                  })
                : resetModelSetting("temperature")
            }
          />
          <small>
            {modelSettings?.temperature !== undefined
              ? "已为此模型单独配置。"
              : "未从目录获取温度范围时，继承连接默认值。"}
          </small>
        </label>
        <label>
          <span>最大输出 Token</span>
          <input
            type="number"
            min="1"
            value={modelSettings?.maxOutputTokens ?? ""}
            placeholder={
              connection.maxOutputTokens === null ||
              connection.maxOutputTokens === undefined
                ? "跟随模型默认"
                : `继承连接默认值 ${connection.maxOutputTokens}`
            }
            onChange={(event) =>
              event.target.value
                ? updateModelSettings({
                    maxOutputTokens: Number(event.target.value),
                  })
                : resetModelSetting("maxOutputTokens")
            }
          />
          <small>
            {modelSettings?.maxOutputTokens !== undefined
              ? "已为此模型单独配置。"
              : "目录未声明输出上限时，使用模型或连接默认值。"}
          </small>
        </label>
        <div className="settings-field-wide">
          <label>
            <span>上下文窗口覆盖值</span>
            <InputDropdown
              value={modelSettings?.contextWindowTokens ?? ""}
              options={contextWindowPresetOptions}
              selectedOptionValue={selectedContextWindowPreset}
              inputProps={{
                type: "number",
                min: 4096,
                step: 1024,
                placeholder: "自动识别",
              }}
              label="选择常用上下文窗口"
              menuLabel="常用上下文窗口"
              onValueChange={(value) =>
                value
                  ? updateModelSettings({ contextWindowTokens: Number(value) })
                  : resetModelSetting("contextWindowTokens")
              }
              onOptionSelect={selectContextWindowPreset}
            />
            <small role="status">
              {contextWindowResolution.source === "model_override"
                ? `正在使用此模型的手动覆盖：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens。`
                : contextWindowResolution.source === "connection_override"
                  ? `正在继承连接默认值：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens。`
                  : contextWindowResolution.source === "detected"
                    ? `自动识别结果：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens（API /models 报告）。`
                    : contextWindowResolution.source === "official"
                      ? `自动识别结果：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens（内置模型表）。`
                      : contextWindowResolution.source === "inferred"
                        ? `自动识别结果：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens（按同前缀上一代 ${contextWindowResolution.inferredFromModelId} 推断）。`
                        : `自动识别结果：${contextWindowResolution.contextWindowTokens.toLocaleString()} tokens（未知模型保守兜底）。`}
            </small>
          </label>
        </div>
        {reasoningCapability.status === "unsupported" ? (
          <div
            className="settings-field-wide settings-reasoning-unavailable"
            role="status"
          >
            <span>思考模式 / 推理强度</span>
            <strong>当前模型不提供推理强度参数。</strong>
          </div>
        ) : (
          <label className="settings-field-wide settings-reasoning-field">
            <span>推理强度</span>
            <select
              value={modelSettings?.reasoningEffort ?? ""}
              onChange={(event) =>
                event.target.value
                  ? updateModelSettings({
                      reasoningEffort: event.target
                        .value as ProviderModelSettings["reasoningEffort"],
                    })
                  : resetModelSetting("reasoningEffort")
              }
            >
              <option value="">自动识别 / 继承连接默认值</option>
              {reasoningCapability.supportedEfforts.map((effort) => (
                <option key={effort} value={effort}>
                  {REASONING_EFFORT_DETAILS[effort].label}
                </option>
              ))}
            </select>
            <small>
              {reasoningCapability.official
                ? reasoningCapability.thinkingToggle
                  ? `已识别 thinking 开关及 ${reasoningCapability.supportedEfforts.length - 1} 个推理档位；None 会关闭 thinking。`
                  : `已识别 ${reasoningCapability.supportedEfforts.length} 个可用推理档位。`
                : "模型能力未知，保留兼容 Provider 支持的推理档位。"}
            </small>
          </label>
        )}
      </div>
      <div className="settings-toggle-stack">
        <SettingsRow
          title="图片输入能力"
          description={
            visionResolution.source === "manual"
              ? `${modelId} · 已手动设为${supportsVision ? "支持" : "不支持"}图片输入；${automaticVisionDescription}`
              : visionResolution.source === "detected"
                ? `${modelId} · API 已识别为${supportsVision ? "支持" : "不支持"}图片输入`
                : visionResolution.source === "official"
                  ? `${modelId} · 官方能力表标记为${supportsVision ? "支持" : "不支持"}图片输入`
                  : `${modelId} · API 与官方能力表均无记录；自动模式按不支持处理`
          }
          control={
            <Select
              label={`${modelId} 图片输入能力`}
              value={visionPreference}
              options={[
                { value: "automatic", label: "自动" },
                { value: "supported", label: "手动：支持" },
                { value: "unsupported", label: "手动：不支持" },
              ]}
              onChange={(preference) => {
                if (preference === "automatic") {
                  resetModelSetting("supportsVision");
                  return;
                }
                updateModelSettings({
                  supportsVision: preference === "supported",
                });
              }}
            />
          }
        />
      </div>
    </section>
  );
}

/**
 * Per-connection model scope. Users pick whole families rather than individual
 * model ids, because one API key on an aggregator or relay already grants the
 * whole vendor lineup — enumerating every id by hand is busywork that goes
 * stale on every vendor release.
 */
export function ModelFamilySection({
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

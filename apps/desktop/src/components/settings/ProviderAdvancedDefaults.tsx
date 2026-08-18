import {
  REASONING_EFFORT_DETAILS,
  normalizeReasoningEffortForModel,
  resolveModelReasoningCapability,
} from "../../providerSettings";
import type { ProviderSettings } from "../../types";
import { Button, Switch } from "../ui";
import { SettingsRow } from "../SettingsLayout";
import { providerAllowedAdapters } from "./providerSettingsModel";

export function ProviderAdvancedDefaults({
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
  const usesResponsesAdapter =
    providerAllowedAdapters(connection).includes("open_ai_responses");
  const reasoningCapability = resolveModelReasoningCapability(
    connection.kind,
    connection.model,
  );
  const selectedReasoningEffort = normalizeReasoningEffortForModel(
    connection.kind,
    connection.model,
    connection.reasoningEffort,
  );
  const reportedContextWindow =
    connection.modelContextWindows?.[connection.model.trim()];

  return (
    <details className="settings-advanced-fields">
      <summary>连接默认参数</summary>
      <div className="settings-form-grid">
        <label>
          <span>默认 Temperature</span>
          <input
            type="number"
            min="0"
            max="2"
            step="0.1"
            value={connection.temperature ?? ""}
            placeholder="默认（跟随模型）"
            title="留空则不发送 temperature 参数，使用模型供应商的默认值"
            onChange={(event) =>
              onUpdateProvider(
                connection.id,
                "temperature",
                event.target.value ? Number(event.target.value) : null,
              )
            }
          />
          <small>
            留空则不发送此参数，使用模型默认值。推理模型（o
            系列、GPT-5）不支持自定义温度。
          </small>
        </label>
        <label>
          <span>默认最大输出 Token</span>
          <input
            type="number"
            min="1"
            value={connection.maxOutputTokens ?? ""}
            placeholder="跟随供应商"
            onChange={(event) =>
              onUpdateProvider(
                connection.id,
                "maxOutputTokens",
                event.target.value ? Number(event.target.value) : null,
              )
            }
          />
        </label>
        <div className="settings-field-wide">
          <label>
            <span>默认上下文窗口覆盖值</span>
            <input
              type="number"
              min="4096"
              step="1024"
              value={connection.contextWindowTokens ?? ""}
              placeholder="自动识别"
              title="仅在特殊网关或套餐需要时手动填写；留空则自动使用 API 报告、内置表或 128K 兜底"
              onChange={(event) =>
                onUpdateProvider(
                  connection.id,
                  "contextWindowTokens",
                  event.target.value ? Number(event.target.value) : null,
                )
              }
            />
            <small role="status">
              {connection.contextWindowTokens !== null
                ? `正在使用手动覆盖：${connection.contextWindowTokens.toLocaleString()} tokens。它会压过 API 探测与内置模型表。`
                : reportedContextWindow
                  ? `API /models 已报告此模型为 ${reportedContextWindow.toLocaleString()} tokens，将作为上下文上限。`
                  : "未从 API 获得此模型的上限；会依次使用内置模型表、未知模型 128K 兜底。"}
            </small>
          </label>
          {connection.contextWindowTokens !== null ? (
            <Button
              size="compact"
              variant="quiet"
              onClick={() =>
                onUpdateProvider(connection.id, "contextWindowTokens", null)
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
            <span>思考模式 / 推理强度</span>
            <strong>
              {reasoningCapability.official
                ? "当前模型不提供推理强度"
                : "当前供应商类型不使用此参数"}
            </strong>
          </div>
        ) : (
          <label className="settings-field-wide settings-reasoning-field">
            <span>默认思考模式 / 推理强度</span>
            <select
              value={selectedReasoningEffort ?? ""}
              onChange={(event) =>
                onUpdateProvider(
                  connection.id,
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
                ? reasoningCapability.thinkingToggle
                  ? `已按官方能力显示 ${reasoningCapability.supportedEfforts.length} 个可用档位；None 会关闭 thinking。`
                  : `已按官方能力显示 ${reasoningCapability.supportedEfforts.length} 个可用档位。`
                : "模型能力未知，保留兼容供应商支持的全部档位。"}
            </small>
          </label>
        )}
        <label className="settings-field-wide">
          <span>Prompt cache key</span>
          <input
            value={connection.promptCacheKey ?? ""}
            placeholder="按工作区自动生成"
            onChange={(event) =>
              onUpdateProvider(
                connection.id,
                "promptCacheKey",
                event.target.value || null,
              )
            }
          />
        </label>
        {usesResponsesAdapter ? (
          <>
            <label>
              <span>缓存策略</span>
              <select
                value={connection.promptCachePolicy ?? ""}
                onChange={(event) =>
                  onUpdateProvider(
                    connection.id,
                    "promptCachePolicy",
                    (event.target.value ||
                      null) as ProviderSettings["promptCachePolicy"],
                  )
                }
              >
                <option value="">自动</option>
                <option value="explicit_30m">显式断点（30 分钟）</option>
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
                value={connection.responsesCompactionThresholdTokens ?? ""}
                placeholder="关闭"
                onChange={(event) =>
                  onUpdateProvider(
                    connection.id,
                    "responsesCompactionThresholdTokens",
                    event.target.value ? Number(event.target.value) : null,
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
              checked={connection.parallelToolCalls}
              onChange={(checked) =>
                onUpdateProvider(connection.id, "parallelToolCalls", checked)
              }
            />
          }
        />
        {usesResponsesAdapter ? (
          <SettingsRow
            title="延续 Responses 状态"
            description="在多轮请求间保留供应商响应状态。"
            control={
              <Switch
                label="延续 Responses 状态"
                checked={connection.storeResponses}
                onChange={(checked) =>
                  onUpdateProvider(connection.id, "storeResponses", checked)
                }
              />
            }
          />
        ) : null}
      </div>
    </details>
  );
}

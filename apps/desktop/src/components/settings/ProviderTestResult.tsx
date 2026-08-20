import type { ProviderHealthCheckResult, ProviderSettings } from "../../types";
import { providerFeatureSupportLabel } from "./providerSettingsModel";

export function ProviderTestResult({
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
          ? `兼容性检测通过${result?.latencyMs ? ` · 总用时 ${formatCompatibilityCheckDuration(result.latencyMs)}（含多项请求）` : ""}`
          : (result?.error ?? "连接失败，请检查地址、模型和密钥。")}
      </div>
      {result?.openaiCompatibility ? (
        <OpenAiCompatibilityDetails report={result.openaiCompatibility} />
      ) : null}
    </div>
  );
}

function formatCompatibilityCheckDuration(durationMs: number): string {
  return durationMs >= 1_000
    ? `${(durationMs / 1_000).toFixed(1)} 秒`
    : `${durationMs} 毫秒`;
}

export function OpenAiCompatibilityDetails({
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
            ? "已选择兼容编码：Adapter 会按已保存的能力档案合并 developer 指令并编码工具历史。"
            : "已选择原生 Chat 编码；若供应商能力变化，请重新测试连接以更新 Adapter 档案。"}
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

import { FileJson, Server, X } from "lucide-react";
import {
  PROVIDER_IMPORT_PRESETS,
  createProviderDraftFromPreset,
  type ProviderImportDraft,
} from "../../providerImport";
import { formatImportFormat } from "./providerSettingsModel";

export function ProviderImportDialog({
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

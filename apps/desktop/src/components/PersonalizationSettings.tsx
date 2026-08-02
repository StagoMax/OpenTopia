import { Info } from "lucide-react";
import {
  CUSTOM_INSTRUCTIONS_MAX_LENGTH,
  type PersonalizationSettings,
} from "../personalization";
import type { AgentRuntimeSettings } from "../types";
import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsLayout";
import { Select } from "./ui";

type PersonalityOption = AgentRuntimeSettings["personality"];

const personalityOptions: Array<{ value: PersonalityOption; label: string }> = [
  { value: "focused", label: "专注" },
  { value: "professional", label: "专业" },
  { value: "warm", label: "自然" },
];

type PersonalizationSettingsViewProps = {
  agentRuntime: AgentRuntimeSettings;
  personalization: PersonalizationSettings;
  onAgentRuntimeChange(value: AgentRuntimeSettings): void;
  onPersonalizationChange(value: PersonalizationSettings): void;
};

export function PersonalizationSettingsView({
  agentRuntime,
  personalization,
  onAgentRuntimeChange,
  onPersonalizationChange,
}: PersonalizationSettingsViewProps) {
  return (
    <SettingsPage title="个性化" description="语气与自定义指令">
      <div className="settings-notice" role="note">
        <Info size={16} aria-hidden="true" focusable="false" />
        <span>
          并非所有模型都支持个性设置。可在自定义指令中调整 OpenTopia 的语气。
        </span>
      </div>

      <SettingsGroup title="语气">
        <SettingsRow
          title="个性"
          description="选择智能体回复的默认语气"
          control={
            <Select
              label="个性"
              value={agentRuntime.personality}
              options={personalityOptions}
              onChange={(personality) =>
                onAgentRuntimeChange({ ...agentRuntime, personality })
              }
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        title="自定义指令"
        description="为此主机上的所有任务提供额外说明和上下文。"
        actions={
          <>
            <span className="settings-inline-count">
              {personalization.customInstructions.length} /{" "}
              {CUSTOM_INSTRUCTIONS_MAX_LENGTH}
            </span>
          </>
        }
      >
        <div className="settings-textarea-row">
          <label className="ot-sr-only" htmlFor="custom-instructions">
            自定义指令
          </label>
          <textarea
            id="custom-instructions"
            className="settings-textarea"
            placeholder="添加自定义指令…"
            maxLength={CUSTOM_INSTRUCTIONS_MAX_LENGTH}
            value={personalization.customInstructions}
            onChange={(event) =>
              onPersonalizationChange({
                customInstructions: event.target.value,
              })
            }
          />
        </div>
      </SettingsGroup>
    </SettingsPage>
  );
}

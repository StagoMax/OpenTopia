import { Check, Paperclip, Plug, Plus } from "lucide-react";
import { ModelSelector } from "../../components/ModelSelector";
import { Tooltip } from "../../components/ui";
import type {
  AppSettings,
  CollaborationMode,
  ProviderSettings,
  SkillDescriptor,
  ThreadModelSelection,
} from "../../types";
import {
  collaborationModeOptions,
  normalizedPermissionMode,
  permissionModeOptions,
} from "./composerModes";
import type {
  ComposerOpenMenu,
  ExecutionPermissionMode,
} from "./composerTypes";

export function ComposerToolbar({
  openMenu,
  queuedMessageCount,
  providers,
  activeProviderId,
  modelSelection,
  permissionMode,
  collaborationMode,
  skills,
  selectedSkillIds,
  isRunning,
  isSending,
  onToggleMenu,
  onCloseMenu,
  onPickFiles,
  onChangePermissionMode,
  onChangeCollaborationMode,
  onChangeModelSelection,
  onOpenSettings,
  onToggleSkill,
}: {
  openMenu: ComposerOpenMenu;
  queuedMessageCount: number;
  providers: ProviderSettings[];
  activeProviderId: string;
  modelSelection: ThreadModelSelection | null;
  permissionMode: AppSettings["permissionMode"];
  collaborationMode: CollaborationMode;
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  isRunning: boolean;
  isSending: boolean;
  onToggleMenu(menu: Exclude<ComposerOpenMenu, null>): void;
  onCloseMenu(): void;
  onPickFiles(): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeCollaborationMode(mode: CollaborationMode): void;
  onChangeModelSelection(selection: ThreadModelSelection): void;
  onOpenSettings(): void;
  onToggleSkill(skillId: string): void;
}) {
  const activePermissionMode = normalizedPermissionMode(permissionMode);
  const activePermissionOption =
    permissionModeOptions.find(
      (option) => option.value === activePermissionMode,
    ) ?? permissionModeOptions[1];
  const ActivePermissionIcon = activePermissionOption.icon;

  return (
    <>
      <div className="composer-toolbar">
        <div className="composer-menu-wrap">
          <button
            className="composer-icon-button"
            type="button"
            title="添加内容或选择模式"
            aria-label="添加内容或选择模式"
            aria-expanded={openMenu === "actions"}
            onClick={() => onToggleMenu("actions")}
          >
            <Plus size={16} />
          </button>
        </div>
        <div className="composer-menu-wrap">
          <button
            className={`composer-mode is-${activePermissionOption.appearance}`}
            type="button"
            aria-expanded={openMenu === "permission"}
            onClick={() => onToggleMenu("permission")}
          >
            <ActivePermissionIcon size={14} aria-hidden="true" />
            <span>{activePermissionOption.label}</span>
          </button>
          {openMenu === "permission" ? (
            <div className="tool-popover permission-popover" role="menu">
              <div className="permission-popover-header">
                <span>应如何批准 OpenTopia 操作？</span>
                <span title="权限预设会同时调整审批策略和本地沙箱">
                  了解更多
                </span>
              </div>
              {permissionModeOptions.map((option) => {
                const Icon = option.icon;
                const selected = activePermissionMode === option.value;
                return (
                  <button
                    className={`permission-option is-${option.appearance} ${selected ? "active" : ""}`}
                    disabled={isRunning || isSending}
                    key={option.value}
                    role="menuitemradio"
                    aria-checked={selected}
                    onClick={() => {
                      onChangePermissionMode(option.value);
                      onCloseMenu();
                    }}
                  >
                    <Icon size={17} aria-hidden="true" />
                    <span className="permission-option-copy">
                      <strong>{option.label}</strong>
                      <small>{option.detail}</small>
                    </span>
                    {selected ? <Check size={15} aria-hidden="true" /> : null}
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
        {queuedMessageCount > 0 ? (
          <span className="composer-queue-status">
            {queuedMessageCount} queued
          </span>
        ) : null}
        <ModelSelector
          activeConnectionId={activeProviderId}
          connections={providers}
          disabled={isRunning || isSending}
          onChange={onChangeModelSelection}
          onOpenSettings={onOpenSettings}
          selection={modelSelection}
        />
      </div>
      {openMenu === "actions" ? (
        <div className="tool-popover composer-actions-popover" role="menu">
          <div className="composer-actions-section-label">添加</div>
          <button
            role="menuitem"
            onClick={() => {
              onPickFiles();
              onCloseMenu();
            }}
          >
            <Paperclip size={14} />
            <span>文件和文件夹</span>
          </button>
          <div className="tool-popover-separator" />
          <div className="composer-actions-section-label">模式</div>
          {collaborationModeOptions.map((option) => {
            const Icon = option.icon;
            const selected = collaborationMode === option.value;
            return (
              <button
                className={`composer-mode-option is-${option.value} ${selected ? "active" : ""}`}
                disabled={isRunning || isSending}
                key={option.value}
                role="menuitemcheckbox"
                aria-checked={selected}
                onClick={() => {
                  onChangeCollaborationMode(
                    selected ? "default" : option.value,
                  );
                  onCloseMenu();
                }}
              >
                <Icon size={15} aria-hidden="true" />
                <span className="composer-action-copy">
                  <strong>{option.label}</strong>
                  <small>{option.detail}</small>
                </span>
                {selected ? <Check size={14} aria-hidden="true" /> : null}
              </button>
            );
          })}
          {skills.length > 0 ? (
            <>
              <div className="tool-popover-separator" />
              <div className="composer-actions-section-label">插件</div>
              {skills.map((skill) => {
                const selected = selectedSkillIds.includes(skill.id);
                return (
                  <Tooltip
                    anchor="pointer"
                    content={skill.description || skill.path}
                    key={skill.id}
                    placement="top"
                  >
                    {(tooltipProps) => (
                      <button
                        {...tooltipProps}
                        className={`composer-tool-option ${selected ? "active" : ""}`}
                        role="menuitemcheckbox"
                        aria-checked={selected}
                        disabled={!selected && selectedSkillIds.length >= 5}
                        onClick={() => onToggleSkill(skill.id)}
                      >
                        <Plug size={14} aria-hidden="true" />
                        <span className="composer-action-copy">
                          <strong>{skill.name}</strong>
                          {skill.description ? (
                            <small>{skill.description}</small>
                          ) : null}
                        </span>
                        {selected ? (
                          <Check size={14} aria-hidden="true" />
                        ) : null}
                      </button>
                    )}
                  </Tooltip>
                );
              })}
            </>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

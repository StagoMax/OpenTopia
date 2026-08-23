import {
  Activity,
  Bot,
  Check,
  FileCode2,
  FileText,
  Folder,
  Presentation,
  RotateCcw,
  Search,
  ShieldCheck,
  Table2,
  TerminalSquare,
  Workflow,
} from "lucide-react";
import { useEffect, useState } from "react";
import {
  Composer,
  type ExecutionPermissionMode,
  type NewTaskLaunchMode,
} from "../composer/Composer";
import type {
  AppSettings,
  BackendStartupStatus,
  CollaborationMode,
  ContextSourceFile,
  ExperienceMode,
  InlineImageAttachment,
  InlineMessageContentPart,
  Project,
  ProviderSettings,
  SkillDescriptor,
  ThreadModelSelection,
} from "../../types";
import { workspaceName } from "../../workspaceName";
import type { SendShortcut } from "../../editorPreferences";
import {
  backendStartupLabel,
  formatBackendStartupElapsed,
} from "./backendStartupProgress";

export function NewTaskState({
  value,
  workspaceRoot,
  projectName,
  projects,
  modelSelection,
  providers,
  activeProviderId,
  permissionMode,
  collaborationMode,
  sandboxMode,
  contextSources,
  skills,
  selectedSkillIds,
  isSending,
  sendShortcut,
  launchMode,
  experienceMode,
  onChange,
  onChangeLaunchMode,
  onPickWorkspace,
  onSelectProject,
  onChangePermissionMode,
  onChangeCollaborationMode,
  onChangeSandboxMode,
  onChangeModelSelection,
  onOpenSettings,
  onAddContextSources,
  onRemoveContextSource,
  onToggleSkill,
  onSubmit,
}: {
  value: string;
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  modelSelection: ThreadModelSelection | null;
  providers: ProviderSettings[];
  activeProviderId: string;
  permissionMode: AppSettings["permissionMode"];
  collaborationMode: CollaborationMode;
  sandboxMode: AppSettings["sandbox"]["sandboxMode"];
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  isSending: boolean;
  sendShortcut: SendShortcut;
  launchMode: NewTaskLaunchMode;
  experienceMode: ExperienceMode;
  onChange(value: string): void;
  onChangeLaunchMode(mode: NewTaskLaunchMode): void;
  onPickWorkspace(): void;
  onSelectProject(projectId: string): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeCollaborationMode(mode: CollaborationMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onChangeModelSelection(selection: ThreadModelSelection): void;
  onOpenSettings(): void;
  onAddContextSources(files?: File[]): Promise<ContextSourceFile[]>;
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
  onSubmit(
    value: string,
    imageAttachments: InlineImageAttachment[],
    contentParts: InlineMessageContentPart[],
  ): Promise<boolean>;
}) {
  const suggestions =
    experienceMode === "flow"
      ? [
          {
            icon: Workflow,
            label: "描述企业流程",
            prompt: "根据我描述的角色、步骤、条件和审批点整理 Flow 设计",
          },
          {
            icon: Activity,
            label: "总结已完成流程",
            prompt: "分析一次已经正确完成的任务，并提炼可复用的 FlowDraft",
          },
          {
            icon: Bot,
            label: "规划多 Agent 协作",
            prompt: "设计参与 Agent 的职责、依赖、输入输出与验证闭环",
          },
          {
            icon: ShieldCheck,
            label: "检查流程边界",
            prompt: "检查流程中的权限、数据流、审批、预算和终止条件",
          },
        ]
      : experienceMode === "work"
        ? [
            {
              icon: Search,
              label: "研究并汇总资料",
              prompt: "研究这个主题，核对来源并整理成清晰的结论",
            },
            {
              icon: FileText,
              label: "撰写与整理文档",
              prompt: "根据项目资料撰写并整理一份完整文档",
            },
            {
              icon: Table2,
              label: "分析表格与数据",
              prompt: "分析项目中的表格和数据，并总结关键发现",
            },
            {
              icon: Presentation,
              label: "制作演示或报告",
              prompt: "根据项目内容制作一份结构清晰的演示或报告",
            },
          ]
        : [
            {
              icon: Search,
              label: "探索并理解代码",
              prompt: "分析这个项目的架构和核心模块",
            },
            {
              icon: FileCode2,
              label: "构建新功能",
              prompt: "为这个项目实现一个新功能",
            },
            {
              icon: Check,
              label: "审查代码更改",
              prompt: "审查当前工作区中的代码更改",
            },
            {
              icon: Activity,
              label: "修复问题",
              prompt: "检查并修复当前项目中的问题",
            },
          ];

  return (
    <>
      <div className="new-task-state">
        <Bot size={34} />
        <h2>
          {experienceMode === "flow"
            ? "要在"
            : experienceMode === "work"
              ? "今天想在"
              : "我们应该在"}{" "}
          <u>
            {projectName ??
              (workspaceRoot ? workspaceName(workspaceRoot) : "项目")}
          </u>{" "}
          {experienceMode === "flow"
            ? "中设计什么流程？"
            : experienceMode === "work"
              ? "中完成什么？"
              : "中构建什么？"}
        </h2>
        <div className="task-suggestions">
          {suggestions.map((suggestion) => {
            const Icon = suggestion.icon;
            return (
              <button
                key={suggestion.label}
                type="button"
                onClick={() => onChange(suggestion.prompt)}
              >
                <Icon size={15} />
                <span>{suggestion.label}</span>
              </button>
            );
          })}
        </div>
        {!workspaceRoot && (
          <button className="workspace-picker-button" onClick={onPickWorkspace}>
            <Folder size={15} />
            选择项目文件夹
          </button>
        )}
      </div>
      <Composer
        value={value}
        sendShortcut={sendShortcut}
        isSending={isSending}
        isRunning={false}
        isCancelling={false}
        modelSelection={modelSelection}
        providers={providers}
        activeProviderId={activeProviderId}
        permissionMode={permissionMode}
        collaborationMode={collaborationMode}
        sandboxMode={sandboxMode}
        contextSources={contextSources}
        skills={skills}
        selectedSkillIds={selectedSkillIds}
        launchMode={launchMode}
        workspaceRoot={workspaceRoot}
        projectName={
          projectName ?? (workspaceRoot ? workspaceName(workspaceRoot) : null)
        }
        projects={projects}
        onChange={onChange}
        onSubmit={onSubmit}
        onCancel={() => undefined}
        onPickWorkspace={onPickWorkspace}
        onSelectProject={onSelectProject}
        onChangeLaunchMode={onChangeLaunchMode}
        onChangePermissionMode={onChangePermissionMode}
        onChangeCollaborationMode={onChangeCollaborationMode}
        onChangeSandboxMode={onChangeSandboxMode}
        onChangeModelSelection={onChangeModelSelection}
        onOpenSettings={onOpenSettings}
        onAddContextSources={onAddContextSources}
        onRemoveContextSource={onRemoveContextSource}
        onToggleSkill={onToggleSkill}
      />
    </>
  );
}

export function OfflineState({
  backendUrl,
  error,
  attempt,
  isProbing,
  startupStatus,
  onRetry,
}: {
  backendUrl?: string;
  error: string | null;
  attempt: number;
  isProbing: boolean;
  startupStatus: BackendStartupStatus | null;
  onRetry: () => void;
}) {
  const [openedAt] = useState(() => new Date().toISOString());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const startupLabel = backendStartupLabel(startupStatus, isProbing);
  const elapsedLabel = formatBackendStartupElapsed(
    startupStatus?.startedAt ?? openedAt,
    now,
  );

  return (
    <div className="empty-state offline">
      <TerminalSquare size={48} />
      <h2>正在等待本地服务</h2>
      <p>
        {import.meta.env.DEV ? (
          <>
            开发模式下桌面应用会编译并启动本地服务。首次编译或改动 Rust
            代码后可能需要几分钟；下方会显示当前启动阶段。
          </>
        ) : (
          "本地服务正在启动。"
        )}
        此页面会自动重连，无需手动刷新。
      </p>
      <small>{backendUrl ?? "http://127.0.0.1:8787"}</small>
      <div
        className="offline-progress"
        role="progressbar"
        aria-label="本地服务启动进度"
        aria-valuetext={`${startupLabel}，${elapsedLabel}`}
      >
        <div className="offline-progress__bar" />
        <div className="offline-progress__summary" aria-live="polite">
          <strong>{startupLabel}</strong>
          <span>{elapsedLabel}</span>
        </div>
      </div>
      <div className="offline-actions">
        <button
          className="secondary-button"
          type="button"
          disabled={isProbing}
          onClick={onRetry}
        >
          <RotateCcw size={14} className={isProbing ? "spin" : undefined} />
          {isProbing ? "连接中…" : "立即重试"}
        </button>
        <small>已尝试 {attempt + 1} 次</small>
      </div>
      {/* Early failures are just the build still running, so the raw error only
          matters once retrying has clearly stopped helping. */}
      {error && attempt >= 10 && <pre>{error}</pre>}
    </div>
  );
}

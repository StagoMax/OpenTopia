import {
  ArrowLeft,
  CheckCircle2,
  FileText,
  Loader2,
  Plus,
  Trash2,
  WandSparkles,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import type {
  CreatedSkill,
  CreateSkillInput,
  GenerateSkillInput,
  SkillDescriptor,
  SkillDraft,
  SkillDraftPreview,
  SkillScope,
} from "../types";
import "./SkillCreatorDialog.css";

type SkillCreatorStage = "describe" | "review" | "complete";

type SkillCreatorDialogProps = {
  workspaceRoot: string | null;
  projectName: string | null;
  onGenerate(input: GenerateSkillInput): Promise<SkillDraftPreview>;
  onCreate(input: CreateSkillInput): Promise<CreatedSkill>;
  onCreated(skill: SkillDescriptor): void;
  onClose(): void;
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function yamlString(value: string): string {
  return JSON.stringify(value);
}

function renderSkillMd(draft: SkillDraft): string {
  return `---\nname: ${yamlString(draft.name)}\ndescription: ${yamlString(draft.description)}\n---\n\n${draft.instructions.trim()}\n`;
}

function renderOpenAiYaml(draft: SkillDraft): string {
  return [
    "interface:",
    `  display_name: ${yamlString(draft.displayName)}`,
    `  short_description: ${yamlString(draft.shortDescription)}`,
    `  default_prompt: ${yamlString(draft.defaultPrompt)}`,
    "",
  ].join("\n");
}

function targetPathForName(targetPath: string, name: string): string {
  const slash = Math.max(
    targetPath.lastIndexOf("/"),
    targetPath.lastIndexOf("\\"),
  );
  return slash >= 0 ? `${targetPath.slice(0, slash + 1)}${name}` : name;
}

function validateDraft(draft: SkillDraft): string | null {
  if (
    !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(draft.name) ||
    draft.name.length > 64
  ) {
    return "标识必须是 1-64 位小写字母、数字和单连字符。";
  }
  if (!draft.description.trim()) return "请填写触发描述。";
  if (!draft.instructions.trim()) return "请填写 Skill 指令。";
  if (!draft.displayName.trim()) return "请填写显示名称。";
  if (!draft.shortDescription.trim()) return "请填写简短说明。";
  if (!draft.defaultPrompt.includes(`$${draft.name}`)) {
    return `默认提示必须显式包含 $${draft.name}。`;
  }
  const paths = new Set<string>();
  for (const resource of draft.resources) {
    const path = resource.path.trim().replaceAll("\\", "/");
    if (
      !/^(references|scripts|assets)\/[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(path)
    ) {
      return `资源路径无效：${resource.path}`;
    }
    if (path.split("/").some((part) => part === ".." || part.startsWith("."))) {
      return `资源路径不能包含隐藏目录或路径穿越：${resource.path}`;
    }
    const key = path.toLowerCase();
    if (paths.has(key)) return `资源路径重复：${resource.path}`;
    paths.add(key);
  }
  return null;
}

export function SkillCreatorDialog({
  workspaceRoot,
  projectName,
  onGenerate,
  onCreate,
  onCreated,
  onClose,
}: SkillCreatorDialogProps) {
  const [stage, setStage] = useState<SkillCreatorStage>("describe");
  const [scope, setScope] = useState<SkillScope>(
    workspaceRoot ? "workspace" : "user",
  );
  const [prompt, setPrompt] = useState("");
  const [preview, setPreview] = useState<SkillDraftPreview | null>(null);
  const [draft, setDraft] = useState<SkillDraft | null>(null);
  const [previewFile, setPreviewFile] = useState("SKILL.md");
  const [created, setCreated] = useState<CreatedSkill | null>(null);
  const [generating, setGenerating] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reviewNameRef = useRef<HTMLInputElement>(null);
  const busy = generating || creating;

  const previewFiles = useMemo(
    () =>
      draft
        ? [
            "SKILL.md",
            "agents/openai.yaml",
            ...draft.resources.map((resource) => resource.path),
          ]
        : [],
    [draft],
  );
  const previewContent = useMemo(() => {
    if (!draft) return "";
    if (previewFile === "SKILL.md") return renderSkillMd(draft);
    if (previewFile === "agents/openai.yaml") return renderOpenAiYaml(draft);
    return (
      draft.resources.find((resource) => resource.path === previewFile)
        ?.content ?? ""
    );
  }, [draft, previewFile]);

  useEffect(() => {
    if (stage === "review") reviewNameRef.current?.focus();
  }, [stage]);

  function requestClose() {
    if (busy) return;
    const hasUnsavedWork =
      stage !== "complete" && Boolean(prompt.trim() || draft);
    if (
      hasUnsavedWork &&
      !window.confirm("关闭后将丢失尚未创建的 Skill 草稿。确定关闭吗？")
    ) {
      return;
    }
    onClose();
  }

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      requestClose();
    };
    document.addEventListener("keydown", handleKeyDown, true);
    return () => document.removeEventListener("keydown", handleKeyDown, true);
  });

  async function generate(event: FormEvent) {
    event.preventDefault();
    if (!prompt.trim() || generating) return;
    if (scope === "workspace" && !workspaceRoot) {
      setError("创建项目 Skill 前需要先选择工作区。是否改为用户 Skill？");
      return;
    }
    setGenerating(true);
    setError(null);
    try {
      const next = await onGenerate({
        prompt: prompt.trim(),
        scope,
        workspaceRoot: scope === "workspace" ? workspaceRoot : null,
      });
      setPreview(next);
      setDraft(next.draft);
      setPreviewFile("SKILL.md");
      setStage("review");
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setGenerating(false);
    }
  }

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!draft || creating) return;
    const validationError = validateDraft(draft);
    if (validationError) {
      setError(validationError);
      return;
    }
    setCreating(true);
    setError(null);
    try {
      const result = await onCreate({
        draft,
        scope,
        workspaceRoot: scope === "workspace" ? workspaceRoot : null,
      });
      setCreated(result);
      setStage("complete");
      onCreated(result.skill);
    } catch (nextError) {
      setError(errorMessage(nextError));
    } finally {
      setCreating(false);
    }
  }

  function updateDraft<K extends keyof SkillDraft>(
    key: K,
    value: SkillDraft[K],
  ) {
    setDraft((current) => (current ? { ...current, [key]: value } : current));
    setError(null);
  }

  function updateResource(
    index: number,
    key: "path" | "content",
    value: string,
  ) {
    if (!draft) return;
    const resources = draft.resources.map((resource, resourceIndex) =>
      resourceIndex === index ? { ...resource, [key]: value } : resource,
    );
    const previousPath = draft.resources[index]?.path;
    updateDraft("resources", resources);
    if (key === "path" && previewFile === previousPath) setPreviewFile(value);
  }

  function addResource() {
    if (!draft || draft.resources.length >= 24) return;
    const nextPath = `references/reference-${draft.resources.length + 1}.md`;
    updateDraft("resources", [
      ...draft.resources,
      { path: nextPath, content: "# Reference\n\n" },
    ]);
    setPreviewFile(nextPath);
  }

  function removeResource(index: number) {
    if (!draft) return;
    const removed = draft.resources[index];
    updateDraft(
      "resources",
      draft.resources.filter((_, resourceIndex) => resourceIndex !== index),
    );
    if (removed?.path === previewFile) setPreviewFile("SKILL.md");
  }

  function createAnother() {
    setStage("describe");
    setPrompt("");
    setPreview(null);
    setDraft(null);
    setCreated(null);
    setPreviewFile("SKILL.md");
    setError(null);
  }

  const targetPath =
    preview && draft ? targetPathForName(preview.targetPath, draft.name) : "";
  const hasKnownConflict =
    Boolean(preview?.targetExists) && draft?.name === preview?.draft.name;

  return (
    <div
      className="modal-backdrop skill-creator-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) requestClose();
      }}
    >
      <section
        className="skill-creator-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-creator-title"
        aria-describedby="skill-creator-subtitle"
        aria-busy={busy}
      >
        <header className="skill-creator-header">
          <div className="skill-creator-heading">
            <span className="skill-creator-mark" aria-hidden="true">
              <WandSparkles size={17} />
            </span>
            <div>
              <h2 id="skill-creator-title">创建 Skill</h2>
              <p id="skill-creator-subtitle">{projectName ?? "个人工作区"}</p>
            </div>
          </div>
          <ol className="skill-creator-progress" aria-label="创建进度">
            {[
              ["describe", "描述"],
              ["review", "检查"],
              ["complete", "完成"],
            ].map(([value, label], index) => {
              const stages: SkillCreatorStage[] = [
                "describe",
                "review",
                "complete",
              ];
              const activeIndex = stages.indexOf(stage);
              const itemIndex = stages.indexOf(value as SkillCreatorStage);
              return (
                <li
                  className={itemIndex <= activeIndex ? "active" : ""}
                  aria-current={value === stage ? "step" : undefined}
                  key={value}
                >
                  <span>{index + 1}</span>
                  {label}
                </li>
              );
            })}
          </ol>
          <button
            className="icon-button small"
            type="button"
            aria-label="关闭 Skill 创建器"
            title="关闭"
            disabled={busy}
            onClick={requestClose}
          >
            <X size={15} />
          </button>
        </header>

        {stage === "describe" && (
          <form className="skill-creator-describe" onSubmit={generate}>
            <fieldset className="skill-scope-fieldset">
              <legend>保存位置</legend>
              <div className="skill-scope-control">
                <button
                  className={scope === "workspace" ? "active" : ""}
                  type="button"
                  aria-pressed={scope === "workspace"}
                  disabled={!workspaceRoot || generating}
                  title={workspaceRoot ?? "选择工作区后可用"}
                  onClick={() => setScope("workspace")}
                >
                  项目
                </button>
                <button
                  className={scope === "user" ? "active" : ""}
                  type="button"
                  aria-pressed={scope === "user"}
                  disabled={generating}
                  onClick={() => setScope("user")}
                >
                  用户
                </button>
              </div>
              <span>
                {scope === "workspace"
                  ? workspaceRoot
                  : "所有项目均可发现此 Skill"}
              </span>
            </fieldset>

            <label
              className="skill-prompt-field"
              htmlFor="skill-creator-prompt"
            >
              <span>需求</span>
              <textarea
                id="skill-creator-prompt"
                autoFocus
                maxLength={12000}
                value={prompt}
                placeholder="例如：创建一个用于审查 REST API 兼容性的 Skill，检查错误格式、分页、幂等性和破坏性变更。"
                disabled={generating}
                onChange={(event) => {
                  setPrompt(event.target.value);
                  setError(null);
                }}
              />
              <small>{prompt.length.toLocaleString()} / 12,000</small>
            </label>

            {error && (
              <p className="skill-creator-error" role="alert">
                {error}
              </p>
            )}
            <footer className="skill-creator-footer">
              <button
                className="secondary-button"
                type="button"
                onClick={requestClose}
              >
                取消
              </button>
              <button
                className="primary-button"
                type="submit"
                disabled={!prompt.trim() || generating}
              >
                {generating ? (
                  <Loader2 className="spin" size={14} />
                ) : (
                  <WandSparkles size={14} />
                )}
                {generating ? "正在生成" : "生成草稿"}
              </button>
            </footer>
          </form>
        )}

        {stage === "review" && draft && preview && (
          <form className="skill-creator-review" onSubmit={create}>
            <div className="skill-creator-editor">
              <div className="skill-form-grid">
                <label htmlFor="skill-draft-name">
                  <span>标识</span>
                  <input
                    id="skill-draft-name"
                    ref={reviewNameRef}
                    maxLength={64}
                    value={draft.name}
                    disabled={creating}
                    onChange={(event) =>
                      updateDraft("name", event.target.value)
                    }
                  />
                </label>
                <label htmlFor="skill-draft-display-name">
                  <span>显示名称</span>
                  <input
                    id="skill-draft-display-name"
                    maxLength={64}
                    value={draft.displayName}
                    disabled={creating}
                    onChange={(event) =>
                      updateDraft("displayName", event.target.value)
                    }
                  />
                </label>
              </div>
              <label htmlFor="skill-draft-description">
                <span>触发描述</span>
                <textarea
                  id="skill-draft-description"
                  className="compact"
                  maxLength={1024}
                  value={draft.description}
                  disabled={creating}
                  onChange={(event) =>
                    updateDraft("description", event.target.value)
                  }
                />
              </label>
              <div className="skill-form-grid">
                <label htmlFor="skill-draft-short-description">
                  <span>简短说明</span>
                  <input
                    id="skill-draft-short-description"
                    maxLength={64}
                    value={draft.shortDescription}
                    disabled={creating}
                    onChange={(event) =>
                      updateDraft("shortDescription", event.target.value)
                    }
                  />
                </label>
                <label htmlFor="skill-draft-default-prompt">
                  <span>默认提示</span>
                  <input
                    id="skill-draft-default-prompt"
                    maxLength={512}
                    value={draft.defaultPrompt}
                    disabled={creating}
                    onChange={(event) =>
                      updateDraft("defaultPrompt", event.target.value)
                    }
                  />
                </label>
              </div>
              <label
                className="skill-instructions-field"
                htmlFor="skill-draft-instructions"
              >
                <span>指令</span>
                <textarea
                  id="skill-draft-instructions"
                  value={draft.instructions}
                  disabled={creating}
                  onChange={(event) =>
                    updateDraft("instructions", event.target.value)
                  }
                />
              </label>

              <section
                className="skill-resources"
                aria-labelledby="skill-resources-title"
              >
                <header>
                  <div>
                    <strong id="skill-resources-title">资源</strong>
                    <span>{draft.resources.length} / 24</span>
                  </div>
                  <button
                    className="icon-button small"
                    type="button"
                    title="添加资源"
                    aria-label="添加 Skill 资源"
                    disabled={creating || draft.resources.length >= 24}
                    onClick={addResource}
                  >
                    <Plus size={14} />
                  </button>
                </header>
                {draft.resources.map((resource, index) => (
                  <details
                    className="skill-resource-row"
                    key={`${index}-${resource.path}`}
                  >
                    <summary>
                      <FileText size={13} aria-hidden="true" />
                      <span>{resource.path || `资源 ${index + 1}`}</span>
                      <button
                        className="icon-button small danger"
                        type="button"
                        title="移除资源"
                        aria-label={`移除 ${resource.path || `资源 ${index + 1}`}`}
                        disabled={creating}
                        onClick={(event) => {
                          event.preventDefault();
                          removeResource(index);
                        }}
                      >
                        <Trash2 size={13} />
                      </button>
                    </summary>
                    <label>
                      <span>路径</span>
                      <input
                        value={resource.path}
                        disabled={creating}
                        onChange={(event) =>
                          updateResource(index, "path", event.target.value)
                        }
                      />
                    </label>
                    <label>
                      <span>内容</span>
                      <textarea
                        value={resource.content}
                        disabled={creating}
                        onChange={(event) =>
                          updateResource(index, "content", event.target.value)
                        }
                      />
                    </label>
                  </details>
                ))}
              </section>
            </div>

            <aside className="skill-preview" aria-label="Skill 文件预览">
              <header>
                <strong>文件预览</strong>
                <select
                  aria-label="选择预览文件"
                  value={previewFile}
                  onChange={(event) => setPreviewFile(event.target.value)}
                >
                  {previewFiles.map((file) => (
                    <option key={file} value={file}>
                      {file}
                    </option>
                  ))}
                </select>
              </header>
              <pre>
                <code>{previewContent}</code>
              </pre>
              <div className="skill-target-path" title={targetPath}>
                <span>目标</span>
                <code>{targetPath}</code>
              </div>
            </aside>

            <div className="skill-review-footer">
              <div>
                {hasKnownConflict && (
                  <p className="skill-creator-error" role="alert">
                    目标位置已存在同名 Skill，请修改标识。
                  </p>
                )}
                {error && !hasKnownConflict && (
                  <p className="skill-creator-error" role="alert">
                    {error}
                  </p>
                )}
              </div>
              <div className="skill-review-actions">
                <button
                  className="secondary-button"
                  type="button"
                  disabled={creating}
                  onClick={() => {
                    setStage("describe");
                    setError(null);
                  }}
                >
                  <ArrowLeft size={14} />
                  返回
                </button>
                <button
                  className="primary-button"
                  type="submit"
                  disabled={creating || hasKnownConflict}
                >
                  {creating && <Loader2 className="spin" size={14} />}
                  {creating ? "正在创建" : "确认创建"}
                </button>
              </div>
            </div>
          </form>
        )}

        {stage === "complete" && created && (
          <div className="skill-creator-complete" role="status">
            <CheckCircle2 size={28} aria-hidden="true" />
            <h3>{created.skill.name}</h3>
            <p>{created.skill.path}</p>
            <span>{created.files.length} 个文件已创建</span>
            <footer className="skill-creator-footer">
              <button
                className="secondary-button"
                type="button"
                onClick={createAnother}
              >
                继续创建
              </button>
              <button
                className="primary-button"
                type="button"
                onClick={onClose}
              >
                完成
              </button>
            </footer>
          </div>
        )}
      </section>
    </div>
  );
}

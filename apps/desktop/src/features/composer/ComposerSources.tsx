import { Plug, X } from "lucide-react";
import { FileTypeIcon } from "../../components/FileTypeIcon";
import { formatBytes } from "../../formatBytes";
import type { ContextSourceFile, SkillDescriptor } from "../../types";
import type { ComposerImageAttachment } from "./composerDom";

export function ComposerSources({
  contextSources,
  skills,
  selectedSkillIds,
  imageAttachments = [],
  onRemoveContextSource,
  onPreviewImage,
  onRemoveImage,
  onToggleSkill,
}: {
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  imageAttachments?: ComposerImageAttachment[];
  onRemoveContextSource(path: string): void;
  onPreviewImage?(id: string): void;
  onRemoveImage?(id: string): void;
  onToggleSkill(skillId: string): void;
}) {
  if (
    imageAttachments.length === 0 &&
    contextSources.length === 0 &&
    selectedSkillIds.length === 0
  )
    return null;

  return (
    <div className="composer-sources" aria-label="已添加来源">
      {imageAttachments.map((attachment) => (
        <span
          className="composer-source composer-media-source"
          key={attachment.id}
          title={`${attachment.name || "图片"}（ID: ${attachment.id}）`}
        >
          <button
            className="composer-media-preview"
            type="button"
            aria-label={`预览图片 ${attachment.id.slice(0, 8)}`}
            onClick={() => onPreviewImage?.(attachment.id)}
          >
            <img src={attachment.previewUrl} alt="" aria-hidden="true" />
          </button>
          <span>{attachment.name || "图片"}</span>
          <small>[图片 · {attachment.id.slice(0, 8)}]</small>
          <button
            type="button"
            title={`移除 ${attachment.name || "图片"}`}
            aria-label={`移除 ${attachment.name || "图片"}`}
            onClick={() => onRemoveImage?.(attachment.id)}
          >
            <X size={12} />
          </button>
        </span>
      ))}
      {contextSources.map((source) => (
        <span className="composer-source" key={source.path} title={source.path}>
          <FileTypeIcon name={source.name || source.extension} />
          <span>{source.name}</span>
          <small>{formatBytes(source.bytes)}</small>
          <button
            type="button"
            title={`移除 ${source.name}`}
            aria-label={`移除 ${source.name}`}
            onClick={() => onRemoveContextSource(source.path)}
          >
            <X size={12} />
          </button>
        </span>
      ))}
      {skills
        .filter((skill) => selectedSkillIds.includes(skill.id))
        .map((skill) => (
          <span
            className="composer-source is-skill"
            key={skill.id}
            title={skill.description || skill.path}
          >
            <Plug size={12} />
            <span>{skill.name}</span>
            <small>Skill</small>
            <button
              type="button"
              title={`移除 ${skill.name}`}
              aria-label={`移除 ${skill.name}`}
              onClick={() => onToggleSkill(skill.id)}
            >
              <X size={12} />
            </button>
          </span>
        ))}
    </div>
  );
}

import { Plug, X } from "lucide-react";
import { FileTypeIcon } from "../../components/FileTypeIcon";
import { formatBytes } from "../../formatBytes";
import type { ContextSourceFile, SkillDescriptor } from "../../types";

export function ComposerSources({
  contextSources,
  skills,
  selectedSkillIds,
  onRemoveContextSource,
  onToggleSkill,
}: {
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
}) {
  if (contextSources.length === 0 && selectedSkillIds.length === 0) return null;

  return (
    <div className="composer-sources" aria-label="已添加来源">
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

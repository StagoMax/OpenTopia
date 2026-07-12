import { useCallback, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  File,
  FileCode2,
  FileImage,
  FileJson,
  FileText,
} from "lucide-react";
import type { ArtifactContent, ArtifactDescriptor } from "../types";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";

type ArtifactGalleryProps = {
  artifacts: ArtifactDescriptor[];
  onGetArtifact: (
    threadId: string,
    artifactId: string,
  ) => Promise<ArtifactContent>;
  threadId: string | null;
  onOpenPath?: (targetPath: string) => void;
};

function artifactIcon(kind: string) {
  const lower = kind.toLocaleLowerCase();
  if (
    lower.includes("image") ||
    lower.includes("png") ||
    lower.includes("jpg")
  ) {
    return FileImage;
  }
  if (lower.includes("json")) return FileJson;
  if (lower.includes("code") || lower.includes("text")) return FileCode2;
  if (lower.includes("file")) return File;
  return FileText;
}

export function ArtifactGallery({
  artifacts,
  onGetArtifact,
  threadId,
  onOpenPath,
}: ArtifactGalleryProps) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [artifactContent, setArtifactContent] =
    useState<ArtifactContent | null>(null);

  const handleToggle = useCallback(
    async (artifactId: string) => {
      if (expandedId === artifactId) {
        setExpandedId(null);
        setArtifactContent(null);
        return;
      }

      if (!threadId) return;
      setExpandedId(artifactId);
      setLoadingId(artifactId);
      setArtifactContent(null);
      try {
        const content = await onGetArtifact(threadId, artifactId);
        setArtifactContent(content);
      } catch {
        setArtifactContent({
          id: artifactId,
          content: "Failed to load artifact content.",
        });
      } finally {
        setLoadingId(null);
      }
    },
    [expandedId, threadId, onGetArtifact],
  );

  if (!artifacts.length) return null;

  return (
    <section className="panel-card artifact-gallery">
      <div className="artifact-gallery-header">
        <FileCode2 size={14} />
        <span>Artifacts</span>
        <span className="artifact-count">{artifacts.length}</span>
      </div>
      <div className="artifact-list">
        {artifacts.map((artifact) => {
          const Icon = artifactIcon(artifact.kind);
          const isExpanded = expandedId === artifact.id;
          return (
            <div key={artifact.id} className="artifact-item">
              <button
                className="artifact-item-header"
                type="button"
                onClick={() => handleToggle(artifact.id)}
              >
                {isExpanded ? (
                  <ChevronDown size={14} />
                ) : (
                  <ChevronRight size={14} />
                )}
                <Icon size={14} />
                <span className="artifact-kind">{artifact.kind}</span>
                <span className="artifact-meta">{artifact.contentType}</span>
                <span className="artifact-bytes">
                  {formatArtifactBytes(artifact.bytes)}
                </span>
              </button>
              {isExpanded && (
                <div className="artifact-item-body">
                  {loadingId === artifact.id ? (
                    <span className="muted">Loading...</span>
                  ) : artifactContent ? (
                    <>
                      <div className="artifact-content-preview">
                        <MonacoEditor
                          value={artifactContent.content}
                          language={detectLanguage(
                            artifactContent.filePath ?? artifact.kind,
                          )}
                          readOnly
                        />
                      </div>
                      {artifactContent.filePath && (
                        <button
                          className="artifact-file-link"
                          type="button"
                          title={artifactContent.filePath}
                          onClick={() =>
                            onOpenPath?.(artifactContent.filePath ?? "")
                          }
                        >
                          <ExternalLink size={12} />
                          {artifactContent.filePath}
                        </button>
                      )}
                    </>
                  ) : (
                    <span className="muted">No content.</span>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}

function formatArtifactBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB"];
  let amount = value / 1024;
  let unitIndex = 0;
  while (amount >= 1024 && unitIndex < units.length - 1) {
    amount /= 1024;
    unitIndex += 1;
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

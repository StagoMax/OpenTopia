export type ArtifactReference = {
  id: string;
  kind?: string;
  contentType?: string;
  bytes?: number;
  metadata?: unknown;
};

export function artifactReferencesFromText(text: string): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  const pattern =
    /\[Artifact:\s*([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\]/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    refs.push({ id: match[1] });
  }
  return refs;
}

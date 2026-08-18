export type PreviewViewMode = "preview" | "source" | "split";

export type PreviewDocumentSession = {
  mode: PreviewViewMode;
  draft: string;
  baseline: string;
  revision: string;
  dirty: boolean;
  externalChanged: boolean;
};

/**
 * Keeps editor drafts outside the root React tree. App-level consumers only
 * subscribe to the aggregate dirty flag used by close/unload guards, so typing
 * in a document does not rerender the whole desktop shell.
 */
export class PreviewSessionStore {
  private readonly sessions = new Map<string, PreviewDocumentSession>();
  private readonly dirtyListeners = new Set<() => void>();
  private anyDirty = false;

  get(sessionId: string): PreviewDocumentSession | undefined {
    return this.sessions.get(sessionId);
  }

  set(sessionId: string, session: PreviewDocumentSession): void {
    this.sessions.set(sessionId, session);
    this.publishDirtyChange();
  }

  delete(sessionId: string): void {
    if (!this.sessions.delete(sessionId)) return;
    this.publishDirtyChange();
  }

  clear(): void {
    if (this.sessions.size === 0) return;
    this.sessions.clear();
    this.publishDirtyChange();
  }

  isDirty(sessionId: string): boolean {
    return this.sessions.get(sessionId)?.dirty ?? false;
  }

  hasDirtySessions = (): boolean => this.anyDirty;

  subscribeToDirtySessions = (listener: () => void): (() => void) => {
    this.dirtyListeners.add(listener);
    return () => this.dirtyListeners.delete(listener);
  };

  private publishDirtyChange(): void {
    const nextAnyDirty = [...this.sessions.values()].some(
      (session) => session.dirty,
    );
    if (nextAnyDirty === this.anyDirty) return;
    this.anyDirty = nextAnyDirty;
    this.dirtyListeners.forEach((listener) => listener());
  }
}

export type BrowserAddressSyncInput = {
  currentValue: string;
  browserUrl: string;
  loading: boolean;
  error?: string | null;
  editing: boolean;
  dirty: boolean;
  previousBrowserUrl: string;
  pendingUrl: string | null;
};

export type BrowserAddressSyncResult = {
  value: string;
  pendingUrl: string | null;
};

/**
 * Reconciles asynchronous browser URL events with the address-bar draft.
 * Intermediate navigation events must not erase text that the user is editing.
 */
export function syncBrowserAddress(
  input: BrowserAddressSyncInput,
): BrowserAddressSyncResult {
  const {
    browserUrl,
    currentValue,
    dirty,
    editing,
    error,
    loading,
    previousBrowserUrl,
    pendingUrl,
  } = input;

  if (pendingUrl) {
    const reachedRequestedUrl = browserUrl === pendingUrl;
    const reachedTerminalState =
      !loading &&
      (Boolean(error) ||
        browserUrl === pendingUrl ||
        browserUrl !== previousBrowserUrl);
    if (!reachedRequestedUrl && !reachedTerminalState) {
      return { value: currentValue, pendingUrl };
    }
    return {
      value: browserUrl || pendingUrl,
      pendingUrl: loading ? pendingUrl : null,
    };
  }

  if (editing || dirty) return { value: currentValue, pendingUrl: null };
  return { value: browserUrl, pendingUrl: null };
}

const WINDOWS_VERBATIM_PREFIX = "\\\\?\\";
const WINDOWS_VERBATIM_UNC_PREFIX = "\\\\?\\UNC\\";

export function formatPathForDisplay(path: string): string {
  if (
    path.slice(0, WINDOWS_VERBATIM_UNC_PREFIX.length).toUpperCase() ===
    WINDOWS_VERBATIM_UNC_PREFIX
  ) {
    return `\\\\${path.slice(WINDOWS_VERBATIM_UNC_PREFIX.length)}`;
  }
  if (path.startsWith(WINDOWS_VERBATIM_PREFIX)) {
    return path.slice(WINDOWS_VERBATIM_PREFIX.length);
  }
  return path;
}

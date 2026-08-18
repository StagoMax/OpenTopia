export function workspaceRootKey(workspaceRoot: string): string {
  let unified = workspaceRoot.trim().replace(/\\/g, "/");
  if (/^\/\/\?\/unc\//i.test(unified)) {
    unified = `//${unified.slice(8)}`;
  } else if (/^\/\/\?\//.test(unified)) {
    unified = unified.slice(4);
  }
  const prefix = unified.startsWith("//")
    ? "//"
    : unified.startsWith("/")
      ? "/"
      : "";
  const remainder = unified.slice(prefix.length).replace(/^\/+/, "");
  const normalized = `${prefix}${remainder.replace(/\/+/g, "/")}`;
  const withoutTrailingSeparators =
    normalized.length > prefix.length
      ? normalized.replace(/\/+$/, "")
      : normalized;
  return withoutTrailingSeparators.toLowerCase();
}

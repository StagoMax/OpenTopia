const HTTP_SCHEMES = new Set(["http:", "https:"]);

/**
 * Resolves text entered by a person in the browser toolbar. Programmatic
 * browser navigation must continue to require an absolute HTTP(S) URL.
 */
export function resolveAddressBarInput(value: string): string {
  const input = value.trim();
  if (!input) throw new Error("请输入 URL 或搜索内容。");

  const absolute = parseHttpUrl(input);
  if (absolute) return absolute;

  if (looksLikeHost(input)) {
    const scheme = isLocalHost(input) ? "http" : "https";
    const hostUrl = parseHttpUrl(`${scheme}://${input}`);
    if (hostUrl) return hostUrl;
  }

  const search = new URL("https://www.google.com/search");
  search.searchParams.set("q", input);
  return search.toString();
}

function parseHttpUrl(value: string): string | null {
  try {
    const parsed = new URL(value);
    if (!HTTP_SCHEMES.has(parsed.protocol) || !parsed.hostname) return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

function looksLikeHost(value: string): boolean {
  if (/\s/.test(value) || value.includes("://")) return false;

  const authority = value.split(/[/?#]/, 1)[0] ?? "";
  if (!authority || authority.includes("@")) return false;

  const hostname = authority.startsWith("[")
    ? authority.slice(1, authority.indexOf("]"))
    : authority.replace(/:\d+$/, "");
  if (!hostname) return false;

  return (
    hostname === "localhost" ||
    isIpAddress(hostname) ||
    (/^[a-z\d](?:[a-z\d-]{0,61}[a-z\d])?(?:\.[a-z\d](?:[a-z\d-]{0,61}[a-z\d])?)+$/i.test(
      hostname,
    ) &&
      !hostname.endsWith("."))
  );
}

function isLocalHost(value: string): boolean {
  const authority = value.split(/[/?#]/, 1)[0] ?? "";
  const hostname = authority.startsWith("[")
    ? authority.slice(1, authority.indexOf("]"))
    : authority.replace(/:\d+$/, "");
  return (
    hostname.toLowerCase() === "localhost" ||
    hostname === "::1" ||
    /^127(?:\.\d{1,3}){3}$/.test(hostname)
  );
}

function isIpAddress(hostname: string): boolean {
  if (hostname.includes(":")) {
    return /^[\da-f:]+$/i.test(hostname) && hostname.includes(":");
  }
  const parts = hostname.split(".");
  return (
    parts.length === 4 &&
    parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
  );
}

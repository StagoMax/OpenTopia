export function compilePattern(pattern) {
  const names = [];
  const source = pattern.split("/").filter(Boolean).map((segment) => {
    if (segment.startsWith(":")) {
      names.push(segment.slice(1));
      return "([^/]+)";
    }
    if (segment.startsWith("*")) {
      names.push(segment.slice(1));
      return "(.*)";
    }
    return segment;
  }).join("/");
  return { names, expression: new RegExp(`^/${source}/?$`) };
}

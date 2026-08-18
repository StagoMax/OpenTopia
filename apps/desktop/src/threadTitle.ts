const MAX_THREAD_TITLE_CHARS = 50;

export function threadTitleNeedsSummary(prompt: string): boolean {
  return Array.from(prompt.trim()).length > MAX_THREAD_TITLE_CHARS;
}

export function threadTitleFromPrompt(prompt: string): string {
  const title = prompt.trim();
  const chars = Array.from(title);
  if (chars.length <= MAX_THREAD_TITLE_CHARS) return title;
  const singleLineTitle = Array.from(title.replace(/\s+/g, " "));
  return `${singleLineTitle.slice(0, MAX_THREAD_TITLE_CHARS - 1).join("")}…`;
}

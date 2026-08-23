const MAX_THREAD_TITLE_CHARS = 100;
const MAX_CONVERSATION_HEADER_TITLE_CHARS = 50;

export function threadTitleFromPrompt(prompt: string): string {
  const title = prompt.trim().replace(/\s+/g, " ");
  const chars = Array.from(title);
  if (chars.length <= MAX_THREAD_TITLE_CHARS) return title;
  return `${chars.slice(0, MAX_THREAD_TITLE_CHARS - 1).join("")}…`;
}

export function conversationHeaderTitle(title: string): string {
  const chars = Array.from(title);
  if (chars.length <= MAX_CONVERSATION_HEADER_TITLE_CHARS) return title;
  return `${chars
    .slice(0, MAX_CONVERSATION_HEADER_TITLE_CHARS - 1)
    .join("")}…`;
}

import type { AgentEvent, Message } from "./types";

export type TaskNotificationPreferences = {
  enabled: boolean;
  systemNotification: boolean;
  completionSound: boolean;
  onlyWhenUnfocused: boolean;
};

export type TaskCompletionNotificationContent = {
  userMessage: string;
  reply: string;
};

/**
 * Keep both fields within a compact Windows notification card. The last sent
 * message is shown as the title and the reply as the body, so it remains visible
 * instead of being pushed below the toast's text area.
 */
export const taskNotificationReplyMaxChars = 260;
// The Electron main-process boundary counts UTF-16 code units and caps titles
// at 120. Sixty Unicode code points stays within that limit even for astral
// symbols such as emoji.
const taskNotificationTitleMaxChars = 60;

export const defaultTaskNotificationPreferences: TaskNotificationPreferences = {
  enabled: true,
  systemNotification: true,
  completionSound: true,
  onlyWhenUnfocused: true,
};

const taskNotificationStorageKey = "opentopia.taskNotifications.v1";

export function readTaskNotificationPreferences(): TaskNotificationPreferences {
  if (typeof window === "undefined") return defaultTaskNotificationPreferences;
  try {
    const stored = JSON.parse(
      window.localStorage.getItem(taskNotificationStorageKey) ?? "{}",
    ) as Partial<TaskNotificationPreferences>;
    return {
      enabled:
        typeof stored.enabled === "boolean"
          ? stored.enabled
          : defaultTaskNotificationPreferences.enabled,
      systemNotification:
        typeof stored.systemNotification === "boolean"
          ? stored.systemNotification
          : defaultTaskNotificationPreferences.systemNotification,
      completionSound:
        typeof stored.completionSound === "boolean"
          ? stored.completionSound
          : defaultTaskNotificationPreferences.completionSound,
      onlyWhenUnfocused:
        typeof stored.onlyWhenUnfocused === "boolean"
          ? stored.onlyWhenUnfocused
          : defaultTaskNotificationPreferences.onlyWhenUnfocused,
    };
  } catch {
    return defaultTaskNotificationPreferences;
  }
}

export function writeTaskNotificationPreferences(
  preferences: TaskNotificationPreferences,
): void {
  try {
    window.localStorage.setItem(
      taskNotificationStorageKey,
      JSON.stringify(preferences),
    );
  } catch {
    // Desktop preferences remain usable for the session if storage is unavailable.
  }
}

export function shouldDeliverTaskNotification(
  preferences: TaskNotificationPreferences,
  windowHasFocus: boolean,
): boolean {
  return (
    preferences.enabled && (!preferences.onlyWhenUnfocused || !windowHasFocus)
  );
}

export function messageText(message: Message): string {
  return message.parts
    .flatMap((part) => {
      switch (part.type) {
        case "text":
          return [part.text];
        case "file_ref":
          return [part.path];
        case "source_ref":
          return [part.source.name || part.source.path];
        case "skill_ref":
          return [part.skill.name];
        case "image":
        case "image_ref":
          return ["[图片]"];
        case "error":
          return [part.message];
        case "tool_call":
        case "tool_result":
        case "turn_context":
          return [];
        default:
          return [];
      }
    })
    .join("\n");
}

function normalizeNotificationText(value: string): string {
  return value
    .replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "")
    .replace(/[\u200b-\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

export function truncateNotificationText(value: string, maxChars: number): string {
  if (maxChars <= 0) return "";
  const chars = Array.from(value);
  if (chars.length <= maxChars) return value;
  if (maxChars === 1) return "…";
  return `${chars.slice(0, maxChars - 1).join("")}…`;
}

function truncateNotificationTextByCodeUnits(
  value: string,
  maxCodeUnits: number,
): string {
  if (value.length <= maxCodeUnits) return value;
  if (maxCodeUnits <= 0) return "";
  const ellipsis = "…";
  let result = "";
  for (const char of value) {
    if (result.length + char.length + ellipsis.length > maxCodeUnits) break;
    result += char;
  }
  return `${result}${ellipsis}`;
}

/**
 * Resolves the user message and actual final assistant message for one
 * completed turn. The completion summary is only a fallback: it is often a
 * lifecycle sentence rather than the reply shown in the conversation.
 */
export function resolveTaskCompletionNotificationContent(
  messages: Message[],
  events: AgentEvent[],
  finishedEvent: AgentEvent,
): TaskCompletionNotificationContent {
  if (finishedEvent.payload.type !== "turn_finished") {
    return { userMessage: "", reply: "" };
  }

  const turnId = finishedEvent.turnId ?? null;
  let userMessageId: string | undefined;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (
      event.payload.type === "turn_started" &&
      (turnId === null || event.turnId === turnId)
    ) {
      userMessageId = event.payload.user_message_id;
      break;
    }
  }

  const userMessage = userMessageId
    ? messages.find((message) => message.id === userMessageId)
    : [...messages].reverse().find((message) => message.role === "user");
  let reply = "";
  let assistantMessage: Message | undefined;
  const turnStartedAt = [...events].reverse().find(
    (event) =>
      event.payload.type === "turn_started" &&
      (turnId === null || event.turnId === turnId),
  )?.createdAt;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (
      event.payload.type === "assistant_message" &&
      (turnId === null || event.turnId === turnId)
    ) {
      assistantMessage = event.payload.message;
      break;
    }
  }
  if (!assistantMessage && turnId !== null) {
    for (let index = events.length - 1; index >= 0; index -= 1) {
      const event = events[index];
      if (
        event.payload.type === "assistant_message" &&
        !event.turnId &&
        (!turnStartedAt ||
          Date.parse(event.createdAt) >= Date.parse(turnStartedAt))
      ) {
        assistantMessage = event.payload.message;
        break;
      }
    }
  }
  if (!assistantMessage && turnId === null) {
    assistantMessage = [...messages]
      .reverse()
      .find(
        (message) =>
          message.role === "assistant" &&
          (!turnStartedAt ||
            Date.parse(message.createdAt) >= Date.parse(turnStartedAt)),
      );
  }
  if (!assistantMessage && turnStartedAt) {
    assistantMessage = [...messages]
      .reverse()
      .find(
        (message) =>
          message.role === "assistant" &&
          Date.parse(message.createdAt) >= Date.parse(turnStartedAt),
      );
  }
  if (assistantMessage) reply = messageText(assistantMessage);
  if (!reply) reply = finishedEvent.payload.summary;

  return {
    userMessage: userMessage ? messageText(userMessage) : "",
    reply,
  };
}

export function formatTaskCompletionNotificationTitle(
  content: TaskCompletionNotificationContent,
): string {
  const userMessage = normalizeNotificationText(content.userMessage);
  if (!userMessage) return "任务已完成";
  return truncateNotificationText(userMessage, taskNotificationTitleMaxChars);
}

export function formatTaskCompletionNotificationBody(
  content: TaskCompletionNotificationContent,
): string {
  return truncateNotificationTextByCodeUnits(
    normalizeNotificationText(content.reply) || "任务已完成。",
    taskNotificationReplyMaxChars,
  );
}

export function playCompletionChime(): void {
  if (typeof window === "undefined") return;
  try {
    const AudioContextConstructor =
      window.AudioContext ??
      (
        window as typeof window & {
          webkitAudioContext?: typeof AudioContext;
        }
      ).webkitAudioContext;
    if (!AudioContextConstructor) return;

    const context = new AudioContextConstructor();
    const gain = context.createGain();
    const first = context.createOscillator();
    const second = context.createOscillator();
    const start = context.currentTime;

    first.type = "sine";
    first.frequency.setValueAtTime(659.25, start);
    second.type = "sine";
    second.frequency.setValueAtTime(880, start + 0.09);
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.08, start + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.28);

    first.connect(gain);
    second.connect(gain);
    gain.connect(context.destination);
    first.start(start);
    first.stop(start + 0.16);
    second.start(start + 0.09);
    second.stop(start + 0.28);
    second.addEventListener("ended", () => {
      void context.close().catch(() => undefined);
    });
  } catch {
    // Audio feedback must never interrupt task event processing.
  }
}

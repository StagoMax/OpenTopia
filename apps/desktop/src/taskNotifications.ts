export type TaskNotificationPreferences = {
  enabled: boolean;
  systemNotification: boolean;
  completionSound: boolean;
  onlyWhenUnfocused: boolean;
};

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

export function playCompletionChime(): void {
  if (typeof window === "undefined") return;
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
    void context.close();
  });
}

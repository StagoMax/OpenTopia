export const SOLAR_CHROME_SLOT_MINUTES = 30;

export type SolarChromeSegment =
  | "night-sunrise"
  | "sunrise-morning"
  | "morning-noon"
  | "noon-afternoon"
  | "afternoon-sunset"
  | "sunset-night";

export type SolarChromeState = {
  segment: SolarChromeSegment;
  progress: number;
  slotMinutes: number;
};

type SolarAnchor = {
  name: "sunrise" | "morning" | "noon" | "afternoon" | "sunset" | "night";
  minutes: number;
};

const DAY_MINUTES = 24 * 60;
const FIRST_ANCHOR_MINUTES = 6 * 60 + 30;
let previewSlotMinutes: number | null = null;

const anchors: readonly SolarAnchor[] = [
  { name: "sunrise", minutes: FIRST_ANCHOR_MINUTES },
  { name: "morning", minutes: 9 * 60 + 30 },
  { name: "noon", minutes: 13 * 60 },
  { name: "afternoon", minutes: 16 * 60 + 30 },
  { name: "sunset", minutes: 18 * 60 + 30 },
  { name: "night", minutes: 23 * 60 + 30 },
  { name: "sunrise", minutes: FIRST_ANCHOR_MINUTES + DAY_MINUTES },
];

export function getSolarChromeStateForMinutes(
  minutes: number,
): SolarChromeState {
  if (!Number.isFinite(minutes)) {
    throw new RangeError("Solar chrome minutes must be a finite number");
  }

  const slotMinutes =
    Math.floor(
      Math.min(DAY_MINUTES - 1, Math.max(0, minutes)) /
        SOLAR_CHROME_SLOT_MINUTES,
    ) * SOLAR_CHROME_SLOT_MINUTES;
  const normalizedMinutes =
    slotMinutes < FIRST_ANCHOR_MINUTES
      ? slotMinutes + DAY_MINUTES
      : slotMinutes;

  for (let index = 0; index < anchors.length - 1; index += 1) {
    const from = anchors[index];
    const to = anchors[index + 1];
    if (normalizedMinutes < from.minutes || normalizedMinutes >= to.minutes) {
      continue;
    }

    return {
      segment: `${from.name}-${to.name}` as SolarChromeSegment,
      progress:
        (normalizedMinutes - from.minutes) / (to.minutes - from.minutes),
      slotMinutes,
    };
  }

  throw new Error("Unable to resolve solar chrome time segment");
}

export function getSolarChromeState(date: Date): SolarChromeState {
  return getSolarChromeStateForMinutes(
    date.getHours() * 60 + date.getMinutes(),
  );
}

export function millisecondsUntilNextSolarSlot(date: Date): number {
  const next = new Date(date);
  next.setSeconds(0, 0);
  next.setMinutes(
    Math.floor(date.getMinutes() / SOLAR_CHROME_SLOT_MINUTES) *
      SOLAR_CHROME_SLOT_MINUTES +
      SOLAR_CHROME_SLOT_MINUTES,
  );
  return Math.max(1, next.getTime() - date.getTime());
}

export function applySolarChrome(
  date: Date = new Date(),
  root: HTMLElement = document.documentElement,
): SolarChromeState {
  return applySolarChromeState(getSolarChromeState(date), root);
}

export function setSolarChromePreview(
  minutes: number,
  root: HTMLElement = document.documentElement,
): SolarChromeState {
  const state = getSolarChromeStateForMinutes(minutes);
  previewSlotMinutes = state.slotMinutes;
  return applySolarChromeState(state, root);
}

export function clearSolarChromePreview(
  root: HTMLElement = document.documentElement,
): SolarChromeState {
  previewSlotMinutes = null;
  return applySolarChrome(new Date(), root);
}

function applySolarChromeState(
  state: SolarChromeState,
  root: HTMLElement,
): SolarChromeState {
  const progress = `${Math.round(state.progress * 10_000) / 100}%`;
  root.dataset.solarSegment = state.segment;
  root.style.setProperty("--solar-phase-progress", progress);
  return state;
}

export function startSolarChromeClock(
  root: HTMLElement = document.documentElement,
): () => void {
  let timer: number | undefined;

  const refresh = () => {
    if (timer !== undefined) window.clearTimeout(timer);
    const now = new Date();
    if (previewSlotMinutes === null) {
      applySolarChrome(now, root);
    } else {
      applySolarChromeState(
        getSolarChromeStateForMinutes(previewSlotMinutes),
        root,
      );
    }
    timer = window.setTimeout(
      refresh,
      millisecondsUntilNextSolarSlot(now) + 20,
    );
  };
  const refreshWhenVisible = () => {
    if (document.visibilityState === "visible") refresh();
  };

  refresh();
  window.addEventListener("focus", refresh);
  document.addEventListener("visibilitychange", refreshWhenVisible);

  return () => {
    if (timer !== undefined) window.clearTimeout(timer);
    window.removeEventListener("focus", refresh);
    document.removeEventListener("visibilitychange", refreshWhenVisible);
  };
}

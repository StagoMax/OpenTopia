import assert from "node:assert/strict";
import test from "node:test";

import type * as SolarChromeModule from "./solarChrome";

const {
  getSolarChromeState,
  getSolarChromeStateForMinutes,
  millisecondsUntilNextSolarSlot,
} = (await import("./solarChrome" + ".ts")) as typeof SolarChromeModule;

test("quantizes the local clock to half-hour solar slots", () => {
  assert.deepEqual(getSolarChromeState(new Date(2026, 7, 11, 6, 47)), {
    segment: "sunrise-morning",
    progress: 0,
    slotMinutes: 6 * 60 + 30,
  });

  assert.deepEqual(getSolarChromeState(new Date(2026, 7, 11, 9, 59)), {
    segment: "morning-noon",
    progress: 0,
    slotMinutes: 9 * 60 + 30,
  });
});

test("interpolates between adjacent anchor palettes", () => {
  const state = getSolarChromeState(new Date(2026, 7, 11, 11, 0));
  assert.equal(state.segment, "morning-noon");
  assert.equal(state.slotMinutes, 11 * 60);
  assert.equal(state.progress, 3 / 7);
});

test("resolves a preview directly from minutes and keeps it on a valid slot", () => {
  assert.deepEqual(getSolarChromeStateForMinutes(18 * 60 + 17), {
    segment: "afternoon-sunset",
    progress: 3 / 4,
    slotMinutes: 18 * 60,
  });
  assert.equal(getSolarChromeStateForMinutes(-30).slotMinutes, 0);
  assert.equal(
    getSolarChromeStateForMinutes(24 * 60).slotMinutes,
    23 * 60 + 30,
  );
});

test("wraps the night-to-sunrise segment across midnight", () => {
  const beforeMidnight = getSolarChromeState(new Date(2026, 7, 11, 23, 44));
  assert.equal(beforeMidnight.segment, "night-sunrise");
  assert.equal(beforeMidnight.progress, 0);

  const afterMidnight = getSolarChromeState(new Date(2026, 7, 12, 0, 17));
  assert.equal(afterMidnight.segment, "night-sunrise");
  assert.equal(afterMidnight.slotMinutes, 0);
  assert.equal(afterMidnight.progress, 1 / 14);
});

test("schedules the next update on the next wall-clock boundary", () => {
  assert.equal(
    millisecondsUntilNextSolarSlot(new Date(2026, 7, 11, 10, 12, 15, 250)),
    17 * 60 * 1000 + 44_750,
  );
  assert.equal(
    millisecondsUntilNextSolarSlot(new Date(2026, 7, 11, 10, 30, 0, 0)),
    30 * 60 * 1000,
  );
});

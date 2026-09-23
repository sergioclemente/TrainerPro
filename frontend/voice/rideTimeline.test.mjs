import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { transformWithEsbuild } from "vite";

let appendSegmentResult;
let createRideTimelineState;
let observePlayerState;
let observeTrainerStatus;
let recordUserAction;

async function loadTypeScriptModule(relativePath) {
  const sourceUrl = new URL(relativePath, import.meta.url);
  const source = await readFile(sourceUrl, "utf8");
  const transformed = await transformWithEsbuild(source, sourceUrl.pathname, {
    format: "esm",
    loader: "ts",
    target: "es2021",
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(transformed.code).toString("base64")}`;
  return import(moduleUrl);
}

before(async () => {
  ({
    appendSegmentResult,
    createRideTimelineState,
    observePlayerState,
    observeTrainerStatus,
    recordUserAction,
  } = await loadTypeScriptModule("../rideTimeline.ts"));
});

function player(phase, session = "ride-1") {
  return { phase, workout_session_id: session };
}

test("suppresses the phase transition caused by any recent user action", () => {
  let state = createRideTimelineState(player("ready"));
  state = recordUserAction(state, player("ready"), "Start workout", "riding", 100);
  state = observePlayerState(state, player("riding"), 500);

  assert.deepEqual(state.items.map((item) => item.kind), ["user-action"]);
  assert.equal(state.items[0].label, "Start workout");

  state = observePlayerState(state, player("paused"), 3_000);
  assert.equal(state.items.at(-1).message, "Workout paused");
});

test("keeps user actions without a phase beside later ride events", () => {
  let state = createRideTimelineState(player("riding"));
  state = recordUserAction(state, player("riding"), "Skip interval", null);
  state = appendSegmentResult(state, {
    workout_session_id: "ride-1",
    segment_index: 2,
    planned_duration_s: 300,
    ridden_duration_s: 125,
    average_power_w: 247,
    average_cadence_rpm: null,
    skipped: true,
  });

  assert.deepEqual(state.items.map((item) => item.kind), ["user-action", "segment"]);
  assert.equal(state.items[1].result.ridden_duration_s, 125);
});

test("reports trainer loss and recovery only during an active ride", () => {
  let state = createRideTimelineState(player("riding"));
  state = observeTrainerStatus(state, player("riding"), "connected");
  state = observeTrainerStatus(state, player("riding"), "reconnecting");
  state = observeTrainerStatus(state, player("paused"), "connected");

  assert.deepEqual(state.items.map((item) => item.message), [
    "Trainer disconnected",
    "Trainer reconnected",
  ]);
});

test("a new workout session clears the in-memory history", () => {
  let state = createRideTimelineState(player("riding"));
  state = recordUserAction(state, player("riding"), "Pause workout", "paused");
  state = observePlayerState(state, player("ready", "ride-2"));

  assert.equal(state.sessionId, "ride-2");
  assert.deepEqual(state.items, []);
});

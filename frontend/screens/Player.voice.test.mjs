import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let createPlayerVoiceSurface;

async function loadPlayerVoiceModule() {
  const entry = new URL("./Player.voice.ts", import.meta.url).pathname;
  const result = await build({
    entryPoints: [entry],
    bundle: true,
    format: "esm",
    platform: "node",
    target: "es2022",
    write: false,
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`;
  return import(moduleUrl);
}

before(async () => {
  ({ createPlayerVoiceSurface } = await loadPlayerVoiceModule());
});

function player(phase, overrides = {}) {
  return { phase, intensity: 1, erg_enabled: true, ...overrides };
}

function surfaceHarness(initialPlayer) {
  const calls = [];
  let currentPlayer = initialPlayer;
  const surface = createPlayerVoiceSurface({
    getPlayer: () => currentPlayer,
    commands: {
      startRide: async () => calls.push(["startRide"]),
      pauseRide: async () => calls.push(["pauseRide"]),
      resumeRide: async () => calls.push(["resumeRide"]),
      skipSegment: async () => calls.push(["skipSegment"]),
      setIntensity: async (intensity) => calls.push(["setIntensity", intensity]),
      setErg: async (enabled) => calls.push(["setErg", enabled]),
    },
  });
  return {
    calls,
    surface,
    setPlayer: (nextPlayer) => {
      currentPlayer = nextPlayer;
    },
  };
}

function prepare(surface, intentId, transcript) {
  return surface.prepareCommand({
    intentId,
    transcript,
    matchedPhrase: transcript,
    score: 1,
  });
}

function intentIds(surface) {
  return surface.currentIntentGroups().map(({ id }) => id);
}

test("derives one semantic catalog from the Player command registry", () => {
  const { surface } = surfaceHarness(player("ready"));
  assert.deepEqual(surface.catalog.map(({ id }) => id), [
    "start",
    "pause",
    "resume",
    "skip",
    "setIntensity",
    "adjustIntensity",
    "ergOn",
    "ergOff",
  ]);

  const pause = surface.catalog.find(({ id }) => id === "pause");
  assert.ok(pause.phrases.includes("stop"));
  assert.ok(pause.phrases.includes("pause"));
  assert.ok(pause.phrases.includes("take a break"));
  assert.ok(pause.phrases.includes("end the ride"));
});

test("offers only commands available in the live Player phase", () => {
  const harness = surfaceHarness(player("ready"));
  assert.ok(intentIds(harness.surface).includes("start"));
  assert.ok(!intentIds(harness.surface).includes("pause"));

  harness.setPlayer(player("riding"));
  assert.ok(intentIds(harness.surface).includes("pause"));
  assert.ok(!intentIds(harness.surface).includes("resume"));

  harness.setPlayer(player("paused"));
  assert.ok(intentIds(harness.surface).includes("resume"));

  harness.setPlayer(player("finished"));
  assert.deepEqual(intentIds(harness.surface), []);
});

test("parses bounded intensity slots from digits and common spoken forms", async () => {
  const harness = surfaceHarness(player("riding"));

  const spoken = prepare(
    harness.surface,
    "setIntensity",
    "Set intensity to ninety five percent",
  );
  assert.equal(spoken.kind, "command");
  assert.equal(spoken.label, "Set intensity to 95%");
  assert.equal(await spoken.execute(), "Intensity set to 95%");

  const compactHundreds = prepare(
    harness.surface,
    "setIntensity",
    "Set intensity to one twenty percent",
  );
  assert.equal(compactHundreds.kind, "command");
  assert.equal(compactHundreds.label, "Set intensity to 120%");

  assert.deepEqual(harness.calls, [["setIntensity", 0.95]]);
  assert.deepEqual(
    prepare(harness.surface, "setIntensity", "Set intensity to 200 percent"),
    { kind: "rejected", visible: true },
  );
});

test("relative intensity resolves against live state and clamps to product bounds", async () => {
  const harness = surfaceHarness(player("riding", { intensity: 0.95 }));
  const decrease = prepare(
    harness.surface,
    "adjustIntensity",
    "Decrease intensity by ten percent",
  );
  assert.equal(decrease.kind, "command");
  assert.equal(await decrease.execute(), "Intensity set to 85%");

  harness.setPlayer(player("riding", { intensity: 1.45 }));
  const increase = prepare(
    harness.surface,
    "adjustIntensity",
    "Bump intensity up ten percent",
  );
  assert.equal(increase.kind, "command");
  assert.equal(await increase.execute(), "Intensity set to 150%");
  assert.deepEqual(harness.calls, [
    ["setIntensity", 0.85],
    ["setIntensity", 1.5],
  ]);
});

test("all end-like language prepares the pause command", async () => {
  const harness = surfaceHarness(player("riding"));
  const prepared = prepare(harness.surface, "pause", "End the ride");
  assert.equal(prepared.kind, "command");
  assert.equal(prepared.label, "Pause workout");
  assert.equal(prepared.successNotice, "Finish the ride manually when ready");
  assert.equal(await prepared.execute(), "Workout paused — finish manually");
  assert.deepEqual(harness.calls, [["pauseRide"]]);

  const breakCommand = prepare(harness.surface, "pause", "Take a break");
  assert.equal(breakCommand.kind, "command");
  assert.equal(breakCommand.label, "Pause workout");
});

test("recovers the observed end-ride substitution only while riding", () => {
  const harness = surfaceHarness(player("riding"));
  assert.deepEqual(harness.surface.prepareTranscript("And the ride."), {
    kind: "route",
    transcript: "end the ride",
  });

  harness.setPlayer(player("ready"));
  assert.deepEqual(harness.surface.prepareTranscript("And the ride."), {
    kind: "rejected",
    visible: false,
  });
});

test("already-satisfied desired state is a successful no-op", async () => {
  const harness = surfaceHarness(player("riding", { erg_enabled: true }));
  const prepared = prepare(harness.surface, "ergOn", "Enable erg mode");
  assert.equal(prepared.kind, "command");
  assert.equal(await prepared.execute(), "ERG already enabled");
  assert.deepEqual(harness.calls, []);
});

test("prepared commands re-read Player state immediately before execution", async () => {
  const harness = surfaceHarness(player("riding"));
  const prepared = prepare(harness.surface, "skip", "Skip this interval");
  assert.equal(prepared.kind, "command");

  harness.setPlayer(player("ready"));
  await assert.rejects(prepared.execute(), /Start the ride/);
  assert.deepEqual(harness.calls, []);

  harness.setPlayer(null);
  await assert.rejects(prepared.execute(), /no longer active/);
});

test("rejects unknown and phase-ineligible registry keys without exposing them", () => {
  const { surface } = surfaceHarness(player("ready"));
  assert.deepEqual(prepare(surface, "deleteWorkout", "delete the workout"), {
    kind: "rejected",
    visible: false,
  });
  assert.deepEqual(prepare(surface, "pause", "pause"), {
    kind: "rejected",
    visible: false,
  });
});

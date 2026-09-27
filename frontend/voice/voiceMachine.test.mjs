import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { transformWithEsbuild } from "vite";

let createVoiceMachine;
let reduceVoiceMachine;

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
  ({ createVoiceMachine, reduceVoiceMachine } = await loadTypeScriptModule("./voiceMachine.ts"));
});

function reduce(state, event) {
  return reduceVoiceMachine(state, event);
}

test("microphone permission alone does not enable voice", () => {
  let state = createVoiceMachine({ enabled: false, surfaceKey: "player" });
  state = reduce(state, { type: "permission-changed", permission: "granted" });
  assert.equal(state.phase, "off");
  state = reduce(state, { type: "enabled-changed", enabled: true });
  assert.equal(state.phase, "permission-required");
  state = reduce(state, { type: "permission-changed", permission: "granted" });
  assert.equal(state.phase, "preparing");
});

test("enabled voice waits for startup microphone permission", () => {
  let state = createVoiceMachine({ enabled: true });
  assert.equal(state.phase, "suspended");

  state = reduce(state, { type: "surface-changed", key: "player" });
  assert.equal(state.phase, "permission-required");
  assert.equal(reduce(state, { type: "ptt-pressed" }), state);
  state = reduce(state, { type: "permission-changed", permission: "granted" });
  assert.equal(state.phase, "preparing");
  state = reduce(state, { type: "prepared", generation: state.generation });
  assert.equal(state.phase, "idle");
});

test("disabling voice invalidates work and blocks preparation", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  const preparingGeneration = state.generation;

  state = reduce(state, { type: "enabled-changed", enabled: false });
  assert.equal(state.phase, "off");
  assert.ok(state.generation > preparingGeneration);
  assert.equal(
    reduce(state, { type: "prepared", generation: preparingGeneration }),
    state,
  );
  state = reduce(state, { type: "enabled-changed", enabled: true });
  assert.equal(state.phase, "permission-required");
});

test("hiding the Player suspends capture and rejects stale model results", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const inferenceGeneration = state.generation;

  state = reduce(state, { type: "app-activity-changed", active: false });
  assert.equal(state.phase, "suspended");
  assert.equal(
    reduce(state, {
      type: "model-result",
      generation: inferenceGeneration,
    }),
    state,
  );
});

test("permission revocation invalidates routing and makes voice unavailable", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const routingGeneration = state.generation;

  state = reduce(state, { type: "permission-changed", permission: "denied" });
  assert.equal(state.phase, "unavailable");
  assert.ok(state.generation > routingGeneration);
  assert.equal(
    reduce(state, { type: "model-result", generation: routingGeneration }),
    state,
  );
  state = reduce(state, { type: "retry" });
  assert.equal(state.permission, "unknown");
  assert.equal(state.phase, "permission-required");
});

test("accepted commands serialize interpreting, executing, and idle", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  assert.equal(state.phase, "interpreting");

  const generation = state.generation;
  state = reduce(state, { type: "model-result", generation });
  assert.equal(state.phase, "executing");
  state = reduce(state, { type: "command-finished", generation });
  assert.equal(state.phase, "idle");
});

test("a rejected interpretation returns to idle without executing", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const generation = state.generation;

  state = reduce(state, { type: "interpretation-rejected", generation });
  assert.equal(state.phase, "idle");
});

test("errors invalidate in-flight work and retry with a fresh generation", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const generation = state.generation;
  state = reduce(state, { type: "failed", generation, message: "microphone ended" });
  assert.equal(state.phase, "error");
  assert.equal(state.error, "microphone ended");
  assert.ok(state.generation > generation);

  state = reduce(state, { type: "retry" });
  assert.equal(state.phase, "preparing");
  assert.equal(state.error, null);
});

test("a runtime restart discards in-flight work and prepares a fresh generation", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const failedGeneration = state.generation;

  state = reduce(state, {
    type: "semantic-router-restart-requested",
    generation: failedGeneration,
  });
  assert.equal(state.phase, "preparing");
  assert.ok(state.generation > failedGeneration);
  assert.equal(
    reduce(state, { type: "model-result", generation: failedGeneration }),
    state,
  );
});

test("a capture discontinuity invalidates an in-flight command and restarts cleanly", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const interruptedGeneration = state.generation;
  assert.equal(state.phase, "interpreting");

  state = reduce(state, {
    type: "capture-restart-requested",
    generation: interruptedGeneration,
  });
  assert.equal(state.phase, "preparing");
  assert.equal(state.captureRestartAttempts, 1);
  assert.ok(state.generation > interruptedGeneration);
  assert.equal(
    reduce(state, { type: "model-result", generation: interruptedGeneration }),
    state,
  );

  state = reduce(state, { type: "prepared", generation: state.generation });
  assert.equal(state.captureRestartAttempts, 1);

  state = reduce(state, { type: "app-activity-changed", active: false });
  assert.equal(state.phase, "suspended");
  assert.equal(state.captureRestartAttempts, 0);
});

test("a model preparation failure stays inside voice and Retry clears it", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "player",
  });
  const generation = state.generation;

  state = reduce(state, {
    type: "failed",
    generation,
    message: "Command model failed to load: invalid model",
  });
  assert.equal(state.phase, "error");
  assert.equal(state.enabled, true);
  assert.equal(state.surfaceKey, "player");
  assert.equal(state.appActive, true);

  state = reduce(state, { type: "retry" });
  assert.equal(state.phase, "preparing");
  assert.equal(state.captureRestartAttempts, 0);
  assert.equal(state.error, null);
});

test("changing surfaces invalidates work even while voice remains active", () => {
  let state = createVoiceMachine({
    enabled: true,
    permission: "granted",
    surfaceKey: "library",
  });
  state = reduce(state, { type: "prepared", generation: state.generation });
  state = reduce(state, { type: "ptt-pressed" });
  state = reduce(state, { type: "speech-started" });
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  const libraryGeneration = state.generation;

  state = reduce(state, { type: "surface-changed", key: "workout-detail:123" });
  assert.equal(state.phase, "preparing");
  assert.ok(state.generation > libraryGeneration);
  assert.equal(
    reduce(state, { type: "model-result", generation: libraryGeneration }),
    state,
  );
});

function ready() {
  const state = createVoiceMachine({ enabled: true, permission: "granted", surfaceKey: "player" });
  return reduce(state, { type: "prepared", generation: state.generation });
}

test("speech cannot execute before release, and additional holds do not queue", () => {
  let state = ready();
  assert.equal(state.phase, "idle");
  assert.equal(reduce(state, { type: "speech-started" }), state);
  state = reduce(state, { type: "ptt-pressed" });
  const generation = state.generation;
  assert.equal(reduce(state, { type: "ptt-pressed" }), state);
  state = reduce(state, { type: "speech-started" });
  assert.equal(reduce(state, { type: "utterance-accepted" }), state);
  state = reduce(state, { type: "ptt-released" });
  state = reduce(state, { type: "utterance-accepted" });
  assert.equal(reduce(state, { type: "ptt-pressed" }), state);
  state = reduce(state, { type: "ptt-canceled" });
  assert.equal(state.phase, "idle");
  assert.equal(reduce(state, { type: "model-result", generation }), state);
});

test("release during startup, loss of input, and maximum hold cancel rather than submit", () => {
  for (const phase of ["starting", "speech", "finalizing", "interpreting"]) {
    let state = ready();
    state = reduce(state, { type: "ptt-pressed" });
    if (phase !== "starting") state = reduce(state, { type: "speech-started" });
    if (["finalizing", "interpreting"].includes(phase)) state = reduce(state, { type: "ptt-released" });
    if (phase === "interpreting") state = reduce(state, { type: "utterance-accepted" });
    const generation = state.generation;
    state = reduce(state, { type: "ptt-canceled" });
    assert.equal(state.phase, "idle");
    assert.ok(state.generation > generation);
    assert.equal(reduce(state, { type: "utterance-accepted" }), state);
    assert.equal(reduce(state, { type: "model-result", generation }), state);
  }
});

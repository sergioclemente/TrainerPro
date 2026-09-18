import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let createVoiceControlSurface;
let isVoiceControlIntent;

before(async () => {
  const entry = new URL("./voiceControls.ts", import.meta.url).pathname;
  const result = await build({
    entryPoints: [entry],
    bundle: true,
    format: "esm",
    platform: "node",
    target: "es2022",
    write: false,
  });
  const moduleUrl = `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`;
  ({ createVoiceControlSurface, isVoiceControlIntent } = await import(moduleUrl));
});

function harness(initiallySuspended = false) {
  let suspended = initiallySuspended;
  const surface = createVoiceControlSurface({
    getCommandsSuspended: () => suspended,
    setCommandsSuspended: (next) => {
      suspended = next;
    },
  });
  return {
    surface,
    suspended: () => suspended,
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

test("exposes suspend only while active and resume only while suspended", async () => {
  const active = harness();
  assert.deepEqual(
    active.surface.currentIntentGroups().map(({ id }) => id),
    ["suspendVoiceCommands"],
  );
  const suspend = prepare(active.surface, "suspendVoiceCommands", "stop listening");
  assert.equal(suspend.kind, "command");
  assert.equal(await suspend.execute(), "Voice commands paused");
  assert.equal(active.suspended(), true);
  assert.deepEqual(
    active.surface.currentIntentGroups().map(({ id }) => id),
    ["resumeVoiceCommands"],
  );

  const resume = prepare(active.surface, "resumeVoiceCommands", "resume listening");
  assert.equal(resume.kind, "command");
  assert.equal(await resume.execute(), "Voice commands active");
  assert.equal(active.suspended(), false);
});

test("recognizes only shared voice-control registry identifiers", () => {
  assert.equal(isVoiceControlIntent("suspendVoiceCommands"), true);
  assert.equal(isVoiceControlIntent("resumeVoiceCommands"), true);
  assert.equal(isVoiceControlIntent("pause"), false);
});

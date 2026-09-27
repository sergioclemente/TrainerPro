import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let createControllerInputHandler, isPushToTalkKey;
before(async () => {
  const result = await build({ entryPoints: [new URL("./controllerInput.ts", import.meta.url).pathname], bundle: true, format: "esm", platform: "node", write: false });
  ({ createControllerInputHandler, isPushToTalkKey } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
});

function harness(profile = "zwift_ride") {
  const calls = [];
  let visible = true;
  const input = createControllerInputHandler({
    begin: () => calls.push("begin"), finish: () => calls.push("finish"),
    cancel: () => calls.push("cancel"), pauseResume: () => calls.push("pause"), visible: () => visible,
  });
  return { calls, hide: () => { visible = false; },
    button: (button, pressed, generation = 1) => input({ generation, profile, event: { kind: "button", button, pressed } }),
    cancel: (generation = 1) => input({ generation, profile, event: { kind: "cancel" } }),
  };
}

test("presets emit one action per edge; all other buttons remain unassigned", () => {
  for (const [profile, ptt, pause, unassigned] of [
    ["zwift_ride", "y", "a", ["dpad_up", "dpad_down", "left_shift_up", "b"]],
    ["wahoo_virtual_bike", "right_steer", "left_steer", ["right_up", "right_down", "left_shift_up"]],
  ]) {
    const h = harness(profile);
    h.button(ptt, true); h.button(ptt, true); h.button(ptt, false); h.button(ptt, false);
    h.button(pause, true); h.button(pause, true); h.button(pause, false);
    for (const key of unassigned) { h.button(key, true); h.button(key, false); }
    assert.deepEqual(h.calls, ["cancel", "begin", "finish", "pause"]);
  }
});

test("loss during a hold cancels without submitting, and stale generations are ignored", () => {
  const h = harness();
  h.button("y", true); h.cancel(2);
  h.button("y", false, 1); h.button("y", true, 2);
  assert.deepEqual(h.calls, ["cancel", "begin", "cancel"]);
  h.button("y", false, 2); h.button("y", true, 2); h.button("y", false, 2);
  assert.deepEqual(h.calls.slice(-2), ["begin", "finish"]);
});

test("hidden Player ignores actions", () => {
  const h = harness(); h.hide(); h.button("a", true); h.button("y", true); h.button("y", false);
  assert.ok(h.calls.every((call) => call === "cancel"));
});

test("an unknown or absent profile cannot trigger Ride actions", () => {
  for (const profile of [null, "future_controller"]) {
    const h = harness(profile);
    h.button("y", true);
    h.button("y", false);
    h.button("a", true);
    assert.deepEqual(h.calls, ["cancel"]);
  }
});

test("only unmodified Space is the talk key", () => {
  const plain = { code: "Space", altKey: false, ctrlKey: false, metaKey: false, shiftKey: false };
  assert.equal(isPushToTalkKey(plain), true);
  for (const modifier of ["altKey", "ctrlKey", "metaKey", "shiftKey"]) assert.equal(isPushToTalkKey({ ...plain, [modifier]: true }), false);
  assert.equal(isPushToTalkKey({ ...plain, code: "KeyS" }), false);
});

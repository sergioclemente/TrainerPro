import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let hudView;
let changedTargets;
before(async () => {
  const result = await build({
    entryPoints: [new URL("./miniHud.ts", import.meta.url).pathname],
    bundle: true, format: "esm", platform: "node", write: false,
  });
  ({ hudView, changedTargets } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
});

const AMBER = "#d29922";
const RED = "#f85149";
const GREEN = "#3fb950";

function player(overrides = {}) {
  return {
    phase: "riding",
    workout_session_id: "s",
    workout_definition_id: "d",
    scheduled_workout_id: null,
    workout_name: "Sweet Spot 3x10",
    workout_duration_s: 3600,
    seg_idx: 3,
    seg_remaining_s: 272,
    elapsed_s: 1828,
    ride_s: 1828,
    intensity: 1,
    target_power_w: 230,
    target_cadence_rpm: null,
    average_power_w: 210,
    normalized_power_w: 220,
    training_stress_score: 30,
    ef: null,
    kcal: 400,
    erg_enabled: true,
    ...overrides,
  };
}

function input(overrides = {}) {
  return {
    player: player(),
    measurement: { power_w: 240, cadence_rpm: 92, heart_rate_bpm: 152, power_smoothed_3s_w: 238 },
    ftpW: 250,
    segmentDurationS: 600,
    trainer: { role: "trainer", status: "connected", name: "KICKR" },
    hrm: { role: "hrm", status: "connected", name: "HRM-Dual" },
    summary: null,
    cue: null,
    error: null,
    showIntervalElapsed: false,
    powerTargetReveal: null,
    cadenceTargetReveal: null,
    doneHoldRemainingS: null,
    ...overrides,
  };
}

test("riding: four cells, zone colour on power only, bar hidden", () => {
  const v = hudView(input());
  assert.equal(v.mode, "riding");
  assert.deepEqual(v.cells.map((c) => c.value), ["238", "92", "152", "4:32"]);
  assert.deepEqual(v.cells.map((c) => c.label), ["watts · hold", "rpm", "bpm", "interval"]);
  assert.equal(v.cells[0].color, AMBER, "238 W at FTP 250 is Z4");
  assert.equal(v.cells[0].guidance, "hold", "238 is within 5 % of 230");
  assert.equal(v.cells[1].guidance, undefined, "no cadence target");
  assert.equal(v.barPinned, false);
  assert.equal(v.status, null);
  assert.ok(Math.abs(v.progress.fraction - (1 - 272 / 600)) < 1e-9);
  assert.equal(v.progress.color, AMBER, "progress takes the zone colour");
});

test("the watts cell says push, ease, or hold against the target", () => {
  const push = hudView(input({ measurement: { power_w: 212, cadence_rpm: 92, heart_rate_bpm: 148, power_smoothed_3s_w: 212 } }));
  assert.equal(push.cells[0].guidance, "up");
  assert.equal(push.cells[0].label, "watts · push");
  const ease = hudView(input({ measurement: { power_w: 251, cadence_rpm: 92, heart_rate_bpm: 156, power_smoothed_3s_w: 251 } }));
  assert.equal(ease.cells[0].guidance, "down");
  assert.equal(ease.cells[0].label, "watts · ease");
  // No reading, no advice.
  const none = hudView(input({ measurement: null }));
  assert.equal(none.cells[0].guidance, undefined);
  assert.equal(none.cells[0].label, "watts");
});

test("the rpm cell steers toward a cadence target the same way", () => {
  const slow = hudView(input({ player: player({ target_cadence_rpm: 90 }), measurement: { power_w: 238, cadence_rpm: 82, heart_rate_bpm: 152, power_smoothed_3s_w: 238 } }));
  assert.equal(slow.cells[1].guidance, "up");
  assert.equal(slow.cells[1].label, "rpm · spin up");
  const fast = hudView(input({ player: player({ target_cadence_rpm: 90 }), measurement: { power_w: 238, cadence_rpm: 97, heart_rate_bpm: 152, power_smoothed_3s_w: 238 } }));
  assert.equal(fast.cells[1].guidance, "down");
  assert.equal(fast.cells[1].label, "rpm · slow");
  const hold = hudView(input({ player: player({ target_cadence_rpm: 90 }) }));
  assert.equal(hold.cells[1].guidance, "hold");
  assert.equal(hold.cells[1].label, "rpm · hold");
});

test("intensity off 100 % shows only in the bar", () => {
  const v = hudView(input({ player: player({ intensity: 0.95, target_power_w: 219 }) }));
  assert.equal(v.intensityPct, 95);
  assert.equal(v.cells[0].label, "watts · ease", "238 against the adjusted 219");
});

test("ERG off keeps the glyph but says so; an open interval has no advice", () => {
  const off = hudView(input({ player: player({ erg_enabled: false }) }));
  assert.equal(off.cells[0].label, "watts · erg off");
  assert.equal(off.cells[0].labelColor, AMBER);
  assert.equal(off.cells[0].guidance, "hold", "steering by hand still wants the hint");
  assert.equal(off.ergEnabled, false);

  const open = hudView(input({ player: player({ target_power_w: null, target_cadence_rpm: 85 }) }));
  assert.equal(open.cells[0].guidance, undefined);
  assert.equal(open.cells[0].label, "watts · open");
  assert.equal(open.cells[1].label, "rpm · slow", "92 against 85");
  assert.equal(open.progress.dashed, true);
});

test("clicking the clock shows interval elapsed instead of remaining", () => {
  const v = hudView(input({ showIntervalElapsed: true }));
  assert.equal(v.cells[3].value, "5:28", "600 s interval, 272 s left");
  assert.equal(v.cells[3].label, "elapsed");
  // Without the interval's length there is nothing to count up from.
  const unknown = hudView(input({ showIntervalElapsed: true, segmentDurationS: null }));
  assert.equal(unknown.cells[3].value, "4:32");
  assert.equal(unknown.cells[3].label, "interval");
});

test("the interval clock turns amber inside the last five seconds", () => {
  const v = hudView(input({ player: player({ seg_remaining_s: 4 }) }));
  assert.equal(v.cells[3].value, "0:04");
  assert.equal(v.cells[3].color, AMBER);
  assert.equal(v.cells[3].label, "interval", "no next-target preview");
});

test("paused: sensors dim, label says paused, bar pinned, amber accent", () => {
  const v = hudView(input({ player: player({ phase: "paused" }) }));
  assert.equal(v.mode, "paused");
  assert.equal(v.paused, true);
  assert.equal(v.barPinned, true);
  assert.equal(v.accent, AMBER);
  assert.equal(v.cells[0].value, "–", "no live power while paused");
  assert.equal(v.cells[0].guidance, undefined);
  assert.equal(v.cells[0].dimmed, true);
  assert.equal(v.cells[3].label, "paused");
});

test("HRM lost touches only the bpm cell", () => {
  const v = hudView(input({ hrm: { role: "hrm", status: "reconnecting", name: "HRM-Dual" } }));
  assert.equal(v.mode, "riding");
  assert.equal(v.cells[2].value, "–");
  assert.equal(v.cells[2].label, "hrm lost");
  assert.equal(v.cells[2].fault, true);
  assert.equal(v.cells[2].labelColor, RED);
  assert.equal(v.cells[0].value, "238");
  assert.equal(v.barPinned, false);
});

test("no HRM paired is not a fault", () => {
  const v = hudView(input({ hrm: { role: "hrm", status: "disconnected", name: null }, measurement: { power_w: 240, cadence_rpm: 92, heart_rate_bpm: null, power_smoothed_3s_w: 238 } }));
  assert.equal(v.cells[2].value, "–");
  assert.equal(v.cells[2].label, "bpm");
  assert.equal(v.cells[2].fault, undefined);
});

test("trainer reconnecting: red accent, status line, bar pinned, paused label", () => {
  const v = hudView(input({
    player: player({ phase: "paused" }),
    trainer: { role: "trainer", status: "reconnecting", name: "KICKR" },
  }));
  assert.equal(v.mode, "reconnecting");
  assert.equal(v.accent, RED);
  assert.equal(v.barPinned, true);
  assert.equal(v.cells[0].label, "trainer");
  assert.equal(v.cells[0].fault, true);
  assert.equal(v.cells[3].label, "paused");
  assert.match(v.status.text, /Reconnecting to KICKR/);
  assert.equal(v.progress.color, RED);
});

test("a cue goes to the caption and leaves the bar to the controls and errors", () => {
  const cue = hudView(input({ cue: "Hold steady" }));
  assert.equal(cue.caption, "Hold steady");
  assert.equal(cue.status, null);
  assert.equal(cue.barPinned, false);
  const err = hudView(input({ cue: "Hold steady", error: "Command failed" }));
  assert.equal(err.caption, "Hold steady");
  assert.equal(err.status.text, "Command failed");
  assert.equal(err.status.color, RED);
  assert.equal(hudView(input()).caption, null);
});

test("a revealed power target replaces the reading and says why", () => {
  const offTarget = { power_w: 150, cadence_rpm: 92, heart_rate_bpm: 152, power_smoothed_3s_w: 150 };
  const flashed = hudView(input({ measurement: offTarget, powerTargetReveal: "new" }));
  assert.equal(flashed.cells[0].value, "230");
  assert.equal(flashed.cells[0].label, "new target");
  assert.equal(flashed.cells[0].showsTarget, true);
  assert.equal(flashed.cells[0].guidance, undefined);
  // 230 W of a 250 W FTP is threshold, the same colour rule as the reading.
  assert.equal(flashed.cells[0].color, AMBER);
  const peeked = hudView(input({ powerTargetReveal: "peek" }));
  assert.equal(peeked.cells[0].label, "target");
  // The other cells keep their readings.
  assert.equal(peeked.cells[1].value, "92");
});

test("a new target is not flashed once the reading is on it", () => {
  // 238 W against 230 W and 92 rpm against 95 rpm are both within 5 %.
  const onTarget = hudView(input({
    player: player({ target_cadence_rpm: 95 }),
    powerTargetReveal: "new",
    cadenceTargetReveal: "new",
  }));
  assert.equal(onTarget.cells[0].value, "238");
  assert.equal(onTarget.cells[0].showsTarget, undefined);
  assert.equal(onTarget.cells[1].value, "92");
  assert.equal(onTarget.cells[1].showsTarget, undefined);
  // Off the cadence target, the rpm cell flashes it.
  const spinning = hudView(input({
    player: player({ target_cadence_rpm: 110 }),
    cadenceTargetReveal: "new",
  }));
  assert.equal(spinning.cells[1].value, "110");
  assert.equal(spinning.cells[1].label, "new target");
  // A click asks for the target, so a peek shows it even when on target.
  const peeked = hudView(input({ powerTargetReveal: "peek" }));
  assert.equal(peeked.cells[0].value, "230");
});

test("the rpm cell reveals a cadence target the same way", () => {
  const withTarget = input({
    player: player({ target_cadence_rpm: 95 }),
    cadenceTargetReveal: "peek",
  });
  const v = hudView(withTarget);
  assert.equal(v.cells[1].value, "95");
  assert.equal(v.cells[1].label, "target");
  assert.equal(v.cells[1].showsTarget, true);
  assert.equal(v.cells[0].value, "238");
});

test("only a live cell with a target can be peeked", () => {
  const both = hudView(input({ player: player({ target_cadence_rpm: 95 }) }));
  assert.equal(both.cells[0].peek, "power");
  assert.equal(both.cells[1].peek, "cadence");
  assert.equal(both.cells[2].peek, undefined);
  // No cadence target, and an open interval has no power target.
  const open = hudView(input({ player: player({ target_power_w: null }), powerTargetReveal: "peek" }));
  assert.equal(open.cells[0].peek, undefined);
  assert.equal(open.cells[0].showsTarget, undefined);
  assert.equal(open.cells[1].peek, undefined);
  // Paused readings are not being steered by, so nothing is revealed.
  const paused = hudView(input({ player: player({ phase: "paused" }), powerTargetReveal: "new" }));
  assert.equal(paused.cells[0].peek, undefined);
  assert.equal(paused.cells[0].showsTarget, undefined);
});

const targets = (overrides = {}) => ({
  riding: true, segIdx: 3, intensity: 1, targetPowerW: 230, targetCadenceRpm: null, ...overrides,
});

test("a new interval flashes the targets it changed", () => {
  const next = targets({ segIdx: 4, targetPowerW: 180, targetCadenceRpm: 95 });
  assert.deepEqual(changedTargets(targets(), next), { power: true, cadence: true });
  // Same watts in the next interval is not news; an open one has no target.
  assert.deepEqual(changedTargets(targets(), targets({ segIdx: 4 })), { power: false, cadence: false });
  assert.deepEqual(
    changedTargets(targets(), targets({ segIdx: 4, targetPowerW: null })),
    { power: false, cadence: false },
  );
});

test("an intensity change flashes the power target only", () => {
  const before = targets({ targetCadenceRpm: 95 });
  const after = targets({ intensity: 1.01, targetPowerW: 232, targetCadenceRpm: 95 });
  assert.deepEqual(changedTargets(before, after), { power: true, cadence: false });
});

test("a ramp moving its target inside an interval does not flash", () => {
  assert.deepEqual(changedTargets(targets(), targets({ targetPowerW: 231 })), { power: false, cadence: false });
});

test("nothing flashes on the first state or while not riding", () => {
  const next = targets({ segIdx: 4, targetPowerW: 180 });
  assert.deepEqual(changedTargets(null, next), { power: false, cadence: false });
  assert.deepEqual(changedTargets(targets(), { ...next, riding: false }), { power: false, cadence: false });
});

test("finished but not yet recorded: live totals, green, no summary promised yet", () => {
  const v = hudView(input({ player: player({ phase: "finished" }), doneHoldRemainingS: 3 }));
  assert.equal(v.mode, "done");
  assert.equal(v.accent, GREEN);
  assert.equal(v.barPinned, true);
  assert.deepEqual(v.cells.map((c) => c.value), ["210", "220", "30:28", "30"]);
  assert.equal(v.status.text, "Workout complete");
  assert.equal(v.caption, null);
});

test("done: totals replace the live cells and the bar pins green", () => {
  const summary = {
    activity_id: "a", scheduled_workout_id: null, workout_name: "Sweet Spot 3x10",
    started_at_unix_ms: 0, elapsed_s: 3600, timer_s: 3590, average_power_w: 212,
    max_power_w: 300, normalized_power_w: 224, intensity_factor: 0.9,
    training_stress_score: 54.8, average_heart_rate_bpm: 147, max_heart_rate_bpm: 170,
    work_kj: 760, completed_pct: 100, fit_path: "/x.fit", activity_segments: [],
  };
  const v = hudView(input({ player: null, summary }));
  assert.equal(v.mode, "done");
  assert.equal(v.accent, GREEN);
  assert.equal(v.barPinned, true);
  assert.deepEqual(v.cells.map((c) => c.value), ["212", "224", "59:50", "147", "55"]);
  assert.equal(v.cells[4].label, "tss · done");
  assert.equal(v.status.text, "Workout complete");
  assert.equal(v.status.color, GREEN);
  const counting = hudView(input({ player: null, summary, doneHoldRemainingS: 3 }));
  assert.equal(counting.status.text, "Workout complete · summary in 3 s");
});

test("no ride loaded", () => {
  const v = hudView(input({ player: null }));
  assert.equal(v.mode, "empty");
  assert.equal(v.cells.length, 0);
  assert.equal(v.status.text, "No ride loaded");
});

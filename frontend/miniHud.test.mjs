import assert from "node:assert/strict";
import { build } from "esbuild";
import { before, test } from "node:test";

let hudView;
before(async () => {
  const result = await build({
    entryPoints: [new URL("./miniHud.ts", import.meta.url).pathname],
    bundle: true, format: "esm", platform: "node", write: false,
  });
  ({ hudView } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`));
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
    ...overrides,
  };
}

test("riding: five cells, zone colour on power only, bar hidden", () => {
  const v = hudView(input());
  assert.equal(v.mode, "riding");
  assert.deepEqual(v.cells.map((c) => c.value), ["238", "230", "92", "152", "4:32"]);
  assert.deepEqual(v.cells.map((c) => c.label), ["watts", "target", "rpm", "bpm", "interval"]);
  assert.equal(v.cells[0].color, AMBER, "238 W at FTP 250 is Z4");
  assert.notEqual(v.cells[1].color, AMBER);
  assert.equal(v.barPinned, false);
  assert.equal(v.status, null);
  assert.ok(Math.abs(v.progress.fraction - (1 - 272 / 600)) < 1e-9);
  assert.equal(v.progress.color, AMBER, "progress takes the zone colour");
});

test("intensity off 100 % marks the target label amber and reports the percent", () => {
  const v = hudView(input({ player: player({ intensity: 0.95, target_power_w: 219 }) }));
  assert.equal(v.intensityPct, 95);
  assert.equal(v.cells[1].label, "target · 95%");
  assert.equal(v.cells[1].labelColor, AMBER);
});

test("ERG off strikes the target through; an open interval shows a dash", () => {
  const off = hudView(input({ player: player({ erg_enabled: false }) }));
  assert.equal(off.cells[1].label, "erg off");
  assert.equal(off.cells[1].struck, true);
  assert.equal(off.ergEnabled, false);

  const open = hudView(input({ player: player({ target_power_w: null, target_cadence_rpm: 85 }) }));
  assert.equal(open.cells[1].value, "–");
  assert.equal(open.cells[1].label, "open · ride free");
  assert.equal(open.cells[2].label, "rpm · 85 target");
  assert.equal(open.progress.dashed, true);
});

test("clicking the clock shows interval elapsed instead of remaining", () => {
  const v = hudView(input({ showIntervalElapsed: true }));
  assert.equal(v.cells[4].value, "5:28", "600 s interval, 272 s left");
  assert.equal(v.cells[4].label, "elapsed");
  // Without the interval's length there is nothing to count up from.
  const unknown = hudView(input({ showIntervalElapsed: true, segmentDurationS: null }));
  assert.equal(unknown.cells[4].value, "4:32");
  assert.equal(unknown.cells[4].label, "interval");
});

test("the interval clock turns amber inside the last five seconds", () => {
  const v = hudView(input({ player: player({ seg_remaining_s: 4 }) }));
  assert.equal(v.cells[4].value, "0:04");
  assert.equal(v.cells[4].color, AMBER);
  assert.equal(v.cells[4].label, "interval", "no next-target preview");
});

test("paused: sensors dim, label says paused, bar pinned, amber accent", () => {
  const v = hudView(input({ player: player({ phase: "paused" }) }));
  assert.equal(v.mode, "paused");
  assert.equal(v.paused, true);
  assert.equal(v.barPinned, true);
  assert.equal(v.accent, AMBER);
  assert.equal(v.cells[0].value, "–", "no live power while paused");
  assert.equal(v.cells[0].dimmed, true);
  assert.equal(v.cells[4].label, "paused");
});

test("HRM lost touches only the bpm cell", () => {
  const v = hudView(input({ hrm: { role: "hrm", status: "reconnecting", name: "HRM-Dual" } }));
  assert.equal(v.mode, "riding");
  assert.equal(v.cells[3].value, "–");
  assert.equal(v.cells[3].label, "hrm lost");
  assert.equal(v.cells[3].fault, true);
  assert.equal(v.cells[3].labelColor, RED);
  assert.equal(v.cells[0].value, "238");
  assert.equal(v.barPinned, false);
});

test("no HRM paired is not a fault", () => {
  const v = hudView(input({ hrm: { role: "hrm", status: "disconnected", name: null }, measurement: { power_w: 240, cadence_rpm: 92, heart_rate_bpm: null, power_smoothed_3s_w: 238 } }));
  assert.equal(v.cells[3].value, "–");
  assert.equal(v.cells[3].label, "bpm");
  assert.equal(v.cells[3].fault, undefined);
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
  assert.equal(v.cells[4].label, "paused");
  assert.match(v.status.text, /Reconnecting to KICKR/);
  assert.equal(v.progress.color, RED);
});

test("a cue borrows the bar slot; an action error outranks it", () => {
  const cue = hudView(input({ cue: "Hold steady" }));
  assert.equal(cue.status.text, "Hold steady");
  const err = hudView(input({ cue: "Hold steady", error: "Command failed" }));
  assert.equal(err.status.text, "Command failed");
  assert.equal(err.status.color, RED);
});

test("done: totals replace the live cells and the bar pins green", () => {
  const v = hudView(input({
    player: null,
    summary: {
      activity_id: "a", scheduled_workout_id: null, workout_name: "Sweet Spot 3x10",
      started_at_unix_ms: 0, elapsed_s: 3600, timer_s: 3590, average_power_w: 212,
      max_power_w: 300, normalized_power_w: 224, intensity_factor: 0.9,
      training_stress_score: 54.8, average_heart_rate_bpm: 147, max_heart_rate_bpm: 170,
      work_kj: 760, completed_pct: 100, fit_path: "/x.fit", activity_segments: [],
    },
  }));
  assert.equal(v.mode, "done");
  assert.equal(v.accent, GREEN);
  assert.equal(v.barPinned, true);
  assert.deepEqual(v.cells.map((c) => c.value), ["212", "224", "59:50", "147", "55"]);
  assert.equal(v.cells[4].label, "tss · done");
  assert.match(v.status.text, /FIT saved/);
});

test("no ride loaded", () => {
  const v = hudView(input({ player: null }));
  assert.equal(v.mode, "empty");
  assert.equal(v.cells.length, 0);
  assert.equal(v.status.text, "No ride loaded");
});

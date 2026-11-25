// View model for the mini HUD: everything the 400×100 strip shows, derived
// from the same store state the Player uses. Pure, so the state rules
// (what turns amber, red, or green; when the bar pins) are unit-testable.

import type {
  ActivitySummary,
  DeviceStatusEvent,
  PlayerMeasurement,
  PlayerState,
} from "./ipc";
import { fmtDuration } from "./ipc";
import { zoneColor } from "./zones";

/** Seconds before an interval boundary at which its clock turns amber. */
export const HUD_ENDING_LEAD_S = 5;
/** A reading within this fraction of its target is "hold", as in the Player. */
export const HUD_ON_TARGET_TOLERANCE = 0.05;
/** How long a cell shows its target, flashed or clicked, before the reading
    returns. A flash also ends as soon as the reading is on target. */
export const HUD_TARGET_REVEAL_S = 1.5;
/** How long the finished HUD holds the totals before the summary opens. */
export const HUD_DONE_HOLD_S = 4;
/** Colours shared with the app theme (`styles.css` tokens). */
export const HUD_COLOR_NEUTRAL = "#e6edf3";
export const HUD_COLOR_MUTED = "#8b949e";
export const HUD_COLOR_CHANGED = "#d29922"; // rider changed something
export const HUD_COLOR_FAULT = "#f85149"; // a sensor problem
export const HUD_COLOR_DONE = "#3fb950";
const PERCENT_SCALE = 100;

export type HudMode = "empty" | "ready" | "riding" | "paused" | "reconnecting" | "done";

/** What the rider should do to meet the target: more, less, or stay. */
export type HudGuidance = "up" | "down" | "hold";

/** Why a cell shows its target instead of the live reading: the target
    just changed, or the rider clicked the cell to check it. */
export type HudTargetReveal = "new" | "peek";

/** The parts of the player state that decide whether a target changed. */
export interface HudTargetSnapshot {
  riding: boolean;
  segIdx: number | null;
  intensity: number;
  targetPowerW: number | null;
  targetCadenceRpm: number | null;
}

/** Which cells should flash a new target. A target counts as new only when
    the interval or the intensity changed with it, so a ramp, whose target
    moves every second, flashes once at its start and not again. */
export function changedTargets(
  previous: HudTargetSnapshot | null,
  next: HudTargetSnapshot,
): { power: boolean; cadence: boolean } {
  if (previous === null || !next.riding) return { power: false, cadence: false };
  const prescriptionChanged =
    next.segIdx !== previous.segIdx || next.intensity !== previous.intensity;
  return {
    power:
      prescriptionChanged &&
      next.targetPowerW !== null &&
      next.targetPowerW !== previous.targetPowerW,
    cadence:
      prescriptionChanged &&
      next.targetCadenceRpm !== null &&
      next.targetCadenceRpm !== previous.targetCadenceRpm,
  };
}

export interface HudCell {
  value: string;
  label: string;
  /** Direction glyph before the value, when a target exists and a reading
      is live. Shape carries it, so it never competes with the colours. */
  guidance?: HudGuidance;
  /** Colour of the value; the label takes `labelColor` when set. */
  color: string;
  labelColor?: string;
  /** Dim the whole cell (paused sensors, a target ERG is not holding). */
  dimmed?: boolean;
  /** A red dot before the label marks a sensor fault. */
  fault?: boolean;
  /** The value is the target, not a reading; an arrow glyph marks it. */
  showsTarget?: boolean;
  /** Clicking the cell peeks at this target. Absent when there is none. */
  peek?: "power" | "cadence";
}

export interface HudStatus {
  text: string;
  color: string;
}

export interface HudView {
  mode: HudMode;
  /** Border tint; transparent-ish white when nothing is going on. */
  accent: string;
  cells: HudCell[];
  progress: { fraction: number; color: string; dashed: boolean };
  /** The control bar stays up without hover. */
  barPinned: boolean;
  /** Text for the bar slot, replacing the controls while present. */
  status: HudStatus | null;
  /** Workout text cue for the caption below the strip. */
  caption: string | null;
  intensityPct: number;
  ergEnabled: boolean;
  paused: boolean;
}

export interface HudInput {
  player: PlayerState | null;
  measurement: PlayerMeasurement | null;
  /** Rider's FTP for the zone colour; null leaves power uncoloured. */
  ftpW: number | null;
  /** Planned length of the current interval, for the progress line. */
  segmentDurationS: number | null;
  trainer: DeviceStatusEvent | null;
  hrm: DeviceStatusEvent | null;
  /** Set once the ride is recorded; the HUD then shows totals. */
  summary: ActivitySummary | null;
  /** A workout text cue still within its display time, if any. */
  cue: string | null;
  /** The latest error from a HUD action, shown in the bar slot. */
  error: string | null;
  /** Interval clock counts up instead of down (click to toggle). */
  showIntervalElapsed: boolean;
  /** Set while the watts cell shows its target instead of the reading. */
  powerTargetReveal: HudTargetReveal | null;
  /** The same for the rpm cell and the cadence target. */
  cadenceTargetReveal: HudTargetReveal | null;
  /** Whole seconds until the finished HUD opens the summary. */
  doneHoldRemainingS: number | null;
}

const EMPTY_PROGRESS = { fraction: 0, color: HUD_COLOR_MUTED, dashed: false };

function dash(value: number | null | undefined): string {
  return value === null || value === undefined ? "–" : String(value);
}

/** Up when the reading is short of the target, down when over, else hold. */
export function guidance(reading: number | null, target: number | null): HudGuidance | undefined {
  if (reading === null || target === null || target <= 0) return undefined;
  const tolerance = target * HUD_ON_TARGET_TOLERANCE;
  if (reading < target - tolerance) return "up";
  if (reading > target + tolerance) return "down";
  return "hold";
}

/** A clicked peek always shows; a flashed new target is only worth the
    cell while the rider still has to move to it. */
function showsReveal(reveal: HudTargetReveal | null, current: HudGuidance | undefined): boolean {
  return reveal === "peek" || (reveal === "new" && current !== "hold");
}

const REVEAL_LABELS: Record<HudTargetReveal, string> = { new: "new target", peek: "target" };
const POWER_WORDS: Record<HudGuidance, string> = { up: "push", down: "ease", hold: "hold" };
const CADENCE_WORDS: Record<HudGuidance, string> = { up: "spin up", down: "slow", hold: "hold" };

function powerColor(power: number | null, ftpW: number | null): string {
  if (power === null) return HUD_COLOR_MUTED;
  if (ftpW === null || ftpW <= 0) return HUD_COLOR_NEUTRAL;
  return zoneColor((power / ftpW) * PERCENT_SCALE);
}

function doneStatus(doneHoldRemainingS: number | null): HudStatus {
  return {
    text:
      doneHoldRemainingS === null
        ? "Workout complete"
        : `Workout complete · summary in ${doneHoldRemainingS} s`,
    color: HUD_COLOR_DONE,
  };
}

function tssCell(tss: number | null): HudCell {
  return {
    value: tss === null ? "–" : String(Math.round(tss)),
    label: "tss · done",
    color: HUD_COLOR_DONE,
    labelColor: HUD_COLOR_DONE,
  };
}

function doneView(cells: HudCell[], doneHoldRemainingS: number | null): HudView {
  return {
    mode: "done",
    accent: HUD_COLOR_DONE,
    cells,
    progress: { fraction: 1, color: HUD_COLOR_DONE, dashed: false },
    barPinned: true,
    status: doneStatus(doneHoldRemainingS),
    caption: null,
    intensityPct: PERCENT_SCALE,
    ergEnabled: true,
    paused: false,
  };
}

/** Totals of the recorded activity. */
function recordedCells(summary: ActivitySummary): HudCell[] {
  return [
    { value: dash(summary.average_power_w), label: "avg watts", color: HUD_COLOR_NEUTRAL },
    { value: dash(summary.normalized_power_w), label: "np", color: HUD_COLOR_NEUTRAL },
    { value: fmtDuration(summary.timer_s), label: "ride time", color: HUD_COLOR_NEUTRAL },
    { value: dash(summary.average_heart_rate_bpm), label: "avg bpm", color: HUD_COLOR_NEUTRAL },
    tssCell(summary.training_stress_score),
  ];
}

/** Live totals of a finished ride whose activity is not recorded yet, or
    could not be. The player state has no average heart rate. */
function finishedCells(player: PlayerState): HudCell[] {
  return [
    { value: dash(player.average_power_w), label: "avg watts", color: HUD_COLOR_NEUTRAL },
    { value: dash(player.normalized_power_w), label: "np", color: HUD_COLOR_NEUTRAL },
    { value: fmtDuration(player.ride_s), label: "ride time", color: HUD_COLOR_NEUTRAL },
    tssCell(player.training_stress_score),
  ];
}

export function hudView(input: HudInput): HudView {
  const {
    player,
    measurement,
    ftpW,
    segmentDurationS,
    trainer,
    hrm,
    summary,
    cue,
    error,
    showIntervalElapsed,
    powerTargetReveal,
    cadenceTargetReveal,
    doneHoldRemainingS,
  } = input;
  if (summary !== null && player === null) {
    return doneView(recordedCells(summary), doneHoldRemainingS);
  }
  if (player !== null && player.phase === "finished") {
    // No summary is promised until the activity is recorded: saving may
    // still fail, and the full window is then where the error shows.
    return doneView(finishedCells(player), null);
  }
  if (player === null) {
    return {
      mode: "empty",
      accent: "rgba(255, 255, 255, 0.10)",
      cells: [],
      progress: EMPTY_PROGRESS,
      barPinned: true,
      status: { text: "No ride loaded", color: HUD_COLOR_MUTED },
      caption: null,
      intensityPct: PERCENT_SCALE,
      ergEnabled: true,
      paused: false,
    };
  }

  const reconnecting = trainer?.status === "reconnecting";
  const paused = player.phase === "paused";
  const mode: HudMode = reconnecting
    ? "reconnecting"
    : paused
      ? "paused"
      : player.phase === "ready"
        ? "ready"
        : "riding";
  const live = mode === "riding";
  const power = live ? (measurement?.power_smoothed_3s_w ?? measurement?.power_w ?? null) : null;
  const cadence = live ? (measurement?.cadence_rpm ?? null) : null;
  const hrmLost = hrm !== null && hrm.name !== null && hrm.status !== "connected";
  const heartRate = hrmLost ? null : (measurement?.heart_rate_bpm ?? null);
  const target = player.target_power_w;
  const intensityPct = Math.round(player.intensity * PERCENT_SCALE);
  const open = target === null;
  const zone = live ? powerColor(power, ftpW) : HUD_COLOR_MUTED;

  const ending =
    live && player.seg_idx !== null && player.seg_remaining_s <= HUD_ENDING_LEAD_S;
  // Elapsed needs the interval's planned length; without it the clock
  // stays on remaining.
  const intervalElapsedS =
    showIntervalElapsed && segmentDurationS !== null
      ? Math.max(0, segmentDurationS - player.seg_remaining_s)
      : null;
  const intervalLabel =
    reconnecting || paused
      ? "paused"
      : mode === "ready"
        ? "ready"
        : intervalElapsedS !== null
          ? "elapsed"
          : "interval";
  const intervalLabelColor = reconnecting || paused ? HUD_COLOR_CHANGED : HUD_COLOR_MUTED;

  const powerGuidance = guidance(power, target);
  const cadenceTarget = player.target_cadence_rpm;
  const cadenceGuidance = guidance(cadence, cadenceTarget);
  // The target cell is gone: the watts and rpm cells say what to do about
  // it. ERG off replaces the word, since the rider is steering by hand.
  const powerLabel = reconnecting
    ? "trainer"
    : !player.erg_enabled && !open
      ? "watts · erg off"
      : open
        ? "watts · open"
        : powerGuidance !== undefined
          ? `watts · ${POWER_WORDS[powerGuidance]}`
          : "watts";
  const cadenceLabel =
    cadenceTarget !== null && cadenceGuidance !== undefined
      ? `rpm · ${CADENCE_WORDS[cadenceGuidance]}`
      : cadenceTarget !== null
        ? `rpm · ${cadenceTarget} target`
        : "rpm";

  // A target is on offer only while the ride is live; paused and
  // reconnecting cells are not readings the rider is steering by.
  const powerPeek = live && target !== null ? ("power" as const) : undefined;
  const cadencePeek = live && cadenceTarget !== null ? ("cadence" as const) : undefined;

  const cells: HudCell[] = [
    reconnecting
      ? { value: "–", label: powerLabel, color: HUD_COLOR_MUTED, labelColor: HUD_COLOR_FAULT, fault: true }
      : powerPeek !== undefined && powerTargetReveal !== null && showsReveal(powerTargetReveal, powerGuidance)
        ? {
            value: String(target),
            label: REVEAL_LABELS[powerTargetReveal],
            color: powerColor(target, ftpW),
            showsTarget: true,
            peek: powerPeek,
          }
        : {
            value: dash(power),
            label: powerLabel,
            guidance: powerGuidance,
            color: zone,
            labelColor: !player.erg_enabled && !open ? HUD_COLOR_CHANGED : undefined,
            dimmed: paused,
            peek: powerPeek,
          },
    cadencePeek !== undefined && cadenceTargetReveal !== null && showsReveal(cadenceTargetReveal, cadenceGuidance)
      ? {
          value: String(cadenceTarget),
          label: REVEAL_LABELS[cadenceTargetReveal],
          color: HUD_COLOR_NEUTRAL,
          showsTarget: true,
          peek: cadencePeek,
        }
      : {
          value: dash(cadence),
          label: cadenceLabel,
          guidance: cadenceGuidance,
          color: cadence === null ? HUD_COLOR_MUTED : HUD_COLOR_NEUTRAL,
          dimmed: paused,
          peek: cadencePeek,
        },
    hrmLost
      ? { value: "–", label: "hrm lost", color: HUD_COLOR_MUTED, labelColor: HUD_COLOR_FAULT, fault: true }
      : { value: dash(heartRate), label: "bpm", color: heartRate === null ? HUD_COLOR_MUTED : HUD_COLOR_NEUTRAL, dimmed: paused },
    {
      value: fmtDuration(intervalElapsedS ?? Math.max(0, player.seg_remaining_s)),
      label: intervalLabel,
      color: ending ? HUD_COLOR_CHANGED : HUD_COLOR_NEUTRAL,
      labelColor: intervalLabelColor,
    },
  ];

  const fraction =
    segmentDurationS !== null && segmentDurationS > 0
      ? Math.min(1, Math.max(0, 1 - player.seg_remaining_s / segmentDurationS))
      : 0;
  const progressColor = reconnecting
    ? HUD_COLOR_FAULT
    : paused
      ? HUD_COLOR_CHANGED
      : open
        ? HUD_COLOR_MUTED
        : zone;

  const accent = reconnecting
    ? HUD_COLOR_FAULT
    : paused
      ? HUD_COLOR_CHANGED
      : "rgba(255, 255, 255, 0.10)";

  const status: HudStatus | null = reconnecting
    ? { text: `Reconnecting to ${trainer?.name ?? "trainer"}…`, color: HUD_COLOR_FAULT }
    : error !== null
      ? { text: error, color: HUD_COLOR_FAULT }
      : null;

  return {
    mode,
    accent,
    cells,
    progress: { fraction, color: progressColor, dashed: open },
    barPinned: reconnecting || paused || mode === "ready",
    status,
    caption: cue,
    intensityPct,
    ergEnabled: player.erg_enabled,
    paused,
  };
}


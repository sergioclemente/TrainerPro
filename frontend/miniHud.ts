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
/** Colours shared with the app theme (`styles.css` tokens). */
export const HUD_COLOR_NEUTRAL = "#e6edf3";
export const HUD_COLOR_MUTED = "#8b949e";
export const HUD_COLOR_CHANGED = "#d29922"; // rider changed something
export const HUD_COLOR_FAULT = "#f85149"; // a sensor problem
export const HUD_COLOR_DONE = "#3fb950";
const PERCENT_SCALE = 100;

export type HudMode = "empty" | "ready" | "riding" | "paused" | "reconnecting" | "done";

export interface HudCell {
  value: string;
  label: string;
  /** Colour of the value; the label takes `labelColor` when set. */
  color: string;
  labelColor?: string;
  /** Dim the whole cell (paused sensors, a target ERG is not holding). */
  dimmed?: boolean;
  struck?: boolean;
  /** A red dot before the label marks a sensor fault. */
  fault?: boolean;
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
}

const EMPTY_PROGRESS = { fraction: 0, color: HUD_COLOR_MUTED, dashed: false };

function dash(value: number | null | undefined): string {
  return value === null || value === undefined ? "–" : String(value);
}

function powerColor(power: number | null, ftpW: number | null): string {
  if (power === null) return HUD_COLOR_MUTED;
  if (ftpW === null || ftpW <= 0) return HUD_COLOR_NEUTRAL;
  return zoneColor((power / ftpW) * PERCENT_SCALE);
}

function doneView(summary: ActivitySummary): HudView {
  const tss = summary.training_stress_score;
  return {
    mode: "done",
    accent: HUD_COLOR_DONE,
    cells: [
      { value: dash(summary.average_power_w), label: "avg watts", color: HUD_COLOR_NEUTRAL },
      { value: dash(summary.normalized_power_w), label: "np", color: HUD_COLOR_NEUTRAL },
      { value: fmtDuration(summary.timer_s), label: "ride time", color: HUD_COLOR_NEUTRAL },
      { value: dash(summary.average_heart_rate_bpm), label: "avg bpm", color: HUD_COLOR_NEUTRAL },
      {
        value: tss === null ? "–" : String(Math.round(tss)),
        label: "tss · done",
        color: HUD_COLOR_DONE,
        labelColor: HUD_COLOR_DONE,
      },
    ],
    progress: { fraction: 1, color: HUD_COLOR_DONE, dashed: false },
    barPinned: true,
    status: { text: `${fmtDuration(summary.elapsed_s)} · ${summary.workout_name} · FIT saved`, color: HUD_COLOR_MUTED },
    intensityPct: PERCENT_SCALE,
    ergEnabled: true,
    paused: false,
  };
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
  } = input;
  if (summary !== null && player === null) return doneView(summary);
  if (player === null) {
    return {
      mode: "empty",
      accent: "rgba(255, 255, 255, 0.10)",
      cells: [],
      progress: EMPTY_PROGRESS,
      barPinned: true,
      status: { text: "No ride loaded", color: HUD_COLOR_MUTED },
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
  const intensityChanged = intensityPct !== PERCENT_SCALE;
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

  const cells: HudCell[] = [
    reconnecting
      ? { value: "–", label: "trainer", color: HUD_COLOR_MUTED, labelColor: HUD_COLOR_FAULT, fault: true }
      : { value: dash(power), label: "watts", color: zone, dimmed: paused },
    !player.erg_enabled && !open
      ? { value: dash(target), label: "erg off", color: HUD_COLOR_MUTED, labelColor: HUD_COLOR_CHANGED, struck: true }
      : open
        ? { value: "–", label: "open · ride free", color: HUD_COLOR_MUTED }
        : {
            value: dash(target),
            label: intensityChanged ? `target · ${intensityPct}%` : "target",
            color: HUD_COLOR_NEUTRAL,
            labelColor: intensityChanged ? HUD_COLOR_CHANGED : undefined,
            dimmed: paused || reconnecting,
          },
    {
      value: dash(cadence),
      label: player.target_cadence_rpm !== null ? `rpm · ${player.target_cadence_rpm} target` : "rpm",
      color: cadence === null ? HUD_COLOR_MUTED : HUD_COLOR_NEUTRAL,
      dimmed: paused,
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
      : cue !== null
        ? { text: cue, color: HUD_COLOR_NEUTRAL }
        : null;

  return {
    mode,
    accent,
    cells,
    progress: { fraction, color: progressColor, dashed: open },
    barPinned: reconnecting || paused || mode === "ready",
    status,
    intensityPct,
    ergEnabled: player.erg_enabled,
    paused,
  };
}


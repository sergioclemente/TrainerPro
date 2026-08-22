// Typed mirror of the tp-app command surface (SPEC.md §9, src-tauri/src/cmd.rs).

import { invoke } from "@tauri-apps/api/core";

export type Role = "trainer" | "hrm";

export interface WorkoutSummary {
  id: string;
  name: string;
  description: string;
  source_format: string;
  duration_s: number;
  est_if: number;
  est_tss: number;
  graph: [number, number][]; // (t_s, %FTP) breakpoints
  origin: string | null;
}

export interface ImportResult {
  summary: WorkoutSummary;
  warnings: string[];
  already_existed: boolean;
}

export interface DeviceSlot {
  role: Role;
  saved_name: string | null;
  saved_platform_id: string | null;
  connected: boolean;
}

export interface ScanResult {
  platform_id: string;
  name: string;
  rssi: number | null;
  role: Role;
}

export interface PlayerState {
  phase: "ready" | "riding" | "paused" | "finished";
  workout_id: string;
  workout_name: string;
  workout_duration_s: number;
  seg_idx: number | null;
  seg_remaining_s: number;
  /** Position in the workout — a skip jumps this forward. */
  elapsed_s: number;
  /** Time actually ridden: pauses and skipped spans excluded. */
  ride_s: number;
  intensity: number;
  target: number | null;
  avg_power: number | null;
  /** Live session totals, same maths as the post-ride summary. */
  np: number | null;
  tss: number | null;
  /** Efficiency factor: NP / average HR. Null without an HRM. */
  ef: number | null;
  kcal: number | null;
  erg_enabled: boolean;
}

export interface Telemetry {
  power: number | null;
  cadence: number | null;
  hr: number | null;
  target: number | null;
  power_smoothed_3s: number | null;
}

export interface LapRow {
  start_s: number;
  duration_s: number;
  avg_power: number | null;
  max_power: number | null;
  avg_hr: number | null;
}

export interface RideSummary {
  ride_id: string;
  workout_name: string;
  started_at: number;
  elapsed_s: number;
  timer_s: number;
  avg_power: number | null;
  max_power: number | null;
  np: number | null;
  if_: number | null;
  tss: number | null;
  avg_hr: number | null;
  max_hr: number | null;
  kj: number;
  completed_pct: number;
  fit_path: string;
  laps: LapRow[];
}

export interface RideRow {
  id: string;
  workout_name: string;
  started_at: number;
  timer_s: number;
  avg_power: number | null;
  np: number | null;
  tss: number | null;
  avg_hr: number | null;
  completed_pct: number;
  fit_path: string;
}

export interface Profile {
  name: string;
  ftp: number;
  weight_kg: number;
}

/** One workout-library provider's persisted state (Rust SourceConfig). */
export interface SourceConfig {
  enabled: boolean;
  values: Record<string, string>;
}

export interface Settings {
  profile: Profile;
  record_distance: boolean;
  intensity_default: number;
  export_dir: string | null;
  /** Workout-library providers keyed by id (planner, woz, …). */
  sources: Record<string, SourceConfig>;
}

export interface SegmentRow {
  kind: "steady" | "ramp" | "freeride";
  label: string;
  note: string | null;
  duration_s: number;
  start_pct: number;
  end_pct: number;
  cadence_rpm: number | null;
}

export interface WorkoutDetail {
  summary: WorkoutSummary;
  segments: SegmentRow[];
}

export interface PlannerPreview {
  wid: number;
  graph: [number, number][];
  duration_s: number;
  est_if: number;
  est_tss: number;
  segments: SegmentRow[];
}

export interface PlannerWorkout {
  wid: number;
  title: string;
  duration_s: number;
  tss: number | null;
  tags: string;
  created: string | null;
  dsl: string;
}

export interface PlannerListResult {
  rows: PlannerWorkout[];
  from_cache: boolean;
  fetched_at_ms: number;
}

export interface WozCollection {
  slug: string;
  title: string;
}

export interface WozWorkout {
  idx: number;
  title: string;
  duration_s: number;
  est_if: number;
  est_tss: number;
  graph: [number, number][];
  segments: SegmentRow[];
}


export interface DeviceReading {
  role: Role;
  power?: number | null;
  cadence?: number | null;
  hr?: number | null;
}

export interface DeviceStatusEvent {
  role: Role;
  status: "disconnected" | "connecting" | "connected" | "reconnecting";
  attempt: number | null;
  name: string | null;
}

export interface AppError {
  code: string;
  message: string;
}

/** Authoring tree sent to `create_workout`. Mirrors tp_core::build::BuildNode
    (serde tags on `kind`); intensities are percent of FTP. */
export type DraftNode =
  | { kind: "simple"; duration_s: number; power_pct: number; cadence_rpm: number | null }
  | {
      kind: "ramp";
      duration_s: number;
      start_pct: number;
      end_pct: number;
      cadence_rpm: number | null;
    }
  | { kind: "repeat"; count: number; children: DraftNode[] };

export interface WorkoutDraft {
  name: string;
  description: string;
  nodes: DraftNode[];
}

export const ipc = {
  importWorkout: (path: string) => invoke<ImportResult>("import_workout", { path }),
  createWorkout: (draft: WorkoutDraft) => invoke<ImportResult>("create_workout", { draft }),
  listWorkouts: () => invoke<WorkoutSummary[]>("list_workouts"),
  deleteWorkout: (id: string) => invoke<void>("delete_workout", { id }),
  getWorkoutDetail: (id: string) => invoke<WorkoutDetail>("get_workout_detail", { id }),

  startScan: (role: Role) => invoke<void>("start_scan", { role }),
  connectDevice: (role: Role, platformId: string, name?: string) =>
    invoke<void>("connect_device", { role, platformId, name }),
  disconnectDevice: (role: Role) => invoke<void>("disconnect_device", { role }),
  forgetDevice: (role: Role) => invoke<void>("forget_device", { role }),
  getDeviceState: () => invoke<DeviceSlot[]>("get_device_state"),

  loadWorkout: (id: string) => invoke<PlayerState>("load_workout", { id }),
  startRide: () => invoke<void>("start_ride"),
  pauseRide: () => invoke<void>("pause_ride"),
  resumeRide: () => invoke<void>("resume_ride"),
  skipSegment: () => invoke<void>("skip_segment"),
  setIntensity: (pct: number) => invoke<void>("set_intensity", { pct }),
  setErg: (enabled: boolean) => invoke<void>("set_erg", { enabled }),
  endRide: () => invoke<RideSummary>("end_ride"),
  clearRide: () => invoke<void>("clear_ride"),
  getPlayerState: () => invoke<PlayerState | null>("get_player_state"),

  listRides: () => invoke<RideRow[]>("list_rides"),
  deleteRide: (id: string) => invoke<void>("delete_ride", { id }),
  saveFitAs: (id: string, destPath: string) => invoke<void>("save_fit_as", { id, destPath }),
  revealFit: (id: string) => invoke<void>("reveal_fit", { id }),
  openGarminImport: () => invoke<void>("open_garmin_import"),

  sourceTest: (id: string, values: Record<string, string>) =>
    invoke<{ ok: boolean; detail: string }>("source_test", { id, values }),
  plannerCached: () => invoke<PlannerListResult | null>("planner_cached"),
  plannerList: () => invoke<PlannerListResult>("planner_list"),
  plannerRide: (wid: number) => invoke<PlayerState>("planner_ride", { wid }),
  plannerOpenEditor: (wid: number) => invoke<void>("planner_open_editor", { wid }),
  plannerPreview: (wid: number) => invoke<PlannerPreview>("planner_preview", { wid }),

  wozCollections: (force = false) => invoke<WozCollection[]>("woz_collections", { force }),
  wozWorkouts: (collection: string, force = false) =>
    invoke<WozWorkout[]>("woz_workouts", { collection, force }),
  wozRide: (collection: string, idx: number) =>
    invoke<PlayerState>("woz_ride", { collection, idx }),
  wozOpenPage: (collection: string) => invoke<void>("woz_open_page", { collection }),

  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
};

export function fmtDuration(totalS: number): string {
  const h = Math.floor(totalS / 3600);
  const m = Math.floor((totalS % 3600) / 60);
  const s = Math.floor(totalS % 60);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return (h > 0 ? `${h}:` : "") + `${mm}:${String(s).padStart(2, "0")}`;
}

/** Platform-appropriate label for revealing a file in the OS file manager. */
export const revealLabel = navigator.platform.toUpperCase().includes("MAC")
  ? "Reveal in Finder"
  : "Show in Explorer";

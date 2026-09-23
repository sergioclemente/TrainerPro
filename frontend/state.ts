// Zustand store fed by Tauri events; ongoing runtime state is not polled.

import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import {
  ActivityRow,
  ActivitySummary,
  DeviceMeasurement,
  DeviceSlot,
  DeviceStatusEvent,
  NextUpItem,
  PlannerPreview,
  PlannerWorkout,
  PlayerState,
  Role,
  ScanResult,
  SegmentResult,
  Settings,
  PlayerMeasurement,
  WorkoutSummary,
  ipc,
} from "./ipc";
import {
  appendAppMessage,
  appendSegmentResult,
  clearExpectedUserPhase,
  createRideTimelineState,
  observePlayerState,
  observeTrainerStatus,
  recordUserAction,
  type RideTimelineState,
  type RideTimelineTone,
} from "./rideTimeline";
import { traceEvent } from "./trace";

export type Screen =
  | "library"
  | "libraries"
  | "devices"
  | "activities"
  | "settings"
  | "player"
  | "summary"
  | "workout"
  | "builder";

/** Unified payload for the workout detail screen (any source). */
export interface WorkoutDetailView {
  source: "library" | "planner" | "woz";
  id?: string; // library workout id
  wid?: number; // planner workout id
  wozRef?: { collection: string; idx: number }; // whatsonzwift ref
  /** Set when a scheduled Next Up item opened this definition. */
  scheduledWorkoutId?: string;
  /** Source content identity (planner: DSL text) for change detection. */
  contentKey?: string;
  /** A newer version of this workout arrived from the network. */
  updateAvailable?: boolean;
  name: string;
  description: string;
  duration_s: number;
  est_if: number | null;
  est_tss: number | null;
  tags: string;
  origin?: string | null;
  graph: [number, number][];
  segments: import("./ipc").SegmentRow[];
}

export interface Toast {
  id: number;
  level: "info" | "warn" | "error";
  message: string;
}

interface Store {
  screen: Screen;
  /** Which tab the Settings screen opens on (basic | export | libraries | connections). */
  settingsTab: string;
  nextUp: NextUpItem[];
  nextUpStatus: "idle" | "loading" | "ready" | "error";
  nextUpError: string;
  workouts: WorkoutSummary[];
  devices: DeviceSlot[];
  scanResults: ScanResult[];
  scanning: boolean;
  deviceStatus: Record<Role, DeviceStatusEvent | null>;
  deviceMeasurement: Record<Role, DeviceMeasurement | null>;
  player: PlayerState | null;
  measurement: PlayerMeasurement | null;
  rideTimeline: RideTimelineState;
  summary: ActivitySummary | null;
  detail: WorkoutDetailView | null;
  activities: ActivityRow[];
  // Planner state lives here (not in the tab component) so navigating to the
  // detail view and back does NOT re-sync or re-fetch previews.
  plannerRows: PlannerWorkout[];
  plannerPreviews: Record<number, PlannerPreview | "error">;
  plannerStatus: "idle" | "loading" | "waking" | "ready" | "error";
  plannerError: string;
  plannerSyncedAt: number | null;
  plannerFromCache: boolean;
  plannerSync: () => Promise<void>;
  // whatsonzwift browse state (survives navigation like the planner's).
  wozCollections: import("./ipc").WozCollection[];
  wozCollection: string | null;
  wozWorkouts: import("./ipc").WozWorkout[];
  wozStatus: "idle" | "loading" | "ready" | "error";
  wozError: string;
  settings: Settings | null;
  toasts: Toast[];

  go: (s: Screen) => void;
  refreshNextUp: () => Promise<void>;
  refreshWorkouts: () => Promise<void>;
  refreshDevices: () => Promise<void>;
  refreshActivities: () => Promise<void>;
  refreshSettings: () => Promise<void>;
  loadPlayer: (player: PlayerState) => void;
  recordUserAction: (
    label: string,
    expectedPhase?: PlayerState["phase"] | null,
  ) => void;
  recordUserActionFailure: (message: string) => void;
  appendRideMessage: (message: string, tone?: RideTimelineTone) => void;
  clearRideTimeline: () => void;
  pushToast: (level: Toast["level"], message: string) => void;
  dismissToast: (id: number) => void;
}

let toastSeq = 0;
let lastTracedPlayerPhase: PlayerState["phase"] | null = null;

export const useStore = create<Store>((set, get) => ({
  screen: "library",
  settingsTab: "basic",
  nextUp: [],
  nextUpStatus: "idle",
  nextUpError: "",
  workouts: [],
  devices: [],
  scanResults: [],
  scanning: false,
  deviceStatus: { trainer: null, hrm: null },
  deviceMeasurement: { trainer: null, hrm: null },
  player: null,
  measurement: null,
  rideTimeline: createRideTimelineState(),
  summary: null,
  detail: null,
  activities: [],
  plannerRows: [],
  plannerPreviews: {},
  plannerStatus: "idle",
  plannerError: "",
  plannerSyncedAt: null,
  plannerFromCache: false,
  wozCollections: [],
  wozCollection: null,
  wozWorkouts: [],
  wozStatus: "idle",
  wozError: "",
  plannerSync: async () => {
    const hadRows = get().plannerRows.length > 0;
    if (!hadRows) set({ plannerStatus: "loading", plannerError: "" });
    const wake = window.setTimeout(() => {
      if (get().plannerStatus === "loading") set({ plannerStatus: "waking" });
    }, 5000);
    try {
      const r = await ipc.plannerList();
      set({
        plannerRows: r.rows,
        plannerStatus: "ready",
        plannerSyncedAt: r.fetched_at_ms,
        plannerFromCache: r.from_cache,
      });
      // Change detection for an open detail view (same wid, new DSL).
      const d = get().detail;
      if (d && d.source === "planner" && d.wid != null && d.contentKey) {
        const row = r.rows.find((w) => w.wid === d.wid);
        if (row && row.dsl !== d.contentKey && !d.updateAvailable) {
          set({ detail: { ...d, updateAvailable: true } });
        }
      }
      void loadPlannerPreviews(r.rows);
    } catch (e) {
      const err = e as { code?: string; message?: string };
      set({
        plannerStatus: "error",
        plannerError: err.message ?? String(e),
      });
      if (err.code === "planner_auth") {
        get().pushToast("warn", "WorkoutPlanner credentials rejected — check Settings");
      }
    } finally {
      window.clearTimeout(wake);
    }
  },
  settings: null,
  toasts: [],

  go: (s) => set({ screen: s }),
  refreshNextUp: async () => {
    if (get().nextUp.length === 0) set({ nextUpStatus: "loading", nextUpError: "" });
    try {
      set({ nextUp: await ipc.listNextUp(), nextUpStatus: "ready", nextUpError: "" });
    } catch (e) {
      const err = e as { message?: string };
      set({ nextUpStatus: "error", nextUpError: err.message ?? String(e) });
    }
  },
  refreshWorkouts: async () => set({ workouts: await ipc.listWorkouts() }),
  refreshDevices: async () => set({ devices: await ipc.getDeviceState() }),
  refreshActivities: async () => set({ activities: await ipc.listActivities() }),
  refreshSettings: async () => set({ settings: await ipc.getSettings() }),
  loadPlayer: (player) => set((state) => ({
    player,
    rideTimeline: {
      ...createRideTimelineState(player),
      lastTrainerStatus: state.deviceStatus.trainer?.status ??
        state.rideTimeline.lastTrainerStatus,
    },
  })),
  recordUserAction: (label, expectedPhase = null) => set((state) => ({
    rideTimeline: recordUserAction(
      state.rideTimeline,
      state.player,
      label,
      expectedPhase,
    ),
  })),
  recordUserActionFailure: (message) => set((state) => ({
    rideTimeline: appendAppMessage(
      clearExpectedUserPhase(state.rideTimeline),
      state.player,
      message,
      "danger",
    ),
  })),
  appendRideMessage: (message, tone = "neutral") => set((state) => ({
    rideTimeline: appendAppMessage(state.rideTimeline, state.player, message, tone),
  })),
  clearRideTimeline: () => set((state) => ({
    rideTimeline: {
      ...createRideTimelineState(),
      lastTrainerStatus: state.deviceStatus.trainer?.status ?? null,
    },
  })),
  pushToast: (level, message) => {
    const id = ++toastSeq;
    set({ toasts: [...get().toasts, { id, level, message }] });
    setTimeout(
      () => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
      level === "error" ? 15000 : 6000,
    );
  },
  dismissToast: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));

let previewRun = 0;
/** Sequential lazy preview loading; survives tab unmounts (lives here, not
 * in the component). Backend caches by content hash, so hits are instant. */
async function loadPlannerPreviews(rows: PlannerWorkout[]): Promise<void> {
  const run = ++previewRun;
  for (const w of rows) {
    if (previewRun !== run) return;
    if (useStore.getState().plannerPreviews[w.wid]) continue;
    try {
      const p = await ipc.plannerPreview(w.wid);
      useStore.setState((s) => ({
        plannerPreviews: { ...s.plannerPreviews, [w.wid]: p },
      }));
    } catch {
      useStore.setState((s) => ({
        plannerPreviews: { ...s.plannerPreviews, [w.wid]: "error" },
      }));
    }
  }
}

/** Wire backend events into the store. Called once from main.tsx. */
export async function wireEvents(): Promise<void> {
  const s = useStore;

  // Hydrate the planner tab from the persistent cache — instant, offline-safe.
  void ipc
    .plannerCached()
    .then((r) => {
      if (r && s.getState().plannerRows.length === 0) {
        s.setState({
          plannerRows: r.rows,
          plannerStatus: "ready",
          plannerSyncedAt: r.fetched_at_ms,
          plannerFromCache: true,
        });
        void loadPlannerPreviews(r.rows);
      }
    })
    .catch(() => {});

  await listen<PlayerMeasurement>("player_measurement", (e) =>
    s.setState({ measurement: e.payload }),
  );

  await listen<PlayerState>("player_state", (e) => {
    if (e.payload.phase !== lastTracedPlayerPhase) {
      lastTracedPlayerPhase = e.payload.phase;
      traceEvent("player_phase", { phase: e.payload.phase });
    }
    s.setState((state) => ({
      player: e.payload,
      rideTimeline: observePlayerState(state.rideTimeline, e.payload),
    }));
  });

  await listen<ScanResult>("scan_result", (e) => {
    const cur = s.getState().scanResults;
    if (!cur.some((r) => r.platform_id === e.payload.platform_id && r.role === e.payload.role)) {
      s.setState({ scanResults: [...cur, e.payload] });
    }
  });

  await listen("scan_done", () => s.setState({ scanning: false }));

  await listen<DeviceStatusEvent>("device_status", (e) => {
    s.setState((state) => ({
      deviceStatus: { ...state.deviceStatus, [e.payload.role]: e.payload },
      rideTimeline: e.payload.role === "trainer"
        ? observeTrainerStatus(state.rideTimeline, state.player, e.payload.status)
        : state.rideTimeline,
    }));
    void s.getState().refreshDevices();
  });

  await listen<DeviceMeasurement>("device_measurement", (e) => {
    s.setState({
      deviceMeasurement: {
        ...s.getState().deviceMeasurement,
        [e.payload.role]: e.payload,
      },
    });
  });

  await listen<{ message: string; duration_s: number }>("text_event", (e) => {
    s.setState((state) => ({
      rideTimeline: appendAppMessage(
        state.rideTimeline,
        state.player,
        e.payload.message,
      ),
    }));
  });

  await listen<SegmentResult>("segment_result", (e) => {
    s.setState((state) => ({
      rideTimeline: appendSegmentResult(state.rideTimeline, e.payload),
    }));
  });

  await listen<ActivitySummary>("activity_recorded", (e) => {
    void ipc.clearRide();
    s.setState((state) => ({
      summary: e.payload,
      screen: "summary",
      player: null,
      rideTimeline: {
        ...createRideTimelineState(),
        lastTrainerStatus: state.deviceStatus.trainer?.status ?? null,
      },
    }));
    void s.getState().refreshActivities();
    void s.getState().refreshNextUp();
  });

  await listen<{ level: Toast["level"]; message: string }>("toast", (e) =>
    s.getState().pushToast(e.payload.level, e.payload.message),
  );
}

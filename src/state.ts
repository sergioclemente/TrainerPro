// Zustand store fed by Tauri events (SPEC.md §9: no polling from the UI).

import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import {
  DeviceReading,
  DeviceSlot,
  DeviceStatusEvent,
  PlannerPreview,
  PlannerWorkout,
  PlayerState,
  RideRow,
  RideSummary,
  Role,
  ScanResult,
  Settings,
  Telemetry,
  WorkoutSummary,
  ipc,
} from "./ipc";

export type Screen =
  | "library"
  | "libraries"
  | "devices"
  | "history"
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
  /** Which tab the Settings screen opens on (basic | export | libraries). */
  settingsTab: string;
  workouts: WorkoutSummary[];
  devices: DeviceSlot[];
  scanResults: ScanResult[];
  scanning: Role | null;
  deviceStatus: Record<Role, DeviceStatusEvent | null>;
  deviceReading: Record<Role, DeviceReading | null>;
  player: PlayerState | null;
  telemetry: Telemetry | null;
  textEvent: { message: string; duration_s: number } | null;
  summary: RideSummary | null;
  detail: WorkoutDetailView | null;
  rides: RideRow[];
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
  refreshWorkouts: () => Promise<void>;
  refreshDevices: () => Promise<void>;
  refreshRides: () => Promise<void>;
  refreshSettings: () => Promise<void>;
  pushToast: (level: Toast["level"], message: string) => void;
  dismissToast: (id: number) => void;
}

let toastSeq = 0;

export const useStore = create<Store>((set, get) => ({
  screen: "library",
  settingsTab: "basic",
  workouts: [],
  devices: [],
  scanResults: [],
  scanning: null,
  deviceStatus: { trainer: null, hrm: null },
  deviceReading: { trainer: null, hrm: null },
  player: null,
  telemetry: null,
  textEvent: null,
  summary: null,
  detail: null,
  rides: [],
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
  refreshWorkouts: async () => set({ workouts: await ipc.listWorkouts() }),
  refreshDevices: async () => set({ devices: await ipc.getDeviceState() }),
  refreshRides: async () => set({ rides: await ipc.listRides() }),
  refreshSettings: async () => set({ settings: await ipc.getSettings() }),
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

  await listen<Telemetry>("telemetry", (e) => s.setState({ telemetry: e.payload }));

  await listen<PlayerState>("player_state", (e) => s.setState({ player: e.payload }));

  await listen<ScanResult>("scan_result", (e) => {
    const cur = s.getState().scanResults;
    if (!cur.some((r) => r.platform_id === e.payload.platform_id)) {
      s.setState({ scanResults: [...cur, e.payload] });
    }
  });

  await listen<{ role: Role }>("scan_done", (e) => {
    if (s.getState().scanning === e.payload.role) s.setState({ scanning: null });
  });

  await listen<DeviceStatusEvent>("device_status", (e) => {
    s.setState({
      deviceStatus: { ...s.getState().deviceStatus, [e.payload.role]: e.payload },
    });
    void s.getState().refreshDevices();
  });

  await listen<DeviceReading>("device_reading", (e) => {
    s.setState({
      deviceReading: { ...s.getState().deviceReading, [e.payload.role]: e.payload },
    });
  });

  await listen<{ message: string; duration_s: number }>("text_event", (e) => {
    s.setState({ textEvent: e.payload });
    setTimeout(() => {
      if (s.getState().textEvent === e.payload) s.setState({ textEvent: null });
    }, e.payload.duration_s * 1000);
  });

  await listen<RideSummary>("ride_finished", (e) => {
    void ipc.clearRide();
    s.setState({ summary: e.payload, screen: "summary", player: null });
    void s.getState().refreshRides();
  });

  await listen<{ level: Toast["level"]; message: string }>("toast", (e) =>
    s.getState().pushToast(e.payload.level, e.payload.message),
  );
}

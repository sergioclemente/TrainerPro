export type VoicePermission = "unknown" | "granted" | "denied";

export type VoicePhase =
  | "off"
  | "permission-required"
  | "preparing"
  | "idle"
  | "starting"
  | "finalizing"
  | "speech"
  | "interpreting"
  | "executing"
  | "suspended"
  | "unavailable"
  | "error";

export interface VoiceMachineState {
  phase: VoicePhase;
  enabled: boolean;
  permission: VoicePermission;
  surfaceKey: string | null;
  appActive: boolean;
  /** Automatic input recovery is limited until the voice context changes or Retry is used. */
  captureRestartAttempts: number;
  /** Invalidates asynchronous results created against an older context. */
  generation: number;
  error: string | null;
}

export type VoiceMachineEvent =
  | { type: "enabled-changed"; enabled: boolean }
  | { type: "permission-changed"; permission: VoicePermission }
  | { type: "surface-changed"; key: string | null }
  | { type: "app-activity-changed"; active: boolean }
  | { type: "prepared"; generation: number }
  | { type: "ptt-pressed" }
  | { type: "ptt-released" }
  | { type: "ptt-canceled" }
  | { type: "speech-started" }
  | { type: "utterance-ignored" }
  | { type: "utterance-accepted" }
  | { type: "interpretation-rejected"; generation: number }
  | { type: "model-result"; generation: number }
  | { type: "command-finished"; generation: number }
  | { type: "capture-restart-requested"; generation: number }
  | { type: "semantic-router-restart-requested"; generation: number }
  | { type: "failed"; generation: number; message: string }
  | { type: "retry" };

export interface VoiceMachineOptions {
  enabled: boolean;
  permission?: VoicePermission;
  surfaceKey?: string | null;
  appActive?: boolean;
}

function activationPhase(state: VoiceMachineState): VoicePhase {
  if (!state.enabled) return "off";
  if (!state.surfaceKey || !state.appActive) return "suspended";
  if (state.permission === "unknown") return "permission-required";
  if (state.permission === "denied") return "unavailable";
  return "preparing";
}

export function createVoiceMachine(options: VoiceMachineOptions): VoiceMachineState {
  const state: VoiceMachineState = {
    phase: "off",
    enabled: options.enabled,
    permission: options.permission ?? "unknown",
    surfaceKey: options.surfaceKey ?? null,
    appActive: options.appActive ?? true,
    captureRestartAttempts: 0,
    generation: 0,
    error: null,
  };
  return { ...state, phase: activationPhase(state) };
}

function contextChanged(
  state: VoiceMachineState,
  patch: Partial<Pick<VoiceMachineState, "enabled" | "permission" | "surfaceKey" | "appActive">>,
): VoiceMachineState {
  const next = { ...state, ...patch };
  return {
    ...next,
    phase: activationPhase(next),
    captureRestartAttempts: 0,
    generation: state.generation + 1,
    error: null,
  };
}

function hasCurrentGeneration(state: VoiceMachineState, generation: number): boolean {
  return state.generation === generation;
}

export function reduceVoiceMachine(
  state: VoiceMachineState,
  event: VoiceMachineEvent,
): VoiceMachineState {
  switch (event.type) {
    case "enabled-changed":
      if (event.enabled === state.enabled) return state;
      return contextChanged(state, {
        enabled: event.enabled,
        permission: event.enabled ? "unknown" : state.permission,
      });
    case "permission-changed":
      return event.permission === state.permission
        ? state
        : contextChanged(state, { permission: event.permission });
    case "surface-changed":
      return event.key === state.surfaceKey
        ? state
        : contextChanged(state, { surfaceKey: event.key });
    case "app-activity-changed":
      return event.active === state.appActive
        ? state
        : contextChanged(state, { appActive: event.active });
    case "prepared":
      return state.phase === "preparing" && hasCurrentGeneration(state, event.generation)
        ? { ...state, phase: "idle" }
        : state;
    case "ptt-pressed":
      return state.phase === "idle" ? { ...state, phase: "starting", generation: state.generation + 1 } : state;
    case "ptt-released":
      return state.phase === "speech" ? { ...state, phase: "finalizing" } : state;
    case "ptt-canceled":
      return ["starting", "speech", "finalizing", "interpreting", "executing"].includes(state.phase)
        ? { ...state, phase: "idle", generation: state.generation + 1 } : state;
    case "speech-started":
      return state.phase === "starting" ? { ...state, phase: "speech" } : state;
    case "utterance-ignored":
      return (state.phase === "speech" || state.phase === "finalizing") ? { ...state, phase: "idle" } : state;
    case "utterance-accepted":
      return state.phase === "finalizing" ? { ...state, phase: "interpreting" } : state;
    case "interpretation-rejected":
      return state.phase === "interpreting" && hasCurrentGeneration(state, event.generation)
        ? { ...state, phase: "idle" }
        : state;
    case "model-result":
      if (state.phase !== "interpreting" || !hasCurrentGeneration(state, event.generation)) {
        return state;
      }
      return { ...state, phase: "executing" };
    case "command-finished":
      return state.phase === "executing" && hasCurrentGeneration(state, event.generation)
        ? { ...state, phase: "idle" }
        : state;
    case "capture-restart-requested":
      if (!hasCurrentGeneration(state, event.generation) ||
          state.phase === "off" || state.phase === "suspended" ||
          state.phase === "permission-required" || state.phase === "unavailable" ||
          state.phase === "error") {
        return state;
      }
      return {
        ...state,
        phase: "preparing",
        captureRestartAttempts: state.captureRestartAttempts + 1,
        generation: state.generation + 1,
        error: null,
      };
    case "semantic-router-restart-requested":
      return state.phase === "interpreting" && hasCurrentGeneration(state, event.generation)
        ? {
            ...state,
            phase: "preparing",
            generation: state.generation + 1,
            error: null,
          }
        : state;
    case "failed":
      return hasCurrentGeneration(state, event.generation)
        ? { ...state, phase: "error", generation: state.generation + 1, error: event.message }
        : state;
    case "retry": {
      if (state.phase !== "error" && state.phase !== "unavailable") return state;
      const next = {
        ...state,
        permission: state.phase === "unavailable" ? "unknown" as const : state.permission,
        captureRestartAttempts: 0,
        generation: state.generation + 1,
        error: null,
      };
      return { ...next, phase: activationPhase(next) };
    }
  }
}

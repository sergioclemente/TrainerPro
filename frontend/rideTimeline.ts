import type { DeviceStatusEvent, PlayerState, SegmentResult } from "./ipc";

const USER_ACTION_CORRELATION_WINDOW_MS = 2_000;

export type RideTimelineTone = "neutral" | "warn" | "danger";

export type RideTimelineItem =
  | {
      id: number;
      kind: "user-action";
      label: string;
    }
  | {
      id: number;
      kind: "app-message";
      message: string;
      tone: RideTimelineTone;
    }
  | {
      id: number;
      kind: "segment";
      result: SegmentResult;
    };

type WithoutId<T> = T extends unknown ? Omit<T, "id"> : never;
type RideTimelineItemInput = WithoutId<RideTimelineItem>;

interface ExpectedUserPhase {
  phase: PlayerState["phase"];
  expiresAtMs: number;
}

export interface RideTimelineState {
  sessionId: string | null;
  items: RideTimelineItem[];
  nextId: number;
  lastPhase: PlayerState["phase"] | null;
  expectedUserPhase: ExpectedUserPhase | null;
  lastTrainerStatus: DeviceStatusEvent["status"] | null;
  trainerLossActive: boolean;
}

export function createRideTimelineState(
  player: PlayerState | null = null,
): RideTimelineState {
  return {
    sessionId: player?.workout_session_id ?? null,
    items: [],
    nextId: 1,
    lastPhase: player?.phase ?? null,
    expectedUserPhase: null,
    lastTrainerStatus: null,
    trainerLossActive: false,
  };
}

function forSession(
  state: RideTimelineState,
  player: PlayerState | null,
): RideTimelineState {
  if (!player) return state;
  if (state.sessionId === player.workout_session_id) return state;
  return {
    ...createRideTimelineState(player),
    lastTrainerStatus: state.lastTrainerStatus,
  };
}

function append(
  state: RideTimelineState,
  item: RideTimelineItemInput,
): RideTimelineState {
  return {
    ...state,
    items: [...state.items, { ...item, id: state.nextId } as RideTimelineItem],
    nextId: state.nextId + 1,
  };
}

function phaseMessage(
  previous: PlayerState["phase"] | null,
  next: PlayerState["phase"],
  trainerLossActive: boolean,
): string | null {
  if (previous === "ready" && next === "riding") return "Ride started";
  if (previous === "riding" && next === "paused") {
    return trainerLossActive ? "Workout auto-paused" : "Workout paused";
  }
  if (previous === "paused" && next === "riding") return "Workout resumed";
  if (next === "finished" && previous !== "finished") return "Workout complete";
  return null;
}

export function observePlayerState(
  state: RideTimelineState,
  player: PlayerState,
  nowMs = Date.now(),
): RideTimelineState {
  let next = forSession(state, player);
  if (next.lastPhase === player.phase) return next;

  const expected = next.expectedUserPhase;
  const suppress = expected?.phase === player.phase && expected.expiresAtMs >= nowMs;
  const message = phaseMessage(next.lastPhase, player.phase, next.trainerLossActive);
  next = {
    ...next,
    lastPhase: player.phase,
    expectedUserPhase: null,
  };
  return message && !suppress
    ? append(next, { kind: "app-message", message, tone: "neutral" })
    : next;
}

export function recordUserAction(
  state: RideTimelineState,
  player: PlayerState | null,
  label: string,
  expectedPhase: PlayerState["phase"] | null,
  nowMs = Date.now(),
): RideTimelineState {
  let next = forSession(state, player);
  next = append(next, { kind: "user-action", label });
  return expectedPhase
    ? {
        ...next,
        expectedUserPhase: {
          phase: expectedPhase,
          expiresAtMs: nowMs + USER_ACTION_CORRELATION_WINDOW_MS,
        },
      }
    : next;
}

export function clearExpectedUserPhase(state: RideTimelineState): RideTimelineState {
  return state.expectedUserPhase ? { ...state, expectedUserPhase: null } : state;
}

export function appendAppMessage(
  state: RideTimelineState,
  player: PlayerState | null,
  message: string,
  tone: RideTimelineTone = "neutral",
): RideTimelineState {
  const next = forSession(state, player);
  return player ? append(next, { kind: "app-message", message, tone }) : next;
}

export function appendSegmentResult(
  state: RideTimelineState,
  result: SegmentResult,
): RideTimelineState {
  if (state.sessionId !== result.workout_session_id) return state;
  return append(state, { kind: "segment", result });
}

export function observeTrainerStatus(
  state: RideTimelineState,
  player: PlayerState | null,
  status: DeviceStatusEvent["status"],
): RideTimelineState {
  let next = forSession(state, player);
  if (!player || player.phase === "ready" || player.phase === "finished") {
    return { ...next, lastTrainerStatus: status };
  }
  if (next.lastTrainerStatus === null) return { ...next, lastTrainerStatus: status };

  const lost = next.lastTrainerStatus === "connected" && status !== "connected";
  const recovered = next.trainerLossActive && status === "connected";
  next = {
    ...next,
    lastTrainerStatus: status,
    trainerLossActive: lost ? true : recovered ? false : next.trainerLossActive,
  };
  if (lost) {
    return append(next, {
      kind: "app-message",
      message: "Trainer disconnected",
      tone: "warn",
    });
  }
  if (recovered) {
    return append(next, {
      kind: "app-message",
      message: "Trainer reconnected",
      tone: "neutral",
    });
  }
  return next;
}

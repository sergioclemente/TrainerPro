import { type AppError, type PlayerState, ipc } from "../ipc";
import { useStore } from "../state";

export interface PlayerActionCommands {
  startRide: () => Promise<void>;
  pauseRide: () => Promise<void>;
  resumeRide: () => Promise<void>;
  skipSegment: () => Promise<void>;
  setIntensity: (intensity: number) => Promise<void>;
  setErg: (enabled: boolean) => Promise<void>;
}

export const INTENSITY_PERCENT_MIN = 50;
export const INTENSITY_PERCENT_MAX = 150;
export const PERCENT_SCALE = 100;

function errorMessage(error: unknown): string {
  return (error as AppError)?.message ?? (error instanceof Error ? error.message : String(error));
}

async function runPlayerAction(
  label: string,
  expectedPhase: PlayerState["phase"] | null,
  execute: () => Promise<void>,
): Promise<void> {
  const store = useStore.getState();
  store.recordUserAction(label, expectedPhase);
  try {
    await execute();
  } catch (error) {
    const message = errorMessage(error);
    store.recordUserActionFailure(`Command failed: ${message}`);
    store.pushToast("error", message);
    throw error;
  }
}

function boundedIntensity(intensity: number): number {
  return Math.min(
    INTENSITY_PERCENT_MAX / PERCENT_SCALE,
    Math.max(INTENSITY_PERCENT_MIN / PERCENT_SCALE, intensity),
  );
}

export const playerActions: PlayerActionCommands = {
  startRide: () => runPlayerAction("Start workout", "riding", ipc.startRide),
  pauseRide: () => runPlayerAction("Pause workout", "paused", ipc.pauseRide),
  resumeRide: () => runPlayerAction("Resume workout", "riding", ipc.resumeRide),
  skipSegment: () => runPlayerAction("Skip interval", null, ipc.skipSegment),
  setIntensity: (intensity) => {
    const bounded = boundedIntensity(intensity);
    const percent = Math.round(bounded * PERCENT_SCALE);
    return runPlayerAction(
      `Set intensity to ${percent}%`,
      null,
      () => ipc.setIntensity(bounded),
    );
  },
  setErg: (enabled) => runPlayerAction(
    enabled ? "Enable ERG" : "Disable ERG",
    null,
    () => ipc.setErg(enabled),
  ),
};

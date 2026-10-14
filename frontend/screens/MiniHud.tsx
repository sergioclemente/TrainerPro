// Mini HUD: the 400×100 always-on-top strip rendered in its own window.
// Five cells (watts, target, rpm, bpm, interval left) and a control bar that
// slides up on hover and pins while paused, reconnecting, or done. It reads
// the same store the Player does — the backend emits every event to both
// windows — and sends the same player commands.

import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc, type AppError, type ControllerInputEvent, type WorkoutSegmentRow } from "../ipc";
import { useStore } from "../state";
import { createControllerInputHandler } from "../voice/controllerInput";
import { ExpandIcon } from "../components/MetricIcons";
import { hudView, type HudCell } from "../miniHud";
import { playerActions } from "./Player.actions";

/** How long an action error stays in the bar slot. */
const ERROR_DISPLAY_MS = 6_000;
/** The interval clock's direction sticks across rides, like the Player's. */
const INTERVAL_ELAPSED_KEY = "trainerpro.hud.intervalElapsed";

interface Cue {
  message: string;
  until: number;
}

function settle(action: Promise<void>, onError: (message: string) => void): void {
  void action.catch((error: unknown) => {
    onError((error as AppError)?.message ?? String(error));
  });
}

function Cell({ cell, last, onClick }: { cell: HudCell; last: boolean; onClick?: () => void }) {
  return (
    <div
      className={`hud-cell ${last ? "hud-cell-last" : ""} ${cell.dimmed ? "hud-dimmed" : ""} ${onClick ? "hud-cell-clickable" : ""}`}
      onClick={onClick}
      title={onClick ? "Click to switch between remaining and elapsed" : undefined}
      data-tauri-drag-region
    >
      <span
        className={`hud-value ${cell.struck ? "hud-struck" : ""}`}
        style={{ color: cell.color }}
        data-tauri-drag-region
      >
        {cell.value}
      </span>
      <span className="hud-label" style={{ color: cell.labelColor }} data-tauri-drag-region>
        {cell.fault && <span className="hud-fault-dot" aria-hidden="true" />}
        {cell.label}
      </span>
    </div>
  );
}

export default function MiniHud() {
  const { player, measurement, deviceStatus, summary, loadPlayer } = useStore();
  const [segments, setSegments] = useState<WorkoutSegmentRow[]>([]);
  const [ftpW, setFtpW] = useState<number | null>(null);
  const [cue, setCue] = useState<Cue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showIntervalElapsed, setShowIntervalElapsed] = useState(
    () => localStorage.getItem(INTERVAL_ELAPSED_KEY) === "1",
  );
  const [, setTick] = useState(0);
  const errorTimer = useRef<number | null>(null);

  // The window opens mid-ride, so hydrate what the events already delivered.
  useEffect(() => {
    void ipc.getPlayerState().then((state) => {
      if (state) loadPlayer(state);
    }).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const workoutSessionId = player?.workout_session_id;
  useEffect(() => {
    if (!workoutSessionId) return;
    let live = true;
    void ipc.getPlayerWorkoutProfile(workoutSessionId).then((profile) => {
      if (!live || !profile) return;
      setSegments(profile.segments);
      setFtpW(profile.ftp_w);
    }).catch(() => undefined);
    return () => {
      live = false;
    };
  }, [workoutSessionId]);

  // Workout text cues borrow the bar slot for their display time.
  useEffect(() => {
    let disposed = false;
    let timer: number | null = null;
    const subscription = listen<{ message: string; duration_s: number }>("text_event", (e) => {
      if (disposed) return;
      const until = Date.now() + e.payload.duration_s * 1000;
      setCue({ message: e.payload.message, until });
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => setTick((n) => n + 1), e.payload.duration_s * 1000);
    });
    return () => {
      disposed = true;
      if (timer !== null) window.clearTimeout(timer);
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

  // The main window is hidden while the HUD is up, so its controller
  // handler ignores input; pause/resume lands here instead. Push-to-talk
  // needs the main window's voice capture and is not available in mini mode.
  useEffect(() => {
    let disposed = false;
    const noop = () => undefined;
    const input = createControllerInputHandler({
      begin: noop,
      finish: noop,
      cancel: noop,
      visible: () => document.visibilityState === "visible",
      pauseResume: () => {
        const current = useStore.getState().player;
        if (current?.phase === "riding") act(playerActions.pauseRide());
        else if (current?.phase === "paused") act(playerActions.resumeRide());
      },
    });
    const subscription = listen<ControllerInputEvent>("controller_input", ({ payload }) => {
      if (!disposed) input(payload);
    });
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    return () => {
      if (errorTimer.current !== null) window.clearTimeout(errorTimer.current);
    };
  }, []);

  function showError(message: string) {
    setError(message);
    if (errorTimer.current !== null) window.clearTimeout(errorTimer.current);
    errorTimer.current = window.setTimeout(() => setError(null), ERROR_DISPLAY_MS);
  }

  function act(action: Promise<void>) {
    settle(action, showError);
  }

  function backToPlayer() {
    void ipc.closeMiniHud().catch(() => undefined);
  }

  function hideHud() {
    void ipc.hideMiniHud().catch(() => undefined);
  }

  function toggleIntervalClock() {
    setShowIntervalElapsed((v) => {
      localStorage.setItem(INTERVAL_ELAPSED_KEY, v ? "0" : "1");
      return !v;
    });
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "m") backToPlayer();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const segment = player?.seg_idx != null ? (segments[player.seg_idx] ?? null) : null;
  const view = hudView({
    player,
    measurement,
    ftpW,
    segmentDurationS: segment?.duration_s ?? null,
    trainer: deviceStatus.trainer,
    hrm: deviceStatus.hrm,
    summary,
    cue: cue !== null && cue.until > Date.now() ? cue.message : null,
    error,
    showIntervalElapsed,
  });
  const intervalCellIndex = view.cells.length - 1;

  return (
    <div
      className={`hud hud-${view.mode} ${view.barPinned ? "hud-bar-pinned" : ""}`}
      style={{ borderColor: view.accent }}
      onDoubleClick={backToPlayer}
      data-tauri-drag-region
    >
      <div className="hud-cells" data-tauri-drag-region>
        {view.cells.map((cell, i) => (
          <Cell
            key={i}
            cell={cell}
            last={i === intervalCellIndex}
            onClick={i === intervalCellIndex && view.mode !== "done" ? toggleIntervalClock : undefined}
          />
        ))}
      </div>

      <div className="hud-bar" onDoubleClick={(e) => e.stopPropagation()}>
        <button className="hud-btn" type="button" aria-label="Back to full Player" onClick={backToPlayer}>
          <ExpandIcon />
        </button>
        {view.status !== null && view.mode !== "done" ? (
          <span className="hud-status" role="status" style={{ color: view.status.color }}>
            {view.status.text}
          </span>
        ) : view.mode === "done" ? (
          <>
            <span className="hud-status" style={{ color: view.status?.color }}>
              {view.status?.text}
            </span>
            <button className="hud-btn hud-primary hud-primary-done" type="button" onClick={backToPlayer}>
              Open summary
            </button>
          </>
        ) : (
          <>
            {view.paused ? (
              <button
                className="hud-btn hud-primary"
                type="button"
                aria-label="Resume ride"
                onClick={() => act(playerActions.resumeRide())}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                  <path d="M8 4.5v15l12-7.5z" />
                </svg>
                <span>resume</span>
              </button>
            ) : (
              <button
                className="hud-btn"
                type="button"
                aria-label={view.mode === "ready" ? "Start ride" : "Pause ride"}
                onClick={() =>
                  act(view.mode === "ready" ? playerActions.startRide() : playerActions.pauseRide())
                }
              >
                {view.mode === "ready" ? (
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <path d="M7 4.5v15l12-7.5z" />
                  </svg>
                ) : (
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <rect x="6" y="4.5" width="4" height="15" rx="1" />
                    <rect x="14" y="4.5" width="4" height="15" rx="1" />
                  </svg>
                )}
              </button>
            )}
            <button
              className="hud-btn"
              type="button"
              aria-label="Skip to next interval"
              disabled={view.mode === "ready"}
              onClick={() => act(playerActions.skipSegment())}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                <path d="M5 4.5v15l10-7.5z" />
                <rect x="17" y="4.5" width="3" height="15" rx="1" />
              </svg>
            </button>
            <span className="hud-intensity">
              <button
                className="hud-btn"
                type="button"
                aria-label="Decrease intensity"
                disabled={!view.ergEnabled || player === null}
                onClick={() => player && act(playerActions.setIntensity(player.intensity - 0.01))}
              >
                −
              </button>
              <span
                className="hud-intensity-value"
                style={{ color: view.intensityPct !== 100 ? "#d29922" : undefined }}
              >
                {view.intensityPct}%
              </span>
              <button
                className="hud-btn"
                type="button"
                aria-label="Increase intensity"
                disabled={!view.ergEnabled || player === null}
                onClick={() => player && act(playerActions.setIntensity(player.intensity + 0.01))}
              >
                +
              </button>
            </span>
            <button
              className={`hud-btn hud-erg ${view.ergEnabled ? "on" : "off"}`}
              type="button"
              aria-pressed={view.ergEnabled}
              aria-label={view.ergEnabled ? "ERG mode on" : "ERG mode off"}
              onClick={() => act(playerActions.setErg(!view.ergEnabled))}
            >
              ERG
            </button>
            <button
              className="hud-btn"
              type="button"
              aria-label="Hide HUD; the Dock icon brings TrainerPro back"
              title="Hide HUD (Dock icon brings TrainerPro back)"
              onClick={hideHud}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M5 9l7 7 7-7" />
              </svg>
            </button>
          </>
        )}
      </div>

      <div className="hud-progress" aria-hidden="true">
        <div
          className={`hud-progress-fill ${view.progress.dashed ? "dashed" : ""}`}
          style={{
            width: `${Math.round(view.progress.fraction * 100)}%`,
            background: view.progress.dashed
              ? `repeating-linear-gradient(90deg, ${view.progress.color} 0 6px, transparent 6px 10px)`
              : view.progress.color,
          }}
        />
      </div>
    </div>
  );
}

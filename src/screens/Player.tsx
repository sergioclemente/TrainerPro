import { useCallback, useEffect, useRef, useState } from "react";
import { AppError, SegmentRow, fmtDuration, ipc } from "../ipc";
import { useStore } from "../state";
import WorkoutGraph from "../components/WorkoutGraph";
import { BoltIcon, CadenceIcon, HeartIcon } from "../components/MetricIcons";
import {
  COUNTDOWN_LEAD_S,
  isMuted,
  playCountdown,
  playEnd,
  primeSounds,
  setMuted,
} from "../sounds";

const STATS_KEY = "trainerpro.player.stats";

/** One cell of the stats strip. A metric with no data yet reads as a dash. */
function Stat({
  value,
  unit,
  label,
  digits = 0,
}: {
  value: number | null;
  unit?: string;
  label: string;
  digits?: number;
}) {
  return (
    <div className="stat">
      <span className="stat-value">
        {value === null ? "–" : value.toFixed(digits)}
        {value !== null && unit && <span className="stat-unit">{unit}</span>}
      </span>
      <span className="stat-label">{label}</span>
    </div>
  );
}

export default function Player() {
  const { player, telemetry, textEvent, workouts, settings, go, pushToast } = useStore();
  /** Both clocks show elapsed by default; clicking one toggles it to remaining. */
  const [showRemaining, setShowRemaining] = useState(false);
  const [showIntervalRemaining, setShowIntervalRemaining] = useState(false);
  const [muted, setMutedState] = useState(isMuted);
  /** Detail stats are opt-in and the choice sticks across rides. */
  const [showStats, setShowStats] = useState(
    () => localStorage.getItem(STATS_KEY) === "1",
  );
  /** Segment rows power the graph tooltip and the interval clock's elapsed side. */
  const [segments, setSegments] = useState<SegmentRow[]>([]);

  const toggleStats = useCallback(() => {
    setShowStats((v) => {
      localStorage.setItem(STATS_KEY, v ? "0" : "1");
      return !v;
    });
  }, []);

  const workoutId = player?.workout_id;
  useEffect(() => {
    if (!workoutId) return;
    let live = true;
    ipc
      .getWorkoutDetail(workoutId)
      .then((d) => live && setSegments(d.segments))
      .catch(() => live && setSegments([])); // graph and clocks degrade gracefully
    return () => {
      live = false;
    };
  }, [workoutId]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!player) return;
      if (e.code === "Space") {
        e.preventDefault();
        if (player.phase === "riding") void ipc.pauseRide();
        else if (player.phase === "paused") {
          primeSounds(); // inside the keydown gesture — see sounds.ts
          void ipc.resumeRide();
        } else if (player.phase === "ready") {
          primeSounds();
          void ipc.startRide();
        }
      } else if (e.key === "s") {
        void ipc.skipSegment();
      } else if (e.key === "e") {
        void ipc.setErg(!player.erg_enabled);
      } else if (e.key === "d") {
        toggleStats();
      } else if (e.key === "ArrowUp") {
        void ipc.setIntensity(player.intensity + 0.01);
      } else if (e.key === "ArrowDown") {
        void ipc.setIntensity(player.intensity - 0.01);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [player, toggleStats]);

  // Countdown into each segment boundary. State arrives about once a second,
  // so arm a timer as soon as the boundary is within a tick of the lead-in and
  // let it land on the beat; seg_idx keeps it to one cue per segment.
  const cueTimer = useRef<number | null>(null);
  const cuedSeg = useRef<number | null>(null);
  const chimed = useRef(false);
  useEffect(() => {
    return () => {
      if (cueTimer.current !== null) window.clearTimeout(cueTimer.current);
    };
  }, []);
  useEffect(() => {
    if (!player) return;
    const { phase, seg_idx, seg_remaining_s, elapsed_s, workout_duration_s } = player;
    if (phase !== "riding") {
      // Pausing drops the pending cue; resuming re-arms it from the new state.
      if (cueTimer.current !== null) {
        window.clearTimeout(cueTimer.current);
        cueTimer.current = null;
      }
      cuedSeg.current = null;
      if (phase === "finished" && !chimed.current) {
        chimed.current = true;
        playEnd();
      }
      return;
    }
    chimed.current = false;
    if (seg_idx === null || seg_idx === cuedSeg.current) return;
    const lead_ms = (seg_remaining_s - COUNTDOWN_LEAD_S) * 1000;
    if (lead_ms > 1000) return; // boundary still more than a tick away
    cuedSeg.current = seg_idx; // mark handled so we don't re-check every tick
    // The final segment's boundary is the workout END — that gets end.wav (the
    // `finished` branch above), never a next-interval countdown. Don't arm it.
    if (elapsed_s + seg_remaining_s >= workout_duration_s - 0.5) return;
    cueTimer.current = window.setTimeout(() => {
      cueTimer.current = null;
      playCountdown();
    }, Math.max(0, lead_ms));
  }, [player]);

  if (!player) {
    return (
      <div className="screen">
        <p className="empty">No workout loaded.</p>
        <button onClick={() => go("library")}>Back to library</button>
      </div>
    );
  }

  const workout = workouts.find((w) => w.id === player.workout_id);
  const power = telemetry?.power_smoothed_3s ?? telemetry?.power ?? null;
  const target = player.target;
  const weight = settings?.profile.weight_kg ?? null;
  const wkg =
    power !== null && weight !== null && weight > 0 ? (power / weight).toFixed(1) : null;
  const powerClass =
    power !== null && target !== null && target > 0
      ? Math.abs(power - target) / target <= 0.05
        ? "on-target"
        : "off-target"
      : "";
  const remaining_s = Math.max(0, player.workout_duration_s - player.elapsed_s);
  // Interval elapsed needs the segment's length, which only the detail rows
  // carry; without them the interval clock stays on remaining.
  const segment = player.seg_idx !== null ? (segments[player.seg_idx] ?? null) : null;
  const segElapsed_s = segment
    ? Math.max(0, segment.duration_s - player.seg_remaining_s)
    : null;
  const showSegRemaining = showIntervalRemaining || segElapsed_s === null;
  // Live EF, the same shape as the W/kg beside it: what the ratio is right now,
  // off the displayed (3 s smoothed) watts. The session figure — NP over average
  // HR — is the one in the stats strip.
  const hr = telemetry?.hr ?? null;
  const liveEf = power !== null && hr !== null && hr > 0 ? (power / hr).toFixed(2) : null;
  /** The interval's prescribed cadence, when the ZWO named one. */
  const targetCadence = segment?.cadence_rpm ?? null;
  const targetLabel = !player.erg_enabled
    ? "ERG off"
    : player.phase === "ready"
      ? "ready to start"
      : target !== null
        ? `target ${target} W`
        : "free ride";

  async function endRide() {
    if (
      player!.phase !== "finished" &&
      !confirm("End the ride now? The completed portion will be saved.")
    )
      return;
    try {
      const summary = await ipc.endRide();
      useStore.setState({ summary, screen: "summary", player: null, telemetry: null });
      void useStore.getState().refreshRides();
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  async function toggleErg() {
    try {
      await ipc.setErg(!player!.erg_enabled);
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  return (
    <div className="player">
      <h1 className="player-title">{player.workout_name}</h1>

      <div className="player-graph">
        {workout && (
          <WorkoutGraph
            graph={workout.graph}
            durationS={player.workout_duration_s}
            progressS={player.elapsed_s}
            height={140}
            segments={segments}
            ftp={settings?.profile.ftp}
            activeIndex={player.seg_idx}
          />
        )}
        {textEvent && <div className="textevent">{textEvent.message}</div>}
      </div>

      <div className="player-metrics">
        <div className={`metric ${powerClass}`}>
          <span className="metric-value">{power ?? "–"}</span>
          <span className={`metric-target ${player.erg_enabled ? "" : "erg-off"}`}>
            {targetLabel}
            {player.intensity !== 1.0 && (
              <span className="intensity-badge">
                {" "}· {Math.round(player.intensity * 100)}% intensity
              </span>
            )}
            {wkg !== null && <span className="metric-wkg"> · {wkg} W/kg</span>}
          </span>
          <span className="metric-label">
            <BoltIcon /> watts
          </span>
        </div>
        <div className="metric">
          <span className="metric-value">{telemetry?.cadence ?? "–"}</span>
          <span className="metric-sub">
            {targetCadence !== null ? `target ${targetCadence} rpm` : ""}
          </span>
          <span className="metric-label">
            <CadenceIcon /> rpm
          </span>
        </div>
        <div className="metric">
          <span className="metric-value">{hr ?? "–"}</span>
          {/* Efficiency factor rides under the heart rate for the same reason
              W/kg rides under the watts: it is that number, per body. */}
          <span className="metric-sub">{liveEf !== null ? `${liveEf} EF` : ""}</span>
          <span className="metric-label">
            <HeartIcon /> bpm
          </span>
        </div>
      </div>

      <div className="player-clocks">
        <button
          className="clock clock-toggle"
          title="Click to switch between elapsed and remaining"
          onClick={() => setShowRemaining((v) => !v)}
        >
          <div className="countdown">
            {fmtDuration(showRemaining ? remaining_s : player.ride_s)}
          </div>
          <div className="clock-label">
            {showRemaining ? "total remaining" : "total elapsed"}
          </div>
        </button>

        <button
          className="clock clock-toggle"
          title="Click to switch between elapsed and remaining"
          onClick={() => setShowIntervalRemaining((v) => !v)}
          disabled={segElapsed_s === null}
        >
          <div className="countdown">
            {fmtDuration(showSegRemaining ? player.seg_remaining_s : segElapsed_s!)}
          </div>
          <div className="clock-label">
            {showSegRemaining ? "interval remaining" : "interval elapsed"}
          </div>
        </button>
      </div>

      {showStats && (
        <div className="stats-strip">
          <Stat value={player.avg_power} unit="W" label="avg power" />
          <Stat value={player.np} unit="W" label="NP" />
          <Stat value={player.tss} label="TSS" digits={0} />
          {/* Named apart from the live EF under the heart rate — same units,
              different question: NP over average HR for the ride so far. */}
          <Stat value={player.ef} label="session EF" digits={2} />
          <Stat value={player.kcal} unit="kcal" label="calories" />
        </div>
      )}

      <div className="player-controls">
        {player.phase === "ready" && (
          <button
            className="go big"
            onClick={() => {
              primeSounds(); // inside the click gesture — see sounds.ts
              void ipc.startRide();
            }}
          >
            ▶ Start
          </button>
        )}
        {player.phase === "riding" && (
          <button className="big" onClick={() => ipc.pauseRide()}>
            ❚❚ Pause
          </button>
        )}
        {player.phase === "paused" && (
          <button
            className="go big"
            onClick={() => {
              primeSounds();
              void ipc.resumeRide();
            }}
          >
            ▶ Resume
          </button>
        )}
        <button onClick={() => ipc.skipSegment()} disabled={player.phase === "ready"}>
          Skip ⏭
        </button>
        <button onClick={() => ipc.setIntensity(player.intensity - 0.01)}>−1%</button>
        <button onClick={() => ipc.setIntensity(player.intensity + 0.01)}>+1%</button>
        <button
          className="stats-toggle"
          onClick={toggleStats}
          title="Show average power, NP, TSS, EF and calories for the ride so far"
        >
          Stats
        </button>
        <button
          className={`sound-toggle ${muted ? "off" : "on"}`}
          onClick={() => {
            setMuted(!muted);
            setMutedState(!muted);
          }}
          title="Interval countdown and end-of-workout sounds"
        >
          {muted ? "🔇" : "🔊"}
        </button>
        <button
          className={`erg-toggle ${player.erg_enabled ? "on" : "off"}`}
          onClick={toggleErg}
          title="ERG mode holds the target watts; off leaves the trainer in free resistance"
        >
          <span className="erg-dot" aria-hidden="true" />
          ERG{player.erg_enabled ? "" : " off"}
        </button>
        <button className="danger" onClick={endRide}>
          End ride
        </button>
      </div>
      <p className="muted footnote">
        space pause/resume · s skip · e ERG · d stats · ↑/↓ intensity
      </p>
    </div>
  );
}

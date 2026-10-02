import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { PlayerState, SegmentResult, SegmentRow } from "../ipc";
import { fmtDuration } from "../ipc";
import type { RideTimelineItem } from "../rideTimeline";
import { useStore } from "../state";
import { zoneColor } from "./WorkoutGraph";
import ConnectionStatus from "./ConnectionStatus";
import VoiceComposer from "./VoiceComposer";

const TIMELINE_BOTTOM_TOLERANCE_PX = 32;
/** The rail's open/closed choice sticks across rides, like the stats strip. */
const RAIL_COLLAPSED_KEY = "trainerpro.player.rail-collapsed";

function formatTimelineDuration(durationS: number): string {
  const seconds = Math.max(0, Math.round(durationS));
  if (seconds < 60) return `${seconds} sec`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder === 0 ? `${minutes} min` : `${minutes} min ${remainder} sec`;
}

/** "2nd", "3rd", … for a re-ridden interval's attempt number. */
function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

function segmentSummary(result: SegmentResult, segment?: SegmentRow): string {
  const duration = result.skipped
    ? `${formatTimelineDuration(result.ridden_duration_s)} / ${formatTimelineDuration(result.planned_duration_s)}`
    : formatTimelineDuration(result.planned_duration_s);
  if (result.average_power_w === null) return duration;
  // An open interval had no target, so "@" would suggest one it never had.
  return segment?.kind === "freeride"
    ? `${duration} · rode ${result.average_power_w} W avg`
    : `${duration} @ ${result.average_power_w} W`;
}

function zoneStyle(segment?: SegmentRow): CSSProperties {
  const startColor = segment ? zoneColor(segment.start_pct) : "#30363d";
  const endColor = segment ? zoneColor(segment.end_pct) : startColor;
  return {
    "--timeline-zone-start": startColor,
    "--timeline-zone-end": endColor,
  } as CSSProperties;
}

function SegmentCard({ result, segment }: { result: SegmentResult; segment?: SegmentRow }) {
  return (
    <div
      className={`timeline-bubble timeline-segment ${result.skipped ? "skipped" : ""}`}
      style={zoneStyle(segment)}
    >
      <div className="timeline-segment-head">
        <span>
          {segment?.label || `Interval ${result.segment_index + 1}`}
          {result.attempt > 1 && ` (${ordinal(result.attempt)})`}
        </span>
        {result.skipped && <span className="timeline-segment-skipped">Skipped</span>}
      </div>
      <div className="timeline-segment-result">{segmentSummary(result, segment)}</div>
      {result.average_cadence_rpm !== null && (
        <div className="timeline-segment-cadence">
          {result.average_cadence_rpm} rpm
        </div>
      )}
    </div>
  );
}

/** The interval being ridden, in the same shape as the finished cards above
    it but reading the plan (targets) rather than a result, so the history
    ends at "now" instead of one step behind the metrics. */
function CurrentSegmentCard({ player, segment }: { player: PlayerState; segment?: SegmentRow }) {
  const index = player.seg_idx!;
  const duration = segment
    ? formatTimelineDuration(segment.duration_s)
    : null;
  const target =
    segment?.kind === "freeride" || player.target_power_w === null
      ? "open"
      : `@ ${player.target_power_w} W`;
  return (
    <div className="timeline-bubble timeline-segment timeline-segment-current" style={zoneStyle(segment)}>
      <div className="timeline-segment-head">
        <span>
          <span className="timeline-now-dot" aria-hidden="true" />
          Now · {segment?.label || `Interval ${index + 1}`}
        </span>
        <span>{fmtDuration(player.seg_remaining_s)} left</span>
      </div>
      <div className="timeline-segment-result">
        {duration ? `${duration} ${target}` : target}
      </div>
      {player.target_cadence_rpm !== null && (
        <div className="timeline-segment-cadence">target {player.target_cadence_rpm} rpm</div>
      )}
    </div>
  );
}

function TimelineEntry({ item, segments }: {
  item: RideTimelineItem;
  segments: SegmentRow[];
}) {
  if (item.kind === "user-action") {
    return (
      <div className="timeline-row timeline-row-user">
        <div className="timeline-bubble timeline-action">
          <span className="timeline-action-label">{item.label}</span>
        </div>
      </div>
    );
  }
  if (item.kind === "segment") {
    return (
      <div className="timeline-row timeline-row-app">
        <SegmentCard result={item.result} segment={segments[item.result.segment_index]} />
      </div>
    );
  }
  return (
    <div className="timeline-row timeline-row-app">
      <div className={`timeline-bubble timeline-message ${item.tone}`}>
        {item.message}
      </div>
    </div>
  );
}

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(RAIL_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

export default function RideEventRail({ segments }: { segments: SegmentRow[] }) {
  const items = useStore((state) => state.rideTimeline.items);
  const player = useStore((state) => state.player);
  const feedRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const [showLatest, setShowLatest] = useState(false);
  const [collapsed, setCollapsed] = useState(readCollapsed);

  const toggleCollapsed = () => {
    setCollapsed((value) => {
      try {
        localStorage.setItem(RAIL_COLLAPSED_KEY, value ? "0" : "1");
      } catch {
        // A blocked store only costs the preference, not the toggle.
      }
      return !value;
    });
  };

  const scrollToLatest = (behavior: ScrollBehavior = "smooth") => {
    const feed = feedRef.current;
    if (!feed) return;
    feed.scrollTo({ top: feed.scrollHeight, behavior });
    pinnedRef.current = true;
    setShowLatest(false);
  };

  // The current card changes with the interval, so follow it like a new item.
  const currentIndex = player && player.phase !== "finished" ? player.seg_idx : null;
  useEffect(() => {
    if (collapsed) return;
    if (pinnedRef.current) scrollToLatest(items.length <= 1 ? "auto" : "smooth");
  }, [items.length, currentIndex, collapsed]);

  // A handle on the rail's edge, halfway down, in both states.
  const toggle = (
    <button
      type="button"
      className="ride-rail-toggle"
      onClick={toggleCollapsed}
      title={collapsed ? "Show the ride timeline" : "Hide the ride timeline"}
      aria-label={collapsed ? "Show the ride timeline" : "Hide the ride timeline"}
      aria-expanded={!collapsed}
    >
      {collapsed ? "›" : "‹"}
    </button>
  );

  if (collapsed) {
    return (
      <aside className="ride-event-rail ride-event-rail-collapsed" aria-label="Ride timeline and status">
        {toggle}
      </aside>
    );
  }

  return (
    <aside className="ride-event-rail" aria-label="Ride timeline and status">
      {toggle}
      <ConnectionStatus className="ride-connections" />
      <div className="ride-event-feed-wrap">
        <div
          ref={feedRef}
          className="ride-event-feed"
          role="log"
          aria-live="polite"
          onScroll={(event) => {
            const feed = event.currentTarget;
            const distance = feed.scrollHeight - feed.scrollTop - feed.clientHeight;
            const pinned = distance <= TIMELINE_BOTTOM_TOLERANCE_PX;
            pinnedRef.current = pinned;
            setShowLatest(!pinned);
          }}
        >
          <div className="ride-event-feed-spacer" />
          {items.map((item) => (
            <TimelineEntry key={item.id} item={item} segments={segments} />
          ))}
          {player && currentIndex !== null && (
            <div className="timeline-row timeline-row-app">
              <CurrentSegmentCard player={player} segment={segments[currentIndex]} />
            </div>
          )}
        </div>
        {showLatest && (
          <button className="timeline-latest" onClick={() => scrollToLatest()}>
            Latest ↓
          </button>
        )}
      </div>
      <VoiceComposer />
    </aside>
  );
}

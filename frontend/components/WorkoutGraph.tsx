// Workout profile graph: zone-colored segment polygons + progress cursor.
// Data is the `(t_s, %FTP)` breakpoint polyline stored at import;
// exactly two points per segment, so polygon i ↔ segment i. When `segments`
// is provided (detail view), bars are hoverable with a step tooltip and each
// interval's cadence target draws as a dashed marker on an rpm scale at the
// right edge. The Player also passes the ridden `trace` (a line on the watt
// scale) and an `onGoTo` handler, which turns right-click on a bar into a
// "Go to" menu.
//
// Layer order, bottom to top: polygons, cadence markers, hit rects, progress
// shading, trace, selection outline, HTML labels, tooltip, menu. Everything
// above the hit rects ignores the pointer, so hover and right-click always
// land on the interval — including intervals already ridden.

import { useEffect, useRef, useState } from "react";
import { RideTracePoint, SegmentRow, fmtDuration } from "../ipc";

interface Props {
  graph: [number, number][];
  durationS: number;
  progressS?: number;
  height?: number;
  segments?: SegmentRow[];
  /** FTP in watts. Supplied to draw a watt scale over the profile. */
  ftp?: number;
  /** Segment being ridden right now. Selected — and described — unless the
      pointer picks another one. */
  activeIndex?: number | null;
  /** Inclusive segment span to highlight, for callers whose selection covers
      more than one segment (a builder Repeat expands to many). Takes
      precedence over `activeIndex`; the pointer still overrides both. */
  highlightRange?: [number, number] | null;
  /** Ridden 1 Hz points in arrival order. Drawn on the watt scale, so it
      needs `ftp`. A jump in `elapsed_s` starts a new pass. */
  trace?: RideTracePoint[];
  /** Present in the Player while a ride can still move: right-clicking a
      bar offers "Go to" that interval. */
  onGoTo?: (index: number) => void;
}

/** Watt spacing of the scale labels. */
const GRID_STEP_W = 100;
/** Height of an open (no-target) block. It is a placeholder for time, not a
    target: the hatch and dashed outline say so, and it never joins `maxPct`. */
const OPEN_BLOCK_NOMINAL_PCT = 60;
/** Bottom of the rpm scale. Targets below it sit on the edge. */
const CADENCE_AXIS_MIN_RPM = 40;
/** Top of the rpm scale unless a target needs more room. */
const CADENCE_AXIS_DEFAULT_MAX_RPM = 130;
/** Room kept above the highest target so its marker never sits on the edge. */
const CADENCE_AXIS_HEADROOM_RPM = 10;
/** The raised top is rounded up to a multiple of this. */
const CADENCE_AXIS_ROUND_RPM = 10;
/** rpm spacing of the right-edge labels. */
const CADENCE_GRID_STEP_RPM = 20;
/** Consecutive trace points further apart than this (a skip, a forward
    Go To) are not joined. Samples arrive once a second. */
const TRACE_GAP_BREAK_S = 2;
const TRACE_PASS_OPACITY = 0.9;
/** Earlier passes over ground ridden again after a backward Go To. */
const TRACE_OLD_PASS_OPACITY = 0.35;
/** Menu box estimate, for flipping it away from the graph's edges. */
const MENU_WIDTH_PX = 190;
const MENU_HEIGHT_PX = 44;
const MENU_POINTER_GAP_PX = 4;

export function zoneColor(pct: number): string {
  if (pct < 55) return "#6e7681"; // Z1 recovery
  if (pct <= 75) return "#388bfd"; // Z2 endurance
  if (pct <= 90) return "#3fb950"; // Z3 tempo
  if (pct <= 105) return "#d29922"; // Z4 threshold
  if (pct <= 120) return "#f0883e"; // Z5 vo2
  if (pct <= 150) return "#f85149"; // Z6 anaerobic
  return "#bc8cff"; // Z7 neuromuscular
}

/** Resolve a displayed %FTP value exactly as the Rust power model does for
    positive workout targets: nearest whole watt. */
export function wattsFromPct(pct: number, ftp: number): number {
  return Math.round((pct / 100) * ftp);
}

export function segmentText(s: SegmentRow, ftp?: number): string {
  if (s.kind === "freeride") {
    const cad = s.cadence_rpm != null ? ` · ${s.cadence_rpm} rpm` : "";
    return `${s.label} — ${fmtDuration(s.duration_s)} · open${cad}`;
  }
  const pct =
    s.kind === "ramp"
      ? `${Math.round(s.start_pct)}% → ${Math.round(s.end_pct)}%`
      : `${Math.round(s.start_pct)}%`;
  const watts =
    ftp && ftp > 0
      ? s.kind === "ramp"
        ? ` (${wattsFromPct(s.start_pct, ftp)} W → ${wattsFromPct(s.end_pct, ftp)} W)`
        : ` (${wattsFromPct(s.start_pct, ftp)} W)`
      : "";
  const cad = s.cadence_rpm != null ? ` · ${s.cadence_rpm} rpm` : "";
  return `${s.label} — ${fmtDuration(s.duration_s)} @ ${pct} FTP${watts}${cad}`;
}

/** Split the trace into passes: runs of points whose position moves forward
    by no more than the gap limit. A backward jump or a larger gap ends one. */
export function splitTracePasses(trace: RideTracePoint[]): RideTracePoint[][] {
  const passes: RideTracePoint[][] = [];
  let current: RideTracePoint[] = [];
  for (const point of trace) {
    const previous = current[current.length - 1];
    if (
      previous &&
      (point.elapsed_s < previous.elapsed_s ||
        point.elapsed_s - previous.elapsed_s > TRACE_GAP_BREAK_S)
    ) {
      passes.push(current);
      current = [];
    }
    current.push(point);
  }
  if (current.length > 0) passes.push(current);
  return passes;
}

/** Runs of consecutive points that have power; a second without data breaks
    the line rather than drawing through it. */
function poweredRuns(pass: RideTracePoint[]): RideTracePoint[][] {
  const runs: RideTracePoint[][] = [];
  let current: RideTracePoint[] = [];
  for (const point of pass) {
    if (point.power_w === null) {
      if (current.length > 0) runs.push(current);
      current = [];
    } else {
      current.push(point);
    }
  }
  if (current.length > 0) runs.push(current);
  return runs;
}

/** What was ridden inside `[t0, t1)`, from the newest pass that covers it.
    Null when nothing was. */
export function riddenSummary(
  passes: RideTracePoint[][],
  t0: number,
  t1: number,
): string | null {
  for (let i = passes.length - 1; i >= 0; i--) {
    const inside = passes[i].filter((p) => p.elapsed_s >= t0 && p.elapsed_s < t1);
    if (inside.length === 0) continue;
    const powers = inside.flatMap((p) => (p.power_w === null ? [] : [p.power_w]));
    const cadences = inside.flatMap((p) => (p.cadence_rpm === null ? [] : [p.cadence_rpm]));
    const parts: string[] = [];
    if (powers.length > 0) {
      parts.push(`${Math.round(powers.reduce((a, b) => a + b, 0) / powers.length)} W avg`);
    }
    if (cadences.length > 0) {
      parts.push(`${Math.round(cadences.reduce((a, b) => a + b, 0) / cadences.length)} rpm`);
    }
    return parts.length > 0 ? `ridden ${parts.join(" · ")}` : null;
  }
  return null;
}

/** Top of the rpm scale for the highest target present. */
export function cadenceAxisMaxRpm(maxTargetRpm: number): number {
  const needed =
    Math.ceil((maxTargetRpm + CADENCE_AXIS_HEADROOM_RPM) / CADENCE_AXIS_ROUND_RPM) *
    CADENCE_AXIS_ROUND_RPM;
  return Math.max(CADENCE_AXIS_DEFAULT_MAX_RPM, needed);
}

interface MenuState {
  index: number;
  /** Pointer position and wrap size at open time, in wrap pixels. */
  x: number;
  y: number;
  w: number;
  h: number;
}

export default function WorkoutGraph({
  graph,
  durationS,
  progressS,
  height = 120,
  segments,
  ftp,
  activeIndex = null,
  highlightRange = null,
  trace,
  onGoTo,
}: Props) {
  const W = 1000;
  const [hover, setHover] = useState<number | null>(null);
  const [mouse, setMouse] = useState({ x: 0, y: 0, w: 1, h: 1 });
  const [menu, setMenu] = useState<MenuState | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const maxPct = Math.max(130, ...graph.map(([, p]) => p)) * 1.05;
  const x = (t: number) => (t / Math.max(durationS, 1)) * W;
  const y = (p: number) => height - (p / maxPct) * height;

  const polys: {
    points: string;
    color: string;
    x0: number;
    x1: number;
    t0: number;
    t1: number;
    /** No power target: both breakpoints are 0 %, which no real target can
        be (POWER_FRACTION_MIN). Drawn as a hatched block, not a zone bar. */
    open: boolean;
  }[] = [];
  for (let i = 0; i + 1 < graph.length; i += 2) {
    const [t0, p0] = graph[i];
    const [t1, p1] = graph[i + 1];
    polys.push({
      points: `${x(t0)},${height} ${x(t0)},${y(p0)} ${x(t1)},${y(p1)} ${x(t1)},${height}`,
      color: zoneColor((p0 + p1) / 2),
      x0: x(t0),
      x1: x(t1),
      t0,
      t1,
      open: p0 === 0 && p1 === 0,
    });
  }
  const openTop = y(OPEN_BLOCK_NOMINAL_PCT);

  /** Midpoint of a bar as a % of the width, kept off the edges so a tooltip
      centred there is not clipped by the (overflow-hidden) graph frame. */
  const anchorPct = (p: { x0: number; x1: number }) =>
    Math.min(85, Math.max(15, (((p.x0 + p.x1) / 2) / W) * 100));

  const interactive = segments !== undefined && segments.length === polys.length;
  // Default selection is whatever the caller marks — one segment being ridden,
  // or a span the editor has selected. The pointer overrides either.
  const clamp = (i: number) => Math.min(polys.length - 1, Math.max(0, i));
  const marked: [number, number] | null =
    highlightRange && polys.length > 0
      ? [clamp(highlightRange[0]), clamp(highlightRange[1])]
      : activeIndex !== null && activeIndex < polys.length
        ? [activeIndex, activeIndex]
        : null;
  const range: [number, number] | null = hover !== null ? [hover, hover] : marked;
  // The single index the tooltip describes: a span is named by its first bar.
  const selected = range ? range[0] : null;
  const hovered = interactive && selected !== null ? segments![selected] : null;

  // Watt scale: labels only, every 100 W. Rules across the whole width read as
  // clutter over the profile, so the numbers sit alone at the left edge. They
  // live in HTML, not SVG — the chart is stretched with
  // preserveAspectRatio="none", which would distort text.
  const gridW: number[] = [];
  if (ftp && ftp > 0) {
    for (let w = GRID_STEP_W; w < (maxPct / 100) * ftp; w += GRID_STEP_W) gridW.push(w);
  }
  const wattY = (w: number) => y((w / ftp!) * 100);

  // rpm scale: its own linear mapping over the same height, shown only when
  // some interval prescribes cadence. Labels mirror the watt labels on the
  // right edge; targets draw as dashed markers across their interval.
  const cadenceTargets = interactive
    ? segments!.map((s) => s.cadence_rpm).filter((c): c is number => c !== null)
    : [];
  const showCadence = cadenceTargets.length > 0;
  const cadenceMax = cadenceAxisMaxRpm(Math.max(0, ...cadenceTargets));
  const cadenceY = (rpm: number) => {
    const clamped = Math.min(cadenceMax, Math.max(CADENCE_AXIS_MIN_RPM, rpm));
    return (
      height - ((clamped - CADENCE_AXIS_MIN_RPM) / (cadenceMax - CADENCE_AXIS_MIN_RPM)) * height
    );
  };
  const gridRpm: number[] = [];
  if (showCadence) {
    for (
      let r = CADENCE_AXIS_MIN_RPM + CADENCE_GRID_STEP_RPM;
      r < cadenceMax;
      r += CADENCE_GRID_STEP_RPM
    ) {
      gridRpm.push(r);
    }
  }

  // Ridden trace on the watt scale. Power above the chart is pinned to the
  // top edge rather than rescaling the plan under the rider.
  const passes = trace && ftp && ftp > 0 ? splitTracePasses(trace) : [];
  const traceY = (w: number) => Math.max(0, y((w / ftp!) * 100));
  const traceLines = passes.flatMap((pass, passIndex) =>
    poweredRuns(pass).map((run, runIndex) => ({
      key: `t${passIndex}-${runIndex}`,
      points: run.map((p) => `${x(p.elapsed_s)},${traceY(p.power_w!)}`).join(" "),
      opacity: passIndex === passes.length - 1 ? TRACE_PASS_OPACITY : TRACE_OLD_PASS_OPACITY,
    })),
  );
  const ridden =
    hovered && selected !== null
      ? riddenSummary(passes, polys[selected].t0, polys[selected].t1)
      : null;

  const closeMenu = () => {
    setMenu(null);
    setHover(null);
  };
  useEffect(() => {
    if (menu === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeMenu();
    };
    const onPointerDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) closeMenu();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onPointerDown);
    };
    // closeMenu only touches state setters; re-arming per open is the point.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [menu]);

  const svg = (
    <svg
      viewBox={`0 0 ${W} ${height}`}
      preserveAspectRatio="none"
      style={{ width: "100%", height: "100%", display: "block" }}
    >
      <defs>
        {/* Diagonal hatch for open blocks. Pattern space is the stretched
            viewBox, so the stripes lean with the aspect ratio; that is fine
            for a texture whose only job is to read as "not a bar". */}
        <pattern
          id="graph-open-hatch"
          width={14}
          height={14}
          patternUnits="userSpaceOnUse"
          patternTransform="rotate(45)"
        >
          <rect width={14} height={14} fill="#6e7681" fillOpacity={0.16} />
          <rect width={4} height={14} fill="#8b949e" fillOpacity={0.45} />
        </pattern>
      </defs>
      <line x1={0} x2={W} y1={y(100)} y2={y(100)} stroke="#30363d" strokeDasharray="6 6" />
      {polys.map((p, i) => {
        const dim = range === null || (i >= range[0] && i <= range[1]) ? 0.95 : 0.5;
        if (!p.open) return <polygon key={i} points={p.points} fill={p.color} opacity={dim} />;
        return (
          <g key={i} opacity={dim}>
            <rect
              x={p.x0}
              y={openTop}
              width={Math.max(p.x1 - p.x0, 1)}
              height={height - openTop}
              fill="url(#graph-open-hatch)"
            />
            <rect
              x={p.x0}
              y={openTop}
              width={Math.max(p.x1 - p.x0, 1)}
              height={height - openTop}
              fill="none"
              stroke="#c9d1d9"
              strokeOpacity={0.8}
              strokeWidth={1.5}
              strokeDasharray="5 4"
              vectorEffect="non-scaling-stroke"
            />
          </g>
        );
      })}
      {showCadence &&
        polys.map((p, i) => {
          const rpm = segments![i].cadence_rpm;
          if (rpm === null) return null;
          return (
            <line
              key={`c${i}`}
              x1={p.x0}
              x2={p.x1}
              y1={cadenceY(rpm)}
              y2={cadenceY(rpm)}
              stroke="#c9d1d9"
              strokeOpacity={0.8}
              strokeWidth={1.5}
              strokeDasharray="4 3"
              vectorEffect="non-scaling-stroke"
              pointerEvents="none"
            />
          );
        })}
      {interactive &&
        polys.map((p, i) => (
          <rect
            key={`h${i}`}
            x={p.x0}
            y={0}
            width={Math.max(p.x1 - p.x0, 1)}
            height={height}
            fill="transparent"
            onMouseEnter={() => {
              if (menu === null) setHover(i);
            }}
            onMouseLeave={() => {
              if (menu === null) setHover(null);
            }}
            onContextMenu={(e) => {
              if (!onGoTo) return;
              e.preventDefault();
              const wrap = wrapRef.current;
              if (!wrap) return;
              const r = wrap.getBoundingClientRect();
              setHover(i);
              setMenu({
                index: i,
                x: e.clientX - r.left,
                y: e.clientY - r.top,
                w: r.width,
                h: r.height,
              });
            }}
          />
        ))}
      {progressS !== undefined && (
        // Drawn over the hit rects, so it must not swallow the hover.
        <>
          <rect
            x={0}
            y={0}
            width={x(progressS)}
            height={height}
            fill="#000"
            opacity={0.35}
            pointerEvents="none"
          />
          <line x1={x(progressS)} x2={x(progressS)} y1={0} y2={height} stroke="#e6edf3" strokeWidth={2} pointerEvents="none" />
        </>
      )}
      {/* Over the progress shading so the ridden line stays bright where the
          plan has been dimmed. Round caps let a one-second run show as a dot. */}
      {traceLines.map((l) => (
        <polyline
          key={l.key}
          points={l.points}
          fill="none"
          stroke="#e6edf3"
          strokeOpacity={l.opacity}
          strokeWidth={1.5}
          strokeLinecap="round"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
          pointerEvents="none"
        />
      ))}
      {/* Over the progress shading, so the selected interval stays legible once
          it has been partly ridden. */}
      {range !== null && (
        <rect
          x={polys[range[0]].x0}
          y={0}
          width={Math.max(polys[range[1]].x1 - polys[range[0]].x0, 1)}
          height={height}
          fill="#e6edf3"
          fillOpacity={0.1}
          stroke="#e6edf3"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
          pointerEvents="none"
        />
      )}
    </svg>
  );

  const scaleLabels = gridW.map((w) => (
    <span key={w} className="graph-scale-label" style={{ top: `${(wattY(w) / height) * 100}%` }}>
      {w} W
    </span>
  ));
  // "OPEN" centred in each open block. HTML like the scale labels, and a
  // container query hides it when the block is too narrow to hold it.
  const openLabels = interactive
    ? polys.flatMap((p, i) =>
        p.open
          ? [
              <div
                key={`o${i}`}
                className="graph-open"
                style={{
                  left: `${(p.x0 / W) * 100}%`,
                  width: `${(Math.max(p.x1 - p.x0, 1) / W) * 100}%`,
                  top: `${(openTop / height) * 100}%`,
                  height: `${((height - openTop) / height) * 100}%`,
                }}
              >
                <span className="graph-open-label">open</span>
              </div>,
            ]
          : [],
      )
    : [];
  const cadenceLabels = gridRpm.map((r) => (
    <span
      key={`r${r}`}
      className="graph-scale-label graph-scale-label-right"
      style={{ top: `${(cadenceY(r) / height) * 100}%` }}
    >
      {r} rpm
    </span>
  ));

  if (!interactive) {
    return scaleLabels.length === 0 ? (
      svg
    ) : (
      <div className="graph-wrap">
        {svg}
        {scaleLabels}
      </div>
    );
  }

  const menuLabel = (m: MenuState) => {
    const name = segments![m.index].label || `Interval ${m.index + 1}`;
    const restart = m.index === activeIndex && (progressS ?? 0) > 0;
    return `${restart ? "Restart" : "Go to"} ${name}`;
  };

  return (
    <div
      ref={wrapRef}
      className="graph-wrap"
      onMouseMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        setMouse({ x: e.clientX - r.left, y: e.clientY - r.top, w: r.width, h: r.height });
      }}
      onMouseLeave={() => {
        if (menu !== null) closeMenu();
      }}
    >
      {svg}
      {scaleLabels}
      {cadenceLabels}
      {openLabels}
      {hovered && menu === null && (
        <div
          className="graph-tooltip"
          style={
            hover !== null
              ? {
                  left: mouse.x > mouse.w * 0.6 ? mouse.x - 250 : mouse.x + 14,
                  top: mouse.y > mouse.h * 0.55 ? mouse.y - 58 : mouse.y + 14,
                }
              : // Not hovering: the label belongs to the interval being ridden,
                // so park it over that interval rather than at the pointer.
                {
                  left: `${anchorPct(polys[selected!])}%`,
                  bottom: 8,
                  transform: "translateX(-50%)",
                }
          }
        >
          <div className="graph-tooltip-title">{segmentText(hovered, ftp)}</div>
          {ridden && <div className="graph-tooltip-ridden">{ridden}</div>}
          {hovered.note && <div className="graph-tooltip-note">{hovered.note}</div>}
        </div>
      )}
      {menu !== null && onGoTo && (
        <div
          ref={menuRef}
          className="graph-menu"
          role="menu"
          style={{
            left:
              menu.x > menu.w * 0.6
                ? menu.x - MENU_WIDTH_PX
                : menu.x + MENU_POINTER_GAP_PX,
            top:
              menu.y > menu.h * 0.55
                ? menu.y - MENU_HEIGHT_PX
                : menu.y + MENU_POINTER_GAP_PX,
          }}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            autoFocus
            onClick={() => {
              onGoTo(menu.index);
              closeMenu();
            }}
          >
            {menuLabel(menu)}
          </button>
        </div>
      )}
    </div>
  );
}

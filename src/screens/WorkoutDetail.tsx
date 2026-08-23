// Detailed workout view, whatsonzwift-style: stat tiles, zone-distribution
// strip, hoverable graph, and a tree-structured interval list where repeats
// become "N×" groups with indented children.

import { AppError, SegmentRow, fmtDuration, ipc } from "../ipc";
import { WorkoutDetailView, useStore } from "../state";
import WorkoutGraph, { segmentText, zoneColor } from "../components/WorkoutGraph";

// ---------------------------------------------------------------------------
// Interval tree: pair-repeats (on/off) and single-step repeats become groups.
// ---------------------------------------------------------------------------

type Node =
  | { type: "single"; seg: SegmentRow }
  | { type: "repeat"; reps: number; children: SegmentRow[] };

function repKey(s: SegmentRow): string {
  const { note, ...rest } = s;
  void note;
  return JSON.stringify(rest);
}

function buildTree(segments: SegmentRow[]): Node[] {
  const nodes: Node[] = [];
  let i = 0;
  while (i < segments.length) {
    const a = repKey(segments[i]);
    // Run of identical single steps.
    if (i + 1 < segments.length && repKey(segments[i + 1]) === a) {
      let k = 1;
      while (i + k < segments.length && repKey(segments[i + k]) === a) k += 1;
      nodes.push({ type: "repeat", reps: k, children: [segments[i]] });
      i += k;
      continue;
    }
    // Repeated (on, off) pairs.
    if (i + 1 < segments.length) {
      const b = repKey(segments[i + 1]);
      let reps = 1;
      let j = i + 2;
      while (
        j + 1 < segments.length &&
        repKey(segments[j]) === a &&
        repKey(segments[j + 1]) === b
      ) {
        reps += 1;
        j += 2;
      }
      if (reps >= 2) {
        nodes.push({ type: "repeat", reps, children: [segments[i], segments[i + 1]] });
        i = j;
        continue;
      }
    }
    nodes.push({ type: "single", seg: segments[i] });
    i += 1;
  }
  return nodes;
}

// ---------------------------------------------------------------------------
// Zone distribution (time in Z1..Z7; ramps sliced for attribution).
// ---------------------------------------------------------------------------

const ZONES = [
  { name: "Z1", color: "#6e7681" },
  { name: "Z2", color: "#388bfd" },
  { name: "Z3", color: "#3fb950" },
  { name: "Z4", color: "#d29922" },
  { name: "Z5", color: "#f0883e" },
  { name: "Z6", color: "#f85149" },
  { name: "Z7", color: "#bc8cff" },
];

function zoneOf(pct: number): number {
  if (pct < 55) return 0;
  if (pct <= 75) return 1;
  if (pct <= 90) return 2;
  if (pct <= 105) return 3;
  if (pct <= 120) return 4;
  if (pct <= 150) return 5;
  return 6;
}

function zoneSeconds(segments: SegmentRow[]): number[] {
  const out = [0, 0, 0, 0, 0, 0, 0];
  for (const s of segments) {
    if (s.kind === "ramp") {
      const slices = 10;
      for (let i = 0; i < slices; i++) {
        const p = s.start_pct + ((s.end_pct - s.start_pct) * (i + 0.5)) / slices;
        out[zoneOf(p)] += s.duration_s / slices;
      }
    } else {
      out[zoneOf(s.start_pct)] += s.duration_s;
    }
  }
  return out;
}

function SegLine({ seg, ftp }: { seg: SegmentRow; ftp: number }) {
  const color =
    seg.kind === "freeride" ? "#6e7681" : zoneColor(Math.max(seg.start_pct, seg.end_pct));
  return (
    <div className="segment-row">
      <span className="segment-dot" style={{ background: color }} />
      <span className="segment-text">
        {segmentText(seg, ftp)}
        {seg.note && <span className="segment-note">{seg.note}</span>}
      </span>
    </div>
  );
}

export default function WorkoutDetail() {
  const { detail, settings, go, pushToast } = useStore();

  if (!detail) {
    return (
      <div className="screen">
        <p className="empty">No workout selected.</p>
        <button onClick={() => go("library")}>Back</button>
      </div>
    );
  }
  const d: WorkoutDetailView = detail;

  async function ride() {
    try {
      const ps =
        d.source === "planner"
          ? await ipc.plannerRide(d.wid!)
          : d.source === "woz"
            ? await ipc.wozRide(d.wozRef!.collection, d.wozRef!.idx)
            : await ipc.loadWorkout(d.id!);
      useStore.setState({ player: ps });
      void useStore.getState().refreshWorkouts();
      go("player");
    } catch (e) {
      const err = e as AppError;
      if (err.code === "no_trainer") {
        pushToast("warn", "Connect a trainer first");
        go("devices");
      } else {
        pushToast("error", err.message ?? String(e));
      }
    }
  }

  async function refreshDetail() {
    if (d.source !== "planner" || d.wid == null) return;
    try {
      const p = await ipc.plannerPreview(d.wid);
      const row = useStore.getState().plannerRows.find((w) => w.wid === d.wid);
      useStore.setState({
        detail: {
          ...d,
          name: row?.title ?? d.name,
          tags: row?.tags ?? d.tags,
          duration_s: p.duration_s,
          est_if: p.est_if,
          est_tss: p.est_tss,
          graph: p.graph,
          segments: p.segments,
          contentKey: row?.dsl ?? d.contentKey,
          updateAvailable: false,
        },
      });
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    }
  }

  async function del() {
    // Import copies files into the app's library folder, so this only
    // removes TrainerPro's copy — the file you imported from is untouched.
    if (
      !confirm(
        `Remove “${d.name}” from your TrainerPro library?\n\n` +
          `This deletes the app's copy only. The original file you imported ` +
          `stays where it is on disk.`,
      )
    )
      return;
    await ipc.deleteWorkout(d.id!);
    await useStore.getState().refreshWorkouts();
    go("library");
  }

  const ftp = settings?.profile.ftp ?? 200;
  const kj = Math.round(
    d.segments.reduce(
      (acc, s) => acc + (((s.start_pct + s.end_pct) / 2 / 100) * ftp * s.duration_s) / 1000,
      0,
    ),
  );
  const zs = zoneSeconds(d.segments);
  const zTotal = Math.max(
    zs.reduce((a, b) => a + b, 0),
    1,
  );
  const tree = buildTree(d.segments);
  const badge =
    d.source === "planner" || d.origin === "planner"
      ? "planner"
      : d.source === "woz" || d.origin === "whatsonzwift"
        ? "zwift"
        : null;

  const stat = (value: string, label: string) => (
    <div className="stat">
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
    </div>
  );

  return (
    <div className="screen detail">
      <header className="screen-head">
        <button className="ghost back" onClick={() => go("library")}>
          ‹ Back
        </button>
        <h1>{d.name}</h1>
        {badge && <span className="badge">{badge}</span>}
      </header>
      {d.updateAvailable && (
        <div className="update-banner">
          This workout changed on the server.
          <button className="primary" onClick={refreshDetail}>
            Refresh
          </button>
        </div>
      )}
      <div className="row gap detail-actions">
        {d.source === "library" && (
          <button className="danger" onClick={del}>
            Remove from library
          </button>
        )}
        <button className="go" onClick={ride}>
          ▶ Ride this workout
        </button>
        {d.source === "planner" && (
          <button onClick={() => ipc.plannerOpenEditor(d.wid!)}>Edit in Planner ↗</button>
        )}
        {d.source === "woz" && (
          <button onClick={() => ipc.wozOpenPage(d.wozRef!.collection)}>
            View on whatsonzwift ↗
          </button>
        )}
      </div>

      {d.tags && <p className="muted detail-meta">{d.tags}</p>}
      {d.description && <p className="detail-desc">{d.description}</p>}

      <div className="stat-row">
        {stat(fmtDuration(d.duration_s), "duration")}
        {d.est_tss != null && stat(String(Math.round(d.est_tss)), "TSS")}
        {d.est_if != null && stat(d.est_if.toFixed(2), "intensity (IF)")}
        {stat(`~${kj} kJ`, `work @ ${ftp} W FTP`)}
      </div>

      <div className="detail-graph">
        <WorkoutGraph
          graph={d.graph}
          durationS={d.duration_s}
          height={160}
          segments={d.segments}
          ftp={ftp}
        />
      </div>

      <div className="zone-strip" title="time in zone">
        {zs.map(
          (sec, i) =>
            sec > 0 && (
              <div
                key={i}
                className="zone-chunk"
                style={{ width: `${(sec / zTotal) * 100}%`, background: ZONES[i].color }}
                title={`${ZONES[i].name} · ${fmtDuration(Math.round(sec))}`}
              />
            ),
        )}
      </div>
      <div className="zone-legend">
        {zs.map(
          (sec, i) =>
            sec >= 60 && (
              <span key={i} className="zone-legend-item">
                <span className="segment-dot" style={{ background: ZONES[i].color }} />
                {ZONES[i].name} {fmtDuration(Math.round(sec))}
              </span>
            ),
        )}
      </div>

      <h2>Intervals</h2>
      <div className="segment-list">
        {tree.map((node, i) =>
          node.type === "single" ? (
            <SegLine key={i} seg={node.seg} ftp={ftp} />
          ) : (
            <div key={i} className="repeat-group">
              <div className="repeat-head">{node.reps}×</div>
              <div className="repeat-children">
                {node.children.map((c, j) => (
                  <SegLine key={j} seg={c} ftp={ftp} />
                ))}
              </div>
            </div>
          ),
        )}
      </div>
    </div>
  );
}

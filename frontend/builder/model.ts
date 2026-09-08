// The workout builder's document model and every derivation off it.
//
// Single source of truth: the screen holds `BuildNode[]` and nothing else.
// Stats, the power-profile graph and the segment rows are all *derived* here,
// so there is no parallel state to fall out of sync after a drop.
//
// The tree is at most two deep — a Repeat may hold leaves, never another
// Repeat. That restriction is what lets drag-and-drop stay a flat multi-list
// problem instead of a recursive one, and the Rust side enforces it again at
// save time (crates/tp-core/src/build.rs).

import type { SegmentRow } from "../ipc";

export const ROOT = "root";

/** Percent-of-FTP bounds the editor allows. The model's own limit is 5–300 %
    (tp-core consts); this is the tighter range a rider actually prescribes. */
export const MIN_PCT = 5;
export const MAX_PCT = 150;
export const MIN_DURATION_S = 5;
export const MAX_DURATION_S = 4 * 3600;
export const MAX_REPEAT_COUNT = 100;

export interface SimpleNode {
  id: string;
  kind: "simple";
  duration_s: number;
  power_pct: number;
  cadence_rpm: number | null;
}
export interface RampNode {
  id: string;
  kind: "ramp";
  duration_s: number;
  start_pct: number;
  end_pct: number;
  cadence_rpm: number | null;
}
export interface RepeatNode {
  id: string;
  kind: "repeat";
  count: number;
  children: LeafNode[];
}
export type LeafNode = SimpleNode | RampNode;
export type BuildNode = LeafNode | RepeatNode;

let seq = 0;
export function newId(): string {
  seq += 1;
  return `n${seq}`;
}

export function makeNode(kind: BuildNode["kind"]): BuildNode {
  switch (kind) {
    case "simple":
      return { id: newId(), kind, duration_s: 300, power_pct: 65, cadence_rpm: null };
    case "ramp":
      return { id: newId(), kind, duration_s: 600, start_pct: 55, end_pct: 75, cadence_rpm: null };
    case "repeat":
      return {
        id: newId(),
        kind,
        count: 3,
        children: [
          { id: newId(), kind: "simple", duration_s: 60, power_pct: 105, cadence_rpm: null },
          { id: newId(), kind: "simple", duration_s: 120, power_pct: 55, cadence_rpm: null },
        ],
      };
  }
}

export function isLeaf(n: BuildNode): n is LeafNode {
  return n.kind !== "repeat";
}

export function nodeDuration(n: BuildNode): number {
  if (n.kind === "repeat") {
    return n.children.reduce((a, c) => a + c.duration_s, 0) * n.count;
  }
  return n.duration_s;
}

export function totalDuration(nodes: BuildNode[]): number {
  return nodes.reduce((a, n) => a + nodeDuration(n), 0);
}

// --- lookup -----------------------------------------------------------------

/** Which list a node lives in: ROOT, or the id of its Repeat. */
export function containerOf(nodes: BuildNode[], id: string): string | null {
  for (const n of nodes) {
    if (n.id === id) return ROOT;
    if (n.kind === "repeat" && n.children.some((c) => c.id === id)) return n.id;
  }
  return null;
}

export function findNode(nodes: BuildNode[], id: string): BuildNode | null {
  for (const n of nodes) {
    if (n.id === id) return n;
    if (n.kind === "repeat") {
      const hit = n.children.find((c) => c.id === id);
      if (hit) return hit;
    }
  }
  return null;
}

/** Ids in the order dnd-kit sees them, per container. */
export function itemIds(nodes: BuildNode[], container: string): string[] {
  if (container === ROOT) return nodes.map((n) => n.id);
  const rep = nodes.find((n) => n.id === container);
  return rep && rep.kind === "repeat" ? rep.children.map((c) => c.id) : [];
}

// --- edits (all pure; every one returns a fresh array) -----------------------

/** A bag of editable fields. Deliberately not `Partial<BuildNode>`: the
    discriminant is never patched, and a union of partials would force every
    caller to narrow before it could set `duration_s`. */
export interface NodePatch {
  duration_s?: number;
  power_pct?: number;
  start_pct?: number;
  end_pct?: number;
  cadence_rpm?: number | null;
  count?: number;
  children?: LeafNode[];
}

export function updateNode(nodes: BuildNode[], id: string, patch: NodePatch): BuildNode[] {
  return nodes.map((n) => {
    if (n.id === id) return { ...n, ...patch } as BuildNode;
    if (n.kind === "repeat") {
      const children = n.children.map((c) =>
        c.id === id ? ({ ...c, ...patch } as LeafNode) : c,
      );
      return children === n.children ? n : { ...n, children };
    }
    return n;
  });
}

export function removeNode(nodes: BuildNode[], id: string): BuildNode[] {
  return nodes
    .filter((n) => n.id !== id)
    .map((n) =>
      n.kind === "repeat" ? { ...n, children: n.children.filter((c) => c.id !== id) } : n,
    );
}

/** Insert into `container` at `index` (clamped; -1 or past the end appends). */
export function insertNode(
  nodes: BuildNode[],
  node: BuildNode,
  container: string,
  index: number,
): BuildNode[] {
  if (container === ROOT) {
    const out = [...nodes];
    const at = index < 0 || index > out.length ? out.length : index;
    out.splice(at, 0, node);
    return out;
  }
  // A Repeat can only ever hold leaves — refuse rather than build an illegal
  // tree that save would reject later.
  if (node.kind === "repeat") return nodes;
  return nodes.map((n) => {
    if (n.id !== container || n.kind !== "repeat") return n;
    const children = [...n.children];
    const at = index < 0 || index > children.length ? children.length : index;
    children.splice(at, 0, node);
    return { ...n, children };
  });
}

/** Move a node to (container, index). Dropping a Repeat inside a Repeat is a
    no-op — the drop rules should prevent it reaching here, but the model
    refuses regardless. */
export function moveNode(
  nodes: BuildNode[],
  id: string,
  container: string,
  index: number,
): BuildNode[] {
  const node = findNode(nodes, id);
  if (!node) return nodes;
  if (node.kind === "repeat" && container !== ROOT) return nodes;
  const without = removeNode(nodes, id);
  return insertNode(without, node, container, index);
}

// --- derivations ------------------------------------------------------------

/** One ridden segment, repeats expanded. `sourceId` tracks which editor node
    produced it, so selecting a row can highlight its span on the graph. */
export interface FlatSegment {
  sourceId: string;
  duration_s: number;
  start_pct: number;
  end_pct: number;
  cadence_rpm: number | null;
  label: string;
}

export function expand(nodes: BuildNode[]): FlatSegment[] {
  const out: FlatSegment[] = [];
  const push = (n: LeafNode, label: string) => {
    out.push({
      sourceId: n.id,
      duration_s: n.duration_s,
      start_pct: n.kind === "ramp" ? n.start_pct : n.power_pct,
      end_pct: n.kind === "ramp" ? n.end_pct : n.power_pct,
      cadence_rpm: n.cadence_rpm,
      label,
    });
  };
  for (const n of nodes) {
    if (n.kind === "repeat") {
      for (let lap = 1; lap <= n.count; lap++) {
        for (const c of n.children) push(c, `Interval ${lap}/${n.count}`);
      }
    } else {
      push(n, n.kind === "ramp" ? "Ramp" : "Steady");
    }
  }
  return out;
}

/** Breakpoint polyline for WorkoutGraph: exactly two points per segment, so
    polygon i ↔ segment i. Mirrors graph_points() in backend/src/cmd.rs. */
export function toGraph(segments: FlatSegment[]): [number, number][] {
  const out: [number, number][] = [];
  let t = 0;
  for (const s of segments) {
    out.push([t, s.start_pct]);
    out.push([t + s.duration_s, s.end_pct]);
    t += s.duration_s;
  }
  return out;
}

/** Rows for the graph's hover tooltip, in WorkoutGraph's shape. */
export function toSegmentRows(segments: FlatSegment[]): SegmentRow[] {
  return segments.map((s) => ({
    kind: s.start_pct === s.end_pct ? "steady" : "ramp",
    label: s.label,
    note: null,
    duration_s: s.duration_s,
    start_pct: s.start_pct,
    end_pct: s.end_pct,
    cadence_rpm: s.cadence_rpm,
  }));
}

/** Segment index span a given editor node occupies, or null if it has none. */
export function spanOf(segments: FlatSegment[], id: string | null): [number, number] | null {
  if (!id) return null;
  let first = -1;
  let last = -1;
  segments.forEach((s, i) => {
    if (s.sourceId === id) {
      if (first < 0) first = i;
      last = i;
    }
  });
  return first < 0 ? null : [first, last];
}

/** Selecting a Repeat should light up the whole block, not just the children
    that carry its id — so widen to every segment produced by its children. */
export function spanOfNode(
  nodes: BuildNode[],
  segments: FlatSegment[],
  id: string | null,
): [number, number] | null {
  if (!id) return null;
  const node = findNode(nodes, id);
  if (node && node.kind === "repeat") {
    const ids = new Set(node.children.map((c) => c.id));
    let first = -1;
    let last = -1;
    segments.forEach((s, i) => {
      if (ids.has(s.sourceId)) {
        if (first < 0) first = i;
        last = i;
      }
    });
    return first < 0 ? null : [first, last];
  }
  return spanOf(segments, id);
}

export interface Estimate {
  duration_s: number;
  np: number | null;
  if_: number | null;
  tss: number | null;
}

/** IF/TSS exactly as tp-core::metrics computes them: a 1 Hz target series →
    30 s rolling mean → mean of 4th powers → 4th root, then TSS = IF² × hours
    × 100. Kept in step with the Rust estimate so the builder's numbers match
    what the library shows after saving. */
export function estimate(segments: FlatSegment[], ftp: number): Estimate {
  const duration_s = segments.reduce((a, s) => a + s.duration_s, 0);
  if (duration_s === 0 || ftp <= 0) return { duration_s, np: null, if_: null, tss: null };

  const series: number[] = [];
  for (const s of segments) {
    for (let t = 0; t < s.duration_s; t++) {
      const pct = s.start_pct + ((s.end_pct - s.start_pct) * t) / s.duration_s;
      series.push((pct / 100) * ftp);
    }
  }

  const w = 30;
  let np: number;
  if (series.length < w) {
    np = series.reduce((a, b) => a + b, 0) / series.length;
  } else {
    let sum = 0;
    for (let i = 0; i < w; i++) sum += series[i];
    let acc = (sum / w) ** 4;
    let count = 1;
    for (let i = w; i < series.length; i++) {
      sum += series[i] - series[i - w];
      acc += (sum / w) ** 4;
      count++;
    }
    np = (acc / count) ** 0.25;
  }
  const if_ = np / ftp;
  return { duration_s, np, if_, tss: (duration_s / 3600) * if_ * if_ * 100 };
}

// --- validation & serialisation ---------------------------------------------

export function clampPct(v: number): number {
  return Math.min(MAX_PCT, Math.max(MIN_PCT, Math.round(v)));
}
export function clampDuration(v: number): number {
  return Math.min(MAX_DURATION_S, Math.max(MIN_DURATION_S, Math.round(v)));
}
export function clampCount(v: number): number {
  return Math.min(MAX_REPEAT_COUNT, Math.max(1, Math.round(v)));
}

/** Why the workout can't be saved yet, or null when it can. */
export function saveBlocker(name: string, nodes: BuildNode[]): string | null {
  if (!name.trim()) return "Name the workout before saving";
  if (nodes.length === 0) return "Add at least one interval";
  const empty = nodes.find((n) => n.kind === "repeat" && n.children.length === 0);
  if (empty) return "A repeat is empty — put an interval inside it or delete it";
  return null;
}

/** Strip editor ids; the Rust DTO is positional and has no use for them. */
export function toDraft(name: string, description: string, nodes: BuildNode[]) {
  const leaf = (n: LeafNode) =>
    n.kind === "ramp"
      ? {
          kind: "ramp" as const,
          duration_s: n.duration_s,
          start_pct: n.start_pct,
          end_pct: n.end_pct,
          cadence_rpm: n.cadence_rpm,
        }
      : {
          kind: "simple" as const,
          duration_s: n.duration_s,
          power_pct: n.power_pct,
          cadence_rpm: n.cadence_rpm,
        };
  return {
    name: name.trim(),
    description,
    nodes: nodes.map((n) =>
      n.kind === "repeat"
        ? { kind: "repeat" as const, count: n.count, children: n.children.map(leaf) }
        : leaf(n),
    ),
  };
}

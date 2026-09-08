// "Build a Workout" — compose intervals, watch the profile and stats update,
// save as ZWO through the ordinary import pipeline.
//
// Everything on screen derives from one `nodes` array (see builder/model.ts).
// Drag-and-drop only ever calls moveNode/insertNode on that array; there is no
// second copy of the ordering to drift out of sync.

import { useCallback, useMemo, useRef, useState } from "react";
import {
  DndContext,
  DragEndEvent,
  DragOverlay,
  DragStartEvent,
  PointerSensor,
  closestCenter,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

import { AppError, fmtDuration, ipc } from "../ipc";
import { useStore } from "../state";
import WorkoutGraph, { wattsFromPct, zoneColor } from "../components/WorkoutGraph";
import {
  BuildNode,
  LeafNode,
  NodePatch,
  ROOT,
  RepeatNode,
  clampCount,
  clampDuration,
  clampPct,
  containerOf,
  estimate,
  expand,
  insertNode,
  itemIds,
  makeNode,
  moveNode,
  nodeDuration,
  removeNode,
  saveBlocker,
  spanOfNode,
  toDraft,
  toGraph,
  toSegmentRows,
  totalDuration,
  updateNode,
} from "../builder/model";

const PALETTE: { kind: BuildNode["kind"]; label: string }[] = [
  { kind: "simple", label: "Simple" },
  { kind: "repeat", label: "Repeat" },
  { kind: "ramp", label: "Ramp" },
];

/** mm:ss ⇄ seconds. The duration field is text so 1:30 can be typed directly. */
function parseDuration(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const parts = t.split(":");
  if (parts.some((p) => p !== "" && !/^\d+$/.test(p))) return null;
  const nums = parts.map((p) => (p === "" ? 0 : parseInt(p, 10)));
  if (nums.length === 1) return nums[0] * 60; // bare number reads as minutes
  if (nums.length === 2) return nums[0] * 60 + nums[1];
  if (nums.length === 3) return nums[0] * 3600 + nums[1] * 60 + nums[2];
  return null;
}

function DurationField({
  value,
  onChange,
}: {
  value: number;
  onChange: (v: number) => void;
}) {
  // Local text while focused so a half-typed "1:" isn't reformatted mid-edit.
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? fmtDuration(value);
  return (
    <input
      className="b-field b-field-time"
      value={shown}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => {
        setDraft(fmtDuration(value));
        e.currentTarget.select();
      }}
      onBlur={() => {
        const parsed = draft === null ? null : parseDuration(draft);
        if (parsed !== null) onChange(clampDuration(parsed));
        setDraft(null);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setDraft(null);
          e.currentTarget.blur();
        }
        e.stopPropagation(); // Delete-to-remove must not fire while typing
      }}
    />
  );
}

function PctField({
  value,
  ftp,
  onChange,
}: {
  value: number;
  ftp: number;
  onChange: (v: number) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const displayedText = draft ?? String(value);
  const displayedPct = displayedText.trim() === "" ? NaN : Number(displayedText);
  const watts = Number.isFinite(displayedPct) ? wattsFromPct(displayedPct, ftp) : null;
  return (
    <div className="b-pct">
      <input
        className="b-field b-field-pct"
        inputMode="numeric"
        value={draft ?? String(value)}
        onChange={(e) => setDraft(e.target.value)}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={() => {
          const n = draft === null ? NaN : Number(draft);
          if (Number.isFinite(n)) onChange(clampPct(n));
          setDraft(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            setDraft(null);
            e.currentTarget.blur();
          }
          e.stopPropagation();
        }}
      />
      <span className="b-unit">%</span>
      {watts !== null && <span className="b-watts">{watts} W</span>}
    </div>
  );
}

function IntField({
  value,
  onChange,
  width = 56,
}: {
  value: number;
  onChange: (v: number) => void;
  width?: number;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <input
      className="b-field"
      style={{ width }}
      inputMode="numeric"
      value={draft ?? String(value)}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={() => {
        const n = draft === null ? NaN : Number(draft);
        if (Number.isFinite(n)) onChange(n);
        setDraft(null);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        e.stopPropagation();
      }}
    />
  );
}

/** Whole number that may be unset — cadence is optional, and a blank field
    says "not prescribed" where a 0 would read as a target of zero rpm. */
function OptIntField({
  value,
  onChange,
}: {
  value: number | null;
  onChange: (v: number | null) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <input
      className="b-field"
      style={{ width: 56 }}
      inputMode="numeric"
      placeholder="—"
      value={draft ?? (value === null ? "" : String(value))}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={() => {
        if (draft !== null) {
          const t = draft.trim();
          const n = Number(t);
          onChange(t === "" || !Number.isFinite(n) || n <= 0 ? null : Math.round(n));
        }
        setDraft(null);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setDraft(null);
          e.currentTarget.blur();
        }
        e.stopPropagation();
      }}
    />
  );
}

/** Zone stripe + type pill, shared by every row. */
function RowHead({ kind, pct }: { kind: string; pct: number }) {
  return (
    <>
      <span className="b-zone" style={{ background: zoneColor(pct) }} />
      <span className="b-kind">{kind}</span>
    </>
  );
}

interface RowProps {
  node: LeafNode;
  ftp: number;
  selected: boolean;
  onSelect: () => void;
  onPatch: (patch: NodePatch) => void;
  onDelete: () => void;
}

function LeafRow({ node, ftp, selected, onSelect, onPatch, onDelete }: RowProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: node.id,
  });
  const pct = node.kind === "ramp" ? (node.start_pct + node.end_pct) / 2 : node.power_pct;
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`b-row ${selected ? "sel" : ""} ${isDragging ? "dragging" : ""}`}
      onMouseDown={onSelect}
    >
      <RowHead kind={node.kind === "ramp" ? "RAMP" : "SIMPLE"} pct={pct} />
      <span className="b-grip" {...attributes} {...listeners} title="Drag to reorder">
        ⠿
      </span>

      {selected ? (
        <div className="b-fields">
          <label className="b-lbl">
            duration
            <DurationField value={node.duration_s} onChange={(v) => onPatch({ duration_s: v })} />
          </label>
          {node.kind === "simple" ? (
            <label className="b-lbl">
              intensity
              <PctField
                value={node.power_pct}
                ftp={ftp}
                onChange={(v) => onPatch({ power_pct: v })}
              />
            </label>
          ) : (
            <>
              <label className="b-lbl">
                from
                <PctField
                  value={node.start_pct}
                  ftp={ftp}
                  onChange={(v) => onPatch({ start_pct: v })}
                />
              </label>
              <span className="b-arrow">→</span>
              <label className="b-lbl">
                to
                <PctField
                  value={node.end_pct}
                  ftp={ftp}
                  onChange={(v) => onPatch({ end_pct: v })}
                />
              </label>
            </>
          )}
          <label className="b-lbl">
            cadence
            <OptIntField
              value={node.cadence_rpm}
              onChange={(v) => onPatch({ cadence_rpm: v })}
            />
          </label>
        </div>
      ) : (
        <span className="b-summary">
          {node.kind === "ramp"
            ? `${fmtDuration(node.duration_s)} @ ${node.start_pct}% → ${node.end_pct}% FTP (${wattsFromPct(node.start_pct, ftp)} W → ${wattsFromPct(node.end_pct, ftp)} W)`
            : `${fmtDuration(node.duration_s)} @ ${node.power_pct}% FTP (${wattsFromPct(node.power_pct, ftp)} W)`}
          {node.cadence_rpm ? ` · ${node.cadence_rpm} rpm` : ""}
        </span>
      )}

      <span className="b-dur">{fmtDuration(node.duration_s)}</span>
      <button
        className="b-x"
        title="Delete"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
      >
        ×
      </button>
    </div>
  );
}

function RepeatBlock({
  node,
  ftp,
  selectedId,
  onSelect,
  onPatch,
  onPatchChild,
  onDelete,
  onDeleteChild,
}: {
  node: RepeatNode;
  ftp: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onPatch: (patch: NodePatch) => void;
  onPatchChild: (id: string, patch: NodePatch) => void;
  onDelete: () => void;
  onDeleteChild: (id: string) => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: node.id,
  });
  // The children list is its own drop target, so an interval can be dragged
  // into an empty repeat (a sortable list with no items accepts nothing).
  const { setNodeRef: setDropRef, isOver } = useDroppable({ id: `drop:${node.id}` });
  const selected = selectedId === node.id;

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`b-repeat ${selected ? "sel" : ""} ${isDragging ? "dragging" : ""}`}
    >
      <div className="b-repeat-head" onMouseDown={() => onSelect(node.id)}>
        <span className="b-grip" {...attributes} {...listeners} title="Drag to reorder">
          ⠿
        </span>
        <span className="b-kind">REPEAT</span>
        <label className="b-lbl">
          count
          <IntField
            value={node.count}
            width={48}
            onChange={(v) => onPatch({ count: clampCount(v) })}
          />
        </label>
        <span className="b-summary">
          × ({node.children.map((c) => fmtDuration(c.duration_s)).join(" · ") || "empty"})
        </span>
        <span className="b-dur">{fmtDuration(nodeDuration(node))}</span>
        <button
          className="b-x"
          title="Delete the repeat and everything in it"
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
        >
          ×
        </button>
      </div>

      <div ref={setDropRef} className={`b-children ${isOver ? "over" : ""}`}>
        <SortableContext
          items={node.children.map((c) => c.id)}
          strategy={verticalListSortingStrategy}
        >
          {node.children.map((c) => (
            <LeafRow
              key={c.id}
              node={c}
              ftp={ftp}
              selected={selectedId === c.id}
              onSelect={() => onSelect(c.id)}
              onPatch={(p) => onPatchChild(c.id, p)}
              onDelete={() => onDeleteChild(c.id)}
            />
          ))}
        </SortableContext>
        {node.children.length === 0 && (
          <div className="b-empty-children">Drop an interval here</div>
        )}
      </div>
    </div>
  );
}

export default function Builder() {
  const { settings, go, pushToast, refreshWorkouts } = useStore();
  const ftp = settings?.profile.ftp ?? 250;

  const [name, setName] = useState("");
  const [nodes, setNodes] = useState<BuildNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dragging, setDragging] = useState<BuildNode | null>(null);
  const [saving, setSaving] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));
  const { setNodeRef: setTailRef, isOver: tailOver } = useDroppable({ id: "drop:root" });

  const segments = useMemo(() => expand(nodes), [nodes]);
  const graph = useMemo(() => toGraph(segments), [segments]);
  const rows = useMemo(() => toSegmentRows(segments), [segments]);
  const stats = useMemo(() => estimate(segments, ftp), [segments, ftp]);
  const span = useMemo(() => spanOfNode(nodes, segments, selectedId), [nodes, segments, selectedId]);
  const blocker = saveBlocker(name, nodes);

  const add = useCallback((kind: BuildNode["kind"]) => {
    const node = makeNode(kind);
    setNodes((ns) => [...ns, node]);
    setSelectedId(node.id);
  }, []);

  const patch = useCallback((id: string, p: NodePatch) => {
    setNodes((ns) => updateNode(ns, id, p));
  }, []);

  const del = useCallback((id: string) => {
    setNodes((ns) => removeNode(ns, id));
    setSelectedId((cur) => (cur === id ? null : cur));
  }, []);

  function onDragStart(e: DragStartEvent) {
    const id = String(e.active.id);
    if (id.startsWith("new:")) {
      setDragging(makeNode(id.slice(4) as BuildNode["kind"]));
    } else {
      setDragging(nodes.find((n) => n.id === id) ?? null);
      setSelectedId(id);
    }
  }

  // All the ordering logic lives here, and it only ever computes a
  // (container, index) pair and hands it to the model.
  function onDragEnd(e: DragEndEvent) {
    const activeId = String(e.active.id);
    const overId = e.over ? String(e.over.id) : null;
    setDragging(null);
    if (!overId) return;

    // Where did it land? Either an explicit drop zone or another item.
    let container: string;
    let index: number;
    if (overId === "drop:root") {
      container = ROOT;
      index = -1;
    } else if (overId.startsWith("drop:")) {
      container = overId.slice(5);
      index = -1;
    } else {
      container = containerOf(nodes, overId) ?? ROOT;
      index = itemIds(nodes, container).indexOf(overId);
    }

    if (activeId.startsWith("new:")) {
      const kind = activeId.slice(4) as BuildNode["kind"];
      // A Repeat only makes sense at the top level; land it there rather than
      // silently dropping the gesture.
      const node = makeNode(kind);
      const target = node.kind === "repeat" ? ROOT : container;
      setNodes((ns) => insertNode(ns, node, target, target === container ? index : -1));
      setSelectedId(node.id);
      return;
    }

    if (activeId === overId) return;
    // moveNode removes before inserting, so an index captured from the
    // pre-move list already lands where the pointer is for both directions.
    setNodes((ns) => moveNode(ns, activeId, container, index));
  }

  async function save() {
    if (blocker) {
      pushToast("error", blocker);
      if (!name.trim()) nameRef.current?.focus();
      return;
    }
    setSaving(true);
    try {
      const res = await ipc.createWorkout(toDraft(name, "", nodes));
      await refreshWorkouts();
      for (const w of res.warnings) pushToast("info", w);
      pushToast(
        "info",
        res.already_existed
          ? `“${res.summary.name}” is already in the library — nothing to save`
          : `Saved “${res.summary.name}”`,
      );
      go("library");
    } catch (e) {
      pushToast("error", (e as AppError).message ?? String(e));
    } finally {
      setSaving(false);
    }
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if ((e.key === "Delete" || e.key === "Backspace") && selectedId) {
      e.preventDefault();
      del(selectedId);
    }
  }

  return (
    <div className="screen builder" onKeyDown={onKeyDown} tabIndex={-1}>
      <div className="b-top">
        <h1>Build a Workout</h1>
        <input
          ref={nameRef}
          className="b-name"
          placeholder="Workout name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <div className="spacer" />
        <button onClick={() => go("library")}>Cancel</button>
        <button className="go" onClick={save} disabled={saving || blocker !== null} title={blocker ?? ""}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>

      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        onDragCancel={() => setDragging(null)}
      >
        <div className="b-band">
          <div className="b-palette">
            <span className="b-caption">DRAG OR CLICK TO ADD</span>
            <div className="b-chips">
              {PALETTE.map((p) => (
                <PaletteChip key={p.kind} kind={p.kind} label={p.label} onClick={() => add(p.kind)} />
              ))}
            </div>
          </div>
          <div className="b-stats">
            <div className="stat">
              <span className="stat-value">{fmtDuration(totalDuration(nodes))}</span>
              <span className="stat-label">duration</span>
            </div>
            <div className="stat">
              <span className="stat-value">{stats.tss === null ? "–" : Math.round(stats.tss)}</span>
              <span className="stat-label">TSS</span>
            </div>
            <div className="stat">
              <span className="stat-value">{stats.if_ === null ? "–" : stats.if_.toFixed(2)}</span>
              <span className="stat-label">IF</span>
            </div>
          </div>
        </div>

        <div className="b-canvas">
          <SortableContext items={nodes.map((n) => n.id)} strategy={verticalListSortingStrategy}>
            {nodes.map((n) =>
              n.kind === "repeat" ? (
                <RepeatBlock
                  key={n.id}
                  node={n}
                  ftp={ftp}
                  selectedId={selectedId}
                  onSelect={setSelectedId}
                  onPatch={(p) => patch(n.id, p)}
                  onPatchChild={patch}
                  onDelete={() => del(n.id)}
                  onDeleteChild={del}
                />
              ) : (
                <LeafRow
                  key={n.id}
                  node={n}
                  ftp={ftp}
                  selected={selectedId === n.id}
                  onSelect={() => setSelectedId(n.id)}
                  onPatch={(p) => patch(n.id, p)}
                  onDelete={() => del(n.id)}
                />
              ),
            )}
          </SortableContext>
          <div ref={setTailRef} className={`b-tail ${tailOver ? "over" : ""}`}>
            {nodes.length === 0
              ? "Drag an interval here to start"
              : "Drop an interval here to add it at the end"}
          </div>
        </div>

        <DragOverlay>
          {dragging && (
            <div className="b-row ghost">
              <RowHead
                kind={dragging.kind.toUpperCase()}
                pct={
                  dragging.kind === "simple"
                    ? dragging.power_pct
                    : dragging.kind === "ramp"
                      ? (dragging.start_pct + dragging.end_pct) / 2
                      : 100
                }
              />
              <span className="b-summary">{fmtDuration(nodeDuration(dragging))}</span>
            </div>
          )}
        </DragOverlay>
      </DndContext>

      <div className="b-graph">
        {segments.length > 0 ? (
          <WorkoutGraph
            graph={graph}
            durationS={totalDuration(nodes)}
            height={180}
            segments={rows}
            ftp={ftp}
            highlightRange={span}
          />
        ) : (
          <p className="empty">The power profile appears as you add intervals.</p>
        )}
      </div>
      <p className="muted footnote">
        Click to select · drag to reorder · Delete removes it · a Repeat can hold intervals but
        not another Repeat
      </p>
    </div>
  );
}

/** Palette entry: draggable onto the canvas, clickable to append. */
function PaletteChip({
  kind,
  label,
  onClick,
}: {
  kind: BuildNode["kind"];
  label: string;
  onClick: () => void;
}) {
  const { attributes, listeners, setNodeRef, isDragging } = useSortable({ id: `new:${kind}` });
  return (
    <button
      ref={setNodeRef}
      className={`b-chip ${isDragging ? "dragging" : ""}`}
      onClick={onClick}
      {...attributes}
      {...listeners}
    >
      <span className="b-chip-icon" data-kind={kind} />
      {label}
    </button>
  );
}

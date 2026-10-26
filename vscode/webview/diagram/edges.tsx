// The edges of the diagram: generalizations (hollow triangle at the supertype) and references
// (open arrow at the target, filled diamond at a container; a pair of opposites is one edge with
// a role at each end). Edges attach to the node borders on the line between the node centres,
// so they follow the nodes; several edges between two nodes bend apart.

import { BaseEdge, EdgeLabelRenderer, useInternalNode, type Edge, type EdgeProps, type InternalNode } from "@xyflow/react";
import { memo, useContext } from "react";
import { useUi } from "./host";
import { ActionsContext } from "./nodes";
import { roleLabel, type EdgeData } from "./model";

interface Point {
  x: number;
  y: number;
}

interface Box extends Point {
  w: number;
  h: number;
}

function box(n: InternalNode): Box {
  const w = n.measured.width ?? 0;
  const h = n.measured.height ?? 0;
  return { x: n.internals.positionAbsolute.x + w / 2, y: n.internals.positionAbsolute.y + h / 2, w, h };
}

/** Where the ray from the centre of `b` towards `p` leaves the box. */
function border(b: Box, p: Point): Point {
  const dx = p.x - b.x;
  const dy = p.y - b.y;
  if (dx === 0 && dy === 0) return b;
  const s = Math.min(dx ? b.w / 2 / Math.abs(dx) : Infinity, dy ? b.h / 2 / Math.abs(dy) : Infinity);
  return { x: b.x + dx * s, y: b.y + dy * s };
}

const unit = (p: Point): Point => {
  const l = Math.hypot(p.x, p.y) || 1;
  return { x: p.x / l, y: p.y / l };
};

interface Geometry {
  path: string;
  start: Point;
  end: Point;
  /** Directions from the ends into the edge. */
  startDir: Point;
  endDir: Point;
  /** The outermost point of a loop (for its label). */
  apex: Point;
}

function geometry(s: Box, t: Box, offset: number, self: boolean): Geometry {
  if (self) {
    const k = Math.abs(offset) + (offset === 0 ? 0 : 0.5);
    const start = { x: s.x + s.w / 2 - 18 - 14 * k, y: s.y - s.h / 2 };
    const end = { x: s.x + s.w / 2, y: s.y - s.h / 2 + 18 + 14 * k };
    const reach = 36 + 16 * k;
    return {
      path: `M ${start.x} ${start.y} C ${start.x} ${start.y - reach}, ${end.x + reach} ${end.y}, ${end.x} ${end.y}`,
      start,
      end,
      startDir: { x: 0, y: -1 },
      endDir: { x: 1, y: 0 },
      apex: { x: end.x + reach * 0.45, y: start.y - reach * 0.45 },
    };
  }
  if (offset === 0) {
    const start = border(s, t);
    const end = border(t, s);
    const d = unit({ x: end.x - start.x, y: end.y - start.y });
    return { path: `M ${start.x} ${start.y} L ${end.x} ${end.y}`, start, end, startDir: d, endDir: { x: -d.x, y: -d.y }, apex: start };
  }
  const d = unit({ x: t.x - s.x, y: t.y - s.y });
  const bend = 40 * offset;
  const control = { x: (s.x + t.x) / 2 - d.y * bend, y: (s.y + t.y) / 2 + d.x * bend };
  const start = border(s, control);
  const end = border(t, control);
  return {
    path: `M ${start.x} ${start.y} Q ${control.x} ${control.y} ${end.x} ${end.y}`,
    start,
    end,
    startDir: unit({ x: control.x - start.x, y: control.y - start.y }),
    endDir: unit({ x: control.x - end.x, y: control.y - end.y }),
    apex: control,
  };
}

/** A label next to an edge end: a little along the edge and to its side, aligned away from it. */
function EndLabel(props: { at: Point; dir: Point; text: string; id: string; className?: string }) {
  const side = { x: -props.dir.y, y: props.dir.x };
  const p = { x: props.at.x + props.dir.x * 16 + side.x * 8, y: props.at.y + props.dir.y * 16 + side.y * 8 };
  const align = (v: number) => (v > 0.35 ? 0 : v < -0.35 ? -100 : -50);
  const actions = useContext(ActionsContext);
  return (
    <div
      className={`edge-label nodrag nopan ${props.className ?? ""}`}
      data-element={props.id}
      onClick={() => actions?.focus(props.id)}
      style={{ transform: `translate(${align(side.x)}%, ${align(side.y)}%) translate(${p.x}px, ${p.y}px)` }}
    >
      {props.text}
    </div>
  );
}

export const DiagramEdge = memo(function DiagramEdge({ id, source, target, data, selected }: EdgeProps<Edge<EdgeData>>) {
  const s = useInternalNode(source);
  const t = useInternalNode(target);
  const focused = useUi((u) => u.focused);
  if (!s || !t || !data || s.className?.includes("unplaced") || t.className?.includes("unplaced")) return null;
  const g = geometry(box(s), box(t), data.offset, source === target);
  const ref = data.reference;
  const opposite = data.opposite;
  const active = selected || focused === id || (!!ref && focused === ref.id) || (!!opposite && focused === opposite.id);
  const state = active ? "-active" : "";
  let markerStart: string | undefined;
  let markerEnd: string | undefined;
  if (data.kind === "generalization") {
    markerEnd = `url(#oxy-triangle${state})`;
  } else if (ref) {
    if (ref.containment) markerStart = `url(#oxy-diamond${state})`;
    if (!opposite) markerEnd = `url(#oxy-arrow${state})`;
    else if (opposite.containment) markerEnd = `url(#oxy-diamond${state})`;
  }
  return (
    <>
      <BaseEdge
        id={id}
        path={g.path}
        markerStart={markerStart}
        markerEnd={markerEnd}
        interactionWidth={14}
        className={`diagram-edge ${data.kind}${active ? " active" : ""}${ref?.derived ? " derived" : ""}`}
      />
      {ref && source === target && (
        <EdgeLabelRenderer>
          <EndLabel at={g.apex} dir={{ x: 0.7, y: -0.7 }} text={roleLabel(ref, data.syntax)} id={ref.id} className={focused === ref.id ? "focused" : undefined} />
        </EdgeLabelRenderer>
      )}
      {ref && source !== target && (
        <EdgeLabelRenderer>
          <EndLabel at={g.end} dir={g.endDir} text={roleLabel(ref, data.syntax)} id={ref.id} className={focused === ref.id ? "focused" : undefined} />
          {opposite && (
            <EndLabel
              at={g.start}
              dir={g.startDir}
              text={roleLabel(opposite, data.syntax)}
              id={opposite.id}
              className={focused === opposite.id ? "focused" : undefined}
            />
          )}
        </EdgeLabelRenderer>
      )}
    </>
  );
});

export const edgeTypes = { diagram: DiagramEdge };

/** The markers of the edges, once per page (normal and highlighted). */
export function Markers() {
  const marker = (name: string, active: boolean, path: string, filled: boolean) => (
    <marker
      key={name + active}
      id={`oxy-${name}${active ? "-active" : ""}`}
      viewBox="-16 -9 18 18"
      markerWidth="18"
      markerHeight="18"
      refX="0"
      refY="0"
      orient="auto-start-reverse"
      markerUnits="userSpaceOnUse"
    >
      <path d={path} className={`marker${active ? " active" : ""}${filled ? " filled" : ""}`} />
    </marker>
  );
  // The markers point along +x with their tip at the origin (the edge end).
  const shapes: [string, string, boolean][] = [
    ["triangle", "M 0 0 L -13 -7 L -13 7 Z", false],
    ["arrow", "M -8 -5 L 0 0 L -8 5", false],
    ["diamond", "M 0 0 L -7 -4.5 L -14 0 L -7 4.5 Z", true],
  ];
  return (
    <svg className="markers" aria-hidden="true">
      <defs>{[false, true].flatMap((active) => shapes.map(([n, p, f]) => marker(n, active, p, f)))}</defs>
    </svg>
  );
}

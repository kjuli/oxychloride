// The edges of rules, as Henshin draws them: a line in the color of the edge's action with a
// filled arrow at the target, labeled with its stereotype (where it differs from «preserve»)
// and its reference. Edges attach to the node borders on the line between the node centres;
// several edges between two nodes bend apart.

import { BaseEdge, EdgeLabelRenderer, useInternalNode, type Edge, type EdgeProps, type InternalNode } from "@xyflow/react";
import { memo, useContext } from "react";
import type { RuleEdgeDiagram } from "../../src/engine";
import { useUi } from "./host";
import { ACTIONS, stereotype } from "./model";
import { ActionsContext } from "./nodes";

export interface RuleEdgeData extends Record<string, unknown> {
  edge: RuleEdgeDiagram;
  /** Bend: 0 for the only edge between two nodes, else ±1, ±2, …. */
  offset: number;
  problems?: string[];
}

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

function geometry(s: Box, t: Box, offset: number, self: boolean): { path: string; label: Point } {
  if (self) {
    const k = Math.abs(offset);
    const start = { x: s.x + s.w / 2 - 18 - 14 * k, y: s.y - s.h / 2 };
    const end = { x: s.x + s.w / 2, y: s.y - s.h / 2 + 18 + 14 * k };
    const reach = 40 + 16 * k;
    return {
      path: `M ${start.x} ${start.y} C ${start.x} ${start.y - reach}, ${end.x + reach} ${end.y}, ${end.x} ${end.y}`,
      label: { x: end.x + reach * 0.5, y: start.y - reach * 0.5 },
    };
  }
  if (offset === 0) {
    const start = border(s, t);
    const end = border(t, s);
    return { path: `M ${start.x} ${start.y} L ${end.x} ${end.y}`, label: { x: (start.x + end.x) / 2, y: (start.y + end.y) / 2 } };
  }
  const len = Math.hypot(t.x - s.x, t.y - s.y) || 1;
  const d = { x: (t.x - s.x) / len, y: (t.y - s.y) / len };
  const bend = 46 * offset;
  const control = { x: (s.x + t.x) / 2 - d.y * bend, y: (s.y + t.y) / 2 + d.x * bend };
  const start = border(s, control);
  const end = border(t, control);
  // The middle of the quadratic curve.
  const label = { x: 0.25 * start.x + 0.5 * control.x + 0.25 * end.x, y: 0.25 * start.y + 0.5 * control.y + 0.25 * end.y };
  return { path: `M ${start.x} ${start.y} Q ${control.x} ${control.y} ${end.x} ${end.y}`, label };
}

export const RuleEdge = memo(function RuleEdge({ id, source, target, data, selected }: EdgeProps<Edge<RuleEdgeData>>) {
  const s = useInternalNode(source);
  const t = useInternalNode(target);
  const focused = useUi((u) => u.focused === id);
  const actions = useContext(ActionsContext);
  if (!s || !t || !data || s.className?.includes("unplaced") || t.className?.includes("unplaced")) return null;
  const e = data.edge;
  const g = geometry(box(s), box(t), data.offset, source === target);
  const active = selected || focused;
  const marked = e.action !== "preserve" || e.multi || e.condition;
  return (
    <>
      <BaseEdge
        id={id}
        path={g.path}
        markerEnd={`url(#henshin-arrow-${e.action}${active ? "-active" : ""})`}
        interactionWidth={14}
        className={`rule-edge action-${e.action}${active ? " active" : ""}${data.problems ? " has-problems" : ""}`}
      />
      <EdgeLabelRenderer>
        <div
          className={`rule-edge-label nodrag nopan action-${e.action}${focused ? " focused" : ""}`}
          style={{ transform: `translate(-50%, -50%) translate(${g.label.x}px, ${g.label.y}px)` }}
          onClick={() => actions?.focus(id)}
          title={data.problems?.join("\n")}
        >
          {marked && <span className="stereotype">{stereotype(e.action, e.multi, e.condition)}</span>}
          <span className="reference">{e.reference}</span>
        </div>
      </EdgeLabelRenderer>
    </>
  );
});

export const edgeTypes = { rule: RuleEdge };

/** The arrow heads, one per action (and highlighted), once per page. */
export function Markers() {
  return (
    <svg className="markers" aria-hidden="true">
      <defs>
        {[false, true].flatMap((active) =>
          ACTIONS.map((a) => (
            <marker
              key={a + active}
              id={`henshin-arrow-${a}${active ? "-active" : ""}`}
              viewBox="-12 -6 13 12"
              markerWidth="13"
              markerHeight="12"
              refX="0"
              refY="0"
              orient="auto-start-reverse"
              markerUnits="userSpaceOnUse"
            >
              <path d="M 0 0 L -11 -4.5 L -11 4.5 Z" className={`henshin-arrow action-${a}${active ? " active" : ""}`} />
            </marker>
          )),
        )}
      </defs>
    </svg>
  );
}

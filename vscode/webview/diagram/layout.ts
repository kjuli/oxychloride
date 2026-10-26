// Placing classifiers: the whole diagram with ELK's layered layout (supertypes above their
// subtypes, containers above their contents), and classifiers that are new to a laid-out diagram
// next to what they are connected to, where there is room.

import ELK from "elkjs/lib/elk.bundled.js";
import type { Layout } from "../../src/diagramProtocol";

export interface Sized {
  id: string;
  width: number;
  height: number;
}

export interface LayoutEdge {
  /** The upper end. */
  from: string;
  to: string;
}

const elk = new ELK();

export async function layoutAll(nodes: Sized[], edges: LayoutEdge[]): Promise<Layout> {
  const ids = new Set(nodes.map((n) => n.id));
  const graph = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "DOWN",
      "elk.layered.spacing.nodeNodeBetweenLayers": "100",
      "elk.spacing.nodeNode": "72",
      "elk.spacing.componentComponent": "72",
      "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
      "elk.layered.cycleBreaking.strategy": "MODEL_ORDER",
      "elk.separateConnectedComponents": "true",
    },
    children: nodes.map((n) => ({ id: n.id, width: n.width, height: n.height })),
    edges: edges
      .filter((e) => e.from !== e.to && ids.has(e.from) && ids.has(e.to))
      .map((e, i) => ({ id: `e${i}`, sources: [e.from], targets: [e.to] })),
  };
  const result = await elk.layout(graph);
  const out: Layout = {};
  for (const c of result.children ?? []) out[c.id] = { x: c.x ?? 0, y: c.y ?? 0 };
  return out;
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const GAP = 48;

const overlaps = (a: Rect, b: Rect) =>
  a.x < b.x + b.width + GAP / 2 && b.x < a.x + a.width + GAP / 2 && a.y < b.y + b.height + GAP / 2 && b.y < a.y + a.height + GAP / 2;

/** Positions for `fresh` classifiers among `placed` ones: next to a placed neighbour, else to the
 * right of everything; moved down until they overlap nothing. */
export function placeNew(fresh: Sized[], placed: (Sized & { x: number; y: number })[], edges: LayoutEdge[]): Layout {
  const rects: Rect[] = placed.map((p) => ({ ...p }));
  const byId = new Map(placed.map((p) => [p.id, p]));
  const right = rects.reduce((m, r) => Math.max(m, r.x + r.width), 0);
  const top = rects.reduce((m, r) => Math.min(m, r.y), Infinity);
  const out: Layout = {};
  for (const n of fresh) {
    const neighbour = edges
      .flatMap((e) => (e.from === n.id ? [e.to] : e.to === n.id ? [e.from] : []))
      .map((id) => byId.get(id))
      .find((p) => p !== undefined);
    const start: Rect = neighbour
      ? { x: neighbour.x, y: neighbour.y + neighbour.height + GAP * 1.5, width: n.width, height: n.height }
      : { x: right + GAP, y: Number.isFinite(top) ? top : 0, width: n.width, height: n.height };
    // Beside the start position (alternately right and left), else further down.
    let candidate = start;
    search: for (let row = 0; row < 50; row++) {
      for (const k of [0, 1, -1, 2, -2, 3]) {
        const c = { ...start, x: start.x + k * (n.width + GAP), y: start.y + row * 40 };
        if (!rects.some((r) => overlaps(r, c))) {
          candidate = c;
          break search;
        }
      }
    }
    out[n.id] = { x: candidate.x, y: candidate.y };
    rects.push(candidate);
    byId.set(n.id, { ...n, x: candidate.x, y: candidate.y });
  }
  return out;
}

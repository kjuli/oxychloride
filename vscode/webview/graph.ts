// A graph drawn as SVG: an incremental force-directed layout (positions are kept between
// renders, so a graph that changes a little keeps its shape), with pan, zoom and dragging.
// Used for object diagrams (the states of a transformation) and for state spaces.

const SVG = "http://www.w3.org/2000/svg";

export function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number | undefined> = {},
  parent?: Element,
): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) if (v !== undefined) e.setAttribute(k, String(v));
  parent?.append(e);
  return e;
}

let measureContext: CanvasRenderingContext2D | null = null;

/** The width of a text in the given font (CSS shorthand). */
export function textWidth(text: string, font: string): number {
  measureContext ??= document.createElement("canvas").getContext("2d");
  if (!measureContext) return text.length * 7;
  measureContext.font = font;
  return measureContext.measureText(text).width;
}

export interface GraphNodeView {
  id: string;
  width: number;
  height: number;
  shape: "rect" | "circle";
  /** A layer (e.g. containment depth or distance from the initial state): lower is higher up. */
  rank?: number;
  classes?: string[];
  title?: string;
  /** Draws the node's content into `g`, centred on (0, 0). */
  draw(g: SVGGElement): void;
  /** Keeps its last position (other nodes keep clear of it): e.g. deleted nodes. */
  ghost?: boolean;
}

export interface GraphEdgeView {
  source: string;
  target: string;
  label?: string;
  classes?: string[];
  title?: string;
  /** A filled diamond at the source (containment). */
  diamond?: boolean;
  /** No arrowhead. */
  undirected?: boolean;
}

interface Point {
  x: number;
  y: number;
}

/** Positions kept across renders. */
class Layout {
  readonly positions = new Map<string, Point>();
  readonly pinned = new Set<string>();
  /** Nodes keep to the rows of their ranks. */
  private rowed = false;

  run(nodes: GraphNodeView[], edges: GraphEdgeView[], rankGap: number, rankStrength: number): void {
    // Ghosts keep their positions, but the other nodes keep clear of them.
    const live = nodes;
    const fixed = (n: GraphNodeView) => this.pinned.has(n.id) || (!!n.ghost && this.positions.has(n.id));
    const index = new Map(live.map((n, i) => [n.id, i]));
    const links = edges
      .map((e) => [index.get(e.source), index.get(e.target)] as const)
      .filter((l): l is readonly [number, number] => l[0] !== undefined && l[1] !== undefined && l[0] !== l[1]);
    const known = live.filter((n) => this.positions.has(n.id)).length;
    const neighbours = live.map(() => [] as number[]);
    for (const [a, b] of links) {
      neighbours[a].push(b);
      neighbours[b].push(a);
    }
    // New nodes start next to positioned neighbours, else on a spiral.
    let spiral = known;
    for (let pass = 0; pass < 3; pass++) {
      live.forEach((n, i) => {
        if (this.positions.has(n.id)) return;
        const placed = neighbours[i].map((j) => this.positions.get(live[j].id)).filter((p): p is Point => !!p);
        if (placed.length === 0 && pass < 2) return;
        if (placed.length) {
          const c = placed.reduce((s, p) => ({ x: s.x + p.x / placed.length, y: s.y + p.y / placed.length }), { x: 0, y: 0 });
          const angle = (i * 2.399) % (2 * Math.PI);
          this.positions.set(n.id, { x: c.x + Math.cos(angle) * 90, y: c.y + Math.sin(angle) * 70 + (n.rank !== undefined ? 30 : 0) });
        } else {
          const angle = spiral * 2.399;
          const r = 40 * Math.sqrt(spiral + 1);
          spiral++;
          this.positions.set(n.id, { x: Math.cos(angle) * r * 1.6, y: n.rank !== undefined ? n.rank * rankGap : Math.sin(angle) * r });
        }
      });
    }
    this.rowed = rankStrength >= 0.4;
    const p = live.map((n) => this.positions.get(n.id)!);
    const radius = live.map((n) => (n.shape === "circle" ? n.width / 2 : Math.hypot(n.width, n.height) / 2.4));
    const warm = known >= live.length * 0.6 && known > 0;
    const big = live.length > 350;
    const iterations = big ? 60 : warm ? 90 : 320;
    const meanRadius = radius.reduce((s, r) => s + r, 0) / Math.max(1, radius.length);
    const k = 70 + meanRadius;
    let temperature = warm ? 25 : 120;
    for (let it = 0; it < iterations; it++) {
      const dx = new Float64Array(live.length);
      const dy = new Float64Array(live.length);
      if (!big) {
        for (let i = 0; i < live.length; i++) {
          for (let j = i + 1; j < live.length; j++) {
            let x = p[i].x - p[j].x;
            let y = p[i].y - p[j].y;
            if (x === 0 && y === 0) {
              x = Math.random() - 0.5;
              y = Math.random() - 0.5;
            }
            const d = Math.max(1, Math.hypot(x, y));
            const gap = Math.max(1, d - radius[i] - radius[j]);
            const f = (k * k) / (gap * 6 + d) + (gap < 20 ? 40 : 0);
            dx[i] += (x / d) * f;
            dy[i] += (y / d) * f;
            dx[j] -= (x / d) * f;
            dy[j] -= (y / d) * f;
          }
        }
      }
      for (const [a, b] of links) {
        const x = p[b].x - p[a].x;
        const y = p[b].y - p[a].y;
        const d = Math.max(1, Math.hypot(x, y));
        const rest = k + radius[a] + radius[b];
        const f = ((d - rest) * Math.abs(d - rest)) / (k * 6);
        dx[a] += (x / d) * f;
        dy[a] += (y / d) * f;
        dx[b] -= (x / d) * f;
        dy[b] -= (y / d) * f;
      }
      live.forEach((n, i) => {
        if (n.rank !== undefined) dy[i] += (n.rank * rankGap - p[i].y) * rankStrength;
        dx[i] -= p[i].x * 0.004;
        if (n.rank === undefined) dy[i] -= p[i].y * 0.004;
        if (fixed(n)) return;
        const d = Math.hypot(dx[i], dy[i]);
        if (d > 0) {
          const step = Math.min(d, temperature);
          p[i].x += (dx[i] / d) * step;
          p[i].y += (dy[i] / d) * step;
        }
      });
      temperature = Math.max(2, temperature * 0.975);
    }
    if (big) this.layered(live, links, p, rankGap);
    this.separate(live, p);
  }

  /** Large graphs: ranks as rows, ordered by the neighbours' positions. */
  private layered(nodes: GraphNodeView[], links: (readonly [number, number])[], p: Point[], rankGap: number): void {
    const rows = new Map<number, number[]>();
    nodes.forEach((n, i) => {
      const r = n.rank ?? 0;
      if (!rows.has(r)) rows.set(r, []);
      rows.get(r)!.push(i);
    });
    const preds = nodes.map(() => [] as number[]);
    for (const [a, b] of links) preds[b].push(a);
    for (const r of [...rows.keys()].sort((a, b) => a - b)) {
      const row = rows.get(r)!;
      const key = (i: number) => (preds[i].length ? preds[i].reduce((s, j) => s + p[j].x, 0) / preds[i].length : p[i].x);
      row.sort((a, b) => key(a) - key(b));
      const width = row.reduce((s, i) => s + nodes[i].width + 24, 0);
      let x = -width / 2;
      for (const i of row) {
        p[i].x = x + nodes[i].width / 2;
        p[i].y = r * rankGap;
        x += nodes[i].width + 24;
      }
    }
  }

  /** Pushes overlapping boxes apart. */
  private separate(nodes: GraphNodeView[], p: Point[]): void {
    const margin = 14;
    for (let pass = 0; pass < 40; pass++) {
      let moved = false;
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const ox = (nodes[i].width + nodes[j].width) / 2 + margin - Math.abs(p[i].x - p[j].x);
          const oy = (nodes[i].height + nodes[j].height) / 2 + margin - Math.abs(p[i].y - p[j].y);
          if (ox <= 0 || oy <= 0) continue;
          moved = true;
          const pi = this.pinned.has(nodes[i].id) || !!nodes[i].ghost;
          const pj = this.pinned.has(nodes[j].id) || !!nodes[j].ghost;
          if (pi && pj) continue;
          const share = pi && !pj ? 0 : pj && !pi ? 1 : 0.5;
          if (ox < oy && (!this.rowed || nodes[i].rank === undefined)) {
            const s = p[i].x < p[j].x ? -1 : 1;
            p[i].x += s * ox * share;
            p[j].x -= s * ox * (1 - share);
          } else if (this.rowed && nodes[i].rank !== undefined && nodes[i].rank === nodes[j].rank) {
            const s = p[i].x < p[j].x ? -1 : 1;
            p[i].x += s * ox * share;
            p[j].x -= s * ox * (1 - share);
          } else {
            const s = p[i].y < p[j].y ? -1 : 1;
            p[i].y += s * oy * share;
            p[j].y -= s * oy * (1 - share);
          }
        }
      }
      if (!moved) break;
    }
  }
}

export interface GraphViewOptions {
  rankGap?: number;
  /** How strongly nodes are pulled to the row of their rank (0: not at all, 1: rows). */
  rankStrength?: number;
  onNodeClick?(id: string, event: MouseEvent): void;
  onNodeDoubleClick?(id: string): void;
  onNodeHover?(id: string | undefined): void;
  onBackgroundClick?(): void;
  /** Shown when there is nothing to draw. */
  empty?: string;
}

let views = 0;

/** An SVG drawing of a graph in `container`. */
export class GraphView {
  readonly element: SVGSVGElement;
  private readonly viewport: SVGGElement;
  private readonly edgeLayer: SVGGElement;
  private readonly nodeLayer: SVGGElement;
  private readonly emptyText: SVGTextElement;
  private readonly layout = new Layout();
  private nodes: GraphNodeView[] = [];
  private edges: GraphEdgeView[] = [];
  private nodeElements = new Map<string, SVGGElement>();
  private scale = 1;
  private tx = 0;
  private ty = 0;
  private fitted = false;
  /** Marker ids are document-wide: each view has its own. */
  private readonly prefix = `g${++views}`;

  constructor(
    private readonly container: HTMLElement,
    private readonly options: GraphViewOptions = {},
  ) {
    this.element = svg("svg", { class: "graph", tabindex: 0 });
    const defs = svg("defs", {}, this.element);
    for (const kind of ["default", "created", "deleted", "highlight", "path"]) {
      const marker = svg("marker", { id: `${this.prefix}-arrow-${kind}`, viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" }, defs);
      svg("path", { d: "M 0 0 L 10 5 L 0 10 z", class: `marker marker-${kind}` }, marker);
      const diamond = svg("marker", { id: `${this.prefix}-diamond-${kind}`, viewBox: "0 0 12 8", refX: 1, refY: 4, markerWidth: 11, markerHeight: 7, orient: "auto" }, defs);
      svg("path", { d: "M 0 4 L 6 0 L 12 4 L 6 8 z", class: `marker marker-${kind}` }, diamond);
    }
    this.viewport = svg("g", {}, this.element);
    this.edgeLayer = svg("g", { class: "edges" }, this.viewport);
    this.nodeLayer = svg("g", { class: "nodes" }, this.viewport);
    this.emptyText = svg("text", { class: "empty", x: "50%", y: "50%", "text-anchor": "middle" }, this.element);
    container.append(this.element);
    this.interact();
    new ResizeObserver(() => {
      if (!this.fitted && this.nodes.length) this.fit();
    }).observe(container);
  }

  private apply(): void {
    this.viewport.setAttribute("transform", `translate(${this.tx},${this.ty}) scale(${this.scale})`);
  }

  private interact(): void {
    const el = this.element;
    el.addEventListener("wheel", (e) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015));
      const scale = Math.min(4, Math.max(0.1, this.scale * factor));
      this.tx = mx - ((mx - this.tx) * scale) / this.scale;
      this.ty = my - ((my - this.ty) * scale) / this.scale;
      this.scale = scale;
      this.fitted = true;
      this.apply();
    }, { passive: false });
    let drag: { kind: "pan" | "node"; id?: string; x: number; y: number; moved: boolean } | undefined;
    el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      const node = (e.target as Element).closest<SVGGElement>("g.node");
      const id = node?.dataset.id;
      drag = { kind: id && !node.classList.contains("ghost") ? "node" : "pan", id, x: e.clientX, y: e.clientY, moved: false };
      el.setPointerCapture(e.pointerId);
    });
    el.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < 3) return;
      drag.moved = true;
      drag.x = e.clientX;
      drag.y = e.clientY;
      if (drag.kind === "pan") {
        this.tx += dx;
        this.ty += dy;
        this.fitted = true;
        this.apply();
      } else if (drag.id) {
        const p = this.layout.positions.get(drag.id);
        if (!p) return;
        p.x += dx / this.scale;
        p.y += dy / this.scale;
        this.layout.pinned.add(drag.id);
        this.place(drag.id);
        this.drawEdges();
      }
    });
    el.addEventListener("pointerup", (e) => {
      const d = drag;
      drag = undefined;
      if (!d || d.moved) return;
      if (d.id) this.options.onNodeClick?.(d.id, e);
      else this.options.onBackgroundClick?.();
    });
    el.addEventListener("dblclick", (e) => {
      const id = (e.target as Element).closest<SVGGElement>("g.node")?.dataset.id;
      if (id) this.options.onNodeDoubleClick?.(id);
      else this.fit();
    });
    el.addEventListener("pointerover", (e) => {
      const id = (e.target as Element).closest<SVGGElement>("g.node")?.dataset.id;
      this.options.onNodeHover?.(id);
    });
    el.addEventListener("pointerleave", () => this.options.onNodeHover?.(undefined));
  }

  /** Draws a (new version of the) graph. */
  render(nodes: GraphNodeView[], edges: GraphEdgeView[]): void {
    this.nodes = nodes;
    this.edges = edges;
    this.layout.run(nodes, edges, this.options.rankGap ?? 110, this.options.rankStrength ?? 0.6);
    this.nodeLayer.replaceChildren();
    this.nodeElements.clear();
    for (const n of nodes) {
      if (!this.layout.positions.has(n.id)) continue;
      const g = svg("g", { class: ["node", n.shape, ...(n.classes ?? []), n.ghost ? "ghost" : ""].join(" ").trim() }, this.nodeLayer);
      g.dataset.id = n.id;
      if (n.title) svg("title", {}, g).textContent = n.title;
      n.draw(g);
      this.nodeElements.set(n.id, g);
      this.place(n.id);
    }
    this.drawEdges();
    this.emptyText.textContent = nodes.length ? "" : (this.options.empty ?? "");
    if (!this.fitted) this.fit();
  }

  /** Forgets positions (the next render lays the graph out anew). */
  reset(): void {
    this.layout.positions.clear();
    this.layout.pinned.clear();
    this.fitted = false;
  }

  /** Updates the classes of drawn nodes without a new layout. */
  setClasses(id: string, classes: Record<string, boolean>): void {
    const g = this.nodeElements.get(id);
    if (!g) return;
    for (const [c, on] of Object.entries(classes)) g.classList.toggle(c, on);
  }

  private place(id: string): void {
    const p = this.layout.positions.get(id);
    this.nodeElements.get(id)?.setAttribute("transform", `translate(${p?.x ?? 0},${p?.y ?? 0})`);
  }

  /** Where the line from the centre of `n` towards `to` leaves the node. */
  private border(n: GraphNodeView, from: Point, to: Point): Point {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    if (dx === 0 && dy === 0) return from;
    if (n.shape === "circle") {
      const d = Math.hypot(dx, dy);
      return { x: from.x + (dx / d) * (n.width / 2 + 1), y: from.y + (dy / d) * (n.width / 2 + 1) };
    }
    const t = Math.min(n.width / 2 / Math.abs(dx || 1e-9), n.height / 2 / Math.abs(dy || 1e-9));
    return { x: from.x + dx * t, y: from.y + dy * t };
  }

  private drawEdges(): void {
    this.edgeLayer.replaceChildren();
    const byId = new Map(this.nodes.map((n) => [n.id, n]));
    // Parallel edges are bent apart.
    const groups = new Map<string, GraphEdgeView[]>();
    for (const e of this.edges) {
      const key = e.source < e.target ? `${e.source}\n${e.target}` : `${e.target}\n${e.source}`;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(e);
    }
    for (const group of groups.values()) {
      group.forEach((e, i) => {
        const s = byId.get(e.source);
        const t = byId.get(e.target);
        const ps = this.layout.positions.get(e.source);
        const pt = this.layout.positions.get(e.target);
        if (!s || !t || !ps || !pt) return;
        const classes = e.classes ?? [];
        const kind = ["created", "deleted", "path", "highlight"].find((c) => classes.includes(c)) ?? "default";
        const g = svg("g", { class: ["edge", ...classes].join(" ") }, this.edgeLayer);
        if (e.title) svg("title", {}, g).textContent = e.title;
        let d: string;
        let label: Point;
        if (e.source === e.target) {
          const top = { x: ps.x + s.width / 4, y: ps.y - s.height / 2 };
          const r = 22 + i * 10;
          d = `M ${top.x - 8} ${top.y} C ${top.x - 8} ${top.y - r * 1.6}, ${top.x + r * 1.4} ${top.y - r * 1.6}, ${top.x + 10} ${top.y + 1}`;
          label = { x: top.x + 2, y: top.y - r * 1.25 };
        } else {
          const offset = (i - (group.length - 1) / 2) * 26 * (e.source < e.target ? 1 : -1);
          const mx = (ps.x + pt.x) / 2;
          const my = (ps.y + pt.y) / 2;
          const len = Math.max(1, Math.hypot(pt.x - ps.x, pt.y - ps.y));
          const c = { x: mx + (-(pt.y - ps.y) / len) * offset, y: my + ((pt.x - ps.x) / len) * offset };
          const a = this.border(s, ps, offset ? c : pt);
          const b = this.border(t, pt, offset ? c : ps);
          d = offset ? `M ${a.x} ${a.y} Q ${c.x} ${c.y} ${b.x} ${b.y}` : `M ${a.x} ${a.y} L ${b.x} ${b.y}`;
          label = offset ? { x: (a.x + 2 * c.x + b.x) / 4, y: (a.y + 2 * c.y + b.y) / 4 } : { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        }
        svg("path", {
          d,
          class: "line",
          "marker-end": e.undirected ? undefined : `url(#${this.prefix}-arrow-${kind})`,
          "marker-start": e.diamond ? `url(#${this.prefix}-diamond-${kind})` : undefined,
        }, g);
        // A wide transparent stroke makes thin edges easy to hover.
        svg("path", { d, class: "hit" }, g);
        if (e.label) {
          const w = textWidth(e.label, "11px sans-serif") + 8;
          svg("rect", { x: label.x - w / 2, y: label.y - 8, width: w, height: 15, rx: 3, class: "label-bg" }, g);
          svg("text", { x: label.x, y: label.y + 3.5, "text-anchor": "middle", class: "label" }, g).textContent = e.label;
        }
      });
    }
  }

  /** Zooms to show the whole graph. */
  fit(): void {
    const rect = this.container.getBoundingClientRect();
    const live = this.nodes.filter((n) => this.layout.positions.has(n.id));
    if (!live.length || rect.width === 0) return;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const n of live) {
      const p = this.layout.positions.get(n.id)!;
      minX = Math.min(minX, p.x - n.width / 2);
      maxX = Math.max(maxX, p.x + n.width / 2);
      minY = Math.min(minY, p.y - n.height / 2 - 30);
      maxY = Math.max(maxY, p.y + n.height / 2);
    }
    const pad = 30;
    this.scale = Math.min(1.4, (rect.width - 2 * pad) / Math.max(1, maxX - minX), (rect.height - 2 * pad) / Math.max(1, maxY - minY));
    this.tx = rect.width / 2 - ((minX + maxX) / 2) * this.scale;
    this.ty = rect.height / 2 - ((minY + maxY) / 2) * this.scale;
    this.fitted = true;
    this.apply();
  }

  /** Moves the view so that a node is visible (keeping the zoom). */
  reveal(id: string): void {
    const p = this.layout.positions.get(id);
    if (!p) return;
    const rect = this.container.getBoundingClientRect();
    const x = p.x * this.scale + this.tx;
    const y = p.y * this.scale + this.ty;
    if (x > 40 && y > 40 && x < rect.width - 40 && y < rect.height - 40) return;
    this.tx = rect.width / 2 - p.x * this.scale;
    this.ty = rect.height / 2 - p.y * this.scale;
    this.apply();
  }
}

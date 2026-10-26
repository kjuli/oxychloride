// The rule diagram (runs in the webview; src/ruleDiagram.ts is its host).
//
// The diagram is drawn from the engine's TransformationDiagram of the document's text (a Henshin
// text or a `.henshin` file), again after every change: each rule is a frame with its nodes
// inside (nodes of multi-rules and condition graphs among them, marked by their stereotypes),
// each unit a frame with its control flow. Edits made here are sent to the host, which turns
// them into text edits; the changed text comes back as a new diagram. Positions are kept by
// element id (frames absolute, nodes relative to their rule) and written to the layout file
// when the user lays the diagram out; elements without a position are placed automatically
// (a rule's nodes with ELK, frames below each other, units to the right).

import {
  Background,
  BackgroundVariant,
  ConnectionMode,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  applyEdgeChanges,
  applyNodeChanges,
  useNodesInitialized,
  useReactFlow,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
  type XYPosition,
} from "@xyflow/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RuleDiagram, TransformationDiagram, TransformationEdit } from "../../src/engine";
import type { Layout, PairInfo } from "../../src/ruleDiagramProtocol";
import { layoutAll, placeNew, type Sized } from "../diagram/layout";
import { edgeTypes, Markers, type RuleEdgeData } from "./edges";
import { edit as requestEdit, edited, onMessage, post, save, saved, ui, useUi } from "./host";
import { drawnIn, elementAt, find, frameId, lowerFirst, parseAttribute, references, ruleOfScope, scopeOf, uniqueName, unitId } from "./model";
import { ActionsContext, frameTop, nodeTypes, type Actions, type Problems } from "./nodes";
import { Properties } from "./properties";
import { Toolbar } from "./toolbar";

export function App() {
  return (
    <ReactFlowProvider>
      <Diagram />
    </ReactFlowProvider>
  );
}

type Notice = { kind: "error" | "info"; message: string };

/** Space around a rule's nodes in its frame, and between frames. */
const PAD = 24;
const GAP = 48;
/** How wide a row of rule frames gets before the next row starts. */
const ROW_WIDTH = 1500;

const size = (n: Node): Sized => ({ id: n.id, width: n.measured?.width ?? 160, height: n.measured?.height ?? 60 });

/** The problems of a diagram by element. */
function problemsOf(d: TransformationDiagram): Problems {
  const out: Problems = {};
  for (const p of d.problems) if (p.element && p.severity !== "info") (out[p.element] ??= []).push(p.message);
  return out;
}

/** The problems of the elements of a rule (or unit), by id. */
function within(problems: Problems, prefix: string): Problems {
  const out: Problems = {};
  for (const [id, list] of Object.entries(problems)) {
    if (id === prefix || id.startsWith(`${prefix}::`) || id.startsWith(`${prefix}/`) || id.startsWith(`${prefix}#`) || id.startsWith(`${prefix}@`)) out[id] = list;
  }
  return out;
}

/** An estimate of how wide a frame's header is. */
const headerWidth = (text: string) => 60 + text.length * 7.2;

/** The React Flow nodes and edges of a diagram (positions are filled in later). */
function buildGraph(d: TransformationDiagram) {
  const problems = problemsOf(d);
  const nodes: { id: string; type: "rule" | "object" | "unit"; parent?: string; data: Record<string, unknown> }[] = [];
  const edges: Edge<RuleEdgeData>[] = [];
  for (const r of d.rules) {
    nodes.push({ id: frameId(r.id), type: "rule", data: { rule: r, problems: within(problems, r.id) } });
    for (const n of r.nodes) nodes.push({ id: n.id, type: "object", parent: frameId(r.id), data: { node: n, problems } });
    // Edges between the same two nodes bend apart.
    const pairs = new Map<string, number>();
    const totals = new Map<string, number>();
    const key = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`);
    for (const e of r.edges) totals.set(key(e.source, e.target), (totals.get(key(e.source, e.target)) ?? 0) + 1);
    for (const e of r.edges) {
      const k = key(e.source, e.target);
      const i = pairs.get(k) ?? 0;
      pairs.set(k, i + 1);
      const total = totals.get(k) ?? 1;
      let offset = total === 1 ? 0 : i - (total - 1) / 2;
      if (e.source > e.target) offset = -offset;
      if (offset !== 0 && Math.abs(offset) < 1) offset = Math.sign(offset);
      edges.push({ id: e.id, source: e.source, target: e.target, type: "rule", data: { edge: e, offset, problems: problems[e.id] } });
    }
  }
  for (const u of d.units) nodes.push({ id: unitId(u.id), type: "unit", data: { unit: u, problems: within(problems, u.id) } });
  return { nodes, edges };
}

/** The frames' sizes from their children: each frame encloses its nodes. */
function fitFrames(nodes: Node[]): Node[] {
  const extent = new Map<string, { w: number; h: number }>();
  for (const n of nodes) {
    if (!n.parentId || n.className === "unplaced") continue;
    const s = size(n);
    const e = extent.get(n.parentId) ?? { w: 0, h: 0 };
    extent.set(n.parentId, { w: Math.max(e.w, n.position.x + s.width), h: Math.max(e.h, n.position.y + s.height) });
  }
  return nodes.map((n) => {
    if (n.type !== "rule") return n;
    const rule = (n.data as { rule: RuleDiagram }).rule;
    const e = extent.get(n.id) ?? { w: 0, h: 0 };
    const head = `${rule.name}(${rule.parameters.map((p) => p.name).join(", ")})`;
    const width = Math.round(Math.max(260, headerWidth(head), e.w + PAD));
    const height = Math.round(Math.max(frameTop(rule) + 70, e.h + PAD));
    if (n.style?.width === width && n.style?.height === height) return n;
    return { ...n, style: { ...n.style, width, height }, width, height };
  });
}

function Diagram() {
  const flow = useReactFlow();
  const [diagram, setDiagram] = useState<TransformationDiagram>();
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge<RuleEdgeData>[]>([]);
  const [title, setTitle] = useState("");
  const [pair, setPair] = useState<PairInfo>({});
  const [unreadable, setUnreadable] = useState<string>();
  const [notice, setNotice] = useState<Notice>();
  const [showProperties, setShowProperties] = useState(saved.properties);
  const [connecting, setConnecting] = useState(false);
  const [picking, setPicking] = useState<{ source: string; target: string; options: string[]; at: XYPosition }>();
  const tool = useUi((s) => s.tool);
  const diagramRef = useRef<TransformationDiagram | undefined>(undefined);
  const nodesRef = useRef<Node[]>([]);
  nodesRef.current = nodes;
  const edgesRef = useRef<Edge<RuleEdgeData>[]>([]);
  edgesRef.current = edges;
  /** Positions by id: frames and units absolute, nodes relative to their frame. */
  const layout = useRef<Layout>({});
  /** Positions for elements being added. */
  const pending = useRef<Layout>({});
  const persistent = useRef(false);
  const persistNext = useRef<"always" | "ifExists" | undefined>(undefined);
  const nodeIds = useRef<string[]>([]);
  const fitted = useRef(false);
  const placing = useRef(false);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const show = useCallback((n: Notice) => {
    setNotice(n);
    clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(undefined), n.kind === "error" ? 8000 : 4000);
  }, []);

  const persist = useCallback((mode: "always" | "ifExists") => {
    if (!persistent.current) return;
    const out: Layout = {};
    for (const id of nodeIds.current) if (layout.current[id]) out[id] = layout.current[id];
    post({ type: "layout", layout: out, persist: mode });
  }, []);

  const edit = useCallback(
    async (e: TransformationEdit) => {
      const r = await requestEdit(e);
      if (r.error) show({ kind: "error", message: r.error });
      return r;
    },
    [show],
  );

  /** Moves the positions of an element (and of what is inside a rule) to its new id. */
  const moved = useCallback((from: string, to: string, kind: "rule" | "unit" | "node") => {
    if (from === to) return;
    for (const [key, p] of Object.entries(layout.current)) {
      if (kind === "rule") {
        if (key === frameId(from)) layout.current[frameId(to)] = p;
        const sep = ["::", "/", "#"].find((x) => key.startsWith(from + x));
        if (sep) layout.current[to + sep + key.slice(from.length + sep.length)] = p;
      } else if (key === (kind === "unit" ? unitId(from) : from)) {
        layout.current[kind === "unit" ? unitId(to) : to] = p;
      }
    }
    persistNext.current ??= "ifExists";
  }, []);

  const selectNode = useCallback((id: string | undefined) => {
    setNodes((ns) => ns.map((n) => (n.selected === (n.id === id) ? n : { ...n, selected: n.id === id })));
  }, []);

  const focus = useCallback((id: string | undefined, reveal = true) => {
    ui.set({ focused: id });
    const d = diagramRef.current;
    if (!id || !d) return;
    const found = find(d, id);
    if (!found) return;
    // New elements of the focused rule go into the focused scope (or the rule).
    const scope = found.kind === "multi" || found.kind === "condition" ? id : "rule" in found ? (found.kind === "node" ? scopeOf(found.node.id) : found.rule.id) : undefined;
    if (scope && ruleOfScope(ui.get().scope ?? "") !== ruleOfScope(scope)) ui.set({ scope });
    if (reveal) post({ type: "reveal", range: found.range, open: false });
  }, []);

  const ensureVisible = useCallback(
    (id: string) => {
      const n = flow.getInternalNode(id);
      const pane = document.querySelector(".react-flow")?.getBoundingClientRect();
      if (!n || !pane) return;
      const { x, y, zoom } = flow.getViewport();
      const w = n.measured.width ?? 0;
      const h = n.measured.height ?? 0;
      const left = n.internals.positionAbsolute.x * zoom + x;
      const top = n.internals.positionAbsolute.y * zoom + y;
      if (left < 0 || top < 0 || left + w * zoom > pane.width || top + h * zoom > pane.height) {
        void flow.setCenter(n.internals.positionAbsolute.x + w / 2, n.internals.positionAbsolute.y + h / 2, { zoom, duration: 250 });
      }
    },
    [flow],
  );

  // ---------------------------------------------------------------- messages

  useEffect(() => {
    const stop = onMessage((m) => {
      switch (m.type) {
        case "init":
          layout.current = { ...m.layout };
          persistent.current = m.persistent;
          setTitle(m.name);
          break;
        case "diagram":
          diagramRef.current = m.diagram;
          setDiagram(m.diagram);
          setUnreadable(undefined);
          break;
        case "layout":
          Object.assign(layout.current, m.layout);
          setNodes((ns) => fitFrames(ns.map((n) => (m.layout[n.id] ? { ...n, position: m.layout[n.id], className: undefined } : n))));
          break;
        case "unreadable":
          setUnreadable(m.message ?? "The document cannot be read.");
          break;
        case "edited":
          edited(m.request, m);
          break;
        case "pair":
          setPair(m.pair);
          break;
        case "cursor": {
          const d = diagramRef.current;
          const id = d && elementAt(d, m.position);
          if (!d || !id) break;
          focus(id, false);
          const drawn = drawnIn(d, id);
          selectNode(drawn);
          if (drawn) ensureVisible(drawn);
          break;
        }
      }
    });
    post({ type: "ready" });
    return stop;
  }, [focus, selectNode, ensureVisible]);

  // ---------------------------------------------------------------- the graph

  useEffect(() => {
    if (!diagram) return;
    const graph = buildGraph(diagram);
    const ids = graph.nodes.map((n) => n.id);
    const previous = new Set(nodeIds.current);
    const added = ids.filter((id) => !previous.has(id));
    const gone = nodeIds.current.filter((id) => !ids.includes(id));
    // An element renamed in the text keeps its place.
    const frames = (l: string[]) => l.filter((id) => id.startsWith("rule:"));
    const objects = (l: string[]) => l.filter((id) => id.includes("::"));
    for (const pick of [frames, objects]) {
      const [g, a] = [pick(gone), pick(added)];
      if (g.length === 1 && a.length === 1 && layout.current[g[0]] && !layout.current[a[0]]) {
        if (g[0].startsWith("rule:")) moved(g[0].slice(5), a[0].slice(5), "rule");
        else moved(g[0], a[0], "node");
      }
    }
    for (const id of added) {
      if (pending.current[id]) {
        layout.current[id] = pending.current[id];
        delete pending.current[id];
        persistNext.current = "always";
      }
    }
    nodeIds.current = ids;
    setNodes((prev) => {
      const byId = new Map(prev.map((n) => [n.id, n]));
      const next = graph.nodes.map((spec): Node => {
        const old = byId.get(spec.id);
        const at = layout.current[spec.id];
        const placed = old && !old.className && old.parentId === (spec.parent ?? old.parentId) ? old.position : at;
        return {
          ...old,
          id: spec.id,
          type: spec.type,
          data: spec.data,
          parentId: spec.parent,
          expandParent: spec.parent ? true : undefined,
          zIndex: spec.type === "rule" ? 0 : 1,
          position: placed ?? { x: 0, y: 0 },
          className: placed ? undefined : "unplaced",
          selected: old?.selected ?? false,
          dragHandle: spec.type === "rule" || spec.type === "unit" ? "header" : undefined,
        };
      });
      return fitFrames(next);
    });
    setEdges((prev) => {
      const selected = new Set(prev.filter((e) => e.selected).map((e) => e.id));
      return graph.edges.map((e) => ({ ...e, selected: selected.has(e.id) }));
    });
    if (persistNext.current) {
      persist(persistNext.current);
      persistNext.current = undefined;
    }
  }, [diagram, persist, moved]);

  /** Positions for unplaced elements: a rule's nodes (all with ELK, new ones beside their
   * neighbours), then frames and units below the others. */
  const place = useCallback(async (all: Node[], everything: boolean): Promise<Layout> => {
    const out: Layout = {};
    const unplaced = (n: Node) => everything || n.className === "unplaced";
    const edgesNow = edgesRef.current;
    for (const frame of all.filter((n) => n.type === "rule")) {
      const rule = (frame.data as { rule: RuleDiagram }).rule;
      const children = all.filter((n) => n.parentId === frame.id);
      const fresh = children.filter(unplaced);
      if (fresh.length === 0) continue;
      const links = edgesNow.filter((e) => children.some((c) => c.id === e.source) && children.some((c) => c.id === e.target)).map((e) => ({ from: e.source, to: e.target }));
      const top = frameTop(rule) + 8;
      if (fresh.length === children.length) {
        const positions = await layoutAll(children.map(size), links);
        for (const [id, p] of Object.entries(positions)) out[id] = { x: p.x + PAD, y: p.y + top };
      } else {
        const placed = children.filter((c) => !unplaced(c)).map((c) => ({ ...size(c), ...c.position }));
        const positions = placeNew(fresh.map(size), placed, links);
        for (const [id, p] of Object.entries(positions)) out[id] = { x: Math.max(PAD, p.x), y: Math.max(top, p.y) };
      }
    }
    // Frames: in rows below the ones placed (a row ends at ROW_WIDTH); units in a column to
    // the right of the rules.
    const sized = fitFrames(all.map((n) => (out[n.id] ? { ...n, position: out[n.id], className: undefined } : n)));
    const box = (n: Node) => ({ w: (n.style?.width as number) ?? size(n).width, h: (n.style?.height as number) ?? size(n).height });
    let bottom = 0;
    let right = 0;
    let unitBottom = 0;
    for (const n of sized) {
      if (n.parentId || unplaced(n)) continue;
      const { w, h } = box(n);
      if (n.type === "rule") {
        bottom = Math.max(bottom, n.position.y + h + GAP);
        right = Math.max(right, n.position.x + w);
      } else if (n.type === "unit") {
        unitBottom = Math.max(unitBottom, n.position.y + h + GAP);
      }
    }
    let x = 0;
    let rowHeight = 0;
    for (const n of sized.filter((n) => n.type === "rule" && unplaced(n))) {
      const { w, h } = box(n);
      if (x > 0 && x + w > ROW_WIDTH) {
        bottom += rowHeight + GAP;
        x = 0;
        rowHeight = 0;
      }
      out[n.id] = { x, y: bottom };
      x += w + GAP;
      rowHeight = Math.max(rowHeight, h);
      right = Math.max(right, x - GAP);
    }
    for (const n of sized.filter((n) => n.type === "unit" && unplaced(n))) {
      out[n.id] = { x: right + GAP * 2, y: unitBottom };
      unitBottom += box(n).h + GAP;
    }
    return out;
  }, []);

  const initialized = useNodesInitialized();
  useEffect(() => {
    if (!initialized || placing.current) return;
    const unplaced = nodes.filter((n) => n.className === "unplaced");
    if (unplaced.length === 0) {
      if (!fitted.current && nodes.length) {
        fitted.current = true;
        requestAnimationFrame(() => void flow.fitView({ padding: 0.08, maxZoom: 1 }));
      }
      return;
    }
    placing.current = true;
    const all = unplaced.length === nodes.length;
    void (async () => {
      const positions = await place(nodes, false);
      Object.assign(layout.current, positions);
      setNodes((ns) => fitFrames(ns.map((n) => (positions[n.id] ? { ...n, position: positions[n.id], className: undefined } : n))));
      placing.current = false;
      persist("ifExists");
      if (all) {
        fitted.current = true;
        requestAnimationFrame(() => void flow.fitView({ padding: 0.08, maxZoom: 1 }));
      }
    })();
  }, [initialized, nodes, flow, persist, place]);

  const autoLayout = useCallback(async () => {
    const positions = await place(nodesRef.current, true);
    Object.assign(layout.current, positions);
    setNodes((ns) => fitFrames(ns.map((n) => (positions[n.id] ? { ...n, position: positions[n.id], className: undefined } : n))));
    persist("always");
    requestAnimationFrame(() => void flow.fitView({ padding: 0.08, maxZoom: 1, duration: 300 }));
  }, [flow, persist, place]);

  // ---------------------------------------------------------------- editing

  const actions: Actions = useMemo(() => {
    const d = () => diagramRef.current!;
    const done = (element?: string) => ui.set({ editing: undefined, focused: element });
    return {
      focus: (id) => focus(id),
      startEditing: (e) => ui.set({ editing: e }),
      stopEditing: () => ui.set({ editing: undefined }),
      rename: async (id, name) => {
        const r = await requestEdit({ kind: "rename", element: id, name: name.trim() });
        if (r.error) return r.error;
        const kind = find(d(), id)?.kind;
        if (r.element) moved(id, r.element, kind === "rule" ? "rule" : kind === "unit" ? "unit" : "node");
        done(r.element ?? id);
        return undefined;
      },
      attributeText: (id) => {
        const found = find(d(), id);
        return found?.kind === "attribute" ? `${found.attribute.name}=${found.attribute.value}` : "";
      },
      changeAttribute: async (id, text) => {
        const found = find(d(), id);
        if (found?.kind !== "attribute") return "The attribute no longer exists.";
        const p = parseAttribute(text);
        if (!p.value) return p.error;
        const a = found.attribute;
        const r = await requestEdit({
          kind: "updateAttribute",
          element: id,
          name: p.value.name !== a.name ? p.value.name : undefined,
          value: p.value.value !== a.value ? p.value.value : undefined,
        });
        if (r.error) return r.error;
        done(r.element ?? id);
        return undefined;
      },
      addAttribute: async (node, text) => {
        const p = parseAttribute(text);
        if (!p.value) return p.error;
        const scope = ui.get().scope;
        const found = find(d(), node);
        // An attribute added while a multi-rule or condition graph is the scope goes there.
        const inScope = found?.kind === "node" && scope && scope !== scopeOf(node) && ruleOfScope(scope) === found.rule.id && scope.startsWith(scopeOf(node)) ? scope : undefined;
        const r = await requestEdit({ kind: "addAttribute", node, scope: inScope, name: p.value.name, value: p.value.value });
        if (r.error) return r.error;
        ui.set({ editing: { kind: "newAttribute", node, seq: Date.now() }, focused: r.element });
        return undefined;
      },
    };
  }, [focus, moved]);

  /** Adds a node of the toolbar's class and action where the user clicked into a rule. */
  const createNode = useCallback(
    async (frame: Node, at: XYPosition) => {
      const d = diagramRef.current;
      const rule = (frame.data as { rule: RuleDiagram }).rule;
      if (!d) return;
      const cls = ui.get().class ?? d.classes.find((c) => !c.isAbstract)?.name;
      if (!cls) {
        show({ kind: "error", message: "The imported packages have no classes." });
        return;
      }
      const chosen = ui.get().scope;
      const scope = chosen && ruleOfScope(chosen) === rule.id ? chosen : rule.id;
      const base = lowerFirst(cls.slice(cls.lastIndexOf(".") + 1));
      const name = uniqueName(base, [...rule.nodes.map((n) => n.name), ...rule.parameters.map((p) => p.name)]);
      const id = `${scope}::${name}`;
      pending.current[id] = { x: Math.max(PAD, at.x - frame.position.x - 60), y: Math.max(frameTop(rule), at.y - frame.position.y - 20) };
      const r = await edit({ kind: "addNode", scope, name, class: cls, action: ui.get().action });
      if (r.error) {
        delete pending.current[id];
        return;
      }
      ui.set({ focused: r.element, editing: { kind: "name", element: r.element ?? id } });
    },
    [edit, show],
  );

  const addEdge = useCallback(
    async (source: string, target: string, reference: string) => {
      const scope = ui.get().scope;
      const r = await edit({ kind: "addEdge", source, target, reference, action: ui.get().action, scope: scope && scope.includes("#") ? scope : undefined });
      if (r.element) focus(r.element);
    },
    [edit, focus],
  );

  const onConnect = useCallback(
    async (c: Connection, at: XYPosition) => {
      ui.set({ tool: { kind: "select" } });
      const d = diagramRef.current;
      const found = d && [find(d, c.source), find(d, c.target)];
      if (!d || !found || found[0]?.kind !== "node" || found[1]?.kind !== "node") return;
      const [s, t] = [found[0], found[1]];
      if (s.rule.id !== t.rule.id) {
        show({ kind: "error", message: "Edges connect nodes of the same rule." });
        return;
      }
      const options = references(d, s.node, t.node);
      if (options.length === 0) {
        show({ kind: "error", message: `${s.node.class} has no reference to ${t.node.class}.` });
      } else if (options.length === 1) {
        await addEdge(s.node.id, t.node.id, options[0]);
      } else {
        setPicking({ source: s.node.id, target: t.node.id, options, at });
      }
    },
    [addEdge, show],
  );

  const addRule = useCallback(async () => {
    const d = diagramRef.current;
    if (!d) return;
    const name = uniqueName("newRule", [...d.rules.map((r) => r.name), ...d.units.map((u) => u.name)]);
    const r = await edit({ kind: "addRule", name });
    if (!r.error) ui.set({ focused: name, editing: { kind: "name", element: name } });
  }, [edit]);

  const addUnit = useCallback(async () => {
    const d = diagramRef.current;
    if (!d) return;
    const name = uniqueName("newUnit", [...d.rules.map((r) => r.name), ...d.units.map((u) => u.name)]);
    const r = await edit({ kind: "addUnit", name });
    if (!r.error) ui.set({ focused: name, editing: { kind: "name", element: name } });
  }, [edit]);

  /** Deletes the selected elements and the focused attribute or unit element. */
  const deleteSelection = useCallback(async () => {
    const d = diagramRef.current;
    if (!d) return;
    const focusedId = ui.get().focused;
    const focusedElement = focusedId ? find(d, focusedId) : undefined;
    if (focusedElement && ["attribute", "step", "multi", "condition"].includes(focusedElement.kind)) {
      await edit({ kind: "delete", elements: [focusedId!] });
      ui.set({ focused: undefined });
      return;
    }
    const elements = new Set<string>();
    for (const n of nodesRef.current) {
      if (!n.selected) continue;
      if (n.type === "object") elements.add(n.id);
      else if (n.type === "rule") elements.add(n.id.slice("rule:".length));
      else if (n.type === "unit") elements.add(n.id.slice("unit:".length));
    }
    for (const e of edgesRef.current) if (e.selected) elements.add(e.id);
    if (focusedElement?.kind === "edge" || focusedElement?.kind === "node") elements.add(focusedId!);
    if (elements.size) await edit({ kind: "delete", elements: [...elements] });
    ui.set({ focused: undefined });
  }, [edit]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest("input, textarea, select")) return;
      if (e.key === "Delete" || (e.key === "Backspace" && !e.metaKey)) {
        e.preventDefault();
        void deleteSelection();
      } else if (e.key === "Escape") {
        ui.set({ tool: { kind: "select" }, editing: undefined });
        setPicking(undefined);
      } else if (e.key === "F2") {
        const f = ui.get().focused;
        const found = f && diagramRef.current && find(diagramRef.current, f);
        if (found && ["rule", "node", "unit"].includes(found.kind)) ui.set({ editing: { kind: "name", element: f } });
        else if (found && found.kind === "attribute") ui.set({ editing: { kind: "attribute", element: f } });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [deleteSelection]);

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setNodes((ns) => applyNodeChanges(changes.filter((c) => c.type !== "remove"), ns));
  }, []);
  const onEdgesChange = useCallback((changes: EdgeChange<Edge<RuleEdgeData>>[]) => {
    setEdges((es) => applyEdgeChanges(changes.filter((c) => c.type !== "remove"), es));
  }, []);

  const problems = diagram?.problems.filter((p) => p.severity !== "info") ?? [];
  const lastPointer = useRef<XYPosition>({ x: 0, y: 0 });

  return (
    <ActionsContext.Provider value={actions}>
      <div
        className={`diagram-app rule-diagram${connecting ? " connecting" : ""}${tool.kind !== "select" ? ` tool-${tool.kind}` : ""}`}
        onPointerUp={(e) => (lastPointer.current = { x: e.clientX, y: e.clientY })}
      >
        <Toolbar
          title={title}
          diagram={diagram}
          pair={pair}
          problems={problems.length}
          onProblems={() => {
            const first = problems.find((p) => p.element);
            if (first?.element && diagram) {
              focus(first.element);
              const drawn = drawnIn(diagram, first.element);
              selectNode(drawn);
              if (drawn) ensureVisible(drawn);
            }
          }}
          onAddRule={() => void addRule()}
          onAddUnit={() => void addUnit()}
          onLayout={() => void autoLayout()}
          onFit={() => void flow.fitView({ padding: 0.08, duration: 300 })}
          onCommand={(command) => post({ type: "command", command })}
          showProperties={showProperties}
          onToggleProperties={() => {
            saved.properties = !showProperties;
            save();
            setShowProperties(!showProperties);
          }}
        />
        <div className="workspace">
          <main>
            <Markers />
            <ReactFlow
              nodes={nodes}
              edges={edges}
              nodeTypes={nodeTypes}
              edgeTypes={edgeTypes}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onNodeDragStop={(_, __, dragged) => {
                for (const n of dragged) layout.current[n.id] = n.position;
                setNodes((ns) => fitFrames(ns));
                persist("always");
              }}
              onNodeClick={(e, node) => {
                const t = ui.get().tool;
                if (t.kind === "node" && node.type === "rule") {
                  ui.set({ tool: { kind: "select" } });
                  void createNode(node, flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }));
                  return;
                }
                const row = (e.target as HTMLElement).closest<HTMLElement>("[data-element]");
                const id = row?.dataset.element ?? (node.type === "rule" ? node.id.slice(5) : node.type === "unit" ? node.id.slice(5) : node.id);
                focus(id);
              }}
              onEdgeClick={(_, edge) => focus(edge.id)}
              onPaneClick={() => {
                ui.set({ focused: undefined, editing: undefined });
                setPicking(undefined);
              }}
              onConnect={(c) => void onConnect(c, lastPointer.current)}
              onConnectStart={() => setConnecting(true)}
              onConnectEnd={(e) => {
                setConnecting(false);
                if (e && "clientX" in e) lastPointer.current = { x: e.clientX, y: e.clientY };
              }}
              connectionMode={ConnectionMode.Strict}
              connectionRadius={0}
              deleteKeyCode={null}
              selectionKeyCode="Shift"
              multiSelectionKeyCode={["Meta", "Control"]}
              minZoom={0.1}
              maxZoom={2.5}
              proOptions={{ hideAttribution: true }}
              nodesConnectable
              elevateEdgesOnSelect
            >
              <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
              <Controls showInteractive={false} position="bottom-left" />
              <MiniMap pannable zoomable position="bottom-right" nodeStrokeWidth={2} />
            </ReactFlow>
            {picking && (
              <div className="reference-picker" style={{ left: picking.at.x, top: picking.at.y }} role="menu">
                <div className="picker-title">Reference</div>
                {picking.options.map((o) => (
                  <button
                    key={o}
                    onClick={() => {
                      setPicking(undefined);
                      void addEdge(picking.source, picking.target, o);
                    }}
                  >
                    {o}
                  </button>
                ))}
              </div>
            )}
            {unreadable && (
              <div className="banner error" role="alert">
                {unreadable}
                {diagram ? " — showing the last version that could be read." : ""}
              </div>
            )}
            {!diagram && !unreadable && <div className="empty">Reading the transformation…</div>}
            {diagram && diagram.rules.length === 0 && diagram.units.length === 0 && (
              <div className="empty">The transformation has no rules yet: add one with the Rule button.</div>
            )}
            {notice && (
              <div className={`notice ${notice.kind}`} role="status" onClick={() => setNotice(undefined)}>
                {notice.message}
              </div>
            )}
          </main>
          {showProperties && diagram && <Properties diagram={diagram} pair={pair} edit={edit} focus={focus} />}
        </div>
      </div>
    </ActionsContext.Provider>
  );
}

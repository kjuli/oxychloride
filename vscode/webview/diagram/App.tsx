// The metamodel diagram (runs in the webview; src/diagram.ts is its host).
//
// The diagram is drawn from the engine's `MetamodelDiagram` of the document's text, again after
// every change. Edits made here are sent to the host, which turns them into text edits; the
// changed text then comes back as a new diagram. Positions are kept by element id and written
// to the layout file when the user lays the diagram out; classifiers without a position are
// placed automatically (all of them with ELK, new ones next to their neighbours).

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
import type { Layout } from "../../src/diagramProtocol";
import type { ElementUpdate, MetamodelDiagram, MetamodelEdit } from "../../src/engine";
import { edgeTypes, Markers } from "./edges";
import { edit as requestEdit, edited, onMessage, post, save, saved, ui, useUi, type Tool } from "./host";
import { layoutAll, placeNew, type LayoutEdge, type Sized } from "./layout";
import {
  buildGraph,
  elementAt,
  featureText,
  find,
  generalizationId,
  lowerFirst,
  ownerOf,
  parametersText,
  parseFeature,
  parseOperation,
  parseParameters,
  uniqueName,
  multiplicity,
  type EdgeData,
} from "./model";
import { ActionsContext, nodeTypes, type Actions } from "./nodes";
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

const size = (n: Node): Sized => ({ id: n.id, width: n.measured?.width ?? 200, height: n.measured?.height ?? 80 });

/** The edges for laying out: from supertypes and containers down. */
function layoutEdges(edges: Edge<EdgeData>[]): LayoutEdge[] {
  return edges.map((e) =>
    e.data?.kind === "generalization" ? { from: e.target, to: e.source } : { from: e.source, to: e.target },
  );
}

function Diagram() {
  const flow = useReactFlow();
  const [diagram, setDiagram] = useState<MetamodelDiagram>();
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge<EdgeData>[]>([]);
  const [title, setTitle] = useState("");
  const [unreadable, setUnreadable] = useState<string>();
  const [notice, setNotice] = useState<Notice>();
  const [showProperties, setShowProperties] = useState(saved.properties);
  const [connecting, setConnecting] = useState(false);
  const [packageId, setPackageId] = useState<string>();
  const tool = useUi((s) => s.tool);
  const diagramRef = useRef<MetamodelDiagram | undefined>(undefined);
  const nodesRef = useRef<Node[]>([]);
  nodesRef.current = nodes;
  const edgesRef = useRef<Edge<EdgeData>[]>([]);
  edgesRef.current = edges;
  /** Positions by element id: from the layout file, dragged, or placed. */
  const layout = useRef<Layout>({});
  /** Positions for classifiers being added. */
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

  /** Writes the layout file (`ifExists`: only if there is one). */
  const persist = useCallback((mode: "always" | "ifExists") => {
    if (!persistent.current) return;
    const out: Layout = {};
    for (const id of nodeIds.current) if (layout.current[id]) out[id] = layout.current[id];
    post({ type: "layout", layout: out, persist: mode });
  }, []);

  const edit = useCallback(
    async (e: MetamodelEdit) => {
      const r = await requestEdit(e);
      if (r.error) show({ kind: "error", message: r.error });
      return r;
    },
    [show],
  );

  /** Selects the node of a classifier (and only that one). */
  const selectNode = useCallback((id: string | undefined) => {
    setNodes((ns) => ns.map((n) => (n.selected === (n.id === id) ? n : { ...n, selected: n.id === id })));
  }, []);

  const focus = useCallback((id: string | undefined, reveal = true) => {
    ui.set({ focused: id });
    const d = diagramRef.current;
    if (!id || !d) return;
    const found = find(d, id);
    if (found && reveal) post({ type: "reveal", range: found.range, open: false });
  }, []);

  /** Pans to a node if it is (partly) out of sight. */
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
          setNodes((ns) => ns.map((n) => (m.layout[n.id] ? { ...n, position: m.layout[n.id], className: undefined } : n)));
          break;
        case "unreadable":
          setUnreadable(m.message ?? "The document cannot be read.");
          break;
        case "edited":
          edited(m.request, m);
          break;
        case "cursor": {
          const d = diagramRef.current;
          const id = d && elementAt(d, m.position);
          if (!d || !id) break;
          focus(id, false);
          // Only a classifier itself is selected (and so deleted by Delete), not for its members.
          const owner = ownerOf(d, id);
          selectNode(owner === id ? id : undefined);
          if (owner) ensureVisible(owner);
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
    // A classifier renamed in the text keeps its place.
    if (gone.length === 1 && added.length === 1 && layout.current[gone[0]] && !layout.current[added[0]]) {
      layout.current[added[0]] = layout.current[gone[0]];
      persistNext.current ??= "ifExists";
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
      return graph.nodes.map((spec) => {
        const old = byId.get(spec.id);
        const at = layout.current[spec.id];
        const placed = old && !old.className ? old.position : at;
        return {
          ...old,
          id: spec.id,
          type: spec.type,
          data: spec.data,
          position: placed ?? { x: 0, y: 0 },
          className: placed ? undefined : "unplaced",
          selected: old?.selected ?? false,
        };
      });
    });
    setEdges((prev) => {
      const selected = new Set(prev.filter((e) => e.selected).map((e) => e.id));
      return graph.edges.map((e) => ({ id: e.id, source: e.source, target: e.target, type: "diagram", data: e.data, selected: selected.has(e.id) }));
    });
    if (persistNext.current) {
      persist(persistNext.current);
      persistNext.current = undefined;
    }
  }, [diagram, persist]);

  // Classifiers without a position are placed once they have been measured.
  const initialized = useNodesInitialized();
  useEffect(() => {
    if (!initialized || placing.current) return;
    const unplaced = nodes.filter((n) => n.className === "unplaced");
    if (unplaced.length === 0) {
      if (!fitted.current && nodes.length) {
        fitted.current = true;
        requestAnimationFrame(() => void flow.fitView({ padding: 0.12, maxZoom: 1 }));
      }
      return;
    }
    placing.current = true;
    const all = unplaced.length === nodes.length;
    void (async () => {
      const links = layoutEdges(edgesRef.current);
      const positions = all
        ? await layoutAll(nodes.map(size), links)
        : placeNew(unplaced.map(size), nodes.filter((n) => n.className !== "unplaced").map((n) => ({ ...size(n), ...n.position })), links);
      Object.assign(layout.current, positions);
      setNodes((ns) => ns.map((n) => (positions[n.id] ? { ...n, position: positions[n.id], className: undefined } : n)));
      placing.current = false;
      persist("ifExists");
      if (all) {
        fitted.current = true;
        requestAnimationFrame(() => void flow.fitView({ padding: 0.12, maxZoom: 1 }));
      }
    })();
  }, [initialized, nodes, flow, persist]);

  const autoLayout = useCallback(async () => {
    const positions = await layoutAll(nodesRef.current.map(size), layoutEdges(edgesRef.current));
    Object.assign(layout.current, positions);
    setNodes((ns) => ns.map((n) => (positions[n.id] ? { ...n, position: positions[n.id], className: undefined } : n)));
    persist("always");
    requestAnimationFrame(() => void flow.fitView({ padding: 0.12, maxZoom: 1, duration: 300 }));
  }, [flow, persist]);

  // ---------------------------------------------------------------- editing

  const actions: Actions = useMemo(() => {
    const d = () => diagramRef.current!;
    const done = (element?: string) => ui.set({ editing: undefined, focused: element });
    return {
      focus: (id) => focus(id),
      startEditing: (e) => ui.set({ editing: e }),
      stopEditing: () => ui.set({ editing: undefined }),
      rename: async (id, name) => {
        const r = await requestEdit({ kind: "update", element: id, properties: { name: name.trim() } });
        if (r.error) return r.error;
        if (r.element && r.element !== id && layout.current[id]) {
          layout.current[r.element] = layout.current[id];
          persistNext.current = "ifExists";
        }
        done(r.element ?? id);
        return undefined;
      },
      memberText: (id) => {
        const found = find(d(), id);
        if (found?.feature) return featureText(found.feature, d().syntax);
        const o = found?.operation;
        if (o) {
          const result = o.typeName ? ` : ${o.typeName}${multiplicity(o.lower, o.upper, d().syntax)}` : "";
          return `${o.name}(${parametersText(o, d().syntax)})${result}`;
        }
        return found?.kind === "literal" ? (found.classifier?.literals.find((l) => l.id === id)?.name ?? "") : "";
      },
      changeMember: async (id, text) => {
        const diagram = d();
        const found = find(diagram, id);
        if (!found) return "The element no longer exists.";
        const properties: ElementUpdate = {};
        if (found.feature) {
          const p = parseFeature(text, diagram.syntax, diagram.types, found.kind === "reference");
          if (!p.value) return p.error;
          const f = found.feature;
          if (p.value.name !== f.name) properties.name = p.value.name;
          if (p.value.type !== undefined && p.value.type !== f.type) properties.type = p.value.type;
          if (p.value.lower !== f.lower) properties.lower = p.value.lower;
          if (p.value.upper !== f.upper) properties.upper = p.value.upper;
        } else if (found.operation) {
          const p = parseOperation(text, diagram.syntax, diagram.types);
          if (!p.value) return p.error;
          const o = found.operation;
          if (p.value.name !== o.name) properties.name = p.value.name;
          if ((p.value.type ?? "") !== (o.type ?? "")) properties.type = p.value.type ?? "";
          if (p.value.type && p.value.lower !== o.lower) properties.lower = p.value.lower;
          if (p.value.type && p.value.upper !== o.upper) properties.upper = p.value.upper;
          const same =
            p.value.parameters.length === o.parameters.length &&
            p.value.parameters.every((q, i) => {
              const old = o.parameters[i];
              return q.name === old.name && q.type === old.type && q.lower === old.lower && q.upper === old.upper;
            });
          if (!same) properties.parameters = p.value.parameters;
        } else if (found.kind === "literal") {
          if (!/^\S+$/.test(text.trim())) return "A literal is named by one word.";
          properties.name = text.trim();
        }
        if (Object.keys(properties).length === 0) {
          done(id);
          return undefined;
        }
        const r = await requestEdit({ kind: "update", element: id, properties });
        if (r.error) return r.error;
        done(r.element ?? id);
        return undefined;
      },
      addMember: async (owner, member, text) => {
        const diagram = d();
        let e: MetamodelEdit;
        if (member === "attribute") {
          const p = parseFeature(text, diagram.syntax, diagram.types, false);
          if (!p.value) return p.error;
          const type = p.value.type ?? diagram.types.find((t) => t.id === "ecore::EString")?.id ?? "ecore::EString";
          e = { kind: "addAttribute", owner, name: p.value.name, type, lower: p.value.lower, upper: p.value.upper };
        } else if (member === "operation") {
          const p = parseOperation(text, diagram.syntax, diagram.types);
          if (!p.value) return p.error;
          e = { kind: "addOperation", owner, ...p.value };
        } else {
          if (!/^\S+$/.test(text.trim())) return "A literal is named by one word.";
          e = { kind: "addLiteral", owner, name: text.trim() };
        }
        const r = await requestEdit(e);
        if (r.error) return r.error;
        // Keep adding: a fresh field for the next member.
        ui.set({ editing: { kind: "new", owner, member, seq: Date.now() }, focused: r.element });
        return undefined;
      },
    };
  }, [focus]);

  const createClassifier = useCallback(
    async (classifier: Extract<Tool, { kind: "classifier" }>["classifier"], at: XYPosition) => {
      const d = diagramRef.current;
      const pkg = d?.packages.find((p) => p.id === packageId) ?? d?.packages[0];
      if (!d || !pkg) return;
      const base = { class: "NewClass", abstractClass: "NewAbstractClass", interface: "NewInterface", enum: "NewEnum", dataType: "NewDataType" }[classifier];
      const name = uniqueName(base, d.classifiers.filter((c) => c.package === pkg.id).map((c) => c.name));
      const id = `${pkg.id}::${name}`;
      pending.current[id] = at;
      const r = await edit({ kind: "addClassifier", package: pkg.id, classifier, name });
      if (r.error) {
        delete pending.current[id];
        return;
      }
      ui.set({ focused: r.element, editing: { kind: "name", element: r.element ?? id } });
    },
    [edit, packageId],
  );

  const onConnect = useCallback(
    async (c: Connection) => {
      const d = diagramRef.current;
      const t = ui.get().tool;
      const kind = t.kind === "connect" ? t.connection : "reference";
      ui.set({ tool: { kind: "select" } });
      const source = d?.classifiers.find((x) => x.id === c.source);
      const target = d?.classifiers.find((x) => x.id === c.target) ?? d?.external.find((x) => x.id === c.target);
      if (!d || !source || !target) return;
      if (source.kind !== "class" || target.kind !== "class") {
        show({ kind: "error", message: "References and supertypes connect classes." });
        return;
      }
      if (kind === "generalization") {
        await edit({ kind: "addSuperType", owner: source.id, superType: target.id });
        return;
      }
      const containment = kind === "containment";
      const taken = [...source.attributes, ...source.references].map((f) => f.name);
      const name = uniqueName(`${lowerFirst(target.name)}${containment ? "s" : ""}`, taken);
      const r = await edit({ kind: "addReference", owner: source.id, name, type: target.id, lower: 0, upper: containment ? -1 : 1, containment });
      if (r.element) focus(r.element);
    },
    [edit, show, focus],
  );

  /** Deletes the selected classifiers and edges and the focused member. */
  const deleteSelection = useCallback(async () => {
    const d = diagramRef.current;
    if (!d) return;
    const elements = new Set<string>();
    const generalizations: { owner: string; superType: string }[] = [];
    const focusedId = ui.get().focused;
    const focusedElement = focusedId ? find(d, focusedId) : undefined;
    // A focused member is deleted on its own, not with the selected classifier around it.
    if (focusedElement && ["attribute", "reference", "operation", "literal"].includes(focusedElement.kind)) {
      await edit({ kind: "delete", elements: [focusedId!] });
      ui.set({ focused: undefined });
      return;
    }
    for (const n of nodesRef.current) if (n.selected && n.type === "classifier") elements.add(n.id);
    const addEdge = (e: Edge<EdgeData>) => {
      if (e.data?.kind === "generalization") {
        if (!generalizations.some((g) => g.owner === e.source && g.superType === e.target)) generalizations.push({ owner: e.source, superType: e.target });
      }
      else if (e.data?.reference) {
        elements.add(e.data.reference.id);
        if (e.data.opposite) elements.add(e.data.opposite.id);
      }
    };
    for (const e of edgesRef.current) if (e.selected) addEdge(e);
    if (focusedElement?.kind === "classifier") elements.add(focusedId!);
    const edge = edgesRef.current.find((e) => e.id === focusedId);
    if (edge) addEdge(edge);
    const remaining = generalizations.filter((g) => !elements.has(g.owner) && !elements.has(g.superType));
    if (elements.size) await edit({ kind: "delete", elements: [...elements] });
    for (const g of remaining) await edit({ kind: "removeSuperType", ...g });
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
      } else if (e.key === "F2") {
        const f = ui.get().focused;
        const found = f && diagramRef.current && find(diagramRef.current, f);
        if (found && found.kind === "classifier") ui.set({ editing: { kind: "name", element: f } });
        else if (found && found.kind !== "package" && found.kind !== "invariant") ui.set({ editing: { kind: "member", element: f } });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [deleteSelection]);

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setNodes((ns) => applyNodeChanges(changes.filter((c) => c.type !== "remove"), ns));
  }, []);
  const onEdgesChange = useCallback((changes: EdgeChange<Edge<EdgeData>>[]) => {
    setEdges((es) => applyEdgeChanges(changes.filter((c) => c.type !== "remove"), es));
  }, []);

  const problems = diagram?.problems.filter((p) => p.severity !== "info") ?? [];

  return (
    <ActionsContext.Provider value={actions}>
      <div className={`diagram-app${connecting ? " connecting" : ""}${tool.kind !== "select" ? ` tool-${tool.kind}` : ""}`}>
        <Toolbar
          title={title}
          diagram={diagram}
          packageId={packageId ?? diagram?.packages[0]?.id}
          onPackage={setPackageId}
          problems={problems.length}
          onProblems={() => {
            const first = problems.find((p) => p.element);
            if (first?.element) {
              focus(first.element);
              const owner = diagram && ownerOf(diagram, first.element);
              selectNode(owner);
              if (owner) ensureVisible(owner);
            }
          }}
          onLayout={() => void autoLayout()}
          onFit={() => void flow.fitView({ padding: 0.12, duration: 300 })}
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
                persist("always");
              }}
              onNodeClick={(e, node) => {
                const row = (e.target as HTMLElement).closest<HTMLElement>("[data-element]");
                focus(row?.dataset.element ?? node.id);
              }}
              onEdgeClick={(_, edge) => focus(edge.data?.reference?.id ?? edge.id)}
              onPaneClick={(e) => {
                const t = ui.get().tool;
                if (t.kind === "classifier") {
                  ui.set({ tool: { kind: "select" } });
                  void createClassifier(t.classifier, flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }));
                } else {
                  ui.set({ focused: undefined, editing: undefined });
                }
              }}
              onConnect={(c) => void onConnect(c)}
              onConnectStart={() => setConnecting(true)}
              onConnectEnd={() => setConnecting(false)}
              connectionMode={ConnectionMode.Strict}
              connectionRadius={0}
              deleteKeyCode={null}
              selectionKeyCode="Shift"
              multiSelectionKeyCode={["Meta", "Control"]}
              minZoom={0.1}
              maxZoom={2.5}
              onlyRenderVisibleElements
              proOptions={{ hideAttribution: true }}
              nodesConnectable
              elevateEdgesOnSelect
            >
              <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
              <Controls showInteractive={false} position="bottom-left" />
              <MiniMap pannable zoomable position="bottom-right" nodeStrokeWidth={2} />
            </ReactFlow>
            {unreadable && (
              <div className="banner error" role="alert">
                {unreadable}
                {diagram ? " — showing the last version that could be read." : ""}
              </div>
            )}
            {!diagram && !unreadable && <div className="empty">Reading the metamodel…</div>}
            {diagram && diagram.classifiers.length === 0 && (
              <div className="empty">The metamodel has no classifiers yet: pick a tool in the toolbar and click into the diagram.</div>
            )}
            {notice && (
              <div className={`notice ${notice.kind}`} role="status" onClick={() => setNotice(undefined)}>
                {notice.message}
              </div>
            )}
          </main>
          {showProperties && diagram && (
            <Properties
              diagram={diagram}
              edit={edit}
              focus={focus}
              parseParameters={(text) => parseParameters(text, diagram.syntax, diagram.types)}
              onRenamed={(from, to) => {
                if (layout.current[from]) {
                  layout.current[to] = layout.current[from];
                  persistNext.current = "ifExists";
                }
              }}
              edges={edges}
              generalizationId={generalizationId}
            />
          )}
        </div>
      </div>
    </ActionsContext.Provider>
  );
}

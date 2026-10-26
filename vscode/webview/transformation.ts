// The transformation view (runs in the webview; transformation.ts is its host).
//
// Steps: pick a rule or unit, give its parameters, find its matches (highlighted in the object
// diagram, with the rule's node names) and apply one; every step shows what it created
// (green), deleted (red) and changed (amber), and the invariants the state violates. Steps can be
// revisited, replayed and exported.
//
// State space: explores the states the selected rules reach from the current step and draws
// them; a state shows its object diagram, the path to it and its violations, and can be adopted
// as the steps of the transformation.

import type {
  GraphChanges,
  GraphEdge,
  GraphInfo,
  GraphNode,
  MatchList,
  ParameterValues,
  StateSpaceInfo,
  StateSpaceStateDetails,
  StateViolation,
  TransformationParameter,
  TransformationState,
  TransformationUnitInfo,
} from "../src/engine";
import type { FromWebview, Tab, ToWebview } from "../src/transformationProtocol";
import { GraphView, svg, textWidth, type GraphEdgeView, type GraphNodeView } from "./graph";

declare function acquireVsCodeApi(): { postMessage(m: unknown): void; getState(): unknown; setState(s: unknown): void };
const vscode = acquireVsCodeApi();
const post = (m: FromWebview) => vscode.postMessage(m);

// ---------------------------------------------------------------- state

interface Saved {
  /** Parameter values by unit and parameter (as typed). */
  params: Record<string, Record<string, string>>;
  selectedUnit?: string;
  spaceRules?: string[];
  maxStates: number;
  maxDepth: string;
  validate: boolean;
  showDeleted: boolean;
  tab: Tab;
}

const saved: Saved = { params: {}, maxStates: 500, maxDepth: "", validate: true, showDeleted: true, tab: "steps", ...(vscode.getState() as Partial<Saved> | undefined) };
const save = () => vscode.setState(saved);

let documentName = "";
let modelName = "";
let units: TransformationUnitInfo[] = [];
let unitErrors = 0;
let state: TransformationState | undefined;
let matches: { rule: string; list: MatchList } | undefined;
let hoveredMatch: number | undefined;
let selectedMatch: number | undefined;
let space: StateSpaceInfo | undefined;
let selectedState: number | undefined;
let details: StateSpaceStateDetails | undefined;
let replay: ReturnType<typeof setInterval> | undefined;
let busyCount = 0;

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? p;
const shortName = (q: string) => q.slice(q.lastIndexOf("::") + 2);

// ---------------------------------------------------------------- DOM helpers

type Child = Node | string | false | null | undefined;

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<Record<string, string | boolean | number | ((e: Event) => void)>> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === false) continue;
    if (typeof v === "function") e.addEventListener(k.replace(/^on/, "").toLowerCase(), v as EventListener);
    else if (k === "class") e.className = String(v);
    else if (k === "value" && (e instanceof HTMLInputElement || e instanceof HTMLSelectElement)) e.value = String(v);
    else if (k === "checked" && e instanceof HTMLInputElement) e.checked = Boolean(v);
    else e.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children) if (c !== false && c !== null && c !== undefined) e.append(c);
  return e;
}

function section(title: string, ...children: Child[]): HTMLElement {
  return h("section", {}, h("h2", {}, title), ...children);
}

const fontFamily = getComputedStyle(document.body).fontFamily || "sans-serif";
const monoFamily = getComputedStyle(document.body).getPropertyValue("--vscode-editor-font-family").trim() || "monospace";

// ---------------------------------------------------------------- layout

const app = document.getElementById("app")!;
const sidebarTitle = h("div", { class: "title" });
const tabButtons = { steps: h("button", { class: "tab", onclick: () => setTab("steps") }, "Steps"), space: h("button", { class: "tab", onclick: () => setTab("space") }, "State Space") };
const notices = h("div", { class: "notices" });
const stepsSide = h("div", { class: "side-content" });
const spaceSide = h("div", { class: "side-content" });
const busyIndicator = h("div", { class: "busy", hidden: true }, h("span", { class: "spinner" }), h("span", { class: "busy-text" }));

const stepsMain = h("div", { class: "pane" });
const stepsToolbar = h("div", { class: "toolbar" });
const stepsCanvas = h("div", { class: "canvas" });
const legend = h("div", { class: "legend" });
stepsMain.append(stepsToolbar, stepsCanvas, legend);

const spaceMain = h("div", { class: "pane split" });
const spaceCanvas = h("div", { class: "canvas" });
const splitter = h("div", { class: "splitter", title: "Drag to resize" });
const spaceStateBar = h("div", { class: "toolbar" });
const spaceStateCanvas = h("div", { class: "canvas" });
const spaceLegend = h("div", { class: "legend" });
const spaceBottom = h("div", { class: "pane-bottom" }, spaceStateBar, spaceStateCanvas);
spaceMain.append(spaceCanvas, spaceLegend, splitter, spaceBottom);

app.append(
  h("aside", { class: "sidebar" }, h("header", {}, sidebarTitle, h("nav", { class: "tabs" }, tabButtons.steps, tabButtons.space)), notices, stepsSide, spaceSide),
  h("main", {}, stepsMain, spaceMain, busyIndicator),
);

{
  let dragging = false;
  splitter.addEventListener("pointerdown", (e) => {
    dragging = true;
    splitter.setPointerCapture(e.pointerId);
  });
  splitter.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    const rect = spaceMain.getBoundingClientRect();
    const top = Math.min(rect.height - 120, Math.max(120, e.clientY - rect.top));
    spaceMain.style.gridTemplateRows = `${top}px 6px 1fr`;
  });
  splitter.addEventListener("pointerup", () => (dragging = false));
}

const objects = new GraphView(stepsCanvas, {
  rankGap: 150,
  rankStrength: 0.08,
  empty: "The model has no objects.",
  onNodeClick: (id) => pickObject(Number(id)),
});
const spaceView = new GraphView(spaceCanvas, {
  rankGap: 90,
  empty: "Select rules and explore the state space.",
  onNodeClick: (id) => selectSpaceState(Number(id)),
  onNodeDoubleClick: (id) => post({ type: "adoptState", state: Number(id) }),
});
const spaceObjects = new GraphView(spaceStateCanvas, { rankGap: 150, rankStrength: 0.08, empty: "Select a state." });

function setTab(tab: Tab): void {
  saved.tab = tab;
  save();
  tabButtons.steps.classList.toggle("active", tab === "steps");
  tabButtons.space.classList.toggle("active", tab === "space");
  stepsSide.hidden = tab !== "steps";
  stepsMain.hidden = tab !== "steps";
  spaceSide.hidden = tab !== "space";
  spaceMain.hidden = tab !== "space";
  if (tab === "steps") renderSteps();
  else renderSpaceSide();
}

function notice(kind: "error" | "warning" | "info", message: string, timeout = kind === "error" ? 0 : 8000): void {
  const close = h("button", { class: "icon", title: "Dismiss" }, "×");
  const n = h("div", { class: `notice ${kind}` }, h("span", {}, message), close);
  close.addEventListener("click", () => n.remove());
  notices.append(n);
  while (notices.children.length > 4) notices.firstElementChild?.remove();
  if (timeout) setTimeout(() => n.remove(), timeout);
}

function setBusy(busy: boolean, what?: string): void {
  busyCount = Math.max(0, busyCount + (busy ? 1 : -1));
  busyIndicator.hidden = busyCount === 0;
  if (what) busyIndicator.querySelector(".busy-text")!.textContent = `${what}…`;
  app.classList.toggle("is-busy", busyCount > 0);
}

// ---------------------------------------------------------------- parameters

function paramValue(unit: string, p: TransformationParameter): string {
  return saved.params[unit]?.[p.name] ?? "";
}

function setParam(unit: string, name: string, value: string): void {
  (saved.params[unit] ??= {})[name] = value;
  save();
}

/** The parameter values of a unit to send: node ids as numbers, the rest as typed. */
function parameters(unit: TransformationUnitInfo): ParameterValues {
  const out: ParameterValues = {};
  for (const p of unit.parameters) {
    const v = paramValue(unit.name, p);
    if (p.kind === "var" || v === "") continue;
    if (p.node) {
      const id = Number(v);
      if (state?.graph.nodes.some((n) => n.id === id)) out[p.name] = id;
    } else {
      out[p.name] = v;
    }
  }
  return out;
}

function paramInput(unit: TransformationUnitInfo, p: TransformationParameter): HTMLElement {
  const value = paramValue(unit.name, p);
  const label = h("label", { class: "param" }, h("span", { class: "param-name" }, h("span", { class: `kind kind-${p.kind}` }, p.kind === "unknown" ? "" : p.kind.toUpperCase()), p.name), h("span", { class: "param-type" }, p.type));
  let input: HTMLInputElement | HTMLSelectElement;
  if (p.kind === "var") {
    input = h("input", { type: "text", disabled: true, placeholder: "bound by matching" });
  } else if (p.node) {
    const nodes = state?.graph.nodes ?? [];
    const fits = nodes.filter((n) => shortName(n.class) === p.type);
    const others = nodes.filter((n) => shortName(n.class) !== p.type);
    input = h(
      "select",
      {},
      h("option", { value: "" }, p.kind === "out" ? "— result —" : "— any (matched) —"),
      fits.length > 0 && h("optgroup", { label: p.type }, ...fits.map((n) => h("option", { value: String(n.id) }, `${n.label}  #${n.id}`))),
      others.length > 0 && h("optgroup", { label: "Other objects" }, ...others.map((n) => h("option", { value: String(n.id) }, `${n.label}  #${n.id}`))),
    );
    input.value = nodes.some((n) => String(n.id) === value) ? value : "";
  } else if (p.literals?.length || p.type === "EBoolean" || p.type === "EBooleanObject") {
    const options = p.literals?.length ? p.literals : ["true", "false"];
    input = h("select", {}, h("option", { value: "" }, "— none —"), ...options.map((o) => h("option", { value: o }, o)));
    input.value = value;
  } else {
    const numeric = /^E(Int|Long|Short|Byte|Double|Float|BigInteger|BigDecimal)/.test(p.type);
    input = h("input", { type: "text", value, placeholder: p.kind === "out" ? "result" : numeric ? "number (empty: matched)" : "value (empty: matched)", spellcheck: "false" });
  }
  input.addEventListener("change", () => {
    setParam(unit.name, p.name, input.value);
    if (matches?.rule === unit.name) {
      matches = undefined;
      renderSteps();
    }
  });
  if (input instanceof HTMLInputElement) {
    input.addEventListener("input", () => setParam(unit.name, p.name, input.value));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") apply(unit);
    });
  }
  label.append(input);
  return label;
}

/** A click on an object fills a node parameter of the selected rule or unit with it: the first
 * empty one of its class, else the first one of its class. */
function pickObject(id: number): void {
  const unit = units.find((u) => u.name === saved.selectedUnit);
  const node = state?.graph.nodes.find((n) => n.id === id);
  if (!unit || !node) return;
  const fitting = unit.parameters.filter((p) => p.node && p.kind !== "var" && p.kind !== "out" && p.type === shortName(node.class));
  const param = fitting.find((p) => paramValue(unit.name, p) === "") ?? fitting[0];
  if (!param) return;
  setParam(unit.name, param.name, String(id));
  if (matches?.rule === unit.name) matches = undefined;
  renderSteps();
  renderObjects();
}

function apply(unit: TransformationUnitInfo, match?: number): void {
  stopReplay();
  post({ type: "apply", unit: unit.name, parameters: parameters(unit), match });
}

// ---------------------------------------------------------------- steps tab

function renderSteps(): void {
  const selected = units.find((u) => u.name === saved.selectedUnit);
  const unitList = h("ul", { class: "list units" });
  for (const u of units) {
    const isSelected = u === selected;
    const li = h(
      "li",
      { class: `item${isSelected ? " selected" : ""}`, title: `${u.kind} ${u.name} — double-click to show its definition` },
      h("span", { class: `badge ${u.kind}` }, u.kind === "rule" ? "R" : "U"),
      h("span", { class: "name" }, u.name),
      h("span", { class: "detail" }, u.parameters.map((p) => p.name).join(", ")),
      (u.multiRules?.length ?? 0) > 0 && h("span", { class: "chip", title: `multi-rules: ${u.multiRules!.join(", ")}` }, `+${u.multiRules!.length}`),
    );
    li.addEventListener("click", () => {
      if (saved.selectedUnit === u.name) return;
      saved.selectedUnit = u.name;
      save();
      renderSteps();
    });
    li.addEventListener("dblclick", () => post({ type: "reveal", unit: u.name }));
    unitList.append(li);
    if (isSelected) unitList.append(h("li", { class: "form" }, unitForm(u)));
  }
  const children: Child[] = [
    unitErrors > 0 && h("div", { class: "notice error inline" }, `The transformation has ${unitErrors} error${unitErrors === 1 ? "" : "s"}: fix ${unitErrors === 1 ? "it" : "them"} to apply rules.`),
    section(`Rules and units · ${baseName(documentName)}`, units.length ? unitList : h("p", { class: "hint" }, "No rules or units yet.")),
  ];
  if (selected?.kind === "rule" && matches?.rule === selected.name) children.push(matchSection(selected));
  if (state) {
    children.push(stepsSection(state), changesSection(state.changes, state.graph, state.steps[state.step]?.label), violationsSection(state.violations));
  }
  stepsSide.replaceChildren(...children.filter((c): c is Node => !!c));
  renderToolbar();
}

function unitForm(u: TransformationUnitInfo): HTMLElement {
  const form = h("div", { class: "unit-form" }, ...u.parameters.map((p) => paramInput(u, p)));
  if (u.parameters.some((p) => p.node && p.kind !== "var" && p.kind !== "out")) {
    form.append(h("p", { class: "hint" }, "Click an object in the diagram to pass it. Parameters without a value are bound by matching."));
  }
  const buttons = h("div", { class: "buttons" });
  if (u.kind === "rule") {
    buttons.append(
      h("button", { class: "secondary", title: "List the matches of the rule", onclick: () => post({ type: "findMatches", rule: u.name, parameters: parameters(u) }) }, "Find Matches"),
      h("button", { class: "primary", title: "Apply the rule at its first match (or the selected one)", onclick: () => apply(u, matches?.rule === u.name ? selectedMatch : undefined) }, "Apply"),
    );
  } else {
    buttons.append(h("button", { class: "primary", title: "Execute the unit", onclick: () => apply(u) }, "Run Unit"));
  }
  form.append(buttons);
  if (state && state.step < state.steps.length - 1) {
    const later = state.steps.length - 1 - state.step;
    form.append(h("p", { class: "hint" }, `Applies to step ${state.step}: the ${later === 1 ? "step after it is" : `${later} steps after it are`} replaced.`));
  }
  return form;
}

function matchSection(rule: TransformationUnitInfo): HTMLElement {
  const list = matches!.list;
  const ul = h("ul", { class: "list matches" });
  for (const m of list.matches) {
    const li = h(
      "li",
      { class: `item wrap${m.index === selectedMatch ? " selected" : ""}`, title: `${m.label}\nClick to select, double-click to apply` },
      h("span", { class: "index" }, String(m.index + 1)),
      h("span", { class: "name" }, m.parameters.length ? m.parameters.map((p) => p.value).join(" · ") : m.nodes.map((n) => `${n.name} → #${n.node}`).join(" · ")),
      (m.instances ?? 0) > 0 && h("span", { class: "chip", title: "multi-rule matches applied with it" }, `×${m.instances}`),
    );
    li.addEventListener("mouseenter", () => {
      hoveredMatch = m.index;
      renderObjects();
    });
    li.addEventListener("mouseleave", () => {
      hoveredMatch = undefined;
      renderObjects();
    });
    li.addEventListener("click", () => {
      selectedMatch = selectedMatch === m.index ? undefined : m.index;
      renderSteps();
      renderObjects();
    });
    li.addEventListener("dblclick", () => apply(rule, m.index));
    ul.append(li);
  }
  const count = list.matches.length;
  return section(
    `Matches of ${rule.name} · ${count}${list.more ? "+" : ""}`,
    count ? ul : h("p", { class: "hint" }, "No matches: the rule is not applicable here."),
    ...list.messages.map((m) => h("p", { class: "hint warning" }, m)),
  );
}

function stepsSection(s: TransformationState): HTMLElement {
  const ul = h("ul", { class: "list steps" });
  for (const step of s.steps) {
    const li = h(
      "li",
      { class: `item wrap${step.index === s.step ? " selected" : ""}`, title: step.applications.map((a) => a.label).join("\n") || step.label },
      h("span", { class: "index" }, String(step.index)),
      h("span", { class: "name" }, step.label),
      step.applications.length > 1 && h("span", { class: "chip", title: "rule applications" }, `${step.applications.length} rules`),
    );
    li.addEventListener("click", () => {
      stopReplay();
      post({ type: "selectStep", step: step.index });
    });
    ul.append(li);
  }
  const at = s.step;
  const last = s.steps.length - 1;
  const nav = h(
    "div",
    { class: "buttons" },
    h("button", { class: "secondary icon-text", title: "First step", disabled: at === 0, onclick: () => post({ type: "selectStep", step: 0 }) }, "⏮"),
    h("button", { class: "secondary icon-text", title: "Previous step (←)", disabled: at === 0, onclick: () => post({ type: "selectStep", step: at - 1 }) }, "◀"),
    h("button", { class: "secondary icon-text", title: "Next step (→)", disabled: at === last, onclick: () => post({ type: "selectStep", step: at + 1 }) }, "▶"),
    h("button", { class: "secondary icon-text", title: "Last step", disabled: at === last, onclick: () => post({ type: "selectStep", step: last }) }, "⏭"),
    h("button", { class: "secondary", title: "Replay the steps from the start", disabled: last === 0, onclick: () => (replay ? stopReplay() : startReplay()) }, replay ? "Stop" : "Replay"),
  );
  return section(`Steps · ${baseName(modelName)}`, ul, nav);
}

function changesSection(c: GraphChanges, graph: GraphInfo, label?: string): HTMLElement {
  const name = (id: number) => graph.nodes.find((n) => n.id === id)?.label ?? c.deletedNodes.find((n) => n.id === id)?.label ?? `#${id}`;
  const lines: Child[] = [];
  for (const id of c.createdNodes) lines.push(h("li", { class: "created" }, "+ ", name(id)));
  for (const n of c.deletedNodes) lines.push(h("li", { class: "deleted" }, "− ", n.label));
  for (const a of c.changedAttributes) {
    lines.push(h("li", { class: "changed" }, "~ ", `${name(a.node)}.${a.feature}: ${a.old.join(", ") || "∅"} → ${a.new.join(", ") || "∅"}`));
  }
  for (const e of c.createdEdges) lines.push(h("li", { class: "created" }, "+ ", `${name(e.source)} —${e.feature}→ ${name(e.target)}`));
  for (const e of c.deletedEdges) lines.push(h("li", { class: "deleted" }, "− ", `${name(e.source)} —${e.feature}→ ${name(e.target)}`));
  const title = state && state.step > 0 ? `Changes of step ${state.step}` : "Changes";
  return section(
    title,
    lines.length ? h("ul", { class: "changes" }, ...lines) : h("p", { class: "hint" }, state?.step ? "Nothing changed." : "The initial model."),
    label && state?.step ? h("p", { class: "hint" }, label) : null,
  );
}

function violationsSection(violations: StateViolation[]): HTMLElement {
  if (!violations.length) return section("Invariants", h("p", { class: "hint ok" }, "✓ The state satisfies all invariants."));
  return section(
    `Invariants · ${violations.length} violated`,
    h(
      "ul",
      { class: "changes" },
      ...violations.map((v) =>
        h("li", { class: v.status === "violated" ? "deleted" : "changed", title: v.message ?? "" }, `${v.constraint} on ${v.label}`, v.message ? h("div", { class: "sub" }, v.message) : null),
      ),
    ),
  );
}

function renderToolbar(): void {
  const showDeleted = h("input", { type: "checkbox", checked: saved.showDeleted });
  showDeleted.addEventListener("change", () => {
    saved.showDeleted = showDeleted.checked;
    save();
    renderObjects();
  });
  stepsToolbar.replaceChildren(
    h("span", { class: "step-title" }, state ? `Step ${state.step}${state.step ? ` · ${state.steps[state.step]?.label ?? ""}` : " · initial model"}` : ""),
    h("span", { class: "spacer" }),
    h("label", { class: "toggle", title: "Show the objects and links the step deleted" }, showDeleted, "deleted"),
    h("button", { class: "secondary", title: "Fit to view (double-click the background)", onclick: () => objects.fit() }, "Fit"),
    h("button", { class: "secondary", title: "Lay the diagram out anew", onclick: () => { objects.reset(); renderObjects(); } }, "Re-layout"),
    h("button", { class: "secondary", title: "Export this step as a HUTN model", onclick: () => post({ type: "export", format: "hutn" }) }, "Export HUTN"),
    h("button", { class: "secondary", title: "Export this step as an XMI model", onclick: () => post({ type: "export", format: "xmi" }) }, "Export XMI"),
    h("button", { class: "secondary", title: "Start again from the model", onclick: () => post({ type: "restart" }) }, "Restart"),
  );
  legend.replaceChildren(
    h("span", { class: "swatch created" }), "created",
    h("span", { class: "swatch deleted" }), "deleted",
    h("span", { class: "swatch changed" }), "changed",
    h("span", { class: "swatch matched" }), "matched",
    h("span", { class: "swatch violated" }), "violates an invariant",
  );
}

function startReplay(): void {
  if (!state) return;
  post({ type: "selectStep", step: 0 });
  replay = setInterval(() => {
    if (!state || state.step >= state.steps.length - 1) return stopReplay();
    post({ type: "selectStep", step: state.step + 1 });
  }, 1200);
  renderSteps();
}

function stopReplay(): void {
  if (!replay) return;
  clearInterval(replay);
  replay = undefined;
  renderSteps();
}

// ---------------------------------------------------------------- object diagrams

interface Highlight {
  changes?: GraphChanges;
  violations?: StateViolation[];
  /** Nodes to mark as matched, with the names of the rule nodes that matched them. */
  matched?: Map<number, string[]>;
  /** Rule node names to show (muted) on nodes, e.g. of the applied step. */
  tags?: Map<number, string[]>;
  showDeleted: boolean;
}

function containmentDepth(graph: GraphNode[]): Map<number, number> {
  const byId = new Map(graph.map((n) => [n.id, n]));
  const depth = new Map<number, number>();
  const of = (n: GraphNode, guard = 0): number => {
    if (depth.has(n.id)) return depth.get(n.id)!;
    const parent = n.container !== undefined ? byId.get(n.container) : undefined;
    const d = parent && guard < 50 ? of(parent, guard + 1) + 1 : 0;
    depth.set(n.id, d);
    return d;
  };
  graph.forEach((n) => of(n));
  return depth;
}

function objectNode(n: GraphNode, hl: Highlight, rank: number | undefined, ghost: boolean): GraphNodeView {
  const changed = new Map((hl.changes?.changedAttributes ?? []).filter((c) => c.node === n.id).map((c) => [c.feature, c]));
  const violations = (hl.violations ?? []).filter((v) => v.node === n.id);
  const lines: { text: string; changed: boolean }[] = n.attributes.map((a) => {
    const c = changed.get(a.name);
    return { text: c ? `${a.name} = ${c.old.join(", ") || "∅"} → ${a.values.join(", ")}` : `${a.name} = ${a.values.join(", ")}`, changed: !!c };
  });
  for (const c of changed.values()) {
    if (!n.attributes.some((a) => a.name === c.feature)) lines.push({ text: `${c.feature} = ${c.old.join(", ")} → ∅`, changed: true });
  }
  const shown = lines.slice(0, 8);
  if (lines.length > shown.length) shown.push({ text: `… ${lines.length - shown.length} more`, changed: false });
  const space = n.label.indexOf(" ");
  const cls = space > 0 ? n.label.slice(0, space) : n.label;
  const name = space > 0 ? n.label.slice(space + 1) : "";
  const headerFont = `600 12px ${fontFamily}`;
  const width = Math.max(
    96,
    textWidth(cls, headerFont) + (name ? textWidth(` ${name}`, `12px ${fontFamily}`) : 0) + 22,
    ...shown.map((l) => textWidth(l.text, `11px ${monoFamily}`) + 18),
  );
  const height = 26 + (shown.length ? shown.length * 15 + 6 : 0);
  const created = hl.changes?.createdNodes.includes(n.id) ?? false;
  const matchedBy = hl.matched?.get(n.id);
  const tags = matchedBy ?? hl.tags?.get(n.id);
  const classes = [
    ghost ? "deleted" : "",
    created ? "created" : "",
    changed.size ? "changed" : "",
    matchedBy ? "matched" : "",
    violations.length ? "violated" : "",
  ].filter(Boolean);
  const title = [
    `${n.label} (#${n.id})`,
    n.class,
    ...violations.map((v) => `✗ ${v.constraint}${v.message ? `: ${v.message}` : ""}`),
    ghost ? "deleted by this step" : created ? "created by this step" : "",
  ].filter(Boolean).join("\n");
  return {
    id: String(n.id),
    width,
    height,
    shape: "rect",
    rank,
    ghost,
    classes,
    title,
    draw(g) {
      svg("rect", { x: -width / 2, y: -height / 2, width, height, rx: 4, class: "box" }, g);
      const header = svg("text", { x: -width / 2 + 10, y: -height / 2 + 17, class: "header" }, g);
      svg("tspan", { class: "cls" }, header).textContent = cls;
      if (name) svg("tspan", { class: "obj" }, header).textContent = ` ${name}`;
      if (shown.length) {
        svg("line", { x1: -width / 2, x2: width / 2, y1: -height / 2 + 25, y2: -height / 2 + 25, class: "sep" }, g);
        shown.forEach((l, i) => {
          svg("text", { x: -width / 2 + 10, y: -height / 2 + 41 + i * 15, class: `attr${l.changed ? " changed" : ""}` }, g).textContent = l.text;
        });
      }
      if (tags?.length) {
        const text = tags.join(", ");
        const w = textWidth(text, `600 10.5px ${fontFamily}`) + 10;
        const tag = svg("g", { class: `tag${matchedBy ? "" : " muted"}`, transform: `translate(${-width / 2},${-height / 2 - 19})` }, g);
        svg("rect", { x: 0, y: 0, width: w, height: 16, rx: 8 }, tag);
        svg("text", { x: w / 2, y: 11.5, "text-anchor": "middle" }, tag).textContent = text;
      }
      if (violations.length) {
        const badge = svg("g", { class: "badge-violation", transform: `translate(${width / 2},${-height / 2})` }, g);
        svg("circle", { r: 8 }, badge);
        svg("text", { y: 4, "text-anchor": "middle" }, badge).textContent = "!";
      }
    },
  };
}

function objectEdge(e: GraphEdge, classes: string[]): GraphEdgeView {
  return {
    source: String(e.source),
    target: String(e.target),
    label: e.feature,
    classes: [e.containment ? "containment" : "reference", ...classes],
    diamond: e.containment,
    title: `${e.feature}${e.opposite ? ` (opposite: ${e.opposite})` : ""}${classes.includes("created") ? " — created" : classes.includes("deleted") ? " — deleted" : ""}`,
  };
}

const edgeKey = (e: GraphEdge) => `${e.source}|${e.feature}|${e.target}`;

function drawObjects(view: GraphView, graph: GraphInfo, hl: Highlight): void {
  const ghosts = hl.showDeleted ? (hl.changes?.deletedNodes ?? []) : [];
  const depth = containmentDepth([...graph.nodes, ...ghosts]);
  const nodes = [
    ...graph.nodes.map((n) => objectNode(n, hl, depth.get(n.id), false)),
    ...ghosts.map((n) => objectNode(n, hl, depth.get(n.id), true)),
  ];
  const created = new Set((hl.changes?.createdEdges ?? []).map(edgeKey));
  const matched = hl.matched;
  const edges = graph.edges.map((e) => {
    const classes = created.has(edgeKey(e)) ? ["created"] : matched?.has(e.source) && matched.has(e.target) ? ["highlight"] : [];
    return objectEdge(e, classes);
  });
  if (hl.showDeleted) {
    const present = new Set([...graph.nodes, ...ghosts].map((n) => n.id));
    for (const e of hl.changes?.deletedEdges ?? []) {
      if (present.has(e.source) && present.has(e.target)) edges.push(objectEdge(e, ["deleted"]));
    }
  }
  view.render(nodes, edges);
}

function renderObjects(): void {
  if (!state) return;
  const m = matches?.list.matches.find((x) => x.index === (hoveredMatch ?? selectedMatch));
  let matched: Map<number, string[]> | undefined;
  if (m) {
    matched = new Map();
    for (const n of m.nodes) matched.set(n.node, [...(matched.get(n.node) ?? []), n.name]);
    for (const id of m.matched) if (!matched.has(id)) matched.set(id, []);
  }
  let tags: Map<number, string[]> | undefined;
  const applications = state.steps[state.step]?.applications ?? [];
  if (!matched && applications.length === 1) {
    tags = new Map();
    for (const n of applications[0].nodes) if (state.graph.nodes.some((x) => x.id === n.node)) tags.set(n.node, [...(tags.get(n.node) ?? []), n.name]);
  }
  drawObjects(objects, state.graph, { changes: state.changes, violations: state.violations, matched, tags, showDeleted: saved.showDeleted });
}

// ---------------------------------------------------------------- state space tab

function renderSpaceSide(): void {
  const rules = units.filter((u) => u.kind === "rule");
  const selected = new Set(saved.spaceRules ?? rules.map((r) => r.name));
  const list = h("ul", { class: "list rules" });
  for (const r of rules) {
    const box = h("input", { type: "checkbox", checked: selected.has(r.name) });
    box.addEventListener("change", () => {
      if (box.checked) selected.add(r.name);
      else selected.delete(r.name);
      saved.spaceRules = rules.map((x) => x.name).filter((n) => selected.has(n));
      save();
    });
    const given = Object.keys(parameters(r));
    list.append(
      h(
        "li",
        { class: "item check" },
        h("label", {}, box, h("span", { class: "name" }, r.name)),
        given.length > 0 && h("span", { class: "detail", title: "Parameter values set in the Steps tab" }, given.map((g) => `${g}=${saved.params[r.name]?.[g]}`).join(", ")),
      ),
    );
  }
  const maxStates = h("input", { type: "number", min: "1", value: String(saved.maxStates) });
  maxStates.addEventListener("change", () => {
    saved.maxStates = Math.max(1, Number(maxStates.value) || 500);
    save();
  });
  const maxDepth = h("input", { type: "number", min: "0", value: saved.maxDepth, placeholder: "unlimited" });
  maxDepth.addEventListener("change", () => {
    saved.maxDepth = maxDepth.value;
    save();
  });
  const validate = h("input", { type: "checkbox", checked: saved.validate });
  validate.addEventListener("change", () => {
    saved.validate = validate.checked;
    save();
  });
  const explore = h(
    "button",
    {
      class: "primary",
      disabled: unitErrors > 0 || !state,
      onclick: () => {
        const names = rules.map((r) => r.name).filter((n) => selected.has(n));
        if (!names.length) return notice("warning", "Select at least one rule.");
        const params: Record<string, ParameterValues> = {};
        for (const r of rules) if (selected.has(r.name)) params[r.name] = parameters(r);
        selectedState = undefined;
        details = undefined;
        post({ type: "explore", rules: names, parameters: params, maxStates: saved.maxStates, maxDepth: saved.maxDepth === "" ? undefined : Number(saved.maxDepth), validate: saved.validate });
      },
    },
    `Explore from Step ${state?.step ?? 0}`,
  );
  const children: Child[] = [
    unitErrors > 0 && h("div", { class: "notice error inline" }, "The transformation has errors: fix them to explore."),
    section(
      "Rules",
      rules.length ? list : h("p", { class: "hint" }, "No rules."),
      h("p", { class: "hint" }, "Parameters without a value are bound by matching; values set in the Steps tab are used."),
    ),
    section(
      "Limits",
      h("label", { class: "param" }, h("span", { class: "param-name" }, "Maximum states"), maxStates),
      h("label", { class: "param" }, h("span", { class: "param-name" }, "Maximum depth"), maxDepth),
      h("label", { class: "toggle" }, validate, "Check the invariants of every state"),
      h("div", { class: "buttons" }, explore),
    ),
  ];
  if (space) children.push(summarySection(space));
  if (space && selectedState !== undefined) children.push(stateSection(space, selectedState));
  spaceSide.replaceChildren(...children.filter((c): c is Node => !!c));
  spaceLegend.replaceChildren(
    h("span", { class: "swatch state initial" }), "initial",
    h("span", { class: "swatch state terminal" }), "no rule applicable",
    h("span", { class: "swatch state violating" }), "violates an invariant",
    h("span", { class: "swatch state open" }), "not explored (limit)",
  );
}

function summarySection(s: StateSpaceInfo): HTMLElement {
  const terminal = s.states.filter((x) => x.terminal).length;
  const violating = s.states.filter((x) => x.violations > 0).length;
  const open = s.states.filter((x) => !x.explored).length;
  const stat = (n: number, label: string, cls = "") => h("div", { class: `stat ${cls}` }, h("span", { class: "n" }, String(n)), h("span", { class: "l" }, label));
  return section(
    `State space · from step ${s.step}`,
    h("div", { class: "stats" }, stat(s.states.length, "states"), stat(s.transitions.length, "transitions"), stat(terminal, "terminal", terminal ? "warn" : ""), stat(violating, "violating", violating ? "bad" : "")),
    !s.complete && h("p", { class: "hint warning" }, `Incomplete: ${open} state${open === 1 ? " was" : "s were"} not explored (limit reached).`),
    h("p", { class: "hint" }, "Click a state to inspect it, double-click to adopt the path to it as the steps."),
  );
}

function stateSection(s: StateSpaceInfo, id: number): HTMLElement {
  const st = s.states[id];
  const pathList = h("ul", { class: "list steps" });
  pathList.append(h("li", { class: "item", onclick: () => selectSpaceState(0) }, h("span", { class: "index" }, "0"), h("span", { class: "name" }, "initial state")));
  for (const t of details?.id === id ? details.path : []) {
    const tr = s.transitions[t];
    pathList.append(h("li", { class: `item${tr.target === id ? " selected" : ""}`, onclick: () => selectSpaceState(tr.target), title: tr.label }, h("span", { class: "index" }, String(tr.target)), h("span", { class: "name" }, tr.label)));
  }
  const outgoing = s.transitions.filter((t) => t.source === id);
  return section(
    `State ${id}`,
    h("p", { class: "hint" }, `${st.nodes} objects · depth ${st.depth} · ${outgoing.length} outgoing transition${outgoing.length === 1 ? "" : "s"}${st.terminal ? " · no rule applicable" : ""}${st.explored ? "" : " · not explored"}`),
    h("h3", {}, "Shortest path"),
    pathList,
    details?.id === id && details.violations.length > 0
      ? h("ul", { class: "changes" }, ...details.violations.map((v) => h("li", { class: "deleted", title: v.message ?? "" }, `${v.constraint} on ${v.label}`, v.message ? h("div", { class: "sub" }, v.message) : null)))
      : details?.id === id && h("p", { class: "hint ok" }, "✓ The state satisfies all invariants."),
    h("div", { class: "buttons" }, h("button", { class: "primary", title: "Replace the steps after the start of the exploration by the path to this state", onclick: () => post({ type: "adoptState", state: id }) }, "Open Path as Steps")),
  );
}

function selectSpaceState(id: number): void {
  selectedState = id;
  post({ type: "selectState", state: id });
  renderSpaceGraph();
  renderSpaceSide();
}

function renderSpaceGraph(): void {
  if (!space) return;
  const s = space;
  const onPath = new Set<number>(details && details.id === selectedState ? details.path : []);
  const pathStates = new Set<number>([0, ...[...onPath].map((t) => s.transitions[t].target)]);
  const nodes: GraphNodeView[] = s.states.map((st) => {
    const classes = [
      st.id === 0 ? "initial" : "",
      st.terminal ? "terminal" : "",
      st.violations > 0 ? "violating" : "",
      st.explored ? "" : "open",
      st.id === selectedState ? "selected" : "",
      selectedState !== undefined && pathStates.has(st.id) ? "on-path" : "",
    ].filter(Boolean);
    const title = [
      `State ${st.id}${st.id === 0 ? " (initial)" : ""}`,
      `${st.nodes} objects, depth ${st.depth}`,
      st.terminal ? "no rule is applicable" : "",
      st.violations ? `${st.violations} invariant violation${st.violations === 1 ? "" : "s"}` : "",
      st.explored ? "" : "not explored (limit reached)",
    ].filter(Boolean).join("\n");
    return {
      id: String(st.id),
      width: 32,
      height: 32,
      shape: "circle",
      rank: st.depth,
      classes,
      title,
      draw(g) {
        if (st.terminal) svg("circle", { r: 20, class: "outer" }, g);
        svg("circle", { r: 16, class: "disc" }, g);
        svg("text", { y: 4, "text-anchor": "middle", class: "id" }, g).textContent = String(st.id);
      },
    };
  });
  const labels = s.transitions.length <= 250;
  const edges: GraphEdgeView[] = s.transitions.map((t, i) => ({
    source: String(t.source),
    target: String(t.target),
    label: labels ? `${t.rule}${t.count > 1 ? ` ×${t.count}` : ""}` : undefined,
    title: `${t.label}${t.count > 1 ? ` (${t.count} matches)` : ""}`,
    classes: onPath.has(i) ? ["path"] : [],
  }));
  spaceView.render(nodes, edges);
}

function renderSpaceState(): void {
  if (!details || details.id !== selectedState || !space) {
    spaceStateBar.replaceChildren(h("span", { class: "step-title" }, selectedState === undefined ? "No state selected" : `State ${selectedState}`));
    return;
  }
  const last = details.path.length ? space.transitions[details.path[details.path.length - 1]] : undefined;
  spaceStateBar.replaceChildren(
    h("span", { class: "step-title" }, `State ${details.id}${last ? ` · reached by ${last.label}` : " · initial state"}`),
    h("span", { class: "spacer" }),
    h("button", { class: "secondary", onclick: () => spaceObjects.fit() }, "Fit"),
  );
  drawObjects(spaceObjects, details.graph, { changes: details.changes, violations: details.violations, showDeleted: true });
}

// ---------------------------------------------------------------- messages

window.addEventListener("message", (event: MessageEvent<ToWebview>) => {
  const m = event.data;
  switch (m.type) {
    case "init":
      documentName = m.document;
      modelName = m.model;
      sidebarTitle.replaceChildren(h("span", { class: "doc" }, baseName(m.document)), h("span", { class: "arrow" }, " ▸ "), h("span", { class: "model" }, baseName(m.model)));
      setTab(m.tab ?? saved.tab);
      break;
    case "units":
      units = m.units;
      unitErrors = m.errors;
      if (!units.some((u) => u.name === saved.selectedUnit)) saved.selectedUnit = units[0]?.name;
      if (matches && !units.some((u) => u.name === matches!.rule)) matches = undefined;
      if (saved.tab === "steps") renderSteps();
      else renderSpaceSide();
      break;
    case "state": {
      const previous = state;
      state = m.state;
      // Matches belong to the step they were found on.
      if (!previous || previous.step !== state.step || previous.steps.length !== state.steps.length) {
        matches = undefined;
        selectedMatch = undefined;
        hoveredMatch = undefined;
      }
      renderSteps();
      renderObjects();
      if (saved.tab === "space") renderSpaceSide();
      break;
    }
    case "matches":
      matches = { rule: m.rule, list: m.list };
      selectedMatch = undefined;
      renderSteps();
      renderObjects();
      break;
    case "space":
      space = m.space;
      spaceView.reset();
      renderSpaceGraph();
      renderSpaceSide();
      renderSpaceState();
      if (space.states.length > 1) selectSpaceState(space.states.findIndex((s) => s.terminal) >= 0 ? space.states.findIndex((s) => s.terminal) : 0);
      break;
    case "spaceState":
      details = m.details;
      renderSpaceGraph();
      renderSpaceSide();
      renderSpaceState();
      break;
    case "tab":
      setTab(m.tab);
      break;
    case "busy":
      setBusy(m.busy, m.what);
      break;
    case "notice":
      notice(m.kind, m.message);
      break;
  }
});

document.addEventListener("keydown", (e) => {
  if (saved.tab !== "steps" || !state || (e.target as Element).closest("input, select, textarea")) return;
  if (e.key === "ArrowLeft" && state.step > 0) post({ type: "selectStep", step: state.step - 1 });
  if (e.key === "ArrowRight" && state.step < state.steps.length - 1) post({ type: "selectStep", step: state.step + 1 });
});

setTab(saved.tab);
post({ type: "ready" });

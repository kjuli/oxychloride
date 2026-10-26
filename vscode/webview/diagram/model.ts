// What the diagram shows of a metamodel, independent of React: labels as the document's syntax
// writes them, parsing what is typed into the diagram, and the nodes and edges of the graph.

import type {
  DiagramClassifier,
  DiagramExternal,
  DiagramFeature,
  DiagramOperation,
  DiagramProblem,
  DiagramType,
  MetamodelDiagram,
  ParameterSpec,
  Position,
  Range,
} from "../../src/engine";

export type Syntax = MetamodelDiagram["syntax"];

// ---------------------------------------------------------------- labels

/** A multiplicity as the syntax writes it; the syntax's default (OCLinEcore: `[1]`, Ecore:
 * `[0..1]`) is not shown, as in the text. */
export function multiplicity(lower: number, upper: number, syntax: Syntax): string {
  if (syntax === "oclinecore") {
    if (lower === 1 && upper === 1) return "";
    if (lower === 0 && upper === 1) return "[?]";
    if (lower === 0 && upper < 0) return "[*]";
    if (lower === 1 && upper < 0) return "[+]";
  } else if (lower === 0 && upper === 1) {
    return "";
  }
  if (lower === upper) return `[${lower}]`;
  return `[${lower}..${upper < 0 ? "*" : upper}]`;
}

/** The default multiplicity of the syntax, for text that does not give one. */
export function defaultMultiplicity(syntax: Syntax): { lower: number; upper: number } {
  return syntax === "oclinecore" ? { lower: 1, upper: 1 } : { lower: 0, upper: 1 };
}

export function featureLabel(f: DiagramFeature, syntax: Syntax): string {
  const type = f.typeName ? ` : ${f.typeName}${multiplicity(f.lower, f.upper, syntax)}` : "";
  return `${f.derived ? "/" : ""}${f.name}${type}`;
}

/** A feature as typed to edit it: `name : Type[m]`. */
export function featureText(f: DiagramFeature, syntax: Syntax): string {
  return `${f.name}${f.typeName ? ` : ${f.typeName}${multiplicity(f.lower, f.upper, syntax)}` : ""}`;
}

export function operationLabel(o: DiagramOperation, syntax: Syntax): string {
  const params = o.parameters.map((p) => `${p.name} : ${p.typeName}${multiplicity(p.lower, p.upper, syntax)}`).join(", ");
  const result = o.typeName ? ` : ${o.typeName}${multiplicity(o.lower, o.upper, syntax)}` : "";
  return `${o.name}(${params})${result}`;
}

export function parametersText(o: DiagramOperation, syntax: Syntax): string {
  return o.parameters.map((p) => `${p.name} : ${p.typeName}${multiplicity(p.lower, p.upper, syntax)}`).join(", ");
}

/** The role label of a reference end on an edge. */
export function roleLabel(f: DiagramFeature, syntax: Syntax): string {
  return `${f.derived ? "/" : ""}${f.name}${multiplicity(f.lower, f.upper, syntax) ? ` ${multiplicity(f.lower, f.upper, syntax)}` : ""}`;
}

export const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/** `base`, or `base2`, `base3`, … whichever is not taken. */
export function uniqueName(base: string, taken: Iterable<string>): string {
  const names = new Set(taken);
  if (!names.has(base)) return base;
  for (let i = 2; ; i++) if (!names.has(`${base}${i}`)) return `${base}${i}`;
}

// ---------------------------------------------------------------- parsing

export interface Parsed<T> {
  value?: T;
  error?: string;
}

/** `[*]`, `[?]`, `[+]`, `[n]`, `[n..m]`, `[n..*]` (with or without brackets). */
export function parseMultiplicity(text: string): { lower: number; upper: number } | undefined {
  const t = text.trim().replace(/^\[/, "").replace(/\]$/, "").trim();
  if (t === "*") return { lower: 0, upper: -1 };
  if (t === "+") return { lower: 1, upper: -1 };
  if (t === "?") return { lower: 0, upper: 1 };
  const m = /^(\d+)(?:\s*\.\.\s*(\d+|\*))?$/.exec(t);
  if (!m) return undefined;
  const lower = Number(m[1]);
  const upper = m[2] === undefined ? lower : m[2] === "*" ? -1 : Number(m[2]);
  if (upper >= 0 && upper < lower) return undefined;
  return { lower, upper };
}

/** The type a label names: as listed, or by id, or by its simple name if that is unique. */
export function resolveType(label: string, types: DiagramType[], allowed?: (t: DiagramType) => boolean): DiagramType | undefined {
  const candidates = allowed ? types.filter(allowed) : types;
  const t = label.trim();
  const simple = (s: string) => s.slice(s.lastIndexOf("::") + 2).toLowerCase();
  return (
    candidates.find((c) => c.label === t) ??
    candidates.find((c) => c.id === t) ??
    candidates.find((c) => c.label.toLowerCase() === t.toLowerCase()) ??
    (() => {
      const matches = candidates.filter((c) => simple(c.label) === simple(t) || simple(c.id) === simple(t));
      return matches.length === 1 ? matches[0] : undefined;
    })()
  );
}

export interface TypedName {
  name: string;
  type?: string;
  lower: number;
  upper: number;
}

/** `name : Type[m]` (the type and multiplicity are optional). */
export function parseTypedName(text: string, syntax: Syntax): Parsed<TypedName> {
  const m = /^\s*([^\s:[\]()]+)\s*(?::\s*([^[\]]+?))?\s*(\[[^\]]*\])?\s*$/.exec(text);
  if (!m) return { error: "Write a name, optionally followed by ': Type' and a multiplicity such as [0..*]." };
  const bounds = m[3] ? parseMultiplicity(m[3]) : defaultMultiplicity(syntax);
  if (!bounds) return { error: `'${m[3]}' is not a multiplicity (e.g. [1], [0..1], [0..*], [*]).` };
  return { value: { name: m[1], type: m[2]?.trim(), ...bounds } };
}

/** A feature typed into the diagram, with its type resolved. */
export function parseFeature(text: string, syntax: Syntax, types: DiagramType[], reference: boolean): Parsed<TypedName> {
  const parsed = parseTypedName(text, syntax);
  if (!parsed.value) return parsed;
  const { type } = parsed.value;
  if (type === undefined) return parsed;
  const t = resolveType(type, types, (t) => (t.kind === "class") === reference);
  if (!t) return { error: `There is no ${reference ? "class" : "data type or enumeration"} '${type}'.` };
  return { value: { ...parsed.value, type: t.id } };
}

export interface OperationSpec {
  name: string;
  parameters: ParameterSpec[];
  /** The id of the result type (`undefined`: none). */
  type?: string;
  lower: number;
  upper: number;
}

/** `name(p : T, q : U[*]) : R[m]` (parameters, result and multiplicity are optional). */
export function parseOperation(text: string, syntax: Syntax, types: DiagramType[]): Parsed<OperationSpec> {
  const m = /^\s*([^\s:()[\]]+)\s*(?:\((.*)\))?\s*(?::\s*([^[\]]+?)\s*(\[[^\]]*\])?)?\s*$/.exec(text);
  if (!m) return { error: "Write an operation as name(p : Type) : Result." };
  const params = parseParameters(m[2] ?? "", syntax, types);
  if (!params.value) return { error: params.error };
  const op: OperationSpec = { name: m[1], parameters: params.value, ...defaultMultiplicity(syntax) };
  if (m[3]) {
    const t = resolveType(m[3], types);
    if (!t) return { error: `There is no type '${m[3]}'.` };
    op.type = t.id;
    const bounds = m[4] ? parseMultiplicity(m[4]) : defaultMultiplicity(syntax);
    if (!bounds) return { error: `'${m[4]}' is not a multiplicity.` };
    Object.assign(op, bounds);
  }
  return { value: op };
}

/** `p : T, q : U[*]`. */
export function parseParameters(text: string, syntax: Syntax, types: DiagramType[]): Parsed<ParameterSpec[]> {
  const out: ParameterSpec[] = [];
  for (const part of text.split(",").map((p) => p.trim()).filter((p) => p)) {
    const p = parseTypedName(part, syntax);
    if (!p.value) return { error: p.error };
    if (!p.value.type) return { error: `The parameter '${p.value.name}' needs a type.` };
    const t = resolveType(p.value.type, types);
    if (!t) return { error: `There is no type '${p.value.type}'.` };
    out.push({ name: p.value.name, type: t.id, lower: p.value.lower, upper: p.value.upper });
  }
  return { value: out };
}

// ---------------------------------------------------------------- elements

export type ElementKind = "package" | "classifier" | "attribute" | "reference" | "operation" | "literal" | "invariant";

export interface Found {
  kind: ElementKind;
  /** The classifier the element belongs to (or is). */
  classifier?: DiagramClassifier;
  feature?: DiagramFeature;
  operation?: DiagramOperation;
  range: Range;
}

/** The element with an id. */
export function find(d: MetamodelDiagram, id: string): Found | undefined {
  for (const p of d.packages) if (p.id === id) return { kind: "package", range: p.range };
  for (const c of d.classifiers) {
    if (c.id === id) return { kind: "classifier", classifier: c, range: c.range };
    for (const f of c.attributes) if (f.id === id) return { kind: "attribute", classifier: c, feature: f, range: f.range };
    for (const f of c.references) if (f.id === id) return { kind: "reference", classifier: c, feature: f, range: f.range };
    for (const o of c.operations) if (o.id === id) return { kind: "operation", classifier: c, operation: o, range: o.range };
    for (const l of c.literals) if (l.id === id) return { kind: "literal", classifier: c, range: l.range };
    for (const i of c.invariants) if (`${c.id}:${i.name}` === id) return { kind: "invariant", classifier: c, range: i.range };
  }
  return undefined;
}

/** The id of an invariant (invariants are not elements of the metamodel). */
export const invariantId = (c: DiagramClassifier, name: string) => `${c.id}:${name}`;

const before = (a: Position, b: Position) => a.line < b.line || (a.line === b.line && a.column <= b.column);
const contains = (r: Range, p: Position) => before(r.start, p) && before(p, r.end);
const size = (r: Range) => (r.end.line - r.start.line) * 10000 + (r.end.column - r.start.column);

/** The innermost element whose definition contains a text position. */
export function elementAt(d: MetamodelDiagram, p: Position): string | undefined {
  let best: { id: string; size: number } | undefined;
  const consider = (id: string, r: Range) => {
    if (contains(r, p) && (!best || size(r) < best.size)) best = { id, size: size(r) };
  };
  for (const c of d.classifiers) {
    consider(c.id, c.range);
    for (const f of [...c.attributes, ...c.references]) consider(f.id, f.range);
    for (const o of c.operations) consider(o.id, o.range);
    for (const l of c.literals) consider(l.id, l.range);
    for (const i of c.invariants) consider(invariantId(c, i.name), i.range);
  }
  return best?.id;
}

/** The classifier (node) an element belongs to. */
export function ownerOf(d: MetamodelDiagram, id: string): string | undefined {
  return find(d, id)?.classifier?.id;
}

/** Problems by element id. */
export function problemsByElement(d: MetamodelDiagram): Map<string, DiagramProblem[]> {
  const out = new Map<string, DiagramProblem[]>();
  for (const p of d.problems) {
    const id = p.element ?? "";
    out.set(id, [...(out.get(id) ?? []), p]);
  }
  return out;
}

// ---------------------------------------------------------------- graph

export interface ClassifierNodeData extends Record<string, unknown> {
  classifier: DiagramClassifier;
  syntax: Syntax;
  /** Messages of problems, by element id (the classifier's and its members'). */
  problems: Record<string, string[]>;
}

export interface ExternalNodeData extends Record<string, unknown> {
  external: DiagramExternal;
}

export type EdgeKind = "generalization" | "reference";

export interface EdgeData extends Record<string, unknown> {
  kind: EdgeKind;
  /** The reference drawn (for a pair of opposites: the containment, else the first). */
  reference?: DiagramFeature;
  /** Its opposite. */
  opposite?: DiagramFeature;
  syntax: Syntax;
  /** Index among the edges between the same two nodes, centred on 0 (for spreading them). */
  offset: number;
}

export interface GraphNodeSpec {
  id: string;
  type: "classifier" | "external";
  data: ClassifierNodeData | ExternalNodeData;
}

export interface GraphEdgeSpec {
  id: string;
  source: string;
  target: string;
  data: EdgeData;
}

/** The id of a generalization edge. */
export const generalizationId = (sub: string, sup: string) => `${sub} extends ${sup}`;

export function buildGraph(d: MetamodelDiagram): { nodes: GraphNodeSpec[]; edges: GraphEdgeSpec[] } {
  const problems = problemsByElement(d);
  const nodes: GraphNodeSpec[] = d.classifiers.map((c) => {
    const own: Record<string, string[]> = {};
    for (const id of [c.id, ...c.attributes.map((f) => f.id), ...c.references.map((f) => f.id), ...c.operations.map((o) => o.id), ...c.literals.map((l) => l.id)]) {
      const list = problems.get(id)?.filter((p) => p.severity !== "info");
      if (list?.length) own[id] = list.map((p) => p.message);
    }
    return { id: c.id, type: "classifier", data: { classifier: c, syntax: d.syntax, problems: own } };
  });
  const ids = new Set(nodes.map((n) => n.id));
  for (const e of d.external) {
    nodes.push({ id: e.id, type: "external", data: { external: e } });
    ids.add(e.id);
  }
  const edges: GraphEdgeSpec[] = [];
  for (const c of d.classifiers) {
    for (const s of c.superTypes) {
      if (ids.has(s)) edges.push({ id: generalizationId(c.id, s), source: c.id, target: s, data: { kind: "generalization", syntax: d.syntax, offset: 0 } });
    }
  }
  const features = new Map(d.classifiers.flatMap((c) => c.references.map((r) => [r.id, { owner: c.id, feature: r }] as const)));
  const drawn = new Set<string>();
  for (const c of d.classifiers) {
    for (const r of c.references) {
      if (drawn.has(r.id) || !r.type || !ids.has(r.type)) continue;
      const other = r.opposite ? features.get(r.opposite) : undefined;
      // A pair of opposites is one edge, drawn from the container if one end is a containment.
      const [main, opposite] = other?.feature.containment && !r.containment ? [other, { owner: c.id, feature: r }] : [{ owner: c.id, feature: r }, other];
      drawn.add(r.id);
      if (other) drawn.add(other.feature.id);
      // The type of one end is the owner of the other (when the type is not resolved).
      const target = main.feature.type ?? (main === other ? c.id : r.type);
      if (!ids.has(target)) continue;
      edges.push({
        id: main.feature.id,
        source: main.owner,
        target,
        data: { kind: "reference", reference: main.feature, opposite: opposite?.feature, syntax: d.syntax, offset: 0 },
      });
    }
  }
  // Spread the edges between the same two nodes.
  const groups = new Map<string, GraphEdgeSpec[]>();
  for (const e of edges) {
    const key = [e.source, e.target].sort().join("\n");
    groups.set(key, [...(groups.get(key) ?? []), e]);
  }
  for (const group of groups.values()) {
    group.forEach((e, i) => {
      const centred = i - (group.length - 1) / 2;
      // Edges in the other direction bend the other way round.
      e.data.offset = e.source <= e.target ? centred : -centred;
    });
  }
  return { nodes, edges };
}

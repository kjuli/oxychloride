// What the rule diagram draws, computed from the engine's TransformationDiagram: Henshin's
// notation for actions (stereotypes like «create», «delete*» with the path of the multi-rule
// for an element of a multi-rule, «forbid#g» for one of the condition graph g), attribute lines (`name=value`, and
// `name=old->new` for a changed value, as Henshin writes it), finding elements by id or text
// position, and parsing what the user types.

import type {
  NodeClass,
  Position,
  Range,
  RuleAction,
  RuleAttributeDiagram,
  RuleDiagram,
  RuleEdgeDiagram,
  RuleNodeDiagram,
  TransformationDiagram,
  TransformationParameterSpec,
  UnitDiagram,
  UnitStepDiagram,
} from "../../src/engine";

export const ACTIONS: RuleAction[] = ["preserve", "create", "delete", "forbid", "require"];

/** React Flow ids of rule frames and units (nodes of rules keep their diagram ids). */
export const frameId = (rule: string) => `rule:${rule}`;
export const unitId = (unit: string) => `unit:${unit}`;

/** The scope part of an element id (`r/m` of `r/m::n`), and the rule of a scope. */
export const scopeOf = (id: string) => id.split("::")[0];
export const ruleOfScope = (scope: string) => scope.split(/[/#]/)[0];

// Henshin's notation of an action: `«create»`, `«delete*` + `/m»` (an element of the
// multi-rule `m`), `«forbid#g»` (an element of the condition graph `g`).
export function stereotype(action: RuleAction, multi?: string, condition?: string): string {
  let s: string = action;
  if (multi) s += `*${multi.slice(multi.indexOf("/"))}`;
  if (condition) s += condition.slice(condition.lastIndexOf("#"));
  return `«${s}»`;
}

/** A line of a node's attribute compartment. */
export interface AttributeLine {
  /** The attribute it edits (for a changed value: the one that sets it). */
  id: string;
  /** The attributes it shows. */
  ids: string[];
  text: string;
  action: RuleAction;
  /** The stereotype, where the attribute's action differs from the node's. */
  stereotype?: string;
}

/** The attribute lines of a node: a matched value and the value it is set to are one line
 * (`name=old->new`); a value set without a matched one is created. */
export function attributeLines(node: RuleNodeDiagram): AttributeLine[] {
  const lines: AttributeLine[] = [];
  const merged = new Set<string>();
  for (const a of node.attributes) {
    if (merged.has(a.id)) continue;
    if (!a.set) {
      const setter = node.attributes.find((b) => b.set && b.name === a.name && b.scope === a.scope && !merged.has(b.id));
      if (setter) {
        merged.add(setter.id);
        lines.push({ id: setter.id, ids: [a.id, setter.id], text: `${a.name}=${a.value}->${setter.value}`, action: a.action, stereotype: own(node, a) });
        continue;
      }
      lines.push({ id: a.id, ids: [a.id], text: `${a.name}=${a.value}`, action: a.action, stereotype: own(node, a) });
    } else {
      const action: RuleAction = node.action === "preserve" ? "create" : a.action;
      lines.push({ id: a.id, ids: [a.id], text: `${a.name}=${a.value}`, action, stereotype: action === node.action ? undefined : `«${action}»` });
    }
  }
  return lines;
}

/** The stereotype of an attribute whose action (or scope) differs from its node's. */
function own(node: RuleNodeDiagram, a: RuleAttributeDiagram): string | undefined {
  const multi = a.scope && a.scope.includes("/") && !a.scope.includes("#") ? a.scope : undefined;
  const condition = a.scope && a.scope.includes("#") ? a.scope : undefined;
  if (a.action === node.action && !a.scope) return undefined;
  return stereotype(a.action, multi, condition);
}

/** `IN a:EInt, b:Course`. */
export function parametersText(params: TransformationParameterSpec[]): string {
  return params
    .map((p) => {
      const kind = p.kind === "unknown" ? "" : `${p.kind.toUpperCase()} `;
      return `${kind}${p.name}${p.type ? `:${p.type}` : ""}`;
    })
    .join(", ");
}

export type Parsed<T> = { value: T; error?: undefined } | { value?: undefined; error: string };

/** Parses `IN a:EInt, VAR x:EDouble, node:Class`. */
export function parseParameters(text: string): Parsed<TransformationParameterSpec[]> {
  const out: TransformationParameterSpec[] = [];
  for (const part of text.split(",").map((p) => p.trim()).filter(Boolean)) {
    const m = /^(?:(IN|OUT|INOUT|VAR)\s+)?([A-Za-z_]\w*)\s*(?::\s*([A-Za-z_][\w.]*))?$/.exec(part);
    if (!m) return { error: `'${part}' is not a parameter: write 'IN name:Type'` };
    const kind = (m[1]?.toLowerCase() ?? "unknown") as TransformationParameterSpec["kind"];
    out.push({ kind, name: m[2], type: m[3] });
  }
  return { value: out };
}

/** Parses `name=value` (an attribute). */
export function parseAttribute(text: string): Parsed<{ name: string; value: string }> {
  const m = /^\s*([A-Za-z_]\w*)\s*=\s*(.+?)\s*$/.exec(text);
  if (!m) return { error: "Write an attribute as 'name=value'." };
  return { value: { name: m[1], value: m[2] } };
}

export const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

export function uniqueName(base: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let i = 2; ; i++) if (!used.has(`${base}${i}`)) return `${base}${i}`;
}

// ---------------------------------------------------------------- finding elements

export type Found =
  | { kind: "rule"; rule: RuleDiagram; range: Range }
  | { kind: "multi"; rule: RuleDiagram; id: string; range: Range }
  | { kind: "condition"; rule: RuleDiagram; id: string; range: Range }
  | { kind: "node"; rule: RuleDiagram; node: RuleNodeDiagram; range: Range }
  | { kind: "attribute"; rule: RuleDiagram; node: RuleNodeDiagram; attribute: RuleAttributeDiagram; range: Range }
  | { kind: "edge"; rule: RuleDiagram; edge: RuleEdgeDiagram; range: Range }
  | { kind: "unit"; unit: UnitDiagram; range: Range }
  | { kind: "step"; unit: UnitDiagram; step: UnitStepDiagram; range: Range };

function steps(list: UnitStepDiagram[]): UnitStepDiagram[] {
  return list.flatMap((s) => [s, ...(s.children ?? []).flatMap(steps)]);
}

export function find(d: TransformationDiagram, id: string): Found | undefined {
  for (const rule of d.rules) {
    if (rule.id === id) return { kind: "rule", rule, range: rule.range };
    const multi = rule.multiRules.find((m) => m.id === id);
    if (multi) return { kind: "multi", rule, id, range: multi.range };
    const formula = rule.formulas.find((f) => f.graphs.includes(id));
    if (formula) return { kind: "condition", rule, id, range: formula.range };
    for (const node of rule.nodes) {
      if (node.id === id) return { kind: "node", rule, node, range: node.range };
      const attribute = node.attributes.find((a) => a.id === id);
      if (attribute) return { kind: "attribute", rule, node, attribute, range: attribute.range };
    }
    const edge = rule.edges.find((e) => e.id === id);
    if (edge) return { kind: "edge", rule, edge, range: edge.range };
  }
  for (const unit of d.units) {
    if (unit.id === id) return { kind: "unit", unit, range: unit.range };
    const step = steps(unit.body).find((s) => s.id === id);
    if (step) return { kind: "step", unit, step, range: step.range };
  }
  return undefined;
}

const contains = (r: Range, p: Position) =>
  (r.start.line < p.line || (r.start.line === p.line && r.start.column <= p.column)) &&
  (p.line < r.end.line || (p.line === r.end.line && p.column <= r.end.column));

const size = (r: Range) => (r.end.line - r.start.line) * 10000 + (r.end.column - r.start.column);

/** The innermost element whose definition contains a text position. */
export function elementAt(d: TransformationDiagram, p: Position): string | undefined {
  const all: { id: string; range: Range }[] = [];
  for (const r of d.rules) {
    all.push({ id: r.id, range: r.range });
    for (const n of r.nodes) {
      all.push({ id: n.id, range: n.range });
      for (const a of n.attributes) all.push({ id: a.id, range: a.range });
    }
    for (const e of r.edges) all.push({ id: e.id, range: e.range });
  }
  for (const u of d.units) {
    all.push({ id: u.id, range: u.range });
    for (const s of steps(u.body)) all.push({ id: s.id, range: s.range });
  }
  return all.filter((x) => contains(x.range, p)).sort((a, b) => size(a.range) - size(b.range))[0]?.id;
}

/** The React Flow node an element is drawn in (its rule's frame, its node, its unit). */
export function drawnIn(d: TransformationDiagram, id: string): string | undefined {
  const found = find(d, id);
  switch (found?.kind) {
    case "node":
    case "attribute":
      return found.node.id;
    case "rule":
    case "multi":
    case "condition":
    case "edge":
      return frameId(found.rule.id);
    case "unit":
    case "step":
      return unitId(found.unit.id);
    default:
      return undefined;
  }
}

/** The class of a node, if it is known. */
export function classOf(d: TransformationDiagram, node: RuleNodeDiagram): NodeClass | undefined {
  return d.classes.find((c) => c.id === node.classId) ?? d.classes.find((c) => c.name === node.class);
}

/** The references from one node to another that fit their classes. */
export function references(d: TransformationDiagram, source: RuleNodeDiagram, target: RuleNodeDiagram): string[] {
  const s = classOf(d, source);
  const t = classOf(d, target);
  if (!s || !t) return [];
  return s.references.filter((r) => t.conformsTo.includes(r.target) || d.classes.find((c) => c.id === r.target)?.conformsTo.includes(t.id)).map((r) => r.name);
}

/** The scopes of a rule new elements can go into: the rule, its multi-rules, its condition
 * graphs. */
export function scopes(rule: RuleDiagram): { id: string; label: string }[] {
  return [
    { id: rule.id, label: rule.name },
    ...rule.multiRules.map((m) => ({ id: m.id, label: `multi-rule ${m.id.slice(rule.id.length + 1)}` })),
    ...rule.formulas.flatMap((f) => f.graphs.map((g) => ({ id: g, label: `condition ${g.slice(g.lastIndexOf("#") + 1)}` }))),
  ];
}

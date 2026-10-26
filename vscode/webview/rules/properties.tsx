// The properties panel: what the diagram does not edit in place — a rule's parameters,
// options and attribute conditions, a node's class and action, its attributes, an edge's
// reference and action, a unit's parameters and body — the element's problems, and an
// overview of the document when nothing is focused.

import { useEffect, useState, type ReactNode } from "react";
import type { RuleAction, TransformationDiagram, TransformationEdit } from "../../src/engine";
import type { PairInfo } from "../../src/ruleDiagramProtocol";
import { post, ui, useUi } from "./host";
import { ACTIONS, classOf, find, parametersText, parseParameters, references, type Found } from "./model";

interface Props {
  diagram: TransformationDiagram;
  pair: PairInfo;
  edit(e: TransformationEdit): Promise<{ element?: string; error?: string }>;
  focus(id: string | undefined): void;
}

/** A text field that commits on Enter or when left, and reverts on Escape. */
function TextField(props: { label: string; value: string; list?: string; placeholder?: string; onCommit?(v: string): Promise<boolean | void> }) {
  const [value, setValue] = useState(props.value);
  useEffect(() => setValue(props.value), [props.value]);
  const commit = async () => {
    if (value === props.value || !props.onCommit) return;
    if ((await props.onCommit(value)) === false) setValue(props.value);
  };
  return (
    <label className="field">
      <span>{props.label}</span>
      <input
        value={value}
        list={props.list}
        placeholder={props.placeholder}
        readOnly={!props.onCommit}
        spellCheck={false}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => void commit()}
        onKeyDown={(e) => {
          if (e.key === "Enter") void commit();
          if (e.key === "Escape") setValue(props.value);
        }}
      />
    </label>
  );
}

/** A multi-line field, committed with the button or when left. */
function TextArea(props: { label: string; value: string; rows?: number; onCommit(v: string): Promise<boolean | void> }) {
  const [value, setValue] = useState(props.value);
  useEffect(() => setValue(props.value), [props.value]);
  return (
    <label className="field area">
      <span>{props.label}</span>
      <textarea
        value={value}
        rows={props.rows ?? 4}
        spellCheck={false}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => {
          if (value !== props.value) void props.onCommit(value).then((ok) => ok === false && setValue(props.value));
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") setValue(props.value);
        }}
      />
    </label>
  );
}

function Check(props: { label: string; checked: boolean; disabled?: boolean; title?: string; onChange(v: boolean): void }) {
  return (
    <label className="check" title={props.title}>
      <input type="checkbox" checked={props.checked} disabled={props.disabled} onChange={(e) => props.onChange(e.target.checked)} />
      <span>{props.label}</span>
    </label>
  );
}

function ActionSelect(props: { value: RuleAction; disabled?: boolean; onChange(a: RuleAction): void }) {
  return (
    <label className="field">
      <span>Action</span>
      <select className={`action-select action-${props.value}`} value={props.value} disabled={props.disabled} onChange={(e) => props.onChange(e.target.value as RuleAction)}>
        {ACTIONS.map((a) => (
          <option key={a} value={a}>
            «{a}»
          </option>
        ))}
      </select>
    </label>
  );
}

function Section(props: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2>{props.title}</h2>
      {props.children}
    </section>
  );
}

function Problems(props: { diagram: TransformationDiagram; ids: string[]; focus(id: string): void }) {
  const list = props.diagram.problems.filter((p) => p.element && props.ids.includes(p.element));
  if (list.length === 0) return null;
  return (
    <Section title="Problems">
      <ul className="problems">
        {list.map((p, i) => (
          <li key={i} className={p.severity} onClick={() => p.element && props.focus(p.element)}>
            {p.message}
          </li>
        ))}
      </ul>
    </Section>
  );
}

function ShowText(props: { found: Found }) {
  return (
    <button className="secondary" onClick={() => post({ type: "reveal", range: props.found.range, open: true })}>
      Show in Text
    </button>
  );
}

export function Properties(props: Props) {
  const focused = useUi((s) => s.focused);
  const d = props.diagram;
  const found = focused ? find(d, focused) : undefined;
  return (
    <aside className="properties">
      <datalist id="henshin-classes">
        {d.classes.map((c) => (
          <option key={c.id} value={c.name} />
        ))}
      </datalist>
      <datalist id="henshin-types">
        {d.types.map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>
      {found && focused ? <ElementProperties {...props} id={focused} found={found} /> : <Overview {...props} />}
    </aside>
  );
}

function ElementProperties(props: Props & { id: string; found: Found }) {
  const { found, edit, focus, id, diagram } = props;
  const ok = async (e: TransformationEdit) => !(await edit(e)).error;
  const rename = async (name: string) => {
    const r = await edit({ kind: "rename", element: id, name: name.trim() });
    if (r.error) return false;
    if (r.element) focus(r.element);
    return true;
  };
  const remove = (
    <button className="secondary" onClick={() => void edit({ kind: "delete", elements: [id] }).then(() => ui.set({ focused: undefined }))}>
      Delete
    </button>
  );
  switch (found.kind) {
    case "rule": {
      const r = found.rule;
      return (
        <>
          <Section title="Rule">
            <TextField label="Name" value={r.name} onCommit={rename} />
            <TextField
              label="Parameters"
              value={parametersText(r.parameters)}
              list="henshin-types"
              placeholder="IN name:Type, …"
              onCommit={async (text) => {
                const p = parseParameters(text);
                if (!p.value) return false;
                return ok({ kind: "setParameters", element: id, parameters: p.value });
              }}
            />
            <div className="checks">
              <Check label="checkDangling" checked={r.checkDangling} title="Rule out matches that would leave dangling edges" onChange={(v) => void edit({ kind: "setRuleOptions", element: id, checkDangling: v })} />
              <Check label="injective" checked={r.injectiveMatching} title="Match different nodes to different objects" onChange={(v) => void edit({ kind: "setRuleOptions", element: id, injectiveMatching: v })} />
            </div>
            <TextArea label="Conditions" value={r.conditions.join("\n")} rows={3} onCommit={(t) => ok({ kind: "setConditions", element: id, conditions: t.split("\n").map((c) => c.trim()).filter(Boolean) })} />
            {r.formulas.map((f) => (
              <p key={f.scope + f.formula} className="summary">
                formula <code>{f.formula}</code>
              </p>
            ))}
            <AddMultiRule scope={id} edit={edit} focus={focus} />
            <ShowText found={found} /> {remove}
          </Section>
          <Problems diagram={diagram} ids={[id, ...r.nodes.map((n) => n.id), ...r.edges.map((e) => e.id)]} focus={focus} />
        </>
      );
    }
    case "multi": {
      const m = found.rule.multiRules.find((x) => x.id === id)!;
      return (
        <Section title="Multi-rule">
          <p className="owner">of {m.kernel}: its elements are marked with *</p>
          <TextField label="Name" value={m.name} onCommit={rename} />
          <div className="checks">
            <Check label="checkDangling" checked={m.checkDangling} onChange={(v) => void edit({ kind: "setRuleOptions", element: id, checkDangling: v })} />
            <Check label="injective" checked={m.injectiveMatching} onChange={(v) => void edit({ kind: "setRuleOptions", element: id, injectiveMatching: v })} />
          </div>
          <TextArea label="Conditions" value={m.conditions.join("\n")} rows={2} onCommit={(t) => ok({ kind: "setConditions", element: id, conditions: t.split("\n").map((c) => c.trim()).filter(Boolean) })} />
          <button onClick={() => ui.set({ scope: id })}>Add Elements Here</button> <AddMultiRule scope={id} edit={edit} focus={focus} />
          <ShowText found={found} /> {remove}
        </Section>
      );
    }
    case "condition":
      return (
        <Section title="Condition graph">
          <p className="owner">of a matching formula: its elements are marked with #</p>
          <TextField label="Name" value={id.slice(id.lastIndexOf("#") + 1)} onCommit={rename} />
          <button onClick={() => ui.set({ scope: id })}>Add Elements Here</button> <ShowText found={found} /> {remove}
        </Section>
      );
    case "node": {
      const n = found.node;
      const cls = classOf(diagram, n);
      const owned = (cls?.attributes ?? []).map((a) => a.name);
      return (
        <>
          <Section title={n.multi ? "Node of a multi-rule" : n.condition ? "Node of a condition graph" : "Node"}>
            <TextField label="Name" value={n.name} onCommit={rename} />
            <TextField label="Class" value={n.class} list="henshin-classes" onCommit={(c) => ok({ kind: "updateNode", element: id, class: c.trim() })} />
            <ActionSelect value={n.action} disabled={!!n.condition} onChange={(a) => void edit({ kind: "updateNode", element: id, action: a })} />
            {n.parameter && <p className="hint">The node is a parameter of the rule (a parameter has its name).</p>}
          </Section>
          <Section title="Attributes">
            {n.attributes.map((a) => (
              <div key={a.id} className={`attribute-row action-${a.action}`}>
                <a href="#" onClick={(e) => (e.preventDefault(), focus(a.id))}>
                  {a.set ? "set " : ""}
                  {a.name}={a.value}
                </a>
                <span className="muted"> «{a.action}»</span>
              </div>
            ))}
            <AddAttribute node={id} names={owned} edit={edit} />
          </Section>
          <Section title="">
            <ShowText found={found} /> {remove}
          </Section>
          <Problems diagram={diagram} ids={[id, ...n.attributes.map((a) => a.id)]} focus={focus} />
        </>
      );
    }
    case "attribute": {
      const a = found.attribute;
      return (
        <>
          <Section title="Attribute">
            <p className="owner">
              of{" "}
              <a href="#" onClick={(e) => (e.preventDefault(), focus(found.node.id))}>
                {found.node.name}
              </a>
            </p>
            <TextField label="Name" value={a.name} onCommit={(v) => ok({ kind: "updateAttribute", element: id, name: v.trim() })} />
            <TextField label="Value" value={a.value} onCommit={(v) => ok({ kind: "updateAttribute", element: id, value: v.trim() })} />
            <ActionSelect value={a.action} disabled={!!found.node.condition} onChange={(x) => void edit({ kind: "updateAttribute", element: id, action: x })} />
            <div className="checks">
              <Check label="set" checked={a.set} disabled={found.node.action !== "preserve"} title="The rule sets the attribute to the value" onChange={(v) => void edit({ kind: "updateAttribute", element: id, set: v })} />
            </div>
            <ShowText found={found} /> {remove}
          </Section>
          <Problems diagram={diagram} ids={[id]} focus={focus} />
        </>
      );
    }
    case "edge": {
      const e = found.edge;
      const s = found.rule.nodes.find((n) => n.id === e.source);
      const t = found.rule.nodes.find((n) => n.id === e.target);
      const options = s && t ? references(diagram, s, t) : [];
      return (
        <>
          <Section title="Edge">
            <p className="summary">
              <b>{s?.name}</b> → <b>{t?.name}</b>
            </p>
            <label className="field">
              <span>Reference</span>
              <select value={e.reference} onChange={(x) => void edit({ kind: "updateEdge", element: id, reference: x.target.value })}>
                {[...new Set([e.reference, ...options])].map((r) => (
                  <option key={r} value={r}>
                    {r}
                  </option>
                ))}
              </select>
            </label>
            <ActionSelect value={e.action} disabled={!!e.condition} onChange={(a) => void edit({ kind: "updateEdge", element: id, action: a })} />
            <ShowText found={found} /> {remove}
          </Section>
          <Problems diagram={diagram} ids={[id]} focus={focus} />
        </>
      );
    }
    case "unit":
    case "step": {
      const u = found.unit;
      return (
        <>
          <Section title="Unit">
            <TextField label="Name" value={u.name} onCommit={async (name) => {
              const r = await edit({ kind: "rename", element: u.id, name: name.trim() });
              if (r.element) focus(r.element);
              return !r.error;
            }} />
            <TextField
              label="Parameters"
              value={parametersText(u.parameters)}
              list="henshin-types"
              placeholder="IN name:Type, …"
              onCommit={async (text) => {
                const p = parseParameters(text);
                if (!p.value) return false;
                return ok({ kind: "setParameters", element: u.id, parameters: p.value });
              }}
            />
            <TextArea label="Body" value={u.text} rows={Math.max(4, u.text.split("\n").length + 1)} onCommit={(t) => ok({ kind: "setUnitBody", element: u.id, body: t })} />
            <p className="hint">
              Calls <code>rule(param, …)</code>, <code>{"{ … }"}</code>, <code>independent [a() b(), c()]</code>, <code>priority […]</code>,{" "}
              <code>{"if (a()) then { … } else { … }"}</code>, <code>{"for (3) { … }"}</code>, <code>{"while { … }"}</code>.
            </p>
            <ShowText found={found} />{" "}
            {found.kind === "step" ? (
              <button className="secondary" onClick={() => void edit({ kind: "delete", elements: [id] }).then(() => focus(u.id))}>
                Delete Step
              </button>
            ) : (
              remove
            )}
          </Section>
          <Problems diagram={diagram} ids={[u.id]} focus={focus} />
        </>
      );
    }
  }
}

function AddMultiRule(props: { scope: string; edit: Props["edit"]; focus(id: string): void }) {
  const [name, setName] = useState("");
  return (
    <div className="inline-form">
      <input placeholder="new multi-rule" value={name} spellCheck={false} onChange={(e) => setName(e.target.value)} />
      <button
        disabled={!name.trim()}
        onClick={() =>
          void props.edit({ kind: "addMultiRule", scope: props.scope, name: name.trim() }).then((r) => {
            if (!r.error) {
              setName("");
              if (r.element) ui.set({ scope: r.element, focused: r.element });
            }
          })
        }
      >
        Add Multi-rule
      </button>
    </div>
  );
}

function AddAttribute(props: { node: string; names: string[]; edit: Props["edit"] }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const listId = `henshin-attributes-${props.node}`;
  return (
    <div className="inline-form">
      <datalist id={listId}>
        {props.names.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      <input placeholder="attribute" list={listId} value={name} spellCheck={false} onChange={(e) => setName(e.target.value)} />
      <input placeholder="value" value={value} spellCheck={false} onChange={(e) => setValue(e.target.value)} />
      <button
        disabled={!name.trim() || !value.trim()}
        onClick={() =>
          void props.edit({ kind: "addAttribute", node: props.node, name: name.trim(), value: value.trim() }).then((r) => {
            if (!r.error) {
              setName("");
              setValue("");
            }
          })
        }
      >
        Add
      </button>
    </div>
  );
}

function Overview(props: Props) {
  const d = props.diagram;
  return (
    <>
      <Section title="Transformation">
        <p className="summary">
          {d.syntax === "model" ? "A Henshin model (.henshin)" : "Henshin's textual syntax"}, importing <b>{d.imports.join(", ") || "nothing"}</b>
        </p>
        <p className="summary">
          {d.rules.length} rule{d.rules.length === 1 ? "" : "s"}, {d.units.length} unit{d.units.length === 1 ? "" : "s"}
        </p>
        {props.pair.partner && (
          <p className="hint">
            {props.pair.inSync
              ? `Kept equivalent to ${props.pair.partner}: a change of either changes the other.`
              : `${props.pair.partner} is not updated while this document has errors: ${props.pair.reason ?? ""}`}
          </p>
        )}
        <p className="hint">
          Pick a tool and an action: click into a rule to add a node, drag from a node to another to add an edge. Double-click names and attributes to edit them;
          Delete removes the selection.
        </p>
      </Section>
      {d.problems.length > 0 && (
        <Section title="Problems">
          <ul className="problems">
            {d.problems.map((p, i) => (
              <li key={i} className={p.severity} onClick={() => p.element && props.focus(p.element)}>
                {p.element && <span className="where">{p.element}: </span>}
                {p.message}
              </li>
            ))}
          </ul>
        </Section>
      )}
    </>
  );
}

// The properties panel: everything about the focused element that the diagram does not edit in
// place (flags, supertypes, multiplicities, parameters), its problems, and an overview of the
// metamodel when nothing is focused.

import type { Edge } from "@xyflow/react";
import { useEffect, useState, type ReactNode } from "react";
import type { DiagramClassifier, DiagramFeature, DiagramOperation, DiagramType, ElementUpdate, MetamodelDiagram, MetamodelEdit, ParameterSpec } from "../../src/engine";
import { post, useUi } from "./host";
import { find, multiplicity, parametersText, parseMultiplicity, resolveType, type EdgeData, type Found, type Parsed, type Syntax } from "./model";

interface Props {
  diagram: MetamodelDiagram;
  edit(e: MetamodelEdit): Promise<{ element?: string; error?: string }>;
  focus(id: string | undefined): void;
  parseParameters(text: string): Parsed<ParameterSpec[]>;
  /** A classifier was renamed (its position moves to the new id). */
  onRenamed(from: string, to: string): void;
  edges: Edge<EdgeData>[];
  generalizationId(sub: string, sup: string): string;
}

/** A text field that commits on Enter or when left, and reverts on Escape. */
function TextField(props: { label: string; value: string; list?: string; placeholder?: string; readOnly?: boolean; onCommit?(v: string): Promise<boolean | void> }) {
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
        readOnly={props.readOnly || !props.onCommit}
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

function Check(props: { label: string; checked: boolean; disabled?: boolean; title?: string; onChange(v: boolean): void }) {
  return (
    <label className="check" title={props.title}>
      <input type="checkbox" checked={props.checked} disabled={props.disabled} onChange={(e) => props.onChange(e.target.checked)} />
      <span>{props.label}</span>
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

const kindName = (c: DiagramClassifier) =>
  c.kind === "enum" ? "Enumeration" : c.kind === "dataType" ? "Data type" : c.isInterface ? "Interface" : c.isAbstract ? "Abstract class" : "Class";

export function Properties(props: Props) {
  const focused = useUi((s) => s.focused);
  const d = props.diagram;
  const found = focused ? find(d, focused) : undefined;
  const generalization = focused && !found ? props.edges.find((e) => e.id === focused && e.data?.kind === "generalization") : undefined;
  const update = async (element: string, properties: ElementUpdate) => {
    const r = await props.edit({ kind: "update", element, properties });
    if (r.error) return false;
    if (properties.name && r.element && r.element !== element) {
      if (found?.kind === "classifier") props.onRenamed(element, r.element);
      props.focus(r.element);
    }
    return true;
  };
  let body: ReactNode;
  if (found && focused) {
    body = <ElementProperties {...props} id={focused} found={found} update={update} />;
  } else if (generalization) {
    const sub = d.classifiers.find((c) => c.id === generalization.source);
    const sup = generalization.target;
    body = (
      <Section title="Generalization">
        <p className="summary">
          <b>{sub?.name}</b> extends <b>{sup.slice(sup.lastIndexOf("::") + 2)}</b>
        </p>
        <button onClick={() => void props.edit({ kind: "removeSuperType", owner: generalization.source, superType: sup })}>Remove</button>
      </Section>
    );
  } else {
    body = <Overview {...props} />;
  }
  return (
    <aside className="properties">
      <datalist id="oxy-data-types">
        {d.types.filter((t) => t.kind !== "class").map((t) => <option key={t.id} value={t.label} />)}
      </datalist>
      <datalist id="oxy-classes">
        {d.types.filter((t) => t.kind === "class").map((t) => <option key={t.id} value={t.label} />)}
      </datalist>
      <datalist id="oxy-types">
        {d.types.map((t) => <option key={t.id} value={t.label} />)}
      </datalist>
      {body}
    </aside>
  );
}

function Problems(props: { diagram: MetamodelDiagram; ids: string[]; focus(id: string): void }) {
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

function Owner(props: { classifier?: DiagramClassifier; focus(id: string): void }) {
  if (!props.classifier) return null;
  return (
    <p className="owner">
      in{" "}
      <a href="#" onClick={(e) => (e.preventDefault(), props.focus(props.classifier!.id))}>
        {props.classifier.name}
      </a>
    </p>
  );
}

function ShowText(props: { found: Found }) {
  return (
    <button className="secondary" onClick={() => post({ type: "reveal", range: props.found.range, open: true })}>
      Show in Text
    </button>
  );
}

function typeOf(label: string, types: DiagramType[], allowed: (t: DiagramType) => boolean): string | undefined {
  return resolveType(label, types, allowed)?.id;
}

function ElementProperties(props: Props & { id: string; found: Found; update(id: string, p: ElementUpdate): Promise<boolean> }) {
  const { found, diagram: d, id, update } = props;
  const c = found.classifier;
  switch (found.kind) {
    case "classifier":
      return <ClassifierProperties {...props} c={c!} />;
    case "attribute":
    case "reference":
      return <FeatureProperties {...props} f={found.feature!} reference={found.kind === "reference"} />;
    case "operation":
      return <OperationProperties {...props} o={found.operation!} />;
    case "literal": {
      const l = c!.literals.find((x) => x.id === id)!;
      return (
        <>
          <Section title="Literal">
            <Owner classifier={c} focus={props.focus} />
            <TextField label="Name" value={l.name} onCommit={(v) => update(id, { name: v })} />
            <TextField label="Value" value={String(l.value)} readOnly />
            {l.literal !== l.name && <TextField label="Literal" value={l.literal} readOnly />}
            <ShowText found={found} />
          </Section>
          <Problems diagram={d} ids={[id]} focus={props.focus} />
        </>
      );
    }
    case "invariant":
      return (
        <Section title="Invariant">
          <Owner classifier={c} focus={props.focus} />
          <p className="summary">{id.slice(id.indexOf(":", c!.id.length) + 1) || "(unnamed)"}</p>
          <p className="hint">The OCL of invariants is edited in the text.</p>
          <ShowText found={found} />
        </Section>
      );
    case "package":
      return <Overview {...props} />;
  }
}

function ClassifierProperties(props: Props & { c: DiagramClassifier; found: Found; update(id: string, p: ElementUpdate): Promise<boolean> }) {
  const { c, diagram: d, update } = props;
  const [adding, setAdding] = useState("");
  const candidates = [...d.classifiers.filter((x) => x.kind === "class" && x.id !== c.id && !c.superTypes.includes(x.id)), ...d.external.filter((x) => x.kind === "class" && !c.superTypes.includes(x.id))];
  const name = (id: string) => d.classifiers.find((x) => x.id === id)?.name ?? d.external.find((x) => x.id === id)?.name ?? id;
  const memberIds = [c.id, ...c.attributes.map((f) => f.id), ...c.references.map((f) => f.id), ...c.operations.map((o) => o.id), ...c.literals.map((l) => l.id)];
  return (
    <>
      <Section title={kindName(c)}>
        <TextField label="Name" value={c.name} onCommit={(v) => update(c.id, { name: v.trim() })} />
        {c.kind === "class" && (
          <div className="checks">
            <Check label="Abstract" checked={c.isAbstract} onChange={(v) => void update(c.id, { isAbstract: v })} />
            <Check label="Interface" checked={c.isInterface} onChange={(v) => void update(c.id, { isInterface: v })} />
          </div>
        )}
        {c.instanceClass && <TextField label="Instance class" value={c.instanceClass} readOnly />}
        <ShowText found={props.found} />
      </Section>
      {c.kind === "class" && (
        <Section title="Supertypes">
          <ul className="chips">
            {c.superTypes.map((s) => (
              <li key={s}>
                <a href="#" onClick={(e) => (e.preventDefault(), props.focus(s))}>
                  {name(s)}
                </a>
                <button title="Remove the supertype" onClick={() => void props.edit({ kind: "removeSuperType", owner: c.id, superType: s })}>
                  ×
                </button>
              </li>
            ))}
          </ul>
          {candidates.length > 0 && (
            <select
              value={adding}
              onChange={(e) => {
                setAdding("");
                if (e.target.value) void props.edit({ kind: "addSuperType", owner: c.id, superType: e.target.value });
              }}
            >
              <option value="">Add a supertype…</option>
              {candidates.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.name}
                </option>
              ))}
            </select>
          )}
        </Section>
      )}
      {c.kind === "enum" && (
        <Section title="Literals">
          <ul className="chips">
            {c.literals.map((l) => (
              <li key={l.id}>
                <a href="#" onClick={(e) => (e.preventDefault(), props.focus(l.id))}>
                  {l.name}
                </a>
                <button title="Delete the literal" onClick={() => void props.edit({ kind: "delete", elements: [l.id] })}>
                  ×
                </button>
              </li>
            ))}
          </ul>
          <TextField label="New literal" value="" placeholder="NAME" onCommit={async (v) => (await props.edit({ kind: "addLiteral", owner: c.id, name: v.trim() }), false)} />
        </Section>
      )}
      <Problems diagram={d} ids={memberIds} focus={props.focus} />
    </>
  );
}

function MultiplicityField(props: { lower: number; upper: number; syntax: Syntax; disabled?: boolean; onCommit(lower: number, upper: number): Promise<boolean> }) {
  const shown = (l: number, u: number) => (l === u ? `${l}` : `${l}..${u < 0 ? "*" : u}`);
  return (
    <TextField
      label="Multiplicity"
      value={shown(props.lower, props.upper)}
      placeholder="0..*"
      readOnly={props.disabled}
      onCommit={async (v) => {
        const m = parseMultiplicity(v);
        if (!m) return false;
        if (m.lower === props.lower && m.upper === props.upper) return true;
        return props.onCommit(m.lower, m.upper);
      }}
    />
  );
}

function FeatureProperties(props: Props & { f: DiagramFeature; reference: boolean; found: Found; update(id: string, p: ElementUpdate): Promise<boolean> }) {
  const { f, reference, diagram: d, update } = props;
  const owner = props.found.classifier;
  const set = (p: ElementUpdate) => void update(f.id, p);
  const opposite = f.opposite ? find(d, f.opposite) : undefined;
  return (
    <>
      <Section title={reference ? (f.containment ? "Containment reference" : "Reference") : "Attribute"}>
        <Owner classifier={owner} focus={props.focus} />
        <TextField label="Name" value={f.name} onCommit={(v) => update(f.id, { name: v.trim() })} />
        <TextField
          label="Type"
          value={f.typeName}
          list={reference ? "oxy-classes" : "oxy-data-types"}
          onCommit={async (v) => {
            const t = typeOf(v, d.types, (t) => (t.kind === "class") === reference);
            if (!t) {
              post({ type: "notice", kind: "error", message: `There is no ${reference ? "class" : "data type or enumeration"} '${v}'.` });
              return false;
            }
            return update(f.id, { type: t });
          }}
        />
        <MultiplicityField lower={f.lower} upper={f.upper} syntax={d.syntax} onCommit={(lower, upper) => update(f.id, { lower, upper })} />
        <div className="checks">
          {reference && <Check label="Containment" checked={f.containment} onChange={(v) => set({ containment: v })} />}
          <Check label="Ordered" checked={f.ordered} onChange={(v) => set({ ordered: v })} />
          <Check label="Unique" checked={f.unique} onChange={(v) => set({ unique: v })} />
          <Check label="Read-only" checked={!f.changeable} onChange={(v) => set({ changeable: !v })} />
          <Check label="Derived" checked={f.derived} onChange={(v) => set({ derived: v })} />
          <Check label="Transient" checked={f.transient} onChange={(v) => set({ transient: v })} />
          <Check label="Volatile" checked={f.volatile} onChange={(v) => set({ volatile: v })} />
          {!reference && <Check label="ID" checked={f.isId} onChange={(v) => set({ isId: v })} />}
        </div>
        {f.defaultValue !== undefined && <TextField label="Default value" value={f.defaultValue} readOnly />}
        {opposite?.feature && (
          <p className="owner">
            opposite{" "}
            <a href="#" onClick={(e) => (e.preventDefault(), props.focus(opposite.feature!.id))}>
              {opposite.classifier?.name}.{opposite.feature.name}
            </a>
          </p>
        )}
        <ShowText found={props.found} />
      </Section>
      <Problems diagram={d} ids={[f.id]} focus={props.focus} />
    </>
  );
}

function OperationProperties(props: Props & { o: DiagramOperation; found: Found; update(id: string, p: ElementUpdate): Promise<boolean> }) {
  const { o, diagram: d, update } = props;
  return (
    <>
      <Section title="Operation">
        <Owner classifier={props.found.classifier} focus={props.focus} />
        <TextField label="Name" value={o.name} onCommit={(v) => update(o.id, { name: v.trim() })} />
        <TextField
          label="Parameters"
          value={parametersText(o, d.syntax)}
          placeholder="name : Type, …"
          onCommit={async (v) => {
            const p = props.parseParameters(v);
            if (!p.value) {
              post({ type: "notice", kind: "error", message: p.error ?? "Invalid parameters." });
              return false;
            }
            return update(o.id, { parameters: p.value });
          }}
        />
        <TextField
          label="Result type"
          value={o.typeName ?? ""}
          list="oxy-types"
          placeholder="none"
          onCommit={async (v) => {
            if (v.trim() === "") return update(o.id, { type: "" });
            const t = typeOf(v, d.types, () => true);
            if (!t) {
              post({ type: "notice", kind: "error", message: `There is no type '${v}'.` });
              return false;
            }
            return update(o.id, { type: t });
          }}
        />
        <MultiplicityField lower={o.lower} upper={o.upper} syntax={d.syntax} disabled={!o.typeName} onCommit={(lower, upper) => update(o.id, { lower, upper })} />
        {o.hasBody && <p className="hint">Its body is OCL, edited in the text.</p>}
        <ShowText found={props.found} />
      </Section>
      <Problems diagram={d} ids={[o.id]} focus={props.focus} />
    </>
  );
}

function Overview(props: Props) {
  const d = props.diagram;
  const classes = d.classifiers.filter((c) => c.kind === "class").length;
  const problems = d.problems.filter((p) => p.severity !== "info");
  return (
    <>
      {d.packages.map((p) => (
        <Section key={p.id} title="Package">
          <TextField
            label="Name"
            value={p.name}
            onCommit={async (v) => {
              const r = await props.edit({ kind: "update", element: p.id, properties: { name: v.trim() } });
              return !r.error;
            }}
          />
          <TextField label="nsURI" value={p.nsUri} readOnly />
          <TextField label="nsPrefix" value={p.nsPrefix} readOnly />
        </Section>
      ))}
      <Section title="Contents">
        <p className="summary">
          {classes} classes, {d.classifiers.filter((c) => c.kind === "enum").length} enumerations, {d.classifiers.filter((c) => c.kind === "dataType").length} data types
        </p>
        <p className="hint">
          Click an element to see its properties; double-click a name or a member to edit it in place (e.g. <code>name : String{multiplicity(0, -1, d.syntax) || " [0..*]"}</code>). Drag from a class's right edge to another class to add a reference.
        </p>
      </Section>
      {problems.length > 0 && (
        <Section title={`Problems (${problems.length})`}>
          <ul className="problems">
            {problems.map((p, i) => (
              <li key={i} className={p.severity} onClick={() => p.element && props.focus(p.element)}>
                {p.element && <span className="where">{p.element.slice(p.element.lastIndexOf("::") + 2)}: </span>}
                {p.message}
              </li>
            ))}
          </ul>
        </Section>
      )}
    </>
  );
}

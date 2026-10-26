// The nodes of the diagram: classifiers of the document (with compartments for attributes,
// operations, literals and invariants, edited in place) and classifiers of other metamodels.

import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import { createContext, memo, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { DiagramClassifier } from "../../src/engine";
import { useUi, type Editing } from "./host";
import { featureLabel, invariantId, operationLabel, type ClassifierNodeData, type ExternalNodeData, type Syntax } from "./model";

/** What the nodes ask the diagram to do. Errors come back as messages. (Clicks are handled by
 * the diagram: a click on a row, which has a `data-element`, focuses that element.) */
export interface Actions {
  /** Shows an element in the properties and selects its text. */
  focus(id: string): void;
  startEditing(e: Editing): void;
  stopEditing(): void;
  rename(id: string, name: string): Promise<string | undefined>;
  changeMember(id: string, text: string): Promise<string | undefined>;
  addMember(owner: string, member: "attribute" | "operation" | "literal", text: string): Promise<string | undefined>;
  /** The text a member is edited as. */
  memberText(id: string): string;
}

export const ActionsContext = createContext<Actions | undefined>(undefined);

const useActions = () => {
  const a = useContext(ActionsContext);
  if (!a) throw new Error("no diagram actions");
  return a;
};

const cls = (...names: (string | false | undefined)[]) => names.filter(Boolean).join(" ");

/** A text field in a node: Enter commits (an error keeps it open), Escape cancels, leaving it
 * commits a change. */
function InlineInput(props: { initial: string; placeholder?: string; onCommit(text: string): Promise<string | undefined>; onCancel(): void }) {
  const [value, setValue] = useState(props.initial);
  const [error, setError] = useState<string>();
  const ref = useRef<HTMLInputElement>(null);
  const busy = useRef(false);
  useEffect(() => {
    // A new node is invisible until React Flow has measured it, and cannot take the focus
    // before: try again for a few frames.
    let frame = 0;
    let handle = 0;
    const take = () => {
      const input = ref.current;
      if (!input || document.activeElement === input) return;
      input.focus();
      input.select();
      if (document.activeElement !== input && frame++ < 30) handle = requestAnimationFrame(take);
    };
    take();
    return () => cancelAnimationFrame(handle);
  }, []);
  const commit = async () => {
    if (busy.current) return;
    if (value.trim() === "" || value === props.initial) {
      props.onCancel();
      return;
    }
    busy.current = true;
    const e = await props.onCommit(value);
    busy.current = false;
    setError(e);
    if (e) ref.current?.focus();
  };
  return (
    <span className="inline-edit">
      <input
        ref={ref}
        className={cls("nodrag nopan nowheel", error && "invalid")}
        value={value}
        placeholder={props.placeholder}
        spellCheck={false}
        size={Math.max(8, value.length + 1)}
        onChange={(e) => {
          setValue(e.target.value);
          setError(undefined);
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") void commit();
          if (e.key === "Escape") props.onCancel();
        }}
        onBlur={() => {
          if (!error) void commit();
        }}
      />
      {error && <span className="inline-error">{error}</span>}
    </span>
  );
}

function Row(props: { id: string; icon: string; label: string; problems?: string[]; muted?: boolean; editable?: boolean }) {
  const actions = useActions();
  const focused = useUi((s) => s.focused === props.id);
  const editing = useUi((s) => s.editing?.kind === "member" && s.editing.element === props.id);
  if (editing) {
    return (
      <div className="row editing">
        <span className={`icon ${props.icon}`} />
        <InlineInput initial={actions.memberText(props.id)} onCommit={(t) => actions.changeMember(props.id, t)} onCancel={actions.stopEditing} />
      </div>
    );
  }
  return (
    <div
      className={cls("row", focused && "focused", props.problems && "has-problems", props.muted && "muted")}
      data-element={props.id}
      title={props.problems?.join("\n")}
      onDoubleClick={(e) => {
        e.stopPropagation();
        if (props.editable !== false) actions.startEditing({ kind: "member", element: props.id });
      }}
    >
      <span className={`icon ${props.icon}`} />
      <span className="label">{props.label}</span>
    </div>
  );
}

function Compartment(props: { owner: string; member?: "attribute" | "operation" | "literal"; title: string; children: ReactNode }) {
  const actions = useActions();
  const member = props.member;
  const adding = useUi((s) => (s.editing?.kind === "new" && s.editing.owner === props.owner && s.editing.member === member ? s.editing.seq : undefined));
  const placeholder = member === "operation" ? "name(p : Type) : Type" : member === "literal" ? "NAME" : "name : Type";
  return (
    <div className="compartment" data-title={props.title}>
      {props.children}
      {adding !== undefined && member && (
        <div className="row editing">
          <span className={`icon ${member}`} />
          <InlineInput
            key={adding}
            initial=""
            placeholder={placeholder}
            onCommit={(t) => actions.addMember(props.owner, member, t)}
            onCancel={actions.stopEditing}
          />
        </div>
      )}
      {member && adding === undefined && (
        <button
          className="add nodrag"
          title={`Add ${member === "attribute" ? "an attribute" : `an ${member}`}`}
          onClick={(e) => {
            e.stopPropagation();
            actions.startEditing({ kind: "new", owner: props.owner, member, seq: Date.now() });
          }}
        >
          +
        </button>
      )}
    </div>
  );
}

function stereotype(c: DiagramClassifier): string | undefined {
  if (c.kind === "enum") return "enumeration";
  if (c.kind === "dataType") return "datatype";
  if (c.isInterface) return "interface";
  if (c.isAbstract) return "abstract";
  return undefined;
}

/** Handles: the whole node is a target while connecting, and a source with a connection tool;
 * the small connector on its right edge starts a reference in any mode. */
function Handles(props: { source: boolean }) {
  const tool = useUi((s) => s.tool);
  return (
    <>
      <Handle type="target" position={Position.Top} className="cover-target" isConnectableStart={false} />
      {props.source && tool.kind === "connect" && <Handle type="source" position={Position.Bottom} className="cover-source" />}
      {props.source && <Handle type="source" position={Position.Right} className="connector" title="Drag to a class to add a reference" />}
    </>
  );
}

function ClassifierView({ data }: { data: ClassifierNodeData }) {
  const { classifier: c, syntax, problems } = data;
  const actions = useActions();
  const focused = useUi((s) => s.focused === c.id);
  const renaming = useUi((s) => s.editing?.kind === "name" && s.editing.element === c.id);
  const kind = c.kind === "class" ? (c.isInterface ? "interface" : "class") : c.kind;
  const own = problems[c.id];
  const count = Object.values(problems).reduce((n, l) => n + l.length, 0);
  const st = stereotype(c);
  return (
    <div className={cls("classifier", `kind-${kind}`, c.isAbstract && "abstract", count > 0 && "has-problems", focused && "focused")}>
      <Handles source={c.kind === "class"} />
      <header
        onDoubleClick={(e) => {
          e.stopPropagation();
          actions.startEditing({ kind: "name", element: c.id });
        }}
        title={own?.join("\n")}
      >
        <span className={`kind-icon ${kind}`} />
        <div className="titles">
          {st && <div className="stereotype">«{st}»</div>}
          {renaming ? (
            <InlineInput initial={c.name} onCommit={(t) => actions.rename(c.id, t)} onCancel={actions.stopEditing} />
          ) : (
            <div className="name">{c.name}</div>
          )}
        </div>
        {count > 0 && (
          <span className="badge" title={Object.values(problems).flat().join("\n")}>
            {count}
          </span>
        )}
      </header>
      {c.kind === "class" && <Members c={c} syntax={syntax} problems={problems} />}
      {c.kind === "enum" && (
        <Compartment owner={c.id} member="literal" title="literals">
          {c.literals.map((l) => (
            <Row key={l.id} id={l.id} icon="literal" label={l.name} problems={problems[l.id]} />
          ))}
        </Compartment>
      )}
      {c.kind === "dataType" && c.instanceClass && <div className="instance-class">{c.instanceClass}</div>}
    </div>
  );
}

function Members({ c, syntax, problems }: { c: DiagramClassifier; syntax: Syntax; problems: Record<string, string[]> }) {
  return (
    <>
      <Compartment owner={c.id} member="attribute" title="attributes">
        {c.attributes.map((f) => (
          <Row key={f.id} id={f.id} icon={f.isId ? "attribute id" : "attribute"} label={featureLabel(f, syntax)} problems={problems[f.id]} muted={f.derived} />
        ))}
      </Compartment>
      <Compartment owner={c.id} member="operation" title="operations">
        {c.operations.map((o) => (
          <Row key={o.id} id={o.id} icon="operation" label={operationLabel(o, syntax)} problems={problems[o.id]} />
        ))}
      </Compartment>
      {c.invariants.length > 0 && (
        <Compartment owner={c.id} title="invariants">
          {c.invariants.map((i) => (
            <Row key={i.name} id={invariantId(c, i.name)} icon="invariant" label={i.name || "invariant"} editable={false} />
          ))}
        </Compartment>
      )}
    </>
  );
}

export const ClassifierNode = memo(function ClassifierNode({ data }: NodeProps<Node<ClassifierNodeData, "classifier">>) {
  return <ClassifierView data={data} />;
});

export const ExternalNode = memo(function ExternalNode({ data }: NodeProps<Node<ExternalNodeData, "external">>) {
  const e = data.external;
  const file = e.resource.split(/[\\/]/).pop();
  return (
    <div
      className={cls("classifier", "external", `kind-${e.isInterface ? "interface" : e.kind}`, e.isAbstract && "abstract")}
      title={`${e.id} (${e.resource})`}
    >
      <Handles source={false} />
      <header>
        <span className={`kind-icon ${e.isInterface ? "interface" : e.kind}`} />
        <div className="titles">
          <div className="name">{e.name}</div>
          <div className="stereotype">{file}</div>
        </div>
      </header>
    </div>
  );
});

export const nodeTypes = { classifier: ClassifierNode, external: ExternalNode };

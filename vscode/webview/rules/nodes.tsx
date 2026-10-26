// The nodes of the rule diagram, in Henshin's notation: a frame per rule (its name and
// parameters, its attribute conditions as a note, its options), the rule's nodes inside it
// (a stereotype for the action, `name:Class`, the attributes; colored by action as in
// Henshin's default color mode; nodes of multi-rules drawn stacked), and a frame per unit with
// its control flow from the start to the end symbol.

import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import { createContext, memo, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { RuleDiagram, RuleNodeDiagram, UnitDiagram, UnitStepDiagram } from "../../src/engine";
import { useUi, type Editing } from "./host";
import { attributeLines, parametersText, stereotype } from "./model";

/** What the nodes ask the diagram to do. Errors come back as messages. */
export interface Actions {
  focus(id: string): void;
  startEditing(e: Editing): void;
  stopEditing(): void;
  rename(id: string, name: string): Promise<string | undefined>;
  changeAttribute(id: string, text: string): Promise<string | undefined>;
  addAttribute(node: string, text: string): Promise<string | undefined>;
  /** The text an attribute is edited as. */
  attributeText(id: string): string;
}

export const ActionsContext = createContext<Actions | undefined>(undefined);

const useActions = () => {
  const a = useContext(ActionsContext);
  if (!a) throw new Error("no diagram actions");
  return a;
};

const cls = (...names: (string | false | undefined)[]) => names.filter(Boolean).join(" ");

/** Problems by element id. */
export type Problems = Record<string, string[]>;

/** A text field in a node: Enter commits (an error keeps it open), Escape cancels, leaving it
 * commits a change. */
export function InlineInput(props: { initial: string; placeholder?: string; onCommit(text: string): Promise<string | undefined>; onCancel(): void }) {
  const [value, setValue] = useState(props.initial);
  const [error, setError] = useState<string>();
  const ref = useRef<HTMLInputElement>(null);
  const busy = useRef(false);
  useEffect(() => {
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

// ---------------------------------------------------------------- rules

export interface FrameData extends Record<string, unknown> {
  rule: RuleDiagram;
  problems: Problems;
}

/** The height of a rule frame's header, with its notes. */
export function frameTop(rule: RuleDiagram): number {
  const notes = (rule.conditions.length ? 1 : 0) + rule.formulas.length + (rule.multiRules.length ? 1 : 0);
  return 40 + notes * 22;
}

function count(problems: Problems): number {
  return Object.values(problems).reduce((n, l) => n + l.length, 0);
}

export const RuleFrame = memo(function RuleFrame({ data }: NodeProps<Node<FrameData, "rule">>) {
  const { rule, problems } = data;
  const actions = useActions();
  const focused = useUi((s) => s.focused === rule.id);
  const renaming = useUi((s) => s.editing?.kind === "name" && s.editing.element === rule.id);
  const tool = useUi((s) => s.tool.kind);
  const n = count(problems);
  const options = [!rule.checkDangling && "checkDangling false", !rule.injectiveMatching && "injectiveMatching false"].filter(Boolean);
  return (
    <div className={cls("rule-frame", focused && "focused", n > 0 && "has-problems", tool === "node" && "accepts")}>
      <header
        data-element={rule.id}
        onDoubleClick={(e) => {
          e.stopPropagation();
          actions.startEditing({ kind: "name", element: rule.id });
        }}
      >
        <span className="rule-icon" />
        <span className="rule-keyword">rule</span>
        {renaming ? (
          <InlineInput initial={rule.name} onCommit={(t) => actions.rename(rule.id, t)} onCancel={actions.stopEditing} />
        ) : (
          <span className="rule-name">{rule.name}</span>
        )}
        <span className="rule-params">({parametersText(rule.parameters)})</span>
        {options.map((o) => (
          <span key={String(o)} className="tag">
            {o}
          </span>
        ))}
        {n > 0 && (
          <span className="badge" title={Object.values(problems).flat().join("\n")}>
            {n}
          </span>
        )}
      </header>
      {rule.conditions.length > 0 && (
        <div className="condition-note" data-element={rule.id} title="Attribute conditions">
          {rule.conditions.join(", ")}
        </div>
      )}
      {rule.formulas.map((f) => (
        <div key={f.scope + f.formula} className="formula-note" data-element={f.graphs[0]} title="Matching formula">
          formula {f.formula}
        </div>
      ))}
      {rule.multiRules.length > 0 && (
        <div className="multi-note">
          {rule.multiRules.map((m) => (
            <span
              key={m.id}
              className="tag multi"
              data-element={m.id}
              title={`Multi-rule of ${m.kernel}: its elements are marked with *${m.id.slice(m.id.indexOf("/"))}`}
            >
              multi-rule {m.name}
            </span>
          ))}
        </div>
      )}
    </div>
  );
});

// ---------------------------------------------------------------- nodes of rules

export interface ObjectData extends Record<string, unknown> {
  node: RuleNodeDiagram;
  problems: Problems;
}

function Handles() {
  const tool = useUi((s) => s.tool.kind);
  return (
    <>
      <Handle type="target" position={Position.Top} className="cover-target" isConnectableStart={false} />
      {tool === "edge" && <Handle type="source" position={Position.Bottom} className="cover-source" />}
      <Handle type="source" position={Position.Right} className="connector" title="Drag to a node to add an edge" />
    </>
  );
}

function AttributeRow(props: { node: RuleNodeDiagram; id: string; ids: string[]; text: string; action: string; stereotype?: string; problems?: string[] }) {
  const actions = useActions();
  const focused = useUi((s) => (s.focused ? props.ids.includes(s.focused) : false));
  const editing = useUi((s) => s.editing?.kind === "attribute" && s.editing.element === props.id);
  if (editing) {
    return (
      <div className="attribute editing">
        <InlineInput initial={actions.attributeText(props.id)} onCommit={(t) => actions.changeAttribute(props.id, t)} onCancel={actions.stopEditing} />
      </div>
    );
  }
  return (
    <div
      className={cls("attribute", `action-${props.action}`, focused && "focused", props.problems && "has-problems")}
      data-element={props.id}
      title={props.problems?.join("\n")}
      onDoubleClick={(e) => {
        e.stopPropagation();
        actions.startEditing({ kind: "attribute", element: props.id });
      }}
    >
      {props.stereotype && <span className="stereotype">{props.stereotype}</span>}
      <span className="label">{props.text}</span>
    </div>
  );
}

export const ObjectNode = memo(function ObjectNode({ data }: NodeProps<Node<ObjectData, "object">>) {
  const { node, problems } = data;
  const actions = useActions();
  const focused = useUi((s) => s.focused === node.id);
  const renaming = useUi((s) => s.editing?.kind === "name" && s.editing.element === node.id);
  const adding = useUi((s) => (s.editing?.kind === "newAttribute" && s.editing.node === node.id ? s.editing.seq : undefined));
  const own = problems[node.id];
  const lines = attributeLines(node);
  return (
    <div
      className={cls("rule-node", `action-${node.action}`, node.multi && "multi", node.condition && "condition", focused && "focused", own && "has-problems")}
      title={own?.join("\n")}
    >
      <Handles />
      <div className="stereotype">{stereotype(node.action, node.multi, node.condition)}</div>
      <div
        className="node-name"
        onDoubleClick={(e) => {
          e.stopPropagation();
          actions.startEditing({ kind: "name", element: node.id });
        }}
      >
        {renaming ? (
          <InlineInput initial={node.name} onCommit={(t) => actions.rename(node.id, t)} onCancel={actions.stopEditing} />
        ) : (
          <>
            <span className={cls("name", node.parameter && "parameter")} title={node.parameter ? "A parameter of the rule" : undefined}>
              {node.name}
            </span>
            :<span className="class">{node.class}</span>
          </>
        )}
      </div>
      {(lines.length > 0 || adding !== undefined) && (
        <div className="attributes">
          {lines.map((l) => (
            <AttributeRow key={l.id} node={node} {...l} problems={l.ids.some((i) => problems[i]) ? l.ids.flatMap((i) => problems[i] ?? []) : undefined} />
          ))}
          {adding !== undefined && (
            <div className="attribute editing">
              <InlineInput key={adding} initial="" placeholder="name=value" onCommit={(t) => actions.addAttribute(node.id, t)} onCancel={actions.stopEditing} />
            </div>
          )}
        </div>
      )}
      {adding === undefined && (
        <button
          className="add nodrag"
          title="Add an attribute"
          onClick={(e) => {
            e.stopPropagation();
            actions.startEditing({ kind: "newAttribute", node: node.id, seq: Date.now() });
          }}
        >
          +
        </button>
      )}
    </div>
  );
});

// ---------------------------------------------------------------- units

export interface UnitData extends Record<string, unknown> {
  unit: UnitDiagram;
  problems: Problems;
}

function Steps(props: { steps: UnitStepDiagram[]; problems: Problems }): ReactNode {
  return (
    <div className="steps">
      {props.steps.map((s) => (
        <Step key={s.id} step={s} problems={props.problems} />
      ))}
    </div>
  );
}

function Step({ step, problems }: { step: UnitStepDiagram; problems: Problems }) {
  const focused = useUi((s) => s.focused === step.id);
  const own = problems[step.id];
  const common = { "data-element": step.id, title: own?.join("\n") };
  const children = step.children ?? [];
  switch (step.kind) {
    case "call":
      return (
        <div className={cls("invocation", focused && "focused", own && "has-problems")} {...common}>
          {step.target}({(step.arguments ?? []).join(", ")})
        </div>
      );
    case "strict":
    case "rollback":
      return (
        <div className={cls("unit-flag", focused && "focused")} {...common}>
          {step.kind} {String(step.value)}
        </div>
      );
    case "independent":
    case "priority":
      return (
        <div className={cls("control", step.kind, focused && "focused")} {...common}>
          <div className="control-label">{step.kind}</div>
          <div className="branches">
            {children.map((list, i) => (
              <div key={i} className="branch">
                {step.kind === "priority" && <span className="branch-index">{i + 1}</span>}
                <Steps steps={list} problems={problems} />
              </div>
            ))}
          </div>
        </div>
      );
    case "if":
      return (
        <div className={cls("control", "if", focused && "focused")} {...common}>
          <div className="control-label">if</div>
          <Steps steps={children[0] ?? []} problems={problems} />
          <div className="control-label">then</div>
          <Steps steps={children[1] ?? []} problems={problems} />
          {children[2] && (
            <>
              <div className="control-label">else</div>
              <Steps steps={children[2]} problems={problems} />
            </>
          )}
        </div>
      );
    default: {
      const label = step.kind === "for" ? `for (${step.count ?? ""})` : step.kind === "while" ? "while" : "{ }";
      return (
        <div className={cls("control", step.kind, focused && "focused")} {...common}>
          <div className="control-label">{label}</div>
          <Steps steps={children[0] ?? []} problems={problems} />
        </div>
      );
    }
  }
}

export const UnitNode = memo(function UnitNode({ data }: NodeProps<Node<UnitData, "unit">>) {
  const { unit, problems } = data;
  const actions = useActions();
  const focused = useUi((s) => s.focused === unit.id);
  const renaming = useUi((s) => s.editing?.kind === "name" && s.editing.element === unit.id);
  const n = count(problems);
  return (
    <div className={cls("unit-frame", focused && "focused", n > 0 && "has-problems")}>
      <header
        data-element={unit.id}
        onDoubleClick={(e) => {
          e.stopPropagation();
          actions.startEditing({ kind: "name", element: unit.id });
        }}
      >
        <span className="unit-icon" />
        <span className="rule-keyword">unit</span>
        {renaming ? (
          <InlineInput initial={unit.name} onCommit={(t) => actions.rename(unit.id, t)} onCancel={actions.stopEditing} />
        ) : (
          <span className="rule-name">{unit.name}</span>
        )}
        <span className="rule-params">({parametersText(unit.parameters)})</span>
        {n > 0 && (
          <span className="badge" title={Object.values(problems).flat().join("\n")}>
            {n}
          </span>
        )}
      </header>
      <div className="flow">
        <div className="symbol start" title="Start" />
        <Steps steps={unit.body} problems={problems} />
        <div className="symbol end" title="End" />
      </div>
    </div>
  );
});

export const nodeTypes = { rule: RuleFrame, object: ObjectNode, unit: UnitNode };

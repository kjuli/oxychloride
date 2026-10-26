// The toolbar of the rule diagram: tools that add nodes (click into a rule) and edges (drag
// from a node to another), the action and class they get, the scope of a rule they go into
// (the rule, a multi-rule, a condition graph), new rules and units, and the view's commands.

import type { ReactNode } from "react";
import type { RuleAction, TransformationDiagram } from "../../src/engine";
import type { PairInfo } from "../../src/ruleDiagramProtocol";
import { save, saved, ui, useUi, type Tool } from "./host";
import { ACTIONS, ruleOfScope, scopes } from "./model";

const icon = (children: ReactNode) => (
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="tool-icon">
    {children}
  </svg>
);

const icons = {
  select: icon(<path d="M3 1.5 L3 13 L6 10 L8.5 14.5 L10.3 13.6 L7.9 9.2 L12 9 Z" className="fill" />),
  node: icon(
    <>
      <rect x="1.5" y="3.5" width="13" height="9" />
      <path d="M1.5 7 H14.5" />
    </>,
  ),
  edge: icon(
    <>
      <path d="M1.5 12.5 L12 4" />
      <path d="M13.8 2.6 L9.6 3.6 L12.4 6.6 Z" className="fill" />
    </>,
  ),
  rule: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1.5" />
      <path d="M1.5 5.5 H14.5" />
    </>,
  ),
  unit: icon(
    <>
      <circle cx="8" cy="2.8" r="1.4" className="fill" />
      <rect x="4.5" y="6" width="7" height="4" />
      <circle cx="8" cy="13.2" r="1.6" />
      <path d="M8 4.2 V6 M8 10 V11.6" />
    </>,
  ),
  layout: icon(
    <>
      <rect x="5.5" y="1.5" width="5" height="3.5" />
      <rect x="1.5" y="11" width="5" height="3.5" />
      <rect x="9.5" y="11" width="5" height="3.5" />
      <path d="M8 5 V8 M4 8 H12 M4 8 V11 M12 8 V11" />
    </>,
  ),
  fit: icon(<path d="M1.5 5.5 V1.5 H5.5 M10.5 1.5 H14.5 V5.5 M14.5 10.5 V14.5 H10.5 M5.5 14.5 H1.5 V10.5" />),
  text: icon(<path d="M3 3.5 H13 M3 6.5 H13 M3 9.5 H10 M3 12.5 H12" />),
  play: icon(<path d="M4.5 2.5 L12.5 8 L4.5 13.5 Z" />),
  space: icon(
    <>
      <circle cx="3.5" cy="8" r="2" />
      <circle cx="12.5" cy="3.5" r="2" />
      <circle cx="12.5" cy="12.5" r="2" />
      <path d="M5.4 7 L10.6 4.4 M5.4 9 L10.6 11.6" />
    </>,
  ),
  sync: icon(<path d="M2.5 6 A5.5 5.5 0 0 1 12.8 4.2 M13.5 10 A5.5 5.5 0 0 1 3.2 11.8 M12.8 1.5 V4.2 H10 M3.2 14.5 V11.8 H6" />),
  properties: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M10 2.5 V13.5" />
    </>,
  ),
  problems: icon(
    <>
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 4.5 V8.8 M8 10.8 V11.6" />
    </>,
  ),
};

function ToolButton(props: { tool: Tool; label: string; icon: ReactNode; hint: string }) {
  const current = useUi((s) => s.tool.kind);
  const active = current === props.tool.kind;
  return (
    <button
      className={`tool${active ? " active" : ""}`}
      title={props.hint}
      aria-pressed={active}
      onClick={() => ui.set({ tool: active ? { kind: "select" } : props.tool })}
    >
      {props.icon}
      <span className="tool-label">{props.label}</span>
    </button>
  );
}

export function Toolbar(props: {
  title: string;
  diagram?: TransformationDiagram;
  pair: PairInfo;
  problems: number;
  onProblems(): void;
  onAddRule(): void;
  onAddUnit(): void;
  onLayout(): void;
  onFit(): void;
  onCommand(command: "openText" | "transform" | "explore" | "createCounterpart"): void;
  showProperties: boolean;
  onToggleProperties(): void;
}) {
  const action = useUi((s) => s.action);
  const cls = useUi((s) => s.class);
  const scope = useUi((s) => s.scope);
  const classes = props.diagram?.classes ?? [];
  const rule = scope && props.diagram?.rules.find((r) => r.id === ruleOfScope(scope));
  const ruleScopes = rule ? scopes(rule) : [];
  const pair = props.pair;
  return (
    <div className="toolbar" role="toolbar">
      <div className="group">
        <ToolButton tool={{ kind: "select" }} label="Select" icon={icons.select} hint="Select and move (Escape)" />
        <ToolButton tool={{ kind: "node" }} label="Node" icon={icons.node} hint="Add a node: click into a rule" />
        <ToolButton tool={{ kind: "edge" }} label="Edge" icon={icons.edge} hint="Add an edge: drag from a node to another" />
      </div>
      <label className="choice" title="The action of new nodes and edges">
        <select
          className={`action-select action-${action}`}
          value={action}
          onChange={(e) => {
            const a = e.target.value as RuleAction;
            saved.action = a;
            save();
            ui.set({ action: a });
          }}
        >
          {ACTIONS.map((a) => (
            <option key={a} value={a}>
              «{a}»
            </option>
          ))}
        </select>
      </label>
      <label className="choice" title="The class of new nodes">
        <select
          value={cls ?? classes.find((c) => !c.isAbstract)?.name ?? ""}
          onChange={(e) => {
            saved.class = e.target.value;
            save();
            ui.set({ class: e.target.value });
          }}
        >
          {classes.map((c) => (
            <option key={c.id} value={c.name}>
              {c.name}
              {c.isAbstract ? " (abstract)" : ""}
            </option>
          ))}
        </select>
      </label>
      {ruleScopes.length > 1 && (
        <label className="choice" title="Where new elements of this rule go">
          <span>into</span>
          <select value={scope} onChange={(e) => ui.set({ scope: e.target.value })}>
            {ruleScopes.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="group">
        <button className="command" title="Add a rule" onClick={props.onAddRule}>
          {icons.rule}
          <span className="tool-label">Rule</span>
        </button>
        <button className="command" title="Add a unit" onClick={props.onAddUnit}>
          {icons.unit}
          <span className="tool-label">Unit</span>
        </button>
      </div>
      <div className="spacer" />
      <span className="title" title={props.title}>
        {props.title}
      </span>
      {pair.partner ? (
        <span
          className={`pair${pair.inSync ? "" : " out-of-sync"}`}
          title={pair.inSync ? `Kept equivalent to ${pair.partner}` : `${pair.partner} is not updated: ${pair.reason ?? ""}`}
        >
          {icons.sync}
          <span className="tool-label">{pair.partner}</span>
        </span>
      ) : (
        <button className="command" title="Write this transformation in the other syntax next to it, and keep the two equivalent" onClick={() => props.onCommand("createCounterpart")}>
          {icons.sync}
          <span className="tool-label">{/\.henshin$/i.test(props.title) ? "Text" : ".henshin"}</span>
        </button>
      )}
      {props.problems > 0 && (
        <button className="command problems" title="Show the first problem" onClick={props.onProblems}>
          {icons.problems}
          <span>{props.problems}</span>
        </button>
      )}
      <button className="command" title="Transform a model with these rules" onClick={() => props.onCommand("transform")}>
        {icons.play}
      </button>
      <button className="command" title="Explore the state space of a model" onClick={() => props.onCommand("explore")}>
        {icons.space}
      </button>
      <button className="command" title="Show the text to the side" onClick={() => props.onCommand("openText")}>
        {icons.text}
      </button>
      <button className="command" title="Lay out the whole diagram automatically" onClick={props.onLayout}>
        {icons.layout}
        <span className="tool-label">Layout</span>
      </button>
      <button className="command" title="Fit the diagram into the view" onClick={props.onFit}>
        {icons.fit}
      </button>
      <button className={`command${props.showProperties ? " active" : ""}`} title="Show or hide the properties" aria-pressed={props.showProperties} onClick={props.onToggleProperties}>
        {icons.properties}
      </button>
    </div>
  );
}

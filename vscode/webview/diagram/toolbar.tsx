// The toolbar: tools that add classifiers (click into the diagram) and connect classes (drag
// from one class to another), the package new classifiers go into, and the view's commands.

import type { ReactNode } from "react";
import type { MetamodelDiagram } from "../../src/engine";
import { ui, useUi, type Tool } from "./host";

const icon = (children: ReactNode) => (
  <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="tool-icon">
    {children}
  </svg>
);

const icons = {
  select: icon(<path d="M3 1.5 L3 13 L6 10 L8.5 14.5 L10.3 13.6 L7.9 9.2 L12 9 Z" className="fill" />),
  class: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M1.5 6.5 H14.5 M1.5 10 H14.5" />
    </>,
  ),
  abstractClass: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" strokeDasharray="2 1.5" />
      <path d="M1.5 6.5 H14.5" />
    </>,
  ),
  interface: icon(
    <>
      <circle cx="8" cy="5" r="3" />
      <path d="M8 8 V14" />
    </>,
  ),
  enum: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M4 6.5 H12 M4 9 H12 M4 11.5 H9" />
    </>,
  ),
  dataType: icon(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M5 6 H11 M8 6 V11.5" />
    </>,
  ),
  reference: icon(<path d="M1.5 12.5 L13.5 3.5 M9 3.2 L13.5 3.5 L12.6 7.6" />),
  containment: icon(
    <>
      <path d="M5.5 9.5 L14 3" />
      <path d="M1.5 12.5 L3.2 9.8 L6.2 9.9 L4.5 12.6 Z" className="fill" />
    </>,
  ),
  generalization: icon(
    <>
      <path d="M8 14.5 V7" />
      <path d="M8 1.5 L12 7 L4 7 Z" />
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

const sameTool = (a: Tool, b: Tool) => JSON.stringify(a) === JSON.stringify(b);

function ToolButton(props: { tool: Tool; label: string; icon: ReactNode; hint: string }) {
  const current = useUi((s) => s.tool);
  const active = sameTool(current, props.tool);
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
  diagram?: MetamodelDiagram;
  packageId?: string;
  onPackage(id: string): void;
  problems: number;
  onProblems(): void;
  onLayout(): void;
  onFit(): void;
  showProperties: boolean;
  onToggleProperties(): void;
}) {
  const packages = props.diagram?.packages ?? [];
  return (
    <div className="toolbar" role="toolbar">
      <div className="group">
        <ToolButton tool={{ kind: "select" }} label="Select" icon={icons.select} hint="Select and move (Escape)" />
      </div>
      <div className="group">
        <ToolButton tool={{ kind: "classifier", classifier: "class" }} label="Class" icon={icons.class} hint="Add a class: click into the diagram" />
        <ToolButton tool={{ kind: "classifier", classifier: "abstractClass" }} label="Abstract" icon={icons.abstractClass} hint="Add an abstract class" />
        <ToolButton tool={{ kind: "classifier", classifier: "interface" }} label="Interface" icon={icons.interface} hint="Add an interface" />
        <ToolButton tool={{ kind: "classifier", classifier: "enum" }} label="Enum" icon={icons.enum} hint="Add an enumeration" />
        <ToolButton tool={{ kind: "classifier", classifier: "dataType" }} label="Data type" icon={icons.dataType} hint="Add a data type" />
      </div>
      <div className="group">
        <ToolButton tool={{ kind: "connect", connection: "reference" }} label="Reference" icon={icons.reference} hint="Add a reference: drag from a class to another" />
        <ToolButton tool={{ kind: "connect", connection: "containment" }} label="Containment" icon={icons.containment} hint="Add a containment: drag from the container to the contained class" />
        <ToolButton tool={{ kind: "connect", connection: "generalization" }} label="Supertype" icon={icons.generalization} hint="Add a supertype: drag from the subclass to its supertype" />
      </div>
      {packages.length > 1 && (
        <label className="package" title="The package new classifiers are added to">
          <span>Package</span>
          <select value={props.packageId} onChange={(e) => props.onPackage(e.target.value)}>
            {packages.map((p) => (
              <option key={p.id} value={p.id}>
                {p.id}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="spacer" />
      <span className="title" title={props.title}>
        {props.title}
      </span>
      {props.problems > 0 && (
        <button className="command problems" title="Show the first problem" onClick={props.onProblems}>
          {icons.problems}
          <span>{props.problems}</span>
        </button>
      )}
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

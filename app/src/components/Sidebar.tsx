import type { MouseEvent } from "react";
import type {
  ClassifierInfo,
  FeatureInfo,
  ModelResourceInfo,
  ObjectInfo,
  OperationInfo,
  PackageInfo,
  Range,
} from "../api";
import type { ContextObject } from "../types";
import { baseName } from "../util";
import { FileIcon, fileKind } from "./FileIcon";
import { TreeCode, TreeItem } from "./Tree";

/** What a context menu of the sidebar was opened on. */
export type SidebarTarget =
  | { kind: "package"; package: PackageInfo }
  | { kind: "classifier"; classifier: ClassifierInfo }
  | { kind: "object"; object: ObjectInfo; resource: string };

type MenuHandler = (e: MouseEvent<HTMLElement>, target: SidebarTarget) => void;

function multiplicity(lower: number, upper: number): string {
  if (lower === 0 && upper === 1) return "";
  const u = upper === -1 ? "*" : upper === -2 ? "?" : String(upper);
  return lower === upper ? `[${u}]` : `[${lower}..${u}]`;
}

function shortName(qualified: string): string {
  const i = qualified.lastIndexOf("::");
  return i >= 0 ? qualified.slice(i + 2) : qualified;
}

function Badge({ kind, text, title }: { kind: string; text: string; title: string }) {
  return (
    <span className={`badge badge-${kind}`} title={title}>
      {text}
    </span>
  );
}

// ---------------------------------------------------------------- metamodels

function FeatureNode({ f, depth }: { f: FeatureInfo; depth: number }) {
  const kind = f.isReference ? (f.containment ? "containment" : "reference") : "attribute";
  const badgeText = f.isReference ? (f.containment ? "◆" : "→") : "a";
  return (
    <TreeItem
      depth={depth}
      badge={<Badge kind={kind} text={badgeText} title={kind} />}
      label={`${f.derived ? "/" : ""}${f.name}`}
      detail={
        <>
          {f.type}
          {multiplicity(f.lower, f.upper)}
          {f.opposite ? ` ⇄ ${f.opposite}` : ""}
        </>
      }
      hasChildren={Boolean(f.derivation)}
    >
      {() => f.derivation && <TreeCode depth={depth + 1} prefix="derive" text={f.derivation} />}
    </TreeItem>
  );
}

function OperationNode({ op, depth }: { op: OperationInfo; depth: number }) {
  const params = op.parameters.map((p) => `${p.name} : ${p.type}`).join(", ");
  return (
    <TreeItem
      depth={depth}
      badge={<Badge kind="operation" text="ƒ" title="operation" />}
      label={`${op.name}(${params})`}
      detail={op.type ? op.type : undefined}
      hasChildren={Boolean(op.body) || Boolean(op.preconditions?.length) || Boolean(op.postconditions?.length)}
    >
      {() => (
        <>
          {op.preconditions?.map((c, i) => (
            <TreeCode key={`pre:${i}`} depth={depth + 1} prefix={c.name ? `pre ${c.name}` : "pre"} text={c.expression} />
          ))}
          {op.body && <TreeCode depth={depth + 1} prefix="body" text={op.body} />}
          {op.postconditions?.map((c, i) => (
            <TreeCode key={`post:${i}`} depth={depth + 1} prefix={c.name ? `post ${c.name}` : "post"} text={c.expression} />
          ))}
        </>
      )}
    </TreeItem>
  );
}

function ClassifierNode({ c, depth, onMenu }: { c: ClassifierInfo; depth: number; onMenu: MenuHandler }) {
  const badge =
    c.kind === "enum" ? (
      <Badge kind="enum" text="E" title="enumeration" />
    ) : c.kind === "dataType" ? (
      <Badge kind="datatype" text="D" title="data type" />
    ) : (
      <Badge
        kind={c.isInterface ? "interface" : c.isAbstract ? "abstract" : "class"}
        text={c.isInterface ? "I" : "C"}
        title={c.isInterface ? "interface" : c.isAbstract ? "abstract class" : "class"}
      />
    );
  const supers = c.superTypes ?? [];
  const count =
    (c.features?.length ?? 0) + (c.operations?.length ?? 0) + (c.literals?.length ?? 0) + (c.invariants?.length ?? 0);
  return (
    <TreeItem
      depth={depth}
      badge={badge}
      label={<span className={c.isAbstract ? "is-abstract" : undefined}>{c.name}</span>}
      detail={supers.length ? `extends ${supers.map(shortName).join(", ")}` : undefined}
      title={c.qualifiedName}
      hasChildren={count > 0}
      onContextMenu={(e) => onMenu(e, { kind: "classifier", classifier: c })}
    >
      {() => (
        <>
          {c.features?.map((f) => <FeatureNode key={`f:${f.name}`} f={f} depth={depth + 1} />)}
          {c.operations?.map((op, i) => <OperationNode key={`o:${op.name}:${i}`} op={op} depth={depth + 1} />)}
          {c.literals?.map((l) => (
            <TreeItem key={`l:${l}`} depth={depth + 1} badge={<Badge kind="literal" text="·" title="literal" />} label={l} />
          ))}
          {c.invariants?.map((inv, i) => (
            <TreeItem
              key={`i:${inv.name}:${i}`}
              depth={depth + 1}
              badge={<Badge kind="invariant" text="✓" title="invariant" />}
              label={inv.name}
              detail="invariant"
              defaultOpen
              hasChildren
            >
              {() => <TreeCode depth={depth + 2} prefix="inv" text={inv.expression} />}
            </TreeItem>
          ))}
        </>
      )}
    </TreeItem>
  );
}

function PackageNode({ p, depth, onMenu }: { p: PackageInfo; depth: number; onMenu: MenuHandler }) {
  return (
    <TreeItem
      depth={depth}
      badge={<Badge kind="package" text="P" title="package" />}
      label={p.name}
      detail={p.nsUri}
      title={`${p.nsUri}\n${p.resource}`}
      defaultOpen={depth === 0}
      hasChildren={p.classifiers.length + p.subpackages.length > 0}
      onContextMenu={(e) => onMenu(e, { kind: "package", package: p })}
    >
      {() => (
        <>
          {p.subpackages.map((s) => (
            <PackageNode key={s.nsUri || s.name} p={s} depth={depth + 1} onMenu={onMenu} />
          ))}
          {p.classifiers.map((c) => (
            <ClassifierNode key={c.qualifiedName} c={c} depth={depth + 1} onMenu={onMenu} />
          ))}
        </>
      )}
    </TreeItem>
  );
}

// ---------------------------------------------------------------- models

interface ObjectNodeProps {
  o: ObjectInfo;
  resource: string;
  depth: number;
  context: ContextObject | null;
  onSelect: (c: ContextObject) => void;
  onJump: (o: { resource: string; range?: Range }) => void;
  onMenu: MenuHandler;
}

function ObjectNode({ o, resource, depth, context, onSelect, onJump, onMenu }: ObjectNodeProps) {
  const selected = context?.resource === resource && context.fragment === o.fragment;
  const hasChildren = o.attributes.length + o.references.length + o.children.length > 0;
  return (
    <TreeItem
      depth={depth}
      badge={<Badge kind="object" text={shortName(o.class).charAt(0)} title={o.class} />}
      label={o.label}
      detail={shortName(o.class)}
      title={`${o.class}\n${o.fragment}\nClick to use as evaluation context${o.range ? ", double-click to show its definition" : ""}`}
      selected={selected}
      onSelect={() => onSelect({ resource, fragment: o.fragment, label: o.label, class: o.class })}
      onOpen={o.range ? () => onJump({ resource, range: o.range }) : undefined}
      onContextMenu={(e) => onMenu(e, { kind: "object", object: o, resource })}
      hasChildren={hasChildren}
      defaultOpen={depth <= 1}
    >
      {() => (
        <>
          {o.attributes.map((s) => (
            <div key={`a:${s.name}`} className="tree-slot" style={{ paddingLeft: 26 + (depth + 1) * 14 }}>
              <span className="slot-name">{s.name}</span>
              <span className="slot-op">=</span>
              <span className="slot-value">{s.values.length === 1 ? s.values[0] : `{${s.values.join(", ")}}`}</span>
            </div>
          ))}
          {o.references.map((s) => (
            <div key={`r:${s.name}`} className="tree-slot" style={{ paddingLeft: 26 + (depth + 1) * 14 }}>
              <span className="slot-name">{s.name}</span>
              <span className="slot-op">→</span>
              <span className="slot-value">{s.values.length ? s.values.join(", ") : "∅"}</span>
            </div>
          ))}
          {o.children.map((c) => (
            <ObjectNode
              key={c.fragment}
              o={c}
              resource={resource}
              depth={depth + 1}
              context={context}
              onSelect={onSelect}
              onJump={onJump}
              onMenu={onMenu}
            />
          ))}
        </>
      )}
    </TreeItem>
  );
}

// ---------------------------------------------------------------- sidebar

export interface SidebarProps {
  packages: PackageInfo[];
  models: ModelResourceInfo[];
  context: ContextObject | null;
  onSelectContext: (c: ContextObject) => void;
  onJumpObject: (o: { resource: string; range?: Range }) => void;
  onLoadMetamodel: () => void;
  onLoadModel: () => void;
  /** Right-click on a package, classifier or object. */
  onContextMenu: MenuHandler;
}

export function Sidebar(props: SidebarProps) {
  const { packages, models, context, onSelectContext, onJumpObject, onLoadMetamodel, onLoadModel, onContextMenu } =
    props;
  return (
    <aside className="sidebar">
      <section className="side-section">
        <header className="side-header">
          <h2>Metamodels</h2>
          <span className="count">{packages.length || ""}</span>
        </header>
        <div className="tree" role="tree" aria-label="Metamodels">
          {packages.length === 0 ? (
            <p className="side-empty">
              No metamodel loaded. <button className="link" onClick={onLoadMetamodel}>Load an .ecore file</button>,
              write one in OCLinEcore, or import one from an OCL document.
            </p>
          ) : (
            packages.map((p) => (
              <PackageNode key={`${p.resource}#${p.nsUri}`} p={p} depth={0} onMenu={onContextMenu} />
            ))
          )}
        </div>
      </section>
      <section className="side-section">
        <header className="side-header">
          <h2>Models</h2>
          <span className="count">{models.length || ""}</span>
        </header>
        <div className="tree" role="tree" aria-label="Models">
          {models.length === 0 ? (
            <p className="side-empty">
              No model loaded. <button className="link" onClick={onLoadModel}>Load an XMI model</button> or write one
              in HUTN to validate and evaluate against it.
            </p>
          ) : (
            models.map((m) => {
              const kind = fileKind(m.uri);
              return (
              <TreeItem
                key={m.uri}
                depth={0}
                badge={kind ? <FileIcon kind={kind} title="resource" /> : <Badge kind="resource" text="R" title="resource" />}
                label={baseName(m.uri)}
                title={m.uri}
                defaultOpen
                hasChildren={m.roots.length > 0}
              >
                {() =>
                  m.roots.map((o) => (
                    <ObjectNode
                      key={o.fragment}
                      o={o}
                      resource={m.uri}
                      depth={1}
                      context={context}
                      onSelect={onSelectContext}
                      onJump={onJumpObject}
                      onMenu={onContextMenu}
                    />
                  ))
                }
              </TreeItem>
              );
            })
          )}
        </div>
      </section>
    </aside>
  );
}

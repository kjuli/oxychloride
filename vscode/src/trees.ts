// The sidebar views: the loaded metamodels, the models (objects of HUTN and XMI documents and
// of loaded XMI files) and the results of the last validation run.

import * as vscode from "vscode";
import { openDocument } from "./documents";
import type {
  ClassifierInfo,
  ConstraintInfo,
  Engine,
  ModelResourceInfo,
  ObjectInfo,
  ObjectRef,
  PackageInfo,
  ValidationReport,
  ValidationResult,
} from "./engine";

export function shortName(qualified: string): string {
  const i = qualified.lastIndexOf("::");
  return i >= 0 ? qualified.slice(i + 2) : qualified;
}

function baseName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path;
}

function multiplicity(lower: number, upper: number): string {
  if (lower === 0 && upper === 1) return "";
  const u = upper === -1 ? "*" : upper === -2 ? "?" : String(upper);
  return lower === upper ? `[${u}]` : `[${lower}..${u}]`;
}

const icon = (id: string, color?: string) =>
  new vscode.ThemeIcon(id, color ? new vscode.ThemeColor(color) : undefined);

// ---------------------------------------------------------------- metamodels

export type MetamodelNode =
  | { kind: "package"; package: PackageInfo }
  | { kind: "classifier"; classifier: ClassifierInfo }
  | { kind: "member"; label: string; description?: string; icon: string; tooltip?: string };

export class MetamodelTree implements vscode.TreeDataProvider<MetamodelNode> {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  packages: PackageInfo[] = [];

  constructor(private readonly engine: Engine) {}

  async refresh(): Promise<void> {
    this.packages = await this.engine.packages().catch(() => []);
    this.changed.fire();
  }

  getChildren(node?: MetamodelNode): MetamodelNode[] {
    if (!node) return this.packages.map((p) => ({ kind: "package", package: p }));
    if (node.kind === "package") {
      return [
        ...node.package.subpackages.map((p): MetamodelNode => ({ kind: "package", package: p })),
        ...node.package.classifiers.map((c): MetamodelNode => ({ kind: "classifier", classifier: c })),
      ];
    }
    if (node.kind === "classifier") return members(node.classifier);
    return [];
  }

  getTreeItem(node: MetamodelNode): vscode.TreeItem {
    if (node.kind === "package") {
      const p = node.package;
      const item = new vscode.TreeItem(p.name, vscode.TreeItemCollapsibleState.Expanded);
      item.description = p.nsUri;
      item.tooltip = `${p.nsUri}\n${p.resource}`;
      item.iconPath = icon("package");
      item.contextValue = "package";
      return item;
    }
    if (node.kind === "classifier") {
      const c = node.classifier;
      const hasMembers = members(c).length > 0;
      const item = new vscode.TreeItem(
        c.name,
        hasMembers ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None,
      );
      const supers = c.superTypes ?? [];
      item.description = [c.isAbstract && !c.isInterface ? "abstract" : "", supers.length ? `extends ${supers.map(shortName).join(", ")}` : ""]
        .filter(Boolean)
        .join(" ");
      item.tooltip = c.qualifiedName;
      item.iconPath = icon(
        c.kind === "enum"
          ? "symbol-enum"
          : c.kind === "dataType"
            ? "symbol-struct"
            : c.isInterface
              ? "symbol-interface"
              : "symbol-class",
      );
      item.contextValue =
        c.kind !== "class" ? c.kind : c.isAbstract || c.isInterface ? "abstractClass" : "class";
      return item;
    }
    const item = new vscode.TreeItem(node.label);
    item.description = node.description;
    item.tooltip = node.tooltip;
    item.iconPath = icon(node.icon);
    return item;
  }
}

function members(c: ClassifierInfo): MetamodelNode[] {
  const out: MetamodelNode[] = [];
  for (const f of c.features ?? []) {
    out.push({
      kind: "member",
      label: `${f.derived ? "/" : ""}${f.name}`,
      description: `${f.type}${multiplicity(f.lower, f.upper)}${f.opposite ? ` ⇄ ${f.opposite}` : ""}`,
      icon: f.isReference ? (f.containment ? "symbol-structure" : "references") : "symbol-field",
      tooltip: f.isReference ? (f.containment ? "containment" : "reference") : "attribute",
    });
  }
  for (const op of c.operations ?? []) {
    const params = op.parameters.map((p) => `${p.name} : ${p.type}`).join(", ");
    out.push({ kind: "member", label: `${op.name}(${params})`, description: op.type, icon: "symbol-method" });
  }
  for (const l of c.literals ?? []) out.push({ kind: "member", label: l, icon: "symbol-enum-member" });
  for (const inv of c.invariants ?? []) {
    out.push({ kind: "member", label: inv.name || "invariant", description: inv.expression, icon: "check", tooltip: inv.expression });
  }
  return out;
}

// ---------------------------------------------------------------- models

export type ModelNode =
  | { kind: "resource"; resource: ModelResourceInfo }
  | { kind: "object"; object: ObjectInfo; resource: string }
  | { kind: "slot"; label: string; description: string; icon: string };

export class ModelTree implements vscode.TreeDataProvider<ModelNode> {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  models: ModelResourceInfo[] = [];
  /** The evaluation context of the console, highlighted in the tree. */
  context: ObjectRef | undefined;

  constructor(private readonly engine: Engine) {}

  async refresh(): Promise<void> {
    this.models = await this.engine.models().catch(() => []);
    this.changed.fire();
  }

  setContext(context: ObjectRef | undefined): void {
    this.context = context;
    this.changed.fire();
  }

  getChildren(node?: ModelNode): ModelNode[] {
    if (!node) return this.models.map((m) => ({ kind: "resource", resource: m }));
    if (node.kind === "resource") {
      return node.resource.roots.map((o) => ({ kind: "object", object: o, resource: node.resource.uri }));
    }
    if (node.kind === "object") {
      const o = node.object;
      return [
        ...o.attributes.map((s): ModelNode => ({
          kind: "slot",
          label: s.name,
          description: `= ${s.values.length === 1 ? s.values[0] : `{${s.values.join(", ")}}`}`,
          icon: "symbol-field",
        })),
        ...o.references.map((s): ModelNode => ({
          kind: "slot",
          label: s.name,
          description: `→ ${s.values.length ? s.values.join(", ") : "∅"}`,
          icon: "references",
        })),
        ...o.children.map((c): ModelNode => ({ kind: "object", object: c, resource: node.resource })),
      ];
    }
    return [];
  }

  getTreeItem(node: ModelNode): vscode.TreeItem {
    if (node.kind === "resource") {
      const r = node.resource;
      const item = new vscode.TreeItem(baseName(r.uri), vscode.TreeItemCollapsibleState.Expanded);
      item.tooltip = r.uri;
      item.iconPath = icon("file-code");
      item.contextValue = "resource";
      return item;
    }
    if (node.kind === "object") {
      const o = node.object;
      const hasChildren = o.attributes.length + o.references.length + o.children.length > 0;
      const item = new vscode.TreeItem(
        o.label,
        hasChildren ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None,
      );
      const isContext = this.context?.resource === node.resource && this.context.fragment === o.fragment;
      item.description = `${shortName(o.class)}${isContext ? " · self" : ""}`;
      item.tooltip = `${o.class}\n${o.fragment}`;
      item.iconPath = isContext ? icon("target", "charts.green") : icon("symbol-object");
      // Objects of HUTN documents can be edited from the tree.
      item.contextValue = o.range && openDocument(node.resource)?.languageId === "hutn" ? "editableObject" : "object";
      if (o.range) {
        item.command = { command: "oxychloride.showDefinition", title: "Show Definition", arguments: [node] };
      }
      return item;
    }
    const item = new vscode.TreeItem(node.label);
    item.description = node.description;
    item.iconPath = icon(node.icon);
    return item;
  }
}

// ---------------------------------------------------------------- validation

export type ValidationNode =
  | { kind: "summary"; text: string; ok: boolean }
  | { kind: "note"; text: string }
  | { kind: "constraint"; constraint: ConstraintInfo; results: ValidationResult[] }
  | { kind: "result"; result: ValidationResult };

export class ValidationTree implements vscode.TreeDataProvider<ValidationNode> {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  report: ValidationReport | undefined;

  show(report: ValidationReport): void {
    this.report = report;
    this.changed.fire();
  }

  getChildren(node?: ValidationNode): ValidationNode[] {
    const report = this.report;
    if (!report) return [];
    if (!node) {
      const s = report.summary;
      const failing = s.violated + s.errors;
      const nodes: ValidationNode[] = [
        {
          kind: "summary",
          ok: failing === 0,
          text: `${failing === 0 ? "Valid" : `${s.violated} violated, ${s.errors} errors`}, ${s.satisfied} satisfied`,
        },
      ];
      const byConstraint = new Map<number, ValidationResult[]>();
      for (const r of report.results) byConstraint.set(r.constraint, [...(byConstraint.get(r.constraint) ?? []), r]);
      const rank = (rs: ValidationResult[]) =>
        rs.some((r) => r.status === "violated") ? 0 : rs.some((r) => r.status === "error") ? 1 : 2;
      const groups = [...byConstraint.entries()].sort((a, b) => rank(a[1]) - rank(b[1]) || a[0] - b[0]);
      for (const [i, results] of groups) nodes.push({ kind: "constraint", constraint: report.constraints[i], results });
      for (const text of report.notes ?? []) nodes.push({ kind: "note", text });
      return nodes;
    }
    if (node.kind === "constraint") return node.results.map((r) => ({ kind: "result", result: r }));
    return [];
  }

  getTreeItem(node: ValidationNode): vscode.TreeItem {
    switch (node.kind) {
      case "summary": {
        const s = this.report!.summary;
        const item = new vscode.TreeItem(node.text);
        item.description = `${s.constraints} constraints on ${s.objects} objects`;
        item.iconPath = node.ok ? icon("pass", "testing.iconPassed") : icon("error", "testing.iconFailed");
        return item;
      }
      case "note": {
        const item = new vscode.TreeItem(node.text);
        item.iconPath = icon("info");
        return item;
      }
      case "constraint": {
        const c = node.constraint;
        const failing = node.results.filter((r) => r.status !== "satisfied").length;
        const item = new vscode.TreeItem(
          `${shortName(c.context)}::${c.name}`,
          failing ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.Collapsed,
        );
        item.description = `${c.kind}${c.operation ? ` of ${c.operation}` : ""}${failing ? ` · ${failing} failing` : ""}`;
        item.tooltip = new vscode.MarkdownString().appendCodeblock(c.text, "ocl");
        item.iconPath = statusIcon(node.results.some((r) => r.status === "violated") ? "violated" : node.results.some((r) => r.status === "error") ? "error" : "satisfied");
        item.contextValue = "constraint";
        if (c.origin.kind === "document") {
          item.command = { command: "oxychloride.showConstraint", title: "Show Constraint", arguments: [c] };
        }
        return item;
      }
      case "result": {
        const r = node.result;
        const item = new vscode.TreeItem(r.call ? `${r.object.label}.${r.call}` : r.object.label);
        item.description = r.message ?? r.status;
        item.tooltip = `${r.object.class} ${r.object.fragment}\n${r.status}${r.message ? `: ${r.message}` : ""}`;
        item.iconPath = statusIcon(r.status);
        item.contextValue = "result";
        if (r.object.range) {
          item.command = { command: "oxychloride.showDefinition", title: "Show Definition", arguments: [node] };
        }
        return item;
      }
    }
  }
}

function statusIcon(status: ValidationResult["status"]): vscode.ThemeIcon {
  if (status === "satisfied") return icon("pass", "testing.iconPassed");
  if (status === "violated") return icon("error", "testing.iconFailed");
  return icon("warning", "problemsWarningIcon.foreground");
}

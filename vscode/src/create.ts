// Creating model content from the views: instances of classes (in HUTN models or as new XMI
// dynamic instances), children of objects, new models.

import * as path from "node:path";
import * as vscode from "vscode";
import { applyEdit, documentName, openDocument } from "./documents";
import { errorMessage, type ClassifierInfo, type Engine, type PackageInfo } from "./engine";
import { shortName, type ModelNode } from "./trees";

/** The root package containing the package `test` accepts. */
export function rootPackage(packages: PackageInfo[], test: (p: PackageInfo) => boolean): PackageInfo | undefined {
  const contains = (p: PackageInfo): boolean => test(p) || p.subpackages.some(contains);
  return packages.find(contains);
}

function packageOf(packages: PackageInfo[], qualifiedName: string): PackageInfo | undefined {
  const find = (ps: PackageInfo[]): PackageInfo | undefined => {
    for (const p of ps) {
      if (p.classifiers.some((c) => c.qualifiedName === qualifiedName)) return p;
      const found = find(p.subpackages);
      if (found) return found;
    }
    return undefined;
  };
  return find(packages);
}

function hutnTemplate(pkg: PackageInfo | undefined): string {
  if (!pkg) return "// A model in HUTN: objects are `Class \"identifier\" { feature: value ... }`.\n";
  return `// A model of ${pkg.name} in HUTN: objects are \`Class "identifier" { feature: value ... }\`.
@Spec {
  metamodel "${pkg.name}" {
    nsUri: "${pkg.nsUri}"
  }
}

${pkg.name} {
}
`;
}

export class Creator {
  /** The HUTN document used last: where new instances go when no HUTN editor is active. */
  private lastModel: vscode.Uri | undefined;

  constructor(
    private readonly engine: Engine,
    private readonly packages: () => PackageInfo[],
  ) {}

  /** Remembers the model documents the user works in. */
  track(editor: vscode.TextEditor | undefined): void {
    if (editor?.document.languageId === "hutn") this.lastModel = editor.document.uri;
  }

  /** Opens a new (unsaved) HUTN model of `pkg` (by default: chosen by the user). */
  async newModel(pkg?: PackageInfo): Promise<vscode.TextDocument | undefined> {
    if (!pkg) {
      const roots = this.packages();
      if (roots.length > 1) {
        const pick = await vscode.window.showQuickPick(
          roots.map((p) => ({ label: p.name, description: p.nsUri, pkg: p })),
          { title: "New model of which metamodel?" },
        );
        if (!pick) return undefined;
        pkg = pick.pkg;
      } else {
        pkg = roots[0];
      }
    }
    const root = pkg && (rootPackage(this.packages(), (p) => p === pkg) ?? pkg);
    const doc = await vscode.workspace.openTextDocument({ language: "hutn", content: hutnTemplate(root) });
    const editor = await vscode.window.showTextDocument(doc, { preview: false });
    // At the end of the package block's first line.
    const end = doc.lineAt(Math.max(0, doc.lineCount - 3)).range.end;
    editor.selection = new vscode.Selection(end, end);
    return doc;
  }

  /** The HUTN document a new instance goes to: the active one, the one used last, one the
   * user picks, or a new one (`undefined`). `null` if the user cancelled. */
  private async target(): Promise<vscode.TextDocument | undefined | null> {
    const active = vscode.window.activeTextEditor?.document;
    if (active?.languageId === "hutn") return active;
    const last = this.lastModel && vscode.workspace.textDocuments.find((d) => d.uri.toString() === this.lastModel?.toString());
    if (last && !last.isClosed) return last;
    const models = vscode.workspace.textDocuments.filter((d) => d.languageId === "hutn" && !d.isClosed);
    if (models.length === 0) return undefined;
    const pick = await vscode.window.showQuickPick(
      [
        ...models.map((d) => ({ label: path.basename(d.uri.path), description: vscode.workspace.asRelativePath(d.uri), doc: d })),
        { label: "$(new-file) New model", description: "", doc: undefined },
      ],
      { title: "Create the instance in which model?" },
    );
    return pick ? pick.doc : null;
  }

  /** Adds an instance of `cls` as a root object of a HUTN model (`inNew`: of a new one). */
  async createInstance(cls: ClassifierInfo, inNew = false): Promise<boolean> {
    if (cls.kind !== "class" || cls.isAbstract || cls.isInterface) {
      void vscode.window.showWarningMessage(`${cls.name} cannot be instantiated: it is ${cls.isInterface ? "an interface" : "abstract"}.`);
      return false;
    }
    let doc = inNew ? undefined : await this.target();
    if (doc === null) return false;
    if (!doc) {
      const packages = this.packages();
      const pkg = packageOf(packages, cls.qualifiedName);
      doc = await this.newModel(pkg && (rootPackage(packages, (p) => p === pkg) ?? pkg));
      if (!doc) return false;
    }
    return this.insert(doc, cls.qualifiedName);
  }

  /** Adds a child to an object of a HUTN document: of class `choice.cls` in `choice.feature`,
   * or in a containment feature and of a class the user picks. */
  async addChild(node: ModelNode, choice?: { feature: string; cls: string }): Promise<boolean> {
    if (node.kind !== "object") return false;
    const doc = openDocument(node.resource);
    if (!doc) {
      void vscode.window.showWarningMessage(`${node.resource} is not open.`);
      return false;
    }
    if (choice) return this.insert(doc, choice.cls, { fragment: node.object.fragment, feature: choice.feature });
    try {
      const options = await this.engine.containmentOptions({ resource: node.resource, fragment: node.object.fragment });
      type Item = vscode.QuickPickItem & { feature?: string; cls?: string };
      const items: Item[] = [];
      for (const o of options) {
        if (!o.classes.length) continue;
        items.push({ label: o.feature, kind: vscode.QuickPickItemKind.Separator });
        for (const cls of o.classes) {
          items.push({
            label: shortName(cls),
            description: o.full ? `${o.feature} (already set)` : o.feature,
            detail: cls,
            feature: o.feature,
            cls,
          });
        }
      }
      const choices = items.filter((i) => i.cls);
      if (choices.length === 0) {
        void vscode.window.showInformationMessage(`${node.object.label} has no containment features.`);
        return false;
      }
      const pick =
        choices.length === 1
          ? choices[0]
          : await vscode.window.showQuickPick(items, { title: `Add a child to ${node.object.label}`, matchOnDetail: true });
      if (!pick?.cls || !pick.feature) return false;
      return this.insert(doc, pick.cls, { fragment: node.object.fragment, feature: pick.feature });
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return false;
    }
  }

  private async insert(
    doc: vscode.TextDocument,
    cls: string,
    container?: { fragment: string; feature: string },
  ): Promise<boolean> {
    const base = doc.getText();
    try {
      const edit = await this.engine.createObject(documentName(doc.uri), cls, container, base);
      if (!(await applyEdit(doc, base, edit))) {
        void vscode.window.showWarningMessage(`${path.basename(doc.uri.path)} changed meanwhile; try again.`);
        return false;
      }
      this.lastModel = doc.uri;
      return true;
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return false;
    }
  }

  /** Creates an XMI file whose root is an instance of `cls` (EMF's "Create Dynamic Instance"). */
  async createXmiInstance(cls: ClassifierInfo): Promise<boolean> {
    if (cls.kind !== "class" || cls.isAbstract || cls.isInterface) {
      void vscode.window.showWarningMessage(`${cls.name} cannot be instantiated.`);
      return false;
    }
    const pkg = packageOf(this.packages(), cls.qualifiedName);
    if (!pkg) return false;
    const folder =
      vscode.window.activeTextEditor?.document.uri.scheme === "file"
        ? vscode.Uri.joinPath(vscode.window.activeTextEditor.document.uri, "..")
        : vscode.workspace.workspaceFolders?.[0]?.uri;
    const target = await vscode.window.showSaveDialog({
      defaultUri: folder && vscode.Uri.joinPath(folder, `${cls.name}.xmi`),
      filters: { "XMI models": ["xmi"] },
      title: `Create a dynamic instance of ${cls.name}`,
    });
    if (!target) return false;
    const prefix = pkg.nsPrefix || pkg.name;
    // EMF names the metamodel file in xsi:schemaLocation; OCLinEcore metamodels are found by nsURI.
    const ecore = pkg.resource.endsWith(".ecore")
      ? ` xsi:schemaLocation="${pkg.nsUri} ${path.relative(path.dirname(target.fsPath), pkg.resource).split(path.sep).join("/")}"`
      : "";
    const text = `<?xml version="1.0" encoding="UTF-8"?>
<${prefix}:${cls.name} xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:${prefix}="${pkg.nsUri}"${ecore}>
  
</${prefix}:${cls.name}>
`;
    await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
    const doc = await vscode.workspace.openTextDocument(target);
    const editor = await vscode.window.showTextDocument(doc, { preview: false });
    const line = doc.lineAt(Math.max(0, doc.lineCount - 3));
    editor.selection = new vscode.Selection(line.range.end, line.range.end);
    return true;
  }
}

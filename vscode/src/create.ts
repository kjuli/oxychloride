// Creating model content from the views: instances of classes (in HUTN models or as new XMI
// dynamic instances, also of a metamodel file or the class at the cursor), children of
// objects, new models.

import * as fs from "node:fs/promises";
import * as path from "node:path";
import * as vscode from "vscode";
import { applyEdit, documentName, fromVsPosition, openDocument, uriOf } from "./documents";
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

/** The classes of `packages` and their subpackages. */
function classesOf(packages: PackageInfo[]): ClassifierInfo[] {
  return packages.flatMap((p) => [...p.classifiers.filter((c) => c.kind === "class"), ...classesOf(p.subpackages)]);
}

const instantiable = (c: ClassifierInfo) => c.kind === "class" && !c.isAbstract && !c.isInterface;

/** Whether no containment feature of `packages` can hold an instance of `cls`: a candidate
 * for the root of a model. (Feature types name classes by their simple names.) */
function isRootCandidate(cls: ClassifierInfo, packages: PackageInfo[]): boolean {
  const all = classesOf(packages);
  const contained = new Set(
    all.flatMap((c) => (c.features ?? []).filter((f) => f.containment).map((f) => f.type.replace(/^\w+\((.*)\)$/, "$1"))),
  );
  const byName = new Map(all.map((c) => [c.qualifiedName, c]));
  const seen = new Set<string>();
  const held = (c: ClassifierInfo): boolean => {
    if (seen.has(c.qualifiedName)) return false;
    seen.add(c.qualifiedName);
    if (contained.has(c.name)) return true;
    return (c.superTypes ?? []).some((s) => {
      const sup = byName.get(s);
      return sup ? held(sup) : contained.has(shortName(s));
    });
  };
  return !held(cls);
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

  /** Creates an XMI file whose root is an instance of `cls` (EMF's "Create Dynamic Instance"),
   * by default next to the metamodel. */
  async createXmiInstance(cls: ClassifierInfo): Promise<boolean> {
    const packages = this.packages();
    const pkg = packageOf(packages, cls.qualifiedName);
    if (!pkg) return false;
    const root = rootPackage(packages, (p) => p === pkg) ?? pkg;
    const metamodel = uriOf(root.resource);
    return (await this.writeXmiInstance(cls, pkg, metamodel.scheme === "file" ? metamodel : undefined)) !== undefined;
  }

  /** Creates an XMI model of the metamodel file `uri` (`.oclinecore` or `.ecore`) next to it:
   * its root is an instance of the class defined at `position` of an OCLinEcore document, or
   * else of a class the user picks (those no containment holds first). `target` is the file
   * to write, instead of one the user chooses. Returns the file written. */
  async instantiate(uri: vscode.Uri, position?: vscode.Position, target?: vscode.Uri): Promise<vscode.Uri | undefined> {
    const name = path.basename(uri.path);
    let packages: PackageInfo[];
    let cls: ClassifierInfo | undefined;
    try {
      let resource = documentName(uri);
      let qualified: string | null | undefined;
      if (/\.ecore$/i.test(uri.path)) {
        resource = (await this.engine.loadEcore(uri.fsPath)).uri;
      } else {
        // Opening the document hands it to the language server (if it is not in a workspace folder).
        const doc = await vscode.workspace.openTextDocument(uri);
        qualified = position && (await this.engine.classAt(resource, fromVsPosition(doc, position)));
      }
      packages = await this.engine.packages();
      cls = qualified ? classesOf(packages).find((c) => c.qualifiedName === qualified) : undefined;
      const own = packages.filter((p) => p.resource === resource);
      if (!own.length) {
        void vscode.window.showWarningMessage(`${name} defines no packages${resource.endsWith(".oclinecore") ? " (does it have errors?)" : ""}.`);
        return undefined;
      }
      if (!cls || !instantiable(cls)) {
        const reason = cls && `${cls.name} is ${cls.isInterface ? "an interface" : "abstract"}`;
        cls = await this.pickClass(classesOf(own).filter(instantiable), packages, reason ? `${reason}: instantiate which class?` : `Instantiate which class of ${name}?`);
        if (!cls) return undefined;
      }
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return undefined;
    }
    const pkg = packageOf(packages, cls.qualifiedName);
    return pkg && this.writeXmiInstance(cls, pkg, uri.scheme === "file" ? uri : undefined, target);
  }

  /** Lets the user pick one of `classes` as the root of a model. */
  private async pickClass(classes: ClassifierInfo[], packages: PackageInfo[], title: string): Promise<ClassifierInfo | undefined> {
    if (!classes.length) {
      void vscode.window.showWarningMessage("The metamodel has no classes that can be instantiated.");
      return undefined;
    }
    const roots = classes.filter((c) => isRootCandidate(c, packages));
    type Item = vscode.QuickPickItem & { cls?: ClassifierInfo };
    const item = (c: ClassifierInfo): Item => ({ label: c.name, description: c.qualifiedName, cls: c });
    const others = classes.filter((c) => !roots.includes(c));
    const items: Item[] =
      roots.length && others.length
        ? [
            { label: "Model roots", kind: vscode.QuickPickItemKind.Separator },
            ...roots.map(item),
            { label: "Contained classes", kind: vscode.QuickPickItemKind.Separator },
            ...others.map(item),
          ]
        : classes.map(item);
    return (await vscode.window.showQuickPick(items, { title, matchOnDescription: true }))?.cls;
  }

  /** Writes an XMI file whose root is an instance of `cls` (of package `pkg`) and opens it: to
   * `target`, or else to a file the user chooses, by default `<Class>.xmi` next to `near`. */
  private async writeXmiInstance(
    cls: ClassifierInfo,
    pkg: PackageInfo,
    near?: vscode.Uri,
    target?: vscode.Uri,
  ): Promise<vscode.Uri | undefined> {
    if (!instantiable(cls)) {
      void vscode.window.showWarningMessage(`${cls.name} cannot be instantiated: it is ${cls.isInterface ? "an interface" : "abstract"}.`);
      return undefined;
    }
    if (!target) {
      const active = vscode.window.activeTextEditor?.document.uri;
      const folder = near
        ? vscode.Uri.joinPath(near, "..")
        : active?.scheme === "file"
          ? vscode.Uri.joinPath(active, "..")
          : vscode.workspace.workspaceFolders?.[0]?.uri;
      target = await vscode.window.showSaveDialog({
        defaultUri: folder && (await freeFile(folder, cls.name, "xmi")),
        filters: { "XMI models": ["xmi"] },
        title: `Create a model with a ${cls.name} as its root`,
      });
      if (!target) return undefined;
    }
    const prefix = pkg.nsPrefix || pkg.name;
    // EMF names the metamodel file in xsi:schemaLocation; OCLinEcore metamodels are found by nsURI.
    const ecore = pkg.resource.endsWith(".ecore")
      ? ` xsi:schemaLocation="${pkg.nsUri} ${await relativePath(path.dirname(target.fsPath), pkg.resource)}"`
      : "";
    const text = `<?xml version="1.0" encoding="UTF-8"?>
<${prefix}:${cls.name} xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:${prefix}="${pkg.nsUri}"${ecore}>
  
</${prefix}:${cls.name}>
`;
    try {
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not write ${path.basename(target.path)}: ${errorMessage(e)}`);
      return undefined;
    }
    const doc = await vscode.workspace.openTextDocument(target);
    const editor = await vscode.window.showTextDocument(doc, { preview: false });
    const line = doc.lineAt(Math.max(0, doc.lineCount - 3));
    editor.selection = new vscode.Selection(line.range.end, line.range.end);
    return target;
  }
}

/** `<stem>.<extension>` in `folder`, or `<stem>2.<extension>`, … if that exists. */
async function freeFile(folder: vscode.Uri, stem: string, extension: string): Promise<vscode.Uri> {
  for (let i = 1; ; i++) {
    const uri = vscode.Uri.joinPath(folder, `${stem}${i > 1 ? i : ""}.${extension}`);
    try {
      await vscode.workspace.fs.stat(uri);
    } catch {
      return uri;
    }
  }
}

/** The path of `file` relative to the directory `from`, with `/`s. Symbolic links are resolved
 * first: the engine names `.ecore` files by their real paths (`/private/var/…` for `/var/…`). */
async function relativePath(from: string, file: string): Promise<string> {
  const real = (p: string) => fs.realpath(p).catch(() => p);
  return path.relative(await real(from), await real(file)).split(path.sep).join("/");
}

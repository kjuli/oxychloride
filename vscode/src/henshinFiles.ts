// New Henshin transformations: a `.henshin` file (Henshin's model, drawn as a rule diagram) or a
// Henshin text that imports the metamodel of a metamodel or model file, and the root element of
// `.henshin` files that are empty when they are opened (e.g. made with New File). The engine
// writes both (`editTransformation` with `setImports`): an empty `.henshin` file is an empty
// module to it, and any edit of one writes the whole module.

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName, toVsRange } from "./documents";
import { errorMessage, type Engine, type PackageInfo } from "./engine";
import { counterpart } from "./henshinPairs";
import { openRuleDiagram } from "./ruleDiagram";

const METAMODEL_FILES = /\.(ecore|oclinecore)$/i;
const isEmptyModel = (doc: vscode.TextDocument) => doc.uri.scheme === "file" && /\.henshin$/i.test(doc.uri.path) && doc.getText().trim() === "";

async function exists(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}

export class HenshinFiles implements vscode.Disposable {
  private readonly disposables: vscode.Disposable[] = [];
  /** The documents being filled. */
  private readonly filling = new Set<string>();

  constructor(
    private readonly engine: Engine,
    private readonly ready: Promise<void>,
  ) {
    this.disposables.push(vscode.workspace.onDidOpenTextDocument((doc) => void this.fill(doc)));
    for (const doc of vscode.workspace.textDocuments) void this.fill(doc);
  }

  dispose(): void {
    for (const d of this.disposables) d.dispose();
  }

  /** Writes the module into an empty `.henshin` file: what the Henshin text of the same name
   * means if there is one (they are a pair from then on), else a module importing nothing. */
  async fill(doc: vscode.TextDocument): Promise<void> {
    const key = doc.uri.toString();
    if (!isEmptyModel(doc) || this.filling.has(key)) return;
    this.filling.add(key);
    try {
      await this.ready;
      if (doc.isClosed || !isEmptyModel(doc)) return;
      const base = doc.getText();
      const text = await this.pairText(doc.uri);
      const edits = text === undefined ? (await this.engine.editTransformation(documentName(doc.uri), base, { kind: "setImports", imports: [] })).edits : [];
      if (doc.getText() !== base) return;
      const workspaceEdit = new vscode.WorkspaceEdit();
      if (text !== undefined) workspaceEdit.replace(doc.uri, new vscode.Range(doc.positionAt(0), doc.positionAt(base.length)), text);
      for (const e of edits) workspaceEdit.replace(doc.uri, toVsRange(doc, e.range), e.text);
      if (await vscode.workspace.applyEdit(workspaceEdit)) await doc.save();
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not write the Henshin module of ${path.basename(doc.uri.path)}: ${errorMessage(e)}`);
    } finally {
      this.filling.delete(key);
    }
  }

  /** The `.henshin` file of the Henshin text next to `target`, if there is one. Fails if the
   * text cannot be converted (an empty module would replace its rules). */
  private async pairText(target: vscode.Uri): Promise<string | undefined> {
    const text = counterpart(target);
    if (!text || !(await exists(text))) return undefined;
    // Opening the document hands it to the language server.
    const doc = await vscode.workspace.openTextDocument(text);
    try {
      return (await this.engine.exportDocument(documentName(doc.uri), target.fsPath)).text;
    } catch (e) {
      throw new Error(`${path.basename(text.path)} next to it cannot be converted (${errorMessage(e)})`);
    }
  }

  /** Creates a Henshin transformation importing the metamodel of `source` (a metamodel, or a
   * model of the metamodel) next to it, and opens its rule diagram. Returns the file written. */
  async create(source: vscode.Uri | undefined, target?: vscode.Uri): Promise<vscode.Uri | undefined> {
    await this.ready;
    let imports: string[] | undefined;
    try {
      imports = source ? await this.importsOf(source) : undefined;
      imports ??= await this.pickImports(source ? `${path.basename(source.path)} names no metamodel: import which packages?` : "Import which packages?");
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return undefined;
    }
    if (!imports) return undefined;
    target ??= await this.chooseTarget(source);
    if (!target) return undefined;
    const pair = counterpart(target);
    if (pair && (await exists(pair))) {
      void vscode.window.showErrorMessage(
        `${path.basename(pair.path)} exists: a new ${path.basename(target.path)} would be kept equivalent to it. Choose another name.`,
      );
      return undefined;
    }
    try {
      const { edits } = await this.engine.editTransformation(documentName(target), "", { kind: "setImports", imports });
      // All edits of an empty text insert at its start.
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(edits.map((e) => e.text).join("")));
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not write ${path.basename(target.path)}: ${errorMessage(e)}`);
      return undefined;
    }
    if (/\.henshin$/i.test(target.path)) {
      await vscode.commands.executeCommand("vscode.open", target);
    } else {
      await vscode.window.showTextDocument(target, { preview: false });
      await openRuleDiagram(target);
    }
    return target;
  }

  /** The top-level packages of a metamodel file, or of the metamodel of a model file's
   * objects (`undefined` if there are none). */
  private async importsOf(uri: vscode.Uri): Promise<string[] | undefined> {
    let resource = documentName(uri);
    if (/\.ecore$/i.test(uri.path)) {
      resource = (await this.engine.loadEcore(uri.fsPath)).uri;
    } else {
      // Opening the document hands it to the language server (if it is not in a workspace folder).
      await vscode.workspace.openTextDocument(uri);
    }
    const packages = await this.engine.packages();
    if (METAMODEL_FILES.test(uri.path)) {
      const names = packages.filter((p) => p.resource === resource).map((p) => p.name);
      if (!names.length) throw new Error(`${path.basename(uri.path)} defines no packages (does it have errors?).`);
      return names;
    }
    const model = (await this.engine.models()).find((m) => m.uri === resource || m.uri === uri.fsPath);
    // Classes are named `root::sub::Class`.
    const names = [...new Set((model?.roots ?? []).map((o) => o.class.split("::")[0]))].filter((n) => packages.some((p) => p.name === n));
    return names.length ? names : undefined;
  }

  private async pickImports(title: string): Promise<string[] | undefined> {
    const packages: PackageInfo[] = await this.engine.packages();
    if (!packages.length) {
      void vscode.window.showWarningMessage("No metamodel is loaded: open an .oclinecore metamodel or register an .ecore file.");
      return undefined;
    }
    const picks = await vscode.window.showQuickPick(
      packages.map((p) => ({ label: p.name, description: p.nsUri })),
      { title, canPickMany: true },
    );
    return picks?.map((p) => p.label);
  }

  /** The file to write, by default `<name>.henshin` next to `near` (a name not taken, with no
   * Henshin text of the same name). */
  private async chooseTarget(near: vscode.Uri | undefined): Promise<vscode.Uri | undefined> {
    const folder =
      near?.scheme === "file" ? vscode.Uri.joinPath(near, "..") : (vscode.workspace.workspaceFolders?.[0]?.uri ?? vscode.Uri.file(process.cwd()));
    const stem = near ? path.basename(near.path).replace(/\.[^.]+$/, "") : "transformation";
    let suggestion = vscode.Uri.joinPath(folder, `${stem}.henshin`);
    for (let n = 2; (await exists(suggestion)) || (await exists(counterpart(suggestion)!)); n++) {
      suggestion = vscode.Uri.joinPath(folder, `${stem}${n}.henshin`);
    }
    return vscode.window.showSaveDialog({
      defaultUri: suggestion,
      title: "New Henshin Transformation",
      filters: { "Henshin model": ["henshin"], "Henshin text": ["henshin_text"] },
      saveLabel: "Create",
    });
  }
}

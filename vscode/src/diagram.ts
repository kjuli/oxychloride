// The metamodel diagram: a custom editor for `.ecore` and `.oclinecore` files that draws the
// metamodel as a class diagram (webview/diagram.tsx) and edits it. It works on the same
// TextDocument as the text editor: an edit in the diagram becomes text edits of the document
// (computed by the engine, `editMetamodel`), and every change of the document — from either
// editor — draws the diagram again (`metamodelDiagram`). Undo, redo, saving and the dirty state
// are the document's.
//
// Where the classifiers are drawn is not part of the metamodel: it is kept in a layout file
// next to it, `<file>.diagram.json` (e.g. `university.oclinecore.diagram.json`), written when
// the diagram's layout is changed.

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName, fromVsPosition, toVsRange } from "./documents";
import { errorMessage, type Engine, type MetamodelDiagram } from "./engine";
import type { FromWebview, Layout, ToWebview } from "./diagramProtocol";

/** The layout file of a metamodel file. */
export function layoutUri(document: vscode.Uri): vscode.Uri | undefined {
  if (document.scheme !== "file") return undefined;
  return document.with({ path: `${document.path}.diagram.json` });
}

/** The contents of a layout file: positions by element id. */
interface LayoutFile {
  version: 1;
  nodes: Layout;
}

export function parseLayout(text: string): Layout {
  const json = JSON.parse(text) as Partial<LayoutFile>;
  const nodes: Layout = {};
  for (const [id, p] of Object.entries(json.nodes ?? {})) {
    if (typeof p?.x === "number" && typeof p?.y === "number") nodes[id] = { x: p.x, y: p.y };
  }
  return nodes;
}

export function printLayout(layout: Layout): string {
  const nodes: Layout = {};
  for (const id of Object.keys(layout).sort()) nodes[id] = { x: Math.round(layout[id].x), y: Math.round(layout[id].y) };
  return `${JSON.stringify({ version: 1, nodes } satisfies LayoutFile, null, 2)}\n`;
}

export class MetamodelDiagramEditor implements vscode.CustomTextEditorProvider {
  static readonly viewType = "oxychloride.metamodelDiagram";
  private static readonly views = new Set<DiagramView>();

  static register(context: vscode.ExtensionContext, engine: Engine, ready: Promise<void>): vscode.Disposable {
    return vscode.window.registerCustomEditorProvider(
      MetamodelDiagramEditor.viewType,
      new MetamodelDiagramEditor(engine, context.extensionUri, ready),
      { webviewOptions: { retainContextWhenHidden: true }, supportsMultipleEditorsPerDocument: true },
    );
  }

  /** The open diagrams (for tests and refreshing). */
  static all(): DiagramView[] {
    return [...MetamodelDiagramEditor.views];
  }

  private constructor(
    private readonly engine: Engine,
    private readonly extensionUri: vscode.Uri,
    private readonly ready: Promise<void>,
  ) {}

  resolveCustomTextEditor(document: vscode.TextDocument, panel: vscode.WebviewPanel): void {
    const view = new DiagramView(this.engine, this.extensionUri, document, panel, this.ready);
    MetamodelDiagramEditor.views.add(view);
    panel.onDidDispose(() => MetamodelDiagramEditor.views.delete(view));
  }
}

export class DiagramView {
  private readonly name: string;
  private readonly layoutFile: vscode.Uri | undefined;
  /** What this view last wrote to (or read from) the layout file. */
  private layoutText: string | undefined;
  private refreshTimer: ReturnType<typeof setTimeout> | undefined;
  /** Counts refreshes, so that a slow answer does not replace a newer one. */
  private generation = 0;
  private diagram: MetamodelDiagram | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly loaded: Promise<void>;
  private resolveLoaded: () => void = () => undefined;

  constructor(
    private readonly engine: Engine,
    private readonly extensionUri: vscode.Uri,
    readonly document: vscode.TextDocument,
    readonly panel: vscode.WebviewPanel,
    private readonly ready: Promise<void>,
  ) {
    this.name = documentName(document.uri);
    this.layoutFile = layoutUri(document.uri);
    this.loaded = new Promise((resolve) => (this.resolveLoaded = resolve));
    panel.webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(extensionUri, "out", "webview"), vscode.Uri.joinPath(extensionUri, "media")] };
    const icon = /\.ecore$/i.test(document.uri.path) ? "ecore" : "oclinecore";
    panel.iconPath = {
      light: vscode.Uri.joinPath(extensionUri, "media", "file-icons", `${icon}.svg`),
      dark: vscode.Uri.joinPath(extensionUri, "media", "file-icons", `${icon}-dark.svg`),
    };
    panel.webview.html = this.html();
    this.disposables.push(
      panel.webview.onDidReceiveMessage((m: FromWebview) => void this.receive(m)),
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.document === document && e.contentChanges.length) this.scheduleRefresh(80);
      }),
      vscode.window.onDidChangeTextEditorSelection((e) => {
        // Only the user's moves: selecting an element in the diagram moves the cursor too.
        if (e.textEditor.document !== document || e.kind === undefined || e.kind === vscode.TextEditorSelectionChangeKind.Command) return;
        this.post({ type: "cursor", position: fromVsPosition(document, e.selections[0].active) });
      }),
    );
    if (this.layoutFile) {
      const watcher = vscode.workspace.createFileSystemWatcher(
        new vscode.RelativePattern(vscode.Uri.joinPath(this.layoutFile, ".."), path.posix.basename(this.layoutFile.path)),
      );
      const reread = () => void this.readLayout().then((layout) => layout && this.post({ type: "layout", layout }));
      this.disposables.push(watcher, watcher.onDidChange(reread), watcher.onDidCreate(reread));
    }
    panel.onDidDispose(() => {
      clearTimeout(this.refreshTimer);
      for (const d of this.disposables) d.dispose();
    });
    void this.start();
  }

  /** The diagram last drawn (for tests). */
  get current(): MetamodelDiagram | undefined {
    return this.diagram;
  }

  private post(message: ToWebview): void {
    void this.panel.webview.postMessage(message);
  }

  private async start(): Promise<void> {
    await this.loaded;
    const layout = (await this.readLayout()) ?? {};
    this.post({ type: "init", name: path.basename(this.document.uri.path), layout, persistent: this.layoutFile !== undefined });
    await this.ready;
    await this.refresh();
  }

  /** Reads the layout file; `undefined` if it does not exist, is unchanged or is not valid. */
  private async readLayout(): Promise<Layout | undefined> {
    if (!this.layoutFile) return undefined;
    let text: string;
    try {
      text = new TextDecoder().decode(await vscode.workspace.fs.readFile(this.layoutFile));
    } catch {
      return undefined;
    }
    if (text === this.layoutText) return undefined;
    this.layoutText = text;
    try {
      return parseLayout(text);
    } catch (e) {
      void vscode.window.showWarningMessage(`${path.basename(this.layoutFile.path)} is not a valid diagram layout: ${errorMessage(e)}`);
      return undefined;
    }
  }

  private async writeLayout(layout: Layout, persist: "always" | "ifExists"): Promise<void> {
    if (!this.layoutFile) return;
    if (persist === "ifExists" && this.layoutText === undefined) return;
    const text = printLayout(layout);
    if (text === this.layoutText) return;
    this.layoutText = text;
    await vscode.workspace.fs.writeFile(this.layoutFile, new TextEncoder().encode(text));
  }

  private scheduleRefresh(delay: number): void {
    clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => void this.refresh(), delay);
  }

  /** Draws the diagram of the document's current text. */
  async refresh(): Promise<void> {
    const generation = ++this.generation;
    try {
      const diagram = await this.engine.metamodelDiagram(this.name, this.document.getText());
      if (generation !== this.generation) return;
      this.diagram = diagram;
      this.post({ type: "diagram", diagram });
    } catch (e) {
      if (generation === this.generation) this.post({ type: "unreadable", message: errorMessage(e) });
    }
  }

  private async receive(m: FromWebview): Promise<void> {
    switch (m.type) {
      case "ready":
        this.resolveLoaded();
        break;
      case "edit": {
        const result = await this.edit(m.edit).catch((e: unknown) => ({ error: errorMessage(e) }));
        this.post({ type: "edited", request: m.request, ...result });
        break;
      }
      case "layout":
        await this.writeLayout(m.layout, m.persist).catch((e: unknown) => {
          void vscode.window.showErrorMessage(`Could not write the diagram layout: ${errorMessage(e)}`);
        });
        break;
      case "reveal":
        await this.reveal(m.range, m.open);
        break;
      case "notice": {
        const show = { error: vscode.window.showErrorMessage, warning: vscode.window.showWarningMessage, info: vscode.window.showInformationMessage }[m.kind];
        void show(m.message);
        break;
      }
    }
  }

  /** Makes a diagram edit in the document. Returns the id of the element it adds or renames. */
  async edit(edit: Parameters<Engine["editMetamodel"]>[2]): Promise<{ element?: string }> {
    // The document may change while the engine computes the edits; then they are computed again.
    for (let attempt = 0; attempt < 3; attempt++) {
      const base = this.document.getText();
      const result = await this.engine.editMetamodel(this.name, base, edit);
      if (this.document.getText() !== base) continue;
      if (result.edits.length === 0) return { element: result.element };
      const workspaceEdit = new vscode.WorkspaceEdit();
      for (const e of result.edits) workspaceEdit.replace(this.document.uri, toVsRange(this.document, e.range), e.text);
      if (!(await vscode.workspace.applyEdit(workspaceEdit))) throw new Error("The document could not be changed.");
      // Draw the result right away rather than after the usual delay.
      this.scheduleRefresh(0);
      return { element: result.element };
    }
    throw new Error("The document kept changing; the edit was not made.");
  }

  /** Selects a range in the text editors showing the document (`open`: opens one beside if none does). */
  private async reveal(range: Parameters<typeof toVsRange>[1], open: boolean): Promise<void> {
    const selection = toVsRange(this.document, range);
    const editors = vscode.window.visibleTextEditors.filter((e) => e.document === this.document);
    for (const editor of editors) {
      editor.selection = new vscode.Selection(selection.start, selection.start);
      editor.revealRange(selection, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
    }
    if (open && editors.length === 0) {
      await vscode.window.showTextDocument(this.document, { viewColumn: vscode.ViewColumn.Beside, selection: new vscode.Range(selection.start, selection.start) });
    }
  }

  private html(): string {
    const webview = this.panel.webview;
    const asset = (...p: string[]) => webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, ...p));
    const nonce = [...Array(32)].map(() => Math.floor(Math.random() * 36).toString(36)).join("");
    return /* html */ `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}'; font-src ${webview.cspSource}; img-src ${webview.cspSource} data:;">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="${asset("out", "webview", "diagram.css")}">
<link rel="stylesheet" href="${asset("media", "diagram.css")}">
<title>Metamodel Diagram</title>
</head>
<body>
<div id="app"></div>
<script nonce="${nonce}" src="${asset("out", "webview", "diagram.js")}"></script>
</body>
</html>`;
  }
}

/** Opens the diagram of a metamodel file beside its text editor (or the text beside the diagram). */
export async function openDiagram(uri: vscode.Uri | undefined, beside = true): Promise<void> {
  uri ??= vscode.window.activeTextEditor?.document.uri;
  if (!uri) return;
  await vscode.commands.executeCommand("vscode.openWith", uri, MetamodelDiagramEditor.viewType, beside ? vscode.ViewColumn.Beside : vscode.ViewColumn.Active);
}

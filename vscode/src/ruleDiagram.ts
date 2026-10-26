// The rule diagram: a custom editor for Henshin documents — texts (`.henshin_text`) and
// Henshin's models (`.henshin`) — that draws their rules in Henshin's notation (nodes,
// edges and attributes with their actions «preserve», «create», «delete», «forbid» and
// «require» in Henshin's colors, multi-rule elements marked with `*`, the condition graphs of
// matching formulas with `#graph`) and their units (webview/rules.tsx), and edits them. Like
// the metamodel diagram it works on the document's TextDocument: an edit in the diagram becomes
// text edits of the document (computed by the engine, `editTransformation`), and every change
// of the document — from any editor — draws the diagram again (`transformationDiagram`). A
// `.henshin` file is edited through its textual form and written back by the engine.
//
// Where the elements are drawn is kept in a layout file next to the document,
// `<file>.diagram.json`, as for metamodel diagrams.

import * as path from "node:path";
import * as vscode from "vscode";
import { layoutUri, parseLayout, printLayout } from "./diagram";
import { documentName, fromVsPosition, toVsRange } from "./documents";
import { errorMessage, type Engine, type TransformationDiagram, type TransformationEdit } from "./engine";
import { counterpart, type HenshinPairs } from "./henshinPairs";
import type { FromRuleWebview, Layout, PairInfo, ToRuleWebview } from "./ruleDiagramProtocol";

export class RuleDiagramEditor implements vscode.CustomTextEditorProvider {
  /** The editor for Henshin texts (opened on request). */
  static readonly viewType = "oxychloride.ruleDiagram";
  /** The editor for `.henshin` files (their default editor). */
  static readonly modelViewType = "oxychloride.henshinDiagram";
  private static readonly views = new Set<RuleDiagramView>();

  static register(context: vscode.ExtensionContext, engine: Engine, pairs: HenshinPairs, ready: Promise<void>): vscode.Disposable {
    const provider = new RuleDiagramEditor(engine, context.extensionUri, pairs, ready);
    const options = { webviewOptions: { retainContextWhenHidden: true }, supportsMultipleEditorsPerDocument: true };
    return vscode.Disposable.from(
      vscode.window.registerCustomEditorProvider(RuleDiagramEditor.viewType, provider, options),
      vscode.window.registerCustomEditorProvider(RuleDiagramEditor.modelViewType, provider, options),
    );
  }

  /** The open rule diagrams (for tests and commands). */
  static all(): RuleDiagramView[] {
    return [...RuleDiagramEditor.views];
  }

  private constructor(
    private readonly engine: Engine,
    private readonly extensionUri: vscode.Uri,
    private readonly pairs: HenshinPairs,
    private readonly ready: Promise<void>,
  ) {}

  resolveCustomTextEditor(document: vscode.TextDocument, panel: vscode.WebviewPanel): void {
    const view = new RuleDiagramView(this.engine, this.extensionUri, document, panel, this.pairs, this.ready);
    RuleDiagramEditor.views.add(view);
    panel.onDidDispose(() => RuleDiagramEditor.views.delete(view));
  }
}

export class RuleDiagramView {
  private readonly name: string;
  private readonly layoutFile: vscode.Uri | undefined;
  /** What this view last wrote to (or read from) the layout file. */
  private layoutText: string | undefined;
  private refreshTimer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private generation = 0;
  private diagram: TransformationDiagram | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly loaded: Promise<void>;
  private resolveLoaded: () => void = () => undefined;

  constructor(
    private readonly engine: Engine,
    private readonly extensionUri: vscode.Uri,
    readonly document: vscode.TextDocument,
    readonly panel: vscode.WebviewPanel,
    private readonly pairs: HenshinPairs,
    private readonly ready: Promise<void>,
  ) {
    this.name = documentName(document.uri);
    this.layoutFile = layoutUri(document.uri);
    this.loaded = new Promise((resolve) => (this.resolveLoaded = resolve));
    panel.webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(extensionUri, "out", "webview"), vscode.Uri.joinPath(extensionUri, "media")],
    };
    panel.iconPath = {
      light: vscode.Uri.joinPath(extensionUri, "media", "file-icons", "henshin.svg"),
      dark: vscode.Uri.joinPath(extensionUri, "media", "file-icons", "henshin-dark.svg"),
    };
    panel.webview.html = this.html();
    this.disposables.push(
      panel.webview.onDidReceiveMessage((m: FromRuleWebview) => void this.receive(m)),
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.document === document && e.contentChanges.length) this.scheduleRefresh(80);
      }),
      vscode.window.onDidChangeTextEditorSelection((e) => {
        if (e.textEditor.document !== document || e.kind === undefined || e.kind === vscode.TextEditorSelectionChangeKind.Command) return;
        this.post({ type: "cursor", position: fromVsPosition(document, e.selections[0].active) });
      }),
      pairs.onDidChangeState((uri) => {
        if (uri.toString() === document.uri.toString()) this.postPair();
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
      this.disposed = true;
      clearTimeout(this.refreshTimer);
      for (const d of this.disposables) d.dispose();
    });
    void this.start();
  }

  /** The diagram last drawn (for tests). */
  get current(): TransformationDiagram | undefined {
    return this.diagram;
  }

  private post(message: ToRuleWebview): void {
    // Answers from the engine may arrive after the panel was closed.
    if (!this.disposed) void this.panel.webview.postMessage(message);
  }

  private postPair(): void {
    const state = this.pairs.state(this.document.uri);
    const pair: PairInfo =
      state.kind === "none"
        ? {}
        : state.kind === "inSync"
          ? { partner: state.partner, inSync: true }
          : { partner: state.partner, inSync: false, reason: state.reason };
    this.post({ type: "pair", pair });
  }

  private async start(): Promise<void> {
    await this.loaded;
    const layout = (await this.readLayout()) ?? {};
    this.post({ type: "init", name: path.basename(this.document.uri.path), layout, persistent: this.layoutFile !== undefined });
    await this.ready;
    await this.refresh();
    this.postPair();
  }

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
      const diagram = await this.engine.transformationDiagram(this.name, this.document.getText());
      if (generation !== this.generation) return;
      this.diagram = diagram;
      this.post({ type: "diagram", diagram });
    } catch (e) {
      if (generation === this.generation) this.post({ type: "unreadable", message: errorMessage(e) });
    }
  }

  private async receive(m: FromRuleWebview): Promise<void> {
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
      case "command":
        await this.command(m.command);
        break;
    }
  }

  private async command(command: "openText" | "transform" | "explore" | "createCounterpart"): Promise<void> {
    switch (command) {
      case "openText":
        await vscode.window.showTextDocument(this.document, { viewColumn: vscode.ViewColumn.Beside });
        break;
      case "transform":
        await vscode.commands.executeCommand("oxychloride.openTransformation", this.document.uri);
        break;
      case "explore":
        await vscode.commands.executeCommand("oxychloride.exploreStateSpace", this.document.uri);
        break;
      case "createCounterpart":
        await vscode.commands.executeCommand("oxychloride.createHenshinCounterpart", this.document.uri);
        break;
    }
  }

  /** Makes a diagram edit in the document. Returns the id of the element it adds or renames. */
  async edit(edit: TransformationEdit): Promise<{ element?: string }> {
    for (let attempt = 0; attempt < 3; attempt++) {
      const base = this.document.getText();
      const result = await this.engine.editTransformation(this.name, base, edit);
      if (this.document.getText() !== base) continue;
      if (result.edits.length === 0) return { element: result.element };
      const workspaceEdit = new vscode.WorkspaceEdit();
      for (const e of result.edits) workspaceEdit.replace(this.document.uri, toVsRange(this.document, e.range), e.text);
      if (!(await vscode.workspace.applyEdit(workspaceEdit))) throw new Error("The document could not be changed.");
      this.scheduleRefresh(0);
      return { element: result.element };
    }
    throw new Error("The document kept changing; the edit was not made.");
  }

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
<link rel="stylesheet" href="${asset("out", "webview", "rules.css")}">
<link rel="stylesheet" href="${asset("media", "diagram.css")}">
<link rel="stylesheet" href="${asset("media", "rules.css")}">
<title>Rule Diagram</title>
</head>
<body>
<div id="app"></div>
<script nonce="${nonce}" src="${asset("out", "webview", "rules.js")}"></script>
</body>
</html>`;
  }
}

/** Opens the rule diagram of a Henshin document beside its text editor (or in place). */
export async function openRuleDiagram(uri: vscode.Uri | undefined, beside = true): Promise<void> {
  uri ??= vscode.window.activeTextEditor?.document.uri;
  if (!uri) return;
  const viewType = /\.henshin$/i.test(uri.path) ? RuleDiagramEditor.modelViewType : RuleDiagramEditor.viewType;
  await vscode.commands.executeCommand("vscode.openWith", uri, viewType, beside ? vscode.ViewColumn.Beside : vscode.ViewColumn.Active);
}

/** The document of the other syntax (`x.henshin` for `x.henshin_text`), for creating it. */
export { counterpart };

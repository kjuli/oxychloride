// The transformation view: a webview panel that applies the rules and units of a Henshin
// document (`.henshin_text` or `.henshin`) to a model step by step, showing what each step changed, and
// explores the state space of a set of rules. The engine keeps the transformation session; the
// model itself is not changed (states can be exported as HUTN or XMI).

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName, reveal, uriOf } from "./documents";
import { errorMessage, type Engine, type ModelResourceInfo, type TransformationState } from "./engine";
import type { FromWebview, Tab, ToWebview } from "./transformationProtocol";

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? p;

export class TransformationPanel {
  static readonly viewType = "oxychloride.transformation";
  private static readonly panels = new Map<string, TransformationPanel>();

  private session: number | undefined;
  private step = 0;
  private state: TransformationState | undefined;
  private refreshTimer: ReturnType<typeof setTimeout> | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  /** Resolves once the webview has loaded its script. */
  private readonly loaded: Promise<void>;
  private resolveLoaded: () => void = () => undefined;

  /** Opens (or reveals) the view for a Henshin document and a model resource. */
  static async open(
    engine: Engine,
    extensionUri: vscode.Uri,
    document: string,
    model: string,
    tab: Tab = "steps",
  ): Promise<TransformationPanel> {
    const key = `${document}\n${model}`;
    const existing = TransformationPanel.panels.get(key);
    if (existing) {
      existing.panel.reveal();
      existing.post({ type: "tab", tab });
      return existing;
    }
    const panel = vscode.window.createWebviewPanel(
      TransformationPanel.viewType,
      `${baseName(document)} ▸ ${baseName(model)}`,
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: false },
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "out", "webview"), vscode.Uri.joinPath(extensionUri, "media")],
      },
    );
    panel.iconPath = {
      light: vscode.Uri.joinPath(extensionUri, "media", "file-icons", "henshin.svg"),
      dark: vscode.Uri.joinPath(extensionUri, "media", "file-icons", "henshin-dark.svg"),
    };
    const view = new TransformationPanel(engine, extensionUri, panel, document, model, tab);
    TransformationPanel.panels.set(key, view);
    panel.onDidDispose(() => TransformationPanel.panels.delete(key));
    await view.start();
    return view;
  }

  /** All open views (e.g. to refresh them when the workspace changed). */
  static all(): TransformationPanel[] {
    return [...TransformationPanel.panels.values()];
  }

  private constructor(
    private readonly engine: Engine,
    private readonly extensionUri: vscode.Uri,
    readonly panel: vscode.WebviewPanel,
    readonly document: string,
    readonly model: string,
    private readonly tab: Tab,
  ) {
    this.loaded = new Promise((resolve) => (this.resolveLoaded = resolve));
    panel.webview.html = this.html();
    this.disposables.push(
      panel.webview.onDidReceiveMessage((m: FromWebview) => void this.receive(m)),
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (documentName(e.document.uri) === this.document) this.scheduleRefresh();
      }),
    );
    panel.onDidDispose(() => {
      for (const d of this.disposables) d.dispose();
      if (this.session !== undefined) void this.engine.closeTransformation(this.session).catch(() => undefined);
    });
  }

  private post(message: ToWebview): void {
    void this.panel.webview.postMessage(message);
  }

  private notice(kind: "error" | "warning" | "info", message: string): void {
    this.post({ type: "notice", kind, message });
  }

  /** Runs an engine request, showing progress and errors in the view. */
  private async busy<T>(what: string, run: () => Promise<T>): Promise<T | undefined> {
    this.post({ type: "busy", busy: true, what });
    try {
      return await run();
    } catch (e) {
      this.notice("error", errorMessage(e));
      return undefined;
    } finally {
      this.post({ type: "busy", busy: false });
    }
  }

  private async start(): Promise<void> {
    await this.loaded;
    this.post({ type: "init", document: this.document, model: this.model, tab: this.tab });
    await this.refreshUnits();
    await this.busy("Starting", async () => {
      const state = await this.engine.startTransformation(this.model);
      this.session = state.session;
      this.showState(state);
    });
  }

  private showState(state: TransformationState): void {
    this.state = state;
    this.step = state.step;
    this.post({ type: "state", state });
  }

  private scheduleRefresh(): void {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => void this.refreshUnits(), 400);
  }

  /** Reloads the rules and units (after the document changed). */
  async refreshUnits(): Promise<void> {
    try {
      const units = await this.engine.transformationUnits(this.document);
      const errors = units.diagnostics.filter((d) => d.severity === "error").length;
      this.post({ type: "units", units: units.units, errors });
    } catch (e) {
      this.notice("error", errorMessage(e));
    }
  }

  /** Shows the current step again (e.g. after the metamodel changed). If the language server
   * restarted meanwhile, the session is gone: the transformation starts again. */
  async refresh(): Promise<void> {
    await this.refreshUnits();
    if (this.session === undefined) return;
    try {
      this.showState(await this.engine.transformationState(this.session, this.step));
    } catch (e) {
      if (!/no transformation session/.test(errorMessage(e))) return;
      this.session = undefined;
      const state = await this.engine.startTransformation(this.model).catch(() => undefined);
      if (!state) return;
      this.session = state.session;
      this.showState(state);
      this.notice("info", "The language server was restarted: the transformation starts again from the model.");
    }
  }

  private async receive(m: FromWebview): Promise<void> {
    if (m.type === "ready") {
      this.resolveLoaded();
      return;
    }
    const session = this.session;
    if (session === undefined && m.type !== "restart" && m.type !== "reveal") return;
    switch (m.type) {
      case "findMatches": {
        const list = await this.busy("Matching", () =>
          this.engine.findMatches(session!, this.document, m.rule, m.parameters, this.step, 200),
        );
        if (list) this.post({ type: "matches", rule: m.rule, list });
        break;
      }
      case "apply": {
        const result = await this.busy("Applying", () =>
          this.engine.applyTransformation(session!, this.document, m.unit, m.parameters, { match: m.match, step: this.step }),
        );
        if (!result) break;
        this.showState(result.state);
        for (const message of result.messages) this.notice(result.applied ? "warning" : "info", message);
        break;
      }
      case "selectStep": {
        const state = await this.busy("Loading", () => this.engine.transformationState(session!, m.step));
        if (state) this.showState(state);
        break;
      }
      case "explore": {
        const space = await this.busy("Exploring", () =>
          this.engine.exploreStateSpace(session!, this.document, m.rules, {
            parameters: m.parameters,
            step: this.step,
            maxStates: m.maxStates,
            maxDepth: m.maxDepth,
            validate: m.validate,
          }),
        );
        if (!space) break;
        this.post({ type: "space", space });
        for (const message of space.messages) this.notice("warning", message);
        break;
      }
      case "selectState": {
        const details = await this.busy("Loading", () => this.engine.stateSpaceState(session!, m.state));
        if (details) this.post({ type: "spaceState", details });
        break;
      }
      case "adoptState": {
        const state = await this.busy("Loading", () => this.engine.adoptState(session!, m.state));
        if (state) {
          this.showState(state);
          this.post({ type: "tab", tab: "steps" });
        }
        break;
      }
      case "export":
        await this.export(session!, m.format);
        break;
      case "restart":
        if (session !== undefined) await this.engine.closeTransformation(session).catch(() => undefined);
        this.session = undefined;
        await this.busy("Starting", async () => {
          const state = await this.engine.startTransformation(this.model);
          this.session = state.session;
          this.showState(state);
        });
        break;
      case "reveal": {
        const units = await this.engine.transformationUnits(this.document).catch(() => undefined);
        const unit = units?.units.find((u) => u.name === m.unit);
        if (unit) await reveal(this.document, unit.range);
        break;
      }
    }
  }

  private async export(session: number, format: "hutn" | "xmi"): Promise<void> {
    const source = uriOf(this.model);
    const stem = baseName(this.model).replace(/\.[^.]*$/, "");
    const folder = source.scheme === "file" ? vscode.Uri.joinPath(source, "..") : vscode.workspace.workspaceFolders?.[0]?.uri;
    const target = await vscode.window.showSaveDialog({
      defaultUri: folder && vscode.Uri.joinPath(folder, `${stem}-step${this.step}.${format}`),
      filters: { [format === "hutn" ? "HUTN models" : "XMI models"]: [format] },
      title: `Export step ${this.step} as ${format.toUpperCase()}`,
    });
    if (!target) return;
    try {
      const { text } = await this.engine.exportTransformation(session, format, this.step, target.fsPath);
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
      const choice = await vscode.window.showInformationMessage(`Exported ${path.basename(target.fsPath)}.`, "Open");
      if (choice) await vscode.window.showTextDocument(target);
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
    }
  }

  /** The current state (for tests and other extensions). */
  get current(): TransformationState | undefined {
    return this.state;
  }

  private html(): string {
    const webview = this.panel.webview;
    const script = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "out", "webview", "transformation.js"));
    const style = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", "transformation.css"));
    const nonce = [...Array(32)].map(() => Math.floor(Math.random() * 36).toString(36)).join("");
    return /* html */ `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}'; font-src ${webview.cspSource};">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="${style}">
<title>Transformation</title>
</head>
<body>
<div id="app"></div>
<script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }
}

/** Lets the user pick the model resource to transform. */
export async function chooseModel(models: ModelResourceInfo[]): Promise<string | undefined> {
  const candidates = models.filter((m) => m.roots.length > 0);
  if (candidates.length === 0) {
    void vscode.window.showInformationMessage("No model is open: open a .hutn or .xmi model first.");
    return undefined;
  }
  if (candidates.length === 1) return candidates[0].uri;
  const pick = await vscode.window.showQuickPick(
    candidates.map((m) => ({ label: baseName(m.uri), description: vscode.workspace.asRelativePath(m.uri), uri: m.uri })),
    { title: "Transform which model?" },
  );
  return pick?.uri;
}

/** Lets the user pick a Henshin document (open ones and those of the workspace). */
export async function chooseTransformation(): Promise<vscode.Uri | undefined> {
  const isHenshin = (d: vscode.TextDocument) => d.languageId === "henshin" || /\.henshin$/i.test(d.uri.path);
  const active = vscode.window.activeTextEditor?.document;
  if (active && isHenshin(active)) return active.uri;
  const open = vscode.workspace.textDocuments.filter(isHenshin).map((d) => d.uri);
  const files = await vscode.workspace.findFiles("**/*.{henshin_text,henshin}", "**/node_modules/**", 100);
  const all = [...open, ...files.filter((f) => !open.some((o) => o.toString() === f.toString()))];
  if (all.length === 0) {
    void vscode.window.showInformationMessage("No Henshin transformation (.henshin_text or .henshin) found in the workspace.");
    return undefined;
  }
  if (all.length === 1) return all[0];
  const pick = await vscode.window.showQuickPick(
    all.map((uri) => ({ label: path.basename(uri.path), description: vscode.workspace.asRelativePath(uri), uri })),
    { title: "Transform with which rules?" },
  );
  return pick?.uri;
}

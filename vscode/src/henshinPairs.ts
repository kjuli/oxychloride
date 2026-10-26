// Keeps a pair of Henshin documents equivalent: a text (`rules.henshin_text`) and the `.henshin`
// file of the same name in the same folder (Henshin's model, which its graphical editors
// write). When one of them changes (in a text editor, a rule diagram, or on disk), the engine
// computes the edits that make the other mean the same (`syncTransformation`: the `.henshin`
// file is written again, keeping its ids; in the text only the changed rules and units are
// written anew, the rest keeps its layout and comments), and they are applied to the other
// document. Saving one saves the other if the synchronization changed it.
//
// While a document has errors it is not synchronized; the status bar says so.

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName, toVsRange } from "./documents";
import { errorMessage, type Engine } from "./engine";

/** The other document of a pair: `x.henshin` for `x.henshin_text` and the other way round. */
export function counterpart(uri: vscode.Uri): vscode.Uri | undefined {
  if (/\.henshin_text$/i.test(uri.path)) return uri.with({ path: uri.path.replace(/\.henshin_text$/i, ".henshin") });
  if (/\.henshin$/i.test(uri.path)) return uri.with({ path: uri.path.replace(/\.henshin$/i, ".henshin_text") });
  return undefined;
}

export const isTransformation = (uri: vscode.Uri) => /\.henshin(_text)?$/i.test(uri.path);

/** How a document stands to its counterpart. */
export type PairState =
  | { kind: "none" }
  | { kind: "inSync"; partner: string }
  | { kind: "outOfSync"; partner: string; reason: string };

export class HenshinPairs implements vscode.Disposable {
  private readonly disposables: vscode.Disposable[] = [];
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  /** The texts this class gave documents: their change events are not synchronized back. */
  private readonly written = new Map<string, string>();
  /** Documents the synchronization changed (they are saved with their counterpart). */
  private readonly changedBySync = new Set<string>();
  private readonly states = new Map<string, PairState>();
  private readonly status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
  private readonly stateChanged = new vscode.EventEmitter<vscode.Uri>();
  /** Fires when the pair state of a document changes. */
  readonly onDidChangeState = this.stateChanged.event;
  /** Resolves when the synchronizations started so far are done (for tests). */
  private pending: Promise<void> = Promise.resolve();

  constructor(private readonly engine: Engine, private readonly ready: Promise<void>) {
    this.status.name = "Henshin";
    this.disposables.push(
      this.status,
      this.stateChanged,
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.contentChanges.length && this.enabled() && isTransformation(e.document.uri)) this.schedule(e.document);
      }),
      vscode.workspace.onDidSaveTextDocument((doc) => void this.saved(doc)),
      vscode.window.onDidChangeActiveTextEditor(() => this.showStatus()),
    );
    // Files changed on disk (e.g. by Henshin's editors in Eclipse) that are not open.
    const watcher = vscode.workspace.createFileSystemWatcher("**/*.{henshin,henshin_text}");
    this.disposables.push(watcher, watcher.onDidChange((uri) => void this.fileChanged(uri)));
  }

  dispose(): void {
    for (const t of this.timers.values()) clearTimeout(t);
    for (const d of this.disposables) d.dispose();
  }

  private enabled(): boolean {
    return vscode.workspace.getConfiguration("oxychloride").get<boolean>("henshin.syncPairs", true);
  }

  /** The pair state of a document. */
  state(uri: vscode.Uri): PairState {
    return this.states.get(uri.toString()) ?? { kind: "none" };
  }

  /** Waits for the synchronizations under way. */
  async settled(): Promise<void> {
    for (;;) {
      const current = this.pending;
      await current;
      await new Promise((r) => setTimeout(r, 0));
      if (current === this.pending && this.timers.size === 0) return;
      if (this.timers.size) await new Promise((r) => setTimeout(r, 50));
    }
  }

  private schedule(doc: vscode.TextDocument): void {
    const key = doc.uri.toString();
    // A change this class made: nothing to synchronize back.
    if (this.written.get(key) === doc.getText()) {
      this.written.delete(key);
      return;
    }
    clearTimeout(this.timers.get(key));
    this.timers.set(
      key,
      setTimeout(() => {
        this.timers.delete(key);
        this.pending = this.pending.then(() => this.sync(doc));
      }, 300),
    );
  }

  /** Makes the counterpart of `source` mean what `source` means (if it exists). */
  async sync(source: vscode.TextDocument): Promise<void> {
    const target = counterpart(source.uri);
    if (!target || source.uri.scheme !== "file" || !(await exists(target))) {
      this.setState(source.uri, { kind: "none" });
      return;
    }
    await this.ready;
    const partner = path.basename(target.path);
    const open = vscode.workspace.textDocuments.find((d) => d.uri.toString() === target.toString());
    const targetText = open ? open.getText() : new TextDecoder().decode(await vscode.workspace.fs.readFile(target));
    let edits: { range: { start: { line: number; column: number }; end: { line: number; column: number } }; text: string }[];
    try {
      ({ edits } = await this.engine.syncTransformation(documentName(source.uri), source.getText(), documentName(target), targetText));
    } catch (e) {
      const reason = errorMessage(e);
      this.setState(source.uri, { kind: "outOfSync", partner, reason });
      this.setState(target, { kind: "outOfSync", partner: path.basename(source.uri.path), reason });
      return;
    }
    this.setState(source.uri, { kind: "inSync", partner });
    this.setState(target, { kind: "inSync", partner: path.basename(source.uri.path) });
    if (edits.length === 0) return;
    const doc = open ?? (await vscode.workspace.openTextDocument(target));
    if (doc.getText() !== targetText) return; // Changed meanwhile: its own change synchronizes.
    const workspaceEdit = new vscode.WorkspaceEdit();
    for (const e of edits) workspaceEdit.replace(target, toVsRange(doc, e.range), e.text);
    // The text the edits make, so that its change event is not synchronized back.
    const expected = applyToText(targetText, doc, edits);
    this.written.set(target.toString(), expected);
    if (!(await vscode.workspace.applyEdit(workspaceEdit))) {
      this.written.delete(target.toString());
      return;
    }
    this.changedBySync.add(target.toString());
    // A counterpart that is not open in an editor is saved right away; an open one is saved
    // with its counterpart.
    if (!vscode.window.visibleTextEditors.some((e) => e.document === doc) && !source.isDirty) await doc.save();
  }

  /** Saving a document saves what the synchronization changed in its counterpart. */
  private async saved(doc: vscode.TextDocument): Promise<void> {
    const target = counterpart(doc.uri);
    if (!target) return;
    await this.pending;
    const key = target.toString();
    const partner = vscode.workspace.textDocuments.find((d) => d.uri.toString() === key);
    if (partner?.isDirty && this.changedBySync.has(key)) {
      this.changedBySync.delete(key);
      await partner.save();
    }
  }

  /** A file changed on disk: if it is not open, its counterpart follows. */
  private async fileChanged(uri: vscode.Uri): Promise<void> {
    if (!this.enabled() || vscode.workspace.textDocuments.some((d) => d.uri.toString() === uri.toString())) return;
    const doc = await vscode.workspace.openTextDocument(uri);
    this.schedule(doc);
  }

  private setState(uri: vscode.Uri, state: PairState): void {
    const key = uri.toString();
    if (JSON.stringify(this.states.get(key)) === JSON.stringify(state)) return;
    this.states.set(key, state);
    this.stateChanged.fire(uri);
    this.showStatus();
  }

  private showStatus(): void {
    const uri = vscode.window.activeTextEditor?.document.uri;
    const state = uri && this.state(uri);
    if (!state || state.kind === "none") {
      this.status.hide();
      return;
    }
    if (state.kind === "inSync") {
      this.status.text = `$(sync) ${state.partner}`;
      this.status.tooltip = `Kept equivalent to ${state.partner}`;
    } else {
      this.status.text = `$(sync-ignored) ${state.partner}`;
      this.status.tooltip = `${state.partner} is not updated: ${state.reason}`;
    }
    this.status.show();
  }
}

async function exists(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}

/** The text that `edits` (engine positions) make of `text` (the text of `doc`). */
function applyToText(text: string, doc: vscode.TextDocument, edits: { range: Parameters<typeof toVsRange>[1]; text: string }[]): string {
  const offsets = edits.map((e) => {
    const r = toVsRange(doc, e.range);
    return { start: doc.offsetAt(r.start), end: doc.offsetAt(r.end), text: e.text };
  });
  let out = text;
  for (const e of offsets.sort((a, b) => b.start - a.start)) out = out.slice(0, e.start) + e.text + out.slice(e.end);
  return out;
}

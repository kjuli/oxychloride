// The OCL console: a webview in the panel that evaluates expressions, optionally with a model
// object as `self`, and shows values, types, problems and the pre-/postconditions violated by
// operation calls the expression makes.

import * as vscode from "vscode";
import { errorMessage, type Engine, type EvaluationResult, type ObjectRef } from "./engine";

export interface ConsoleContext extends ObjectRef {
  label: string;
  class: string;
}

interface Entry {
  id: number;
  expression: string;
  context?: string;
  result?: EvaluationResult;
  error?: string;
}

type FromWebview =
  | { type: "ready" }
  | { type: "evaluate"; expression: string }
  | { type: "chooseContext" }
  | { type: "clearContext" }
  | { type: "clear" };

export class OclConsole implements vscode.WebviewViewProvider {
  static readonly viewId = "oxychloride.console";
  private view: vscode.WebviewView | undefined;
  private entries: Entry[] = [];
  private nextId = 0;
  private context: ConsoleContext | undefined;
  private readonly contextChanged = new vscode.EventEmitter<ConsoleContext | undefined>();
  readonly onDidChangeContext = this.contextChanged.event;

  constructor(
    private readonly engine: Engine,
    private readonly chooseContext: () => Promise<ConsoleContext | undefined>,
  ) {}

  resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    view.webview.options = { enableScripts: true };
    view.webview.html = html(view.webview);
    view.webview.onDidReceiveMessage((m: FromWebview) => void this.receive(m));
    view.onDidDispose(() => (this.view = undefined));
  }

  private post(message: object): void {
    void this.view?.webview.postMessage(message);
  }

  private async receive(m: FromWebview): Promise<void> {
    switch (m.type) {
      case "ready":
        this.post({ type: "context", label: this.contextLabel() });
        for (const e of this.entries) this.post({ type: "entry", entry: e });
        break;
      case "evaluate":
        await this.evaluate(m.expression);
        break;
      case "chooseContext": {
        const c = await this.chooseContext();
        if (c) this.setContext(c);
        break;
      }
      case "clearContext":
        this.setContext(undefined);
        break;
      case "clear":
        this.entries = [];
        this.post({ type: "clear" });
        break;
    }
  }

  private contextLabel(): string | undefined {
    return this.context && `${this.context.label} : ${this.context.class}`;
  }

  get currentContext(): ConsoleContext | undefined {
    return this.context;
  }

  setContext(context: ConsoleContext | undefined): void {
    this.context = context;
    this.post({ type: "context", label: this.contextLabel() });
    this.contextChanged.fire(context);
  }

  /** Shows the console (and focuses its input). */
  async reveal(): Promise<void> {
    await vscode.commands.executeCommand(`${OclConsole.viewId}.focus`);
    this.post({ type: "focus" });
  }

  /** Evaluates `expression` and appends the outcome to the console. */
  async evaluate(expression: string): Promise<EvaluationResult | undefined> {
    const context = this.context;
    const entry: Entry = { id: this.nextId++, expression, context: this.contextLabel() };
    this.entries.push(entry);
    this.post({ type: "entry", entry });
    try {
      const ref = context && { resource: context.resource, fragment: context.fragment };
      entry.result = await this.engine.evaluate(expression, ref);
    } catch (e) {
      entry.error = errorMessage(e);
    }
    this.post({ type: "entry", entry });
    return entry.result;
  }
}

function nonce(): string {
  let s = "";
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  for (let i = 0; i < 32; i++) s += chars.charAt(Math.floor(Math.random() * chars.length));
  return s;
}

function html(webview: vscode.Webview): string {
  const n = nonce();
  return /* html */ `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'unsafe-inline'; script-src 'nonce-${n}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<style>
  html, body { height: 100%; margin: 0; padding: 0; }
  body {
    display: flex; flex-direction: column;
    color: var(--vscode-foreground); background: var(--vscode-panel-background);
    font-family: var(--vscode-font-family); font-size: var(--vscode-font-size);
  }
  .bar { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border-bottom: 1px solid var(--vscode-panel-border); }
  .bar .self { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--vscode-descriptionForeground); }
  .bar .self code { color: var(--vscode-foreground); }
  button {
    border: none; padding: 2px 8px; border-radius: 2px; cursor: pointer; font: inherit;
    color: var(--vscode-button-secondaryForeground); background: var(--vscode-button-secondaryBackground);
  }
  button:hover { background: var(--vscode-button-secondaryHoverBackground); }
  #log { flex: 1; overflow-y: auto; padding: 4px 8px; font-family: var(--vscode-editor-font-family); font-size: var(--vscode-editor-font-size); }
  .entry { padding: 4px 0; border-bottom: 1px solid var(--vscode-panel-border); }
  .expr { white-space: pre-wrap; }
  .expr::before { content: "› "; color: var(--vscode-descriptionForeground); }
  .ctx { color: var(--vscode-descriptionForeground); font-size: 0.9em; margin-left: 1.2em; }
  .value { white-space: pre-wrap; margin: 2px 0 0 1.2em; }
  .type { color: var(--vscode-descriptionForeground); margin-left: 0.8em; }
  .invalid, .error { color: var(--vscode-errorForeground); }
  .warning { color: var(--vscode-editorWarning-foreground); }
  .pending { color: var(--vscode-descriptionForeground); margin-left: 1.2em; }
  .problem, .contract { margin: 2px 0 0 1.2em; white-space: pre-wrap; }
  .input { display: flex; gap: 6px; padding: 6px 8px; border-top: 1px solid var(--vscode-panel-border); }
  textarea {
    flex: 1; resize: none; min-height: 1.5em; max-height: 10em; padding: 3px 6px;
    font-family: var(--vscode-editor-font-family); font-size: var(--vscode-editor-font-size);
    color: var(--vscode-input-foreground); background: var(--vscode-input-background);
    border: 1px solid var(--vscode-input-border, transparent); border-radius: 2px; outline: none;
  }
  textarea:focus { border-color: var(--vscode-focusBorder); }
  .empty { color: var(--vscode-descriptionForeground); padding: 8px 0; font-family: var(--vscode-font-family); }
</style>
</head>
<body>
  <div class="bar">
    <span class="self" id="self"></span>
    <button id="choose" title="Choose the object that is self">Context…</button>
    <button id="clearContext" title="Evaluate without self">No Context</button>
    <button id="clear" title="Clear the console">Clear</button>
  </div>
  <div id="log"><div class="empty" id="empty">Evaluate OCL expressions against the open models, e.g. <code>Student.allInstances()->select(semester > 2)</code>. Enter evaluates, Shift+Enter adds a line, ↑/↓ browse the history.</div></div>
  <div class="input">
    <textarea id="input" rows="1" placeholder="OCL expression" spellcheck="false" aria-label="OCL expression"></textarea>
  </div>
<script nonce="${n}">
  const vscode = acquireVsCodeApi();
  const log = document.getElementById("log");
  const input = document.getElementById("input");
  const self = document.getElementById("self");
  const history = (vscode.getState() || {}).history || [];
  let historyIndex = history.length;
  let draft = "";

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function render(entry) {
    const e = el("div", "entry");
    e.dataset.id = entry.id;
    e.append(el("div", "expr", entry.expression));
    if (entry.context) e.append(el("div", "ctx", "self = " + entry.context));
    if (entry.error) {
      e.append(el("div", "value error", entry.error));
    } else if (!entry.result) {
      e.append(el("div", "pending", "…"));
    } else {
      const r = entry.result;
      const v = el("div", "value" + (r.invalid ? " invalid" : ""), r.display);
      v.append(el("span", "type", r.type));
      e.append(v);
      for (const d of r.diagnostics || []) {
        e.append(el("div", "problem " + d.severity, d.severity + ": " + d.message));
      }
      for (const c of r.contracts || []) {
        const what = c.constraint.kind + " " + (c.constraint.name ? "'" + c.constraint.name + "' " : "") + "of " + c.constraint.context;
        e.append(el("div", "contract " + (c.status === "violated" ? "error" : "warning"),
          what + " " + (c.status === "violated" ? "violated" : "failed") + " by " + c.object.label + "." + c.call + (c.message ? ": " + c.message : "")));
      }
    }
    return e;
  }

  function show(entry) {
    document.getElementById("empty")?.remove();
    const old = log.querySelector('[data-id="' + entry.id + '"]');
    const e = render(entry);
    if (old) old.replaceWith(e); else log.append(e);
    log.scrollTop = log.scrollHeight;
  }

  function resize() {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + "px";
  }

  input.addEventListener("input", resize);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const expression = input.value.trim();
      if (!expression) return;
      if (history[history.length - 1] !== expression) history.push(expression);
      if (history.length > 100) history.shift();
      vscode.setState({ history });
      historyIndex = history.length;
      input.value = "";
      resize();
      vscode.postMessage({ type: "evaluate", expression });
    } else if (e.key === "ArrowUp" && input.selectionStart === 0 && historyIndex > 0) {
      e.preventDefault();
      if (historyIndex === history.length) draft = input.value;
      input.value = history[--historyIndex];
      resize();
    } else if (e.key === "ArrowDown" && input.selectionEnd === input.value.length && historyIndex < history.length) {
      e.preventDefault();
      historyIndex++;
      input.value = historyIndex === history.length ? draft : history[historyIndex];
      resize();
    }
  });
  document.getElementById("choose").addEventListener("click", () => vscode.postMessage({ type: "chooseContext" }));
  document.getElementById("clearContext").addEventListener("click", () => vscode.postMessage({ type: "clearContext" }));
  document.getElementById("clear").addEventListener("click", () => vscode.postMessage({ type: "clear" }));

  window.addEventListener("message", (event) => {
    const m = event.data;
    if (m.type === "entry") show(m.entry);
    else if (m.type === "clear") log.replaceChildren();
    else if (m.type === "focus") input.focus();
    else if (m.type === "context") {
      self.replaceChildren();
      if (m.label) { self.append("self = "); self.append(el("code", "", m.label)); }
      else self.append("No context object: expressions such as Class.allInstances() work without one");
      document.getElementById("clearContext").hidden = !m.label;
    }
  });
  vscode.postMessage({ type: "ready" });
  input.focus();
</script>
</body>
</html>`;
}

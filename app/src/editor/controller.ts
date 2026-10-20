// Owns the single CodeMirror view and one EditorState per open tab, and keeps the engine's
// copy of each document in sync (debounced), turning engine diagnostics into lint markers.
// Each tab has a language (Complete OCL, OCLinEcore or HUTN) that selects its highlighting
// and tells the engine how to read it.

import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab, toggleComment } from "@codemirror/commands";
import { bracketMatching, foldGutter, indentOnInput, indentUnit } from "@codemirror/language";
import { lintGutter, setDiagnostics, type Diagnostic as CmDiagnostic } from "@codemirror/lint";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorSelection, EditorState, type Text } from "@codemirror/state";
import {
  crosshairCursor,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  hoverTooltip,
  keymap,
  lineNumbers,
  rectangularSelection,
  type Tooltip,
} from "@codemirror/view";

import { rpc, type CompletionKind, type Diagnostic, type DocumentEdit, type Language, type Range } from "../api";
import { offsetToPosition, rangeToOffsets } from "../positions";
import { hutn } from "./hutn-language";
import { ocl } from "./ocl-language";
import { oclinecore } from "./oclinecore-language";
import { editorTheme } from "./theme";

function languageSupport(language: Language) {
  if (language === "xmi") return []; // XML: no OCL highlighting
  return language === "oclinecore" ? oclinecore() : language === "hutn" ? hutn() : ocl();
}

const SYNC_DELAY_MS = 300;

export interface ControllerCallbacks {
  /** The text of a document changed (called on every edit). */
  onEdit(id: string, text: string): void;
  /** The engine accepted a new version of a document and reported its diagnostics. */
  onSynced(id: string, diagnostics: Diagnostic[]): void;
  /** Syncing failed (e.g. the engine is unavailable). */
  onError(message: string): void;
}

const COMPLETION_TYPES: Record<CompletionKind, string> = {
  keyword: "keyword",
  type: "type",
  property: "property",
  operation: "method",
  iterator: "function",
  variable: "variable",
  enumLiteral: "enum",
  snippet: "text",
  object: "variable",
};

/** Converts engine diagnostics to CodeMirror diagnostics for `doc`. */
export function toCmDiagnostics(doc: Text, diagnostics: Diagnostic[]): CmDiagnostic[] {
  return diagnostics.map((d) => {
    let { from, to } = rangeToOffsets(doc, d.range);
    if (from === to) {
      // Give empty ranges a visible extent: the next character on the same line, if any.
      const line = doc.lineAt(from);
      if (to < line.to) to = from + 1;
      else if (from > line.from) from = from - 1;
    }
    return { from, to, severity: d.severity, message: d.message, source: d.source };
  });
}

export class EditorController {
  private view: EditorView | null = null;
  private readonly states = new Map<string, EditorState>();
  private readonly names = new Map<string, string>();
  private readonly languages = new Map<string, Language>();
  private readonly languageCompartment = new Compartment();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private activeId: string | null = null;
  private readonly callbacks: ControllerCallbacks;

  constructor(callbacks: ControllerCallbacks) {
    this.callbacks = callbacks;
  }

  // ---------------------------------------------------------------- view lifecycle

  attach(parent: HTMLElement): void {
    if (this.view) return;
    const initial = this.activeId ? this.states.get(this.activeId) : undefined;
    this.view = new EditorView({ parent, state: initial ?? EditorState.create({ extensions: editorTheme }) });
  }

  detach(): void {
    if (!this.view) return;
    if (this.activeId) this.states.set(this.activeId, this.view.state);
    this.view.destroy();
    this.view = null;
  }

  focus(): void {
    this.view?.focus();
  }

  // ---------------------------------------------------------------- documents

  /** Creates the editor state for a new tab (with the cursor at `cursor`) and sends the
   * document to the engine. */
  open(id: string, name: string, text: string, language: Language, cursor = 0): void {
    this.names.set(id, name);
    this.languages.set(id, language);
    const selection = { anchor: Math.min(cursor, text.length) };
    this.states.set(id, EditorState.create({ doc: text, selection, extensions: this.extensions(id, language) }));
    void this.syncNow(id);
  }

  language(id: string): Language {
    return this.languages.get(id) ?? "ocl";
  }

  /** Changes the language of a document (e.g. after Save As with another extension). */
  setLanguage(id: string, language: Language): void {
    if (this.languages.get(id) === language) return;
    this.languages.set(id, language);
    const state = this.state(id);
    if (!state) return;
    const spec = { effects: this.languageCompartment.reconfigure(languageSupport(language)) };
    if (id === this.activeId && this.view) this.view.dispatch(spec);
    else this.states.set(id, state.update(spec).state);
  }

  activate(id: string | null): void {
    if (id === this.activeId) return;
    if (this.view && this.activeId && this.states.has(this.activeId)) {
      this.states.set(this.activeId, this.view.state);
    }
    this.activeId = id;
    if (this.view && id) {
      const state = this.states.get(id);
      if (state) this.view.setState(state);
      this.view.requestMeasure();
    }
  }

  /** Forgets a tab and closes its engine document. */
  async close(id: string): Promise<void> {
    const name = this.names.get(id);
    this.cancelTimer(id);
    this.states.delete(id);
    this.names.delete(id);
    this.languages.delete(id);
    if (this.activeId === id) this.activeId = null;
    if (name) {
      try {
        await rpc.closeDocument(name);
      } catch (e) {
        this.callbacks.onError(String(e instanceof Error ? e.message : e));
      }
    }
  }

  /** Changes the engine name of a document (after Save As). */
  async rename(id: string, name: string): Promise<void> {
    const old = this.names.get(id);
    if (old === name) return;
    this.names.set(id, name);
    if (old) {
      try {
        await rpc.closeDocument(old);
      } catch {
        // The old name may already be gone; the new one is opened below.
      }
    }
    await this.syncNow(id);
  }

  name(id: string): string | undefined {
    return this.names.get(id);
  }

  text(id: string): string {
    return this.state(id)?.doc.toString() ?? "";
  }

  /** Re-sends every open document (e.g. after the workspace was reset). */
  async resyncAll(): Promise<void> {
    for (const id of [...this.names.keys()]) await this.syncNow(id);
  }

  /** Sends documents with pending edits right away. */
  async flush(): Promise<void> {
    const pending = [...this.timers.keys()];
    for (const id of pending) await this.syncNow(id);
  }

  /** Selects `range` in the active document `id` and scrolls it into view. */
  reveal(id: string, range: Range): void {
    if (!this.view || id !== this.activeId) return;
    const { from, to } = rangeToOffsets(this.view.state.doc, range);
    this.view.dispatch({
      selection: EditorSelection.range(from, to),
      effects: EditorView.scrollIntoView(from, { y: "center" }),
    });
    this.view.focus();
  }

  /** Applies `edit`, computed by the engine for the text `base`, to the active document `id`
   * as one undoable change, and puts the cursor at `edit.cursor`. Returns false (and changes
   * nothing) if the document is not active or its text is no longer `base`. */
  applyEdit(id: string, base: string, edit: DocumentEdit): boolean {
    if (!this.view || id !== this.activeId) return false;
    const { state } = this.view;
    if (state.doc.toString() !== base) return false;
    const changes = state.changes(
      edit.edits.map((e) => ({ ...rangeToOffsets(state.doc, e.range), insert: e.text })),
    );
    const cursor = rangeToOffsets(changes.apply(state.doc), { start: edit.cursor, end: edit.cursor }).from;
    this.view.dispatch({
      changes,
      selection: { anchor: cursor },
      effects: EditorView.scrollIntoView(cursor, { y: "center" }),
      userEvent: "input",
    });
    this.view.focus();
    return true;
  }

  // ---------------------------------------------------------------- syncing

  private state(id: string): EditorState | undefined {
    return id === this.activeId && this.view ? this.view.state : this.states.get(id);
  }

  private cancelTimer(id: string): void {
    const timer = this.timers.get(id);
    if (timer !== undefined) clearTimeout(timer);
    this.timers.delete(id);
  }

  private schedule(id: string): void {
    this.cancelTimer(id);
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        void this.syncNow(id);
      }, SYNC_DELAY_MS),
    );
  }

  async syncNow(id: string): Promise<void> {
    this.cancelTimer(id);
    const name = this.names.get(id);
    const state = this.state(id);
    if (!name || !state) return;
    const text = state.doc.toString();
    let diagnostics: Diagnostic[];
    try {
      ({ diagnostics } = await rpc.openDocument(name, text, this.languages.get(id)));
    } catch (e) {
      this.callbacks.onError(e instanceof Error ? e.message : String(e));
      return;
    }
    // Drop stale answers: the document changed (or was closed or renamed) meanwhile.
    const current = this.state(id);
    if (!current || this.names.get(id) !== name || current.doc.toString() !== text) return;
    this.applyDiagnostics(id, diagnostics);
    this.callbacks.onSynced(id, diagnostics);
  }

  private applyDiagnostics(id: string, diagnostics: Diagnostic[]): void {
    const state = this.state(id);
    if (!state) return;
    const spec = setDiagnostics(state, toCmDiagnostics(state.doc, diagnostics));
    if (id === this.activeId && this.view) this.view.dispatch(spec);
    else this.states.set(id, state.update(spec).state);
  }

  // ---------------------------------------------------------------- extensions

  private extensions(id: string, language: Language) {
    const nameOf = () => this.names.get(id) ?? "";
    const languageOf = () => this.languages.get(id) ?? "ocl";
    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      foldGutter(),
      lintGutter(),
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      indentUnit.of("  "),
      bracketMatching(),
      closeBrackets(),
      rectangularSelection(),
      crosshairCursor(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      this.languageCompartment.of(languageSupport(language)),
      autocompletion({ override: [completionSource(nameOf, languageOf)], activateOnTypingDelay: 120 }),
      hoverTooltip(hoverSource(nameOf), { hoverTime: 350 }),
      keymap.of([
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        { key: "Mod-/", run: toggleComment },
        indentWithTab,
      ]),
      editorTheme,
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return;
        this.callbacks.onEdit(id, update.state.doc.toString());
        this.schedule(id);
      }),
    ];
  }
}

function completionSource(nameOf: () => string, languageOf: () => Language) {
  return async (context: CompletionContext): Promise<CompletionResult | null> => {
    const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*/);
    const before = context.state.sliceDoc(Math.max(0, context.pos - 2), context.pos);
    const afterNavigation = before.endsWith(".") || before === "->" || before === "::";
    // In metamodels and models, values and types follow `: ` and `, `; references start with `"`.
    const valueStart = languageOf() !== "ocl" && (before === ": " || before === ", " || before.endsWith('"'));
    if (!context.explicit && !word && !afterNavigation && !valueStart) return null;

    const doc = context.state.doc;
    const list = await rpc
      .complete(nameOf(), offsetToPosition(doc, context.pos), doc.toString())
      .catch(() => null);
    if (!list || context.aborted || list.items.length === 0) return null;

    let { from, to } = rangeToOffsets(doc, list.range);
    if (from > context.pos || to < context.pos) {
      from = word ? word.from : context.pos;
      to = context.pos;
    }
    const options: Completion[] = list.items.map((item, index) => ({
      label: item.label,
      type: COMPLETION_TYPES[item.kind] ?? "text",
      detail: item.detail,
      apply: item.insert,
      // Keep the engine's ordering when the typed prefix does not decide.
      boost: -Math.min(99, Math.floor(index / 10)),
    }));
    return { from, to, options, filter: true };
  };
}

function hoverSource(nameOf: () => string) {
  return async (view: EditorView, pos: number): Promise<Tooltip | null> => {
    const doc = view.state.doc;
    const info = await rpc.hover(nameOf(), offsetToPosition(doc, pos), doc.toString()).catch(() => null);
    if (!info || !info.contents) return null;
    const { from, to } = rangeToOffsets(view.state.doc, info.range);
    return {
      pos: from,
      end: Math.max(from, to),
      above: true,
      create: () => {
        const dom = document.createElement("div");
        dom.className = "oxy-hover";
        dom.textContent = info.contents;
        return { dom };
      },
    };
  };
}

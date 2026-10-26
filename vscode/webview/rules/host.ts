// The rule diagram's connection to its host (src/ruleDiagram.ts), and the small store for the
// state many components read (focus, inline editing, the tool, the scope new elements go into),
// so that changing it only re-renders the components that use the part that changed.

import { useSyncExternalStore } from "react";
import type { RuleAction, TransformationEdit } from "../../src/engine";
import type { FromRuleWebview, ToRuleWebview } from "../../src/ruleDiagramProtocol";

declare function acquireVsCodeApi(): { postMessage(m: unknown): void; getState(): unknown; setState(s: unknown): void };
const vscode = acquireVsCodeApi();

export const post = (m: FromRuleWebview) => vscode.postMessage(m);

export function onMessage(handler: (m: ToRuleWebview) => void): () => void {
  const listener = (e: MessageEvent<ToRuleWebview>) => handler(e.data);
  window.addEventListener("message", listener);
  return () => window.removeEventListener("message", listener);
}

/** Per-view settings kept by VS Code while the view exists. */
export interface Saved {
  properties: boolean;
  action: RuleAction;
  class?: string;
}

export const saved: Saved = { properties: true, action: "preserve", ...(vscode.getState() as Partial<Saved> | undefined) };
export const save = () => vscode.setState(saved);

let nextRequest = 1;
const waiting = new Map<number, (r: { element?: string; error?: string }) => void>();

/** Asks the host to make an edit; resolves with the id of the element it added or renamed. */
export function edit(e: TransformationEdit): Promise<{ element?: string; error?: string }> {
  const request = nextRequest++;
  post({ type: "edit", request, edit: e });
  return new Promise((resolve) => waiting.set(request, resolve));
}

export function edited(request: number, result: { element?: string; error?: string }): void {
  waiting.get(request)?.(result);
  waiting.delete(request);
}

// ---------------------------------------------------------------- store

export type Tool =
  | { kind: "select" }
  /** Click into a rule to add a node (of the toolbar's class and action). */
  | { kind: "node" }
  /** Drag from a node to another to add an edge (of the toolbar's action). */
  | { kind: "edge" };

/** An inline editor. */
export type Editing =
  | { kind: "name"; element: string }
  | { kind: "attribute"; element: string }
  | { kind: "newAttribute"; node: string; seq: number };

export interface UiState {
  /** The element shown in the properties panel (and highlighted). */
  focused?: string;
  editing?: Editing;
  tool: Tool;
  /** The action of new nodes and edges. */
  action: RuleAction;
  /** The class of new nodes (its name as rules write it). */
  class?: string;
  /** The multi-rule or condition graph new elements of its rule go into. */
  scope?: string;
}

type Listener = () => void;

class Store<T> {
  private listeners = new Set<Listener>();
  constructor(private state: T) {}
  get = () => this.state;
  set = (patch: Partial<T>) => {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  };
  subscribe = (l: Listener) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };
}

export const ui = new Store<UiState>({ tool: { kind: "select" }, action: saved.action, class: saved.class });

export function useUi<S>(select: (s: UiState) => S): S {
  return useSyncExternalStore(ui.subscribe, () => select(ui.get()));
}

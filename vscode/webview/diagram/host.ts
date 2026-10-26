// The diagram's connection to its host (src/diagram.ts), and the small store for the state
// that many components read (selection, inline editing, the tool), so that changing it only
// re-renders the components that use the part that changed.

import { useSyncExternalStore } from "react";
import type { MetamodelEdit } from "../../src/engine";
import type { FromWebview, ToWebview } from "../../src/diagramProtocol";

declare function acquireVsCodeApi(): { postMessage(m: unknown): void; getState(): unknown; setState(s: unknown): void };
const vscode = acquireVsCodeApi();

export const post = (m: FromWebview) => vscode.postMessage(m);

export function onMessage(handler: (m: ToWebview) => void): () => void {
  const listener = (e: MessageEvent<ToWebview>) => handler(e.data);
  window.addEventListener("message", listener);
  return () => window.removeEventListener("message", listener);
}

/** Per-view settings kept by VS Code while the view exists. */
export interface Saved {
  properties: boolean;
}

export const saved: Saved = { properties: true, ...(vscode.getState() as Partial<Saved> | undefined) };
export const save = () => vscode.setState(saved);

let nextRequest = 1;
const waiting = new Map<number, (r: { element?: string; error?: string }) => void>();

/** Asks the host to make an edit; resolves with the id of the element it added or renamed. */
export function edit(e: MetamodelEdit): Promise<{ element?: string; error?: string }> {
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
  | { kind: "classifier"; classifier: "class" | "abstractClass" | "interface" | "enum" | "dataType" }
  | { kind: "connect"; connection: "reference" | "containment" | "generalization" };

/** An inline editor in a node. */
export type Editing =
  | { kind: "name"; element: string }
  | { kind: "member"; element: string }
  /** `seq` tells successive new members apart (each gets a fresh field). */
  | { kind: "new"; owner: string; member: "attribute" | "operation" | "literal"; seq: number };

export interface UiState {
  /** The element shown in the properties panel (and highlighted). */
  focused?: string;
  editing?: Editing;
  tool: Tool;
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

export const ui = new Store<UiState>({ tool: { kind: "select" } });

export function useUi<S>(select: (s: UiState) => S): S {
  return useSyncExternalStore(ui.subscribe, () => select(ui.get()));
}

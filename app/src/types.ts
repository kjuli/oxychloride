import type { Diagnostic, EvaluationResult, Language } from "./api";

/** A model object selected as the evaluation context. */
export interface ContextObject {
  resource: string;
  fragment: string;
  label: string;
  class: string;
}

/** An editor tab. The text itself lives in the editor controller. */
export interface Tab {
  id: string;
  /** Name of the document in the engine: its path, or `untitled-N.ocl`. */
  name: string;
  language: Language;
  path: string | null;
  title: string;
  dirty: boolean;
}

export type ConsoleEntry =
  | {
      kind: "evaluation";
      id: number;
      expression: string;
      context: ContextObject | null;
      result?: EvaluationResult;
      error?: string;
    }
  | { kind: "note"; id: number; level: "info" | "error"; text: string; diagnostics?: Diagnostic[] };

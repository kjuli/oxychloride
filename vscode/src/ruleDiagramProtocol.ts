// Messages between the rule diagram (webview/rules.tsx) and its host (ruleDiagram.ts).
// Type-only: the webview bundle imports it too.

import type { Position, Range, TransformationDiagram, TransformationEdit } from "./engine";
import type { Layout } from "./diagramProtocol";

export type { Layout };

/** How the document stands to the document of the other syntax (`x.henshin` for
 * `x.henshin_text`). */
export type PairInfo = { partner?: string; inSync?: boolean; reason?: string };

export type ToRuleWebview =
  /** The layout read from the layout file; `persistent`: there can be one (the document is a file). */
  | { type: "init"; name: string; layout: Layout; persistent: boolean }
  | { type: "diagram"; diagram: TransformationDiagram }
  /** The layout file was changed by someone else. */
  | { type: "layout"; layout: Layout }
  /** The document cannot be drawn (the last diagram stays). */
  | { type: "unreadable"; message?: string }
  /** The result of an `edit` request. */
  | { type: "edited"; request: number; element?: string; error?: string }
  /** The cursor of a text editor of the document moved. */
  | { type: "cursor"; position: Position }
  | { type: "pair"; pair: PairInfo };

export type FromRuleWebview =
  | { type: "ready" }
  | { type: "edit"; request: number; edit: TransformationEdit }
  /** Write the layout file: `always`, or only if it exists already. */
  | { type: "layout"; layout: Layout; persist: "always" | "ifExists" }
  /** Select the text of an element in the text editors of the document (`open`: open one). */
  | { type: "reveal"; range: Range; open: boolean }
  | { type: "notice"; kind: "error" | "warning" | "info"; message: string }
  /** Run a command on the document: open its text, transform a model with it, explore a
   * state space, or create the document of the other syntax. */
  | { type: "command"; command: "openText" | "transform" | "explore" | "createCounterpart" };

// Messages between the metamodel diagram (webview/diagram.tsx) and its host (diagram.ts).
// Type-only: the webview bundle imports it too.

import type { MetamodelDiagram, MetamodelEdit, Position, Range } from "./engine";

/** Where the classifiers are drawn, by element id (the layout file's `nodes`). */
export type Layout = Record<string, { x: number; y: number }>;

export type ToWebview =
  /** The layout read from the layout file; `persistent`: there can be one (the document is a file). */
  | { type: "init"; name: string; layout: Layout; persistent: boolean }
  | { type: "diagram"; diagram: MetamodelDiagram }
  /** The layout file was changed by someone else. */
  | { type: "layout"; layout: Layout }
  /** The document cannot be drawn (the last diagram stays). */
  | { type: "unreadable"; message?: string }
  /** The result of an `edit` request. */
  | { type: "edited"; request: number; element?: string; error?: string }
  /** The cursor of a text editor of the document moved. */
  | { type: "cursor"; position: Position };

export type FromWebview =
  | { type: "ready" }
  | { type: "edit"; request: number; edit: MetamodelEdit }
  /** Write the layout file: `always`, or only if it exists already. */
  | { type: "layout"; layout: Layout; persist: "always" | "ifExists" }
  /** Select the text of an element in the text editors of the document (`open`: open one). */
  | { type: "reveal"; range: Range; open: boolean }
  | { type: "notice"; kind: "error" | "warning" | "info"; message: string };

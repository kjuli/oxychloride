// Messages between the transformation view (webview/transformation.ts) and its host
// (transformation.ts). Type-only: the webview bundle imports it too.

import type {
  MatchList,
  ParameterValues,
  StateSpaceInfo,
  StateSpaceStateDetails,
  TransformationState,
  TransformationUnitInfo,
} from "./engine";

export type Tab = "steps" | "space";

export type ToWebview =
  | { type: "init"; document: string; model: string; tab: Tab }
  | { type: "units"; units: TransformationUnitInfo[]; errors: number }
  | { type: "state"; state: TransformationState }
  | { type: "matches"; rule: string; list: MatchList }
  | { type: "space"; space: StateSpaceInfo }
  | { type: "spaceState"; details: StateSpaceStateDetails }
  | { type: "tab"; tab: Tab }
  | { type: "busy"; busy: boolean; what?: string }
  | { type: "notice"; kind: "error" | "warning" | "info"; message: string };

export type FromWebview =
  | { type: "ready" }
  | { type: "findMatches"; rule: string; parameters: ParameterValues }
  | { type: "apply"; unit: string; parameters: ParameterValues; match?: number }
  | { type: "selectStep"; step: number }
  | { type: "explore"; rules: string[]; parameters: Record<string, ParameterValues>; maxStates: number; maxDepth?: number; validate: boolean }
  | { type: "selectState"; state: number }
  | { type: "adoptState"; state: number }
  | { type: "export"; format: "hutn" | "xmi" }
  | { type: "restart" }
  | { type: "reveal"; unit: string };

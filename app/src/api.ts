// Typed wrapper around the engine's JSON-RPC methods (see crates/oxychloride/src/rpc.rs and
// protocol.rs). Field names are camelCase; lines and columns are 0-based and columns count
// Unicode scalar values, not UTF-16 code units.

import { invoke, isTauri } from "@tauri-apps/api/core";

// ------------------------------------------------------------------ protocol types

export type Severity = "error" | "warning" | "info";

/** Document languages: Complete OCL, OCLinEcore metamodels, HUTN models, XMI models (as text). */
export type Language = "ocl" | "oclinecore" | "hutn" | "xmi";

/** The language of a document, by its file extension (Complete OCL by default). */
export function languageOf(path: string): Language {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  return ext === "oclinecore" ? "oclinecore" : ext === "hutn" ? "hutn" : ext === "xmi" ? "xmi" : "ocl";
}

export interface Position {
  line: number;
  column: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export type Origin =
  | { kind: "document"; name: string }
  | { kind: "ecore"; resource: string; element: string; key: string }
  | { kind: "expression" }
  | { kind: "file"; path: string };

export interface Diagnostic {
  severity: Severity;
  message: string;
  source: string;
  origin: Origin;
  range: Range;
}

export interface ObjectRef {
  resource?: string;
  fragment: string;
}

export interface ObjectSummary {
  resource: string;
  fragment: string;
  class: string;
  label: string;
  /** For objects defined in a HUTN document (named by `resource`): the definition's range. */
  range?: Range;
}

export type ResultStatus = "satisfied" | "violated" | "error";

export type ConstraintKind = "invariant" | "precondition" | "postcondition";

export interface ConstraintInfo {
  name: string;
  kind: ConstraintKind;
  context: string;
  /** For pre- and postconditions: the operation's signature. */
  operation?: string;
  source: string;
  origin: Origin;
  range: Range;
  text: string;
}

export interface ValidationResult {
  constraint: number;
  object: ObjectSummary;
  /** For pre- and postconditions: the checked call, e.g. `mayEnrol(Course #prog1)`. */
  call?: string;
  status: ResultStatus;
  message?: string;
}

export interface ValidationSummary {
  constraints: number;
  objects: number;
  evaluations: number;
  satisfied: number;
  violated: number;
  errors: number;
}

export interface ValidationReport {
  constraints: ConstraintInfo[];
  results: ValidationResult[];
  summary: ValidationSummary;
  diagnostics: Diagnostic[];
  /** What could not be checked (e.g. operations only checked where they are called). */
  notes?: string[];
}

export interface EvaluationResult {
  display: string;
  value: unknown;
  type: string;
  invalid: boolean;
  diagnostics: Diagnostic[];
  /** Pre- and postconditions that failed in operation calls made by the expression. */
  contracts?: ContractResult[];
}

export interface ContractResult {
  constraint: ConstraintInfo;
  object: ObjectSummary;
  call: string;
  status: ResultStatus;
  message?: string;
}

export interface PackageInfo {
  name: string;
  nsUri: string;
  nsPrefix: string;
  resource: string;
  classifiers: ClassifierInfo[];
  subpackages: PackageInfo[];
}

export type ClassifierKind = "class" | "dataType" | "enum";

export interface ClassifierInfo {
  name: string;
  qualifiedName: string;
  kind: ClassifierKind;
  isAbstract?: boolean;
  isInterface?: boolean;
  superTypes?: string[];
  features?: FeatureInfo[];
  operations?: OperationInfo[];
  literals?: string[];
  invariants?: NamedExpression[];
}

export interface NamedExpression {
  name: string;
  expression: string;
}

export interface FeatureInfo {
  name: string;
  type: string;
  isReference: boolean;
  containment?: boolean;
  derived?: boolean;
  lower: number;
  upper: number;
  opposite?: string;
  derivation?: string;
}

export interface OperationInfo {
  name: string;
  parameters: ParameterInfo[];
  type: string;
  body?: string;
  preconditions?: NamedExpression[];
  postconditions?: NamedExpression[];
}

export interface ParameterInfo {
  name: string;
  type: string;
}

export interface ModelResourceInfo {
  uri: string;
  roots: ObjectInfo[];
}

export interface ObjectInfo {
  fragment: string;
  class: string;
  label: string;
  /** For objects defined in a HUTN document: the definition's range. */
  range?: Range;
  attributes: SlotInfo[];
  references: SlotInfo[];
  children: ObjectInfo[];
}

export interface SlotInfo {
  name: string;
  values: string[];
}

export interface LoadResult {
  uri: string;
  packages?: string[];
  objects?: number;
  warnings: Diagnostic[];
}

export interface HoverInfo {
  range: Range;
  contents: string;
}

export type CompletionKind =
  | "keyword"
  | "type"
  | "property"
  | "operation"
  | "iterator"
  | "variable"
  | "enumLiteral"
  | "snippet"
  | "object";

export interface CompletionItem {
  label: string;
  kind: CompletionKind;
  detail?: string;
  insert?: string;
}

export interface CompletionList {
  range: Range;
  items: CompletionItem[];
}

/** A change to a document: `range` is replaced by `text`. */
export interface TextEdit {
  range: Range;
  text: string;
}

/** Edits adding an object to a HUTN document; `cursor` is a position in the edited text. */
export interface DocumentEdit {
  edits: TextEdit[];
  cursor: Position;
}

/** A containment feature of an object and the (qualified) classes it can contain. */
export interface ContainmentOption {
  feature: string;
  type: string;
  many: boolean;
  /** Single-valued and already set. */
  full: boolean;
  classes: string[];
}

export interface ExportResult {
  text: string;
  /** `ecore` for OCLinEcore documents, `xmi` for HUTN documents. */
  format: "ecore" | "xmi";
}

export interface InitializeResult {
  name: string;
  version: string;
  protocolVersion: number;
  methods: string[];
}

// ------------------------------------------------------------------ transport

/** An error reported by the engine (or by the bridge, with code 0). */
export class RpcError extends Error {
  readonly code: number;
  constructor(code: number, message: string) {
    super(message);
    this.name = "RpcError";
    this.code = code;
  }
}

/** Whether the page runs inside the Tauri shell (and can reach the engine). */
export const engineAvailable: boolean = isTauri();

function toError(e: unknown): Error {
  if (e instanceof Error) return e;
  if (e && typeof e === "object" && "message" in e) {
    const { code, message } = e as { code?: unknown; message?: unknown };
    return new RpcError(typeof code === "number" ? code : 0, String(message));
  }
  return new RpcError(0, String(e));
}

async function call<T>(method: string, params: object | null = null): Promise<T> {
  if (!engineAvailable) {
    throw new RpcError(0, "The engine is only available in the desktop app (npm run tauri dev).");
  }
  try {
    return await invoke<T>("rpc", { method, params });
  } catch (e) {
    throw toError(e);
  }
}

export const rpc = {
  initialize: () => call<InitializeResult>("initialize"),
  loadEcore: (path: string) => call<LoadResult>("loadEcore", { path }),
  loadModel: (path: string) => call<LoadResult>("loadModel", { path }),
  /** Maps a URI prefix (e.g. `pathmap://PCM_MODELS/`) to a local directory. */
  mapUri: (from: string, to: string) => call<null>("mapUri", { from, to }),
  reset: () => call<null>("reset"),
  clearModels: () => call<null>("clearModels"),
  reload: () => call<{ problems: Diagnostic[] }>("reload"),
  openDocument: (name: string, text: string, language?: Language) =>
    call<{ diagnostics: Diagnostic[]; language: Language }>("openDocument", { name, text, language }),
  closeDocument: (name: string) => call<{ closed: boolean }>("closeDocument", { name }),
  /** All diagnostics (documents and OCL embedded in metamodels), or one document's. */
  diagnostics: (name?: string) =>
    call<{ diagnostics: Diagnostic[] }>("diagnostics", name === undefined ? {} : { name }),
  validate: (options: {
    ecore?: boolean;
    documents?: string[];
    objects?: ObjectRef[];
    includeSatisfied?: boolean;
    contracts?: boolean;
  }) => call<ValidationReport>("validate", options),
  evaluate: (expression: string, context?: ObjectRef) =>
    call<EvaluationResult>("evaluate", context ? { expression, context } : { expression }),
  packages: () => call<PackageInfo[]>("packages"),
  models: () => call<ModelResourceInfo[]>("models"),
  hover: (name: string, position: Position, text?: string) =>
    call<HoverInfo | null>("hover", { name, position, text }),
  complete: (name: string, position: Position, text?: string) =>
    call<CompletionList | null>("complete", { name, position, text }),
  /** An OCLinEcore document as `.ecore` XML, or a HUTN document as XMI. */
  exportDocument: (name: string, target?: string) => call<ExportResult>("exportDocument", { name, target }),
  /** An `.ecore` file as OCLinEcore text, or an XMI model as HUTN text. */
  convertFile: (path: string, to: Exclude<Language, "ocl">) => call<{ text: string }>("convertFile", { path, to }),
  /** The edits adding an object of `cls` (qualified) to a HUTN document: into `container`'s
   * containment feature, or as a root object. */
  createObject: (name: string, cls: string, container?: { fragment: string; feature: string }, text?: string) =>
    call<DocumentEdit>("createObject", { name, class: cls, container, text }),
  containmentOptions: (object: ObjectRef) => call<ContainmentOption[]>("containmentOptions", { object }),
};

// ------------------------------------------------------------------ files

export async function readTextFile(path: string): Promise<string> {
  try {
    return await invoke<string>("read_text_file", { path });
  } catch (e) {
    throw toError(e);
  }
}

export async function writeTextFile(path: string, contents: string): Promise<void> {
  try {
    await invoke<void>("write_text_file", { path, contents });
  } catch (e) {
    throw toError(e);
  }
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

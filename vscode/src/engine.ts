// The engine's JSON-RPC methods (crates/oxychloride/src/rpc.rs, docs/PROTOCOL.md), called
// through the language server's `oxychloride/rpc` request. The types mirror app/src/api.ts:
// field names are camelCase, lines and columns 0-based, columns count Unicode scalar values.

import type { LanguageClient } from "vscode-languageclient/node";

export type Severity = "error" | "warning" | "info";

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
  range?: Range;
}

export type ResultStatus = "satisfied" | "violated" | "error";

export interface ConstraintInfo {
  name: string;
  kind: "invariant" | "precondition" | "postcondition";
  context: string;
  operation?: string;
  source: string;
  origin: Origin;
  range: Range;
  text: string;
}

export interface ValidationResult {
  constraint: number;
  object: ObjectSummary;
  call?: string;
  status: ResultStatus;
  message?: string;
}

export interface ValidationReport {
  constraints: ConstraintInfo[];
  results: ValidationResult[];
  summary: {
    constraints: number;
    objects: number;
    evaluations: number;
    satisfied: number;
    violated: number;
    errors: number;
  };
  diagnostics: Diagnostic[];
  notes?: string[];
}

export interface ContractResult {
  constraint: ConstraintInfo;
  object: ObjectSummary;
  call: string;
  status: ResultStatus;
  message?: string;
}

export interface EvaluationResult {
  display: string;
  value: unknown;
  type: string;
  invalid: boolean;
  diagnostics: Diagnostic[];
  contracts?: ContractResult[];
}

export interface PackageInfo {
  name: string;
  nsUri: string;
  nsPrefix: string;
  resource: string;
  classifiers: ClassifierInfo[];
  subpackages: PackageInfo[];
}

export interface ClassifierInfo {
  name: string;
  qualifiedName: string;
  kind: "class" | "dataType" | "enum";
  isAbstract?: boolean;
  isInterface?: boolean;
  superTypes?: string[];
  features?: FeatureInfo[];
  operations?: { name: string; parameters: { name: string; type: string }[]; type: string }[];
  literals?: string[];
  invariants?: { name: string; expression: string }[];
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
}

export interface ModelResourceInfo {
  uri: string;
  roots: ObjectInfo[];
}

export interface ObjectInfo {
  fragment: string;
  class: string;
  label: string;
  range?: Range;
  attributes: { name: string; values: string[] }[];
  references: { name: string; values: string[] }[];
  children: ObjectInfo[];
}

export interface LoadResult {
  uri: string;
  packages?: string[];
  objects?: number;
  warnings: Diagnostic[];
}

export interface DocumentEdit {
  edits: { range: Range; text: string }[];
  cursor: Position;
}

export interface ContainmentOption {
  feature: string;
  type: string;
  many: boolean;
  full: boolean;
  classes: string[];
}

export interface SchemaFile {
  /** Namespace URI (absent for the XMI schema). */
  nsUri?: string;
  fileName: string;
  text: string;
}

export class Engine {
  constructor(private readonly client: () => LanguageClient | undefined) {}

  call<T>(method: string, params: object | null = null): Promise<T> {
    const client = this.client();
    if (!client) return Promise.reject(new Error("The Oxychloride language server is not running."));
    return client.sendRequest<T>("oxychloride/rpc", { method, params });
  }

  packages = () => this.call<PackageInfo[]>("packages");
  models = () => this.call<ModelResourceInfo[]>("models");
  evaluate = (expression: string, context?: ObjectRef) =>
    this.call<EvaluationResult>("evaluate", context ? { expression, context } : { expression });
  validate = (options: { ecore?: boolean; includeSatisfied?: boolean; contracts?: boolean }) =>
    this.call<ValidationReport>("validate", options);
  loadEcore = (path: string) => this.call<LoadResult>("loadEcore", { path });
  loadModel = (path: string) => this.call<LoadResult>("loadModel", { path });
  exportDocument = (name: string, target?: string) =>
    this.call<{ text: string; format: "ecore" | "xmi" }>("exportDocument", { name, target });
  convertFile = (path: string, to: "oclinecore" | "hutn") => this.call<{ text: string }>("convertFile", { path, to });
  createObject = (name: string, cls: string, container?: { fragment: string; feature: string }, text?: string) =>
    this.call<DocumentEdit>("createObject", { name, class: cls, container, text });
  containmentOptions = (object: ObjectRef) => this.call<ContainmentOption[]>("containmentOptions", { object });
  schemas = () => this.call<SchemaFile[]>("schemas");
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

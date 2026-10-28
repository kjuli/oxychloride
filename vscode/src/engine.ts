// The engine's JSON-RPC methods (crates/oxychloride/src/rpc.rs, docs/PROTOCOL.md), called
// through the language server's `oxychloride/rpc` request. The types mirror
// crates/oxychloride/src/protocol.rs: field names are camelCase, lines and columns 0-based,
// columns count Unicode scalar values.

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

/** An entry of the package registry: `uri` (an nsURI, or any URI used for a metamodel, reachable
 * or not) stands for the metamodel file `location`; a `uri` ending in `/` maps a prefix to the
 * folder `location`. */
export interface RegistryEntry {
  uri: string;
  location: string;
}

export interface RegisteredPackage {
  name: string;
  nsUri: string;
}

export interface RegisteredResource {
  uri: string;
  aliases?: string[];
  packages: RegisteredPackage[];
}

export interface RegistryInfo extends RegistryEntry {
  status: "loaded" | "folder" | "missing" | "error";
  message?: string;
  resource?: RegisteredResource;
  /** The metamodels the file uses, directly or through others (without Ecore). */
  imports: RegisteredResource[];
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

// ---------------------------------------------------------------- transformations (Henshin)

export interface TransformationParameter {
  name: string;
  kind: "in" | "out" | "inout" | "var" | "unknown";
  type: string;
  /** The parameter is a node (its type is a class): its value is a node id. */
  node: boolean;
  literals?: string[];
}

export interface TransformationUnitInfo {
  name: string;
  kind: "rule" | "unit";
  parameters: TransformationParameter[];
  range: Range;
  multiRules?: string[];
}

export interface TransformationUnits {
  units: TransformationUnitInfo[];
  diagnostics: Diagnostic[];
}

export interface GraphNode {
  id: number;
  class: string;
  label: string;
  attributes: { name: string; values: string[] }[];
  container?: number;
}

export interface GraphEdge {
  source: number;
  target: number;
  feature: string;
  containment: boolean;
  opposite?: string;
}

export interface GraphInfo {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface GraphChanges {
  createdNodes: number[];
  deletedNodes: GraphNode[];
  createdEdges: GraphEdge[];
  deletedEdges: GraphEdge[];
  changedAttributes: { node: number; feature: string; old: string[]; new: string[] }[];
}

export interface NamedValue {
  name: string;
  value: string;
}

export interface RuleApplicationInfo {
  rule: string;
  label: string;
  parameters: NamedValue[];
  nodes: { name: string; node: number }[];
  instances?: number;
}

export interface MatchInfo {
  index: number;
  label: string;
  parameters: NamedValue[];
  nodes: { name: string; node: number }[];
  matched: number[];
  instances?: number;
}

export interface MatchList {
  matches: MatchInfo[];
  more: boolean;
  messages: string[];
}

export interface StateViolation {
  constraint: string;
  context: string;
  node: number;
  label: string;
  status: ResultStatus;
  message?: string;
}

export interface TransformationStep {
  index: number;
  label: string;
  applications: RuleApplicationInfo[];
}

export interface TransformationState {
  session: number;
  model: string;
  steps: TransformationStep[];
  step: number;
  graph: GraphInfo;
  changes: GraphChanges;
  violations: StateViolation[];
}

export interface TransformationResult {
  applied: boolean;
  state: TransformationState;
  messages: string[];
}

export interface StateSpaceState {
  id: number;
  depth: number;
  nodes: number;
  explored: boolean;
  terminal: boolean;
  violations: number;
}

export interface StateSpaceTransition {
  source: number;
  target: number;
  rule: string;
  label: string;
  count: number;
}

export interface StateSpaceInfo {
  session: number;
  step: number;
  rules: string[];
  states: StateSpaceState[];
  transitions: StateSpaceTransition[];
  complete: boolean;
  messages: string[];
}

export interface StateSpaceStateDetails {
  id: number;
  graph: GraphInfo;
  violations: StateViolation[];
  path: number[];
  changes: GraphChanges;
}

/** A language code can be generated for (see `codeTargets`). */
export interface CodeTarget {
  id: string;
  name: string;
  description: string;
}

/** What `generateCode` generates: the packages of a metamodel `resource` (an `.ecore` file's
 * URI or an OCLinEcore document's name, as in `PackageInfo.resource`), or named `packages`. */
export interface CodeRequest {
  target: string;
  resource?: string;
  packages?: string[];
  /** The name of the generated package or crate (default: the first package's name). */
  name?: string;
  /** Complete OCL documents to include (default: all open ones). */
  documents?: string[];
}

export interface GeneratedCode {
  target: string;
  packages: string[];
  /** Paths relative to the output folder, with `/`. */
  files: { path: string; text: string }[];
  /** OCL the generated code does not support (it fails at run time there). */
  problems: { severity: Severity; message: string; origin?: string }[];
}

// ---------------------------------------------------------------- metamodel diagrams

export type ClassifierKind = "class" | "dataType" | "enum";

/** A metamodel document as a class diagram (see `metamodelDiagram`). Elements have ids made
 * of their names: `pkg::Class`, `pkg::Class.feature`, `pkg::Class.operation()`, `pkg::Enum.LITERAL`. */
export interface MetamodelDiagram {
  syntax: "ecore" | "oclinecore";
  packages: DiagramPackage[];
  classifiers: DiagramClassifier[];
  /** Classifiers of other metamodels that the document's classes extend or refer to. */
  external: DiagramExternal[];
  /** The types features can have, as the document's syntax writes them. */
  types: DiagramType[];
  problems: DiagramProblem[];
}

export interface DiagramPackage {
  id: string;
  name: string;
  nsUri: string;
  nsPrefix: string;
  range: Range;
}

export interface DiagramClassifier {
  id: string;
  name: string;
  package: string;
  kind: ClassifierKind;
  isAbstract: boolean;
  isInterface: boolean;
  instanceClass?: string;
  superTypes: string[];
  attributes: DiagramFeature[];
  references: DiagramFeature[];
  operations: DiagramOperation[];
  literals: DiagramLiteral[];
  invariants: { name: string; range: Range }[];
  range: Range;
}

export interface DiagramFeature {
  id: string;
  name: string;
  typeName: string;
  type?: string;
  lower: number;
  /** -1: unbounded. */
  upper: number;
  containment: boolean;
  opposite?: string;
  ordered: boolean;
  unique: boolean;
  derived: boolean;
  changeable: boolean;
  transient: boolean;
  volatile: boolean;
  isId: boolean;
  defaultValue?: string;
  range: Range;
}

export interface DiagramParameter {
  name: string;
  typeName: string;
  type?: string;
  lower: number;
  upper: number;
}

export interface DiagramOperation {
  id: string;
  name: string;
  parameters: DiagramParameter[];
  typeName?: string;
  type?: string;
  lower: number;
  upper: number;
  hasBody: boolean;
  range: Range;
}

export interface DiagramLiteral {
  id: string;
  name: string;
  value: number;
  literal: string;
  range: Range;
}

export interface DiagramExternal {
  id: string;
  name: string;
  kind: ClassifierKind;
  isAbstract: boolean;
  isInterface: boolean;
  resource: string;
}

export interface DiagramType {
  id: string;
  label: string;
  kind: ClassifierKind;
}

export interface DiagramProblem {
  severity: Severity;
  message: string;
  element?: string;
  range: Range;
}

export interface ParameterSpec {
  name: string;
  /** The id of the type. */
  type: string;
  lower?: number;
  upper?: number;
}

/** Properties an `update` changes (absent ones stay). */
export interface ElementUpdate {
  name?: string;
  isAbstract?: boolean;
  isInterface?: boolean;
  /** The id of the type; `""` removes the result type of an operation. */
  type?: string;
  lower?: number;
  upper?: number;
  containment?: boolean;
  ordered?: boolean;
  unique?: boolean;
  derived?: boolean;
  changeable?: boolean;
  transient?: boolean;
  volatile?: boolean;
  isId?: boolean;
  parameters?: ParameterSpec[];
}

/** A change of a metamodel document (see `editMetamodel`); elements and types by id. */
export type MetamodelEdit =
  | { kind: "addClassifier"; package: string; classifier: "class" | "abstractClass" | "interface" | "enum" | "dataType"; name: string }
  | { kind: "addAttribute"; owner: string; name: string; type: string; lower?: number; upper?: number }
  | { kind: "addReference"; owner: string; name: string; type: string; lower?: number; upper?: number; containment?: boolean; opposite?: string }
  | { kind: "addOperation"; owner: string; name: string; type?: string; lower?: number; upper?: number; parameters?: ParameterSpec[] }
  | { kind: "addLiteral"; owner: string; name: string }
  | { kind: "addSuperType"; owner: string; superType: string }
  | { kind: "removeSuperType"; owner: string; superType: string }
  | { kind: "update"; element: string; properties: ElementUpdate }
  | { kind: "delete"; elements: string[] };

export interface MetamodelEditResult {
  edits: { range: Range; text: string }[];
  /** The id of the added or renamed element. */
  element?: string;
}

// ---------------------------------------------------------------- rule diagrams (Henshin)

/** What a rule does with a node, edge or attribute (Henshin's actions). */
export type RuleAction = "preserve" | "create" | "delete" | "forbid" | "require";

export interface TransformationParameterSpec {
  kind: "in" | "out" | "inout" | "var" | "unknown";
  name: string;
  /** As written; none for parameters without a type. */
  type?: string;
}

/** The rules and units of a Henshin document (text or `.henshin` file), for drawing them in
 * Henshin's notation (see `transformationDiagram`). Ids: rule `r`, multi-rule `r/m`, condition
 * graph `r#g`, node `<scope>::n`, edge `<scope>::s->t:ref`, attribute `<scope>::n.a`, unit `u`,
 * unit element `u@1`, `u@2.1`. */
export interface TransformationDiagram {
  syntax: "text" | "model";
  imports: string[];
  /** The top-level packages that can be imported (the workspace's, then Ecore). */
  packages: string[];
  rules: RuleDiagram[];
  units: UnitDiagram[];
  classes: NodeClass[];
  types: string[];
  problems: DiagramProblem[];
}

export interface RuleDiagram {
  id: string;
  name: string;
  parameters: TransformationParameterSpec[];
  checkDangling: boolean;
  injectiveMatching: boolean;
  conditions: string[];
  javaImports: string[];
  multiRules: MultiRuleDiagram[];
  formulas: FormulaDiagram[];
  nodes: RuleNodeDiagram[];
  edges: RuleEdgeDiagram[];
  range: Range;
}

export interface MultiRuleDiagram {
  id: string;
  name: string;
  kernel: string;
  checkDangling: boolean;
  injectiveMatching: boolean;
  conditions: string[];
  range: Range;
}

export interface FormulaDiagram {
  scope: string;
  formula: string;
  graphs: string[];
  range: Range;
}

export interface RuleNodeDiagram {
  id: string;
  name: string;
  class: string;
  classId?: string;
  action: RuleAction;
  multi?: string;
  condition?: string;
  attributes: RuleAttributeDiagram[];
  parameter: boolean;
  range: Range;
}

export interface RuleAttributeDiagram {
  id: string;
  name: string;
  value: string;
  action: RuleAction;
  set: boolean;
  scope?: string;
  range: Range;
}

export interface RuleEdgeDiagram {
  id: string;
  source: string;
  target: string;
  reference: string;
  action: RuleAction;
  multi?: string;
  condition?: string;
  range: Range;
}

export interface UnitDiagram {
  id: string;
  name: string;
  parameters: TransformationParameterSpec[];
  body: UnitStepDiagram[];
  /** The body in the textual syntax. */
  text: string;
  range: Range;
}

export interface UnitStepDiagram {
  id: string;
  kind: "call" | "sequence" | "independent" | "priority" | "if" | "for" | "while" | "strict" | "rollback";
  target?: string;
  arguments?: string[];
  count?: string;
  value?: boolean;
  children?: UnitStepDiagram[][];
  range: Range;
}

export interface NodeClass {
  id: string;
  name: string;
  isAbstract: boolean;
  conformsTo: string[];
  attributes: { name: string; type: string; literals?: string[] }[];
  references: { name: string; target: string; containment: boolean; many: boolean }[];
}

/** A change of a transformation document (see `editTransformation`); elements by id. */
export type TransformationEdit =
  | { kind: "setImports"; imports: string[] }
  | { kind: "addRule"; name: string }
  | { kind: "addUnit"; name: string }
  | { kind: "rename"; element: string; name: string }
  | { kind: "delete"; elements: string[] }
  | { kind: "setParameters"; element: string; parameters: TransformationParameterSpec[] }
  | { kind: "setRuleOptions"; element: string; checkDangling?: boolean; injectiveMatching?: boolean }
  | { kind: "setConditions"; element: string; conditions: string[] }
  | { kind: "addNode"; scope: string; name: string; class: string; action: RuleAction }
  | { kind: "updateNode"; element: string; class?: string; action?: RuleAction }
  | { kind: "addEdge"; scope?: string; source: string; target: string; reference: string; action: RuleAction }
  | { kind: "updateEdge"; element: string; action?: RuleAction; reference?: string }
  | { kind: "addAttribute"; node: string; scope?: string; name: string; value: string; action?: RuleAction; set?: boolean }
  | { kind: "updateAttribute"; element: string; name?: string; value?: string; action?: RuleAction; set?: boolean }
  | { kind: "addMultiRule"; scope: string; name: string }
  | { kind: "setUnitBody"; element: string; body: string };

/** Parameter values by name: strings, numbers, Booleans, or node ids for node parameters. */
export type ParameterValues = Record<string, string | number | boolean | null>;

/** A QVT transformation of the workspace (`qvtTransformations`). */
export interface QvtTransformationInfo {
  /** The document's name (its path). */
  document: string;
  language: "qvto" | "qvtr";
  name: string;
  parameters: QvtParameterInfo[];
  /** QVTo: the `out` and `inout` parameters; QVTr: the parameters relations enforce (the
   * directions it runs in). */
  targets: string[];
  errors: number;
  /** The parameters the model asked about can be bound to. */
  applicable?: string[];
}

export interface QvtParameterInfo {
  name: string;
  /** `in`, `out` or `inout` (QVTo); none for QVTr. */
  direction?: "in" | "out" | "inout";
  modelType: string;
  /** The nsURIs of its metamodel's packages. */
  metamodels: string[];
}

export interface QvtRunRequest {
  document: string;
  /** Models (by engine name: a document's path or a loaded model's URI) of input parameters. */
  models: { parameter: string; model: string }[];
  /** Where the models of output parameters will be written. */
  outputs?: { parameter: string; path: string }[];
  /** QVTr: the parameter to run in the direction of. */
  target?: string;
  /** QVTr: only check the target model. */
  check?: boolean;
  format?: "xmi" | "hutn";
  configuration?: Record<string, string>;
}

export interface QvtOutput {
  parameter: string;
  path: string;
  format: "xmi" | "hutn";
  text: string;
  objects: number;
}

export interface QvtRunResult {
  outputs: QvtOutput[];
  log: string[];
  problems: Diagnostic[];
  violations: string[];
  invariants: string[];
  applications: number;
  created: number;
  aborted?: string;
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
  registry = () => this.call<RegistryInfo[]>("registry");
  /** Replaces the package registry; the metamodels, models and imports are reloaded. */
  setRegistry = (entries: RegistryEntry[]) => this.call<RegistryInfo[]>("setRegistry", { entries });
  exportDocument = (name: string, target?: string) =>
    this.call<{ text: string; format: "ecore" | "xmi" | "henshin" | "henshin_text" }>("exportDocument", { name, target });
  convertFile = (path: string, to: "oclinecore" | "hutn") => this.call<{ text: string }>("convertFile", { path, to });
  createObject = (name: string, cls: string, container?: { fragment: string; feature: string }, text?: string) =>
    this.call<DocumentEdit>("createObject", { name, class: cls, container, text });
  containmentOptions = (object: ObjectRef) => this.call<ContainmentOption[]>("containmentOptions", { object });
  /** The qualified name of the class whose definition in an OCLinEcore document contains `position`. */
  classAt = (name: string, position: Position) => this.call<string | null>("classAt", { name, position });
  schemas = () => this.call<SchemaFile[]>("schemas");
  codeTargets = () => this.call<CodeTarget[]>("codeTargets");
  generateCode = (request: CodeRequest) => this.call<GeneratedCode>("generateCode", request);

  /** The diagram of a metamodel document (an OCLinEcore document or an `.ecore` file) with
   * the text `text`. */
  metamodelDiagram = (name: string, text: string) => this.call<MetamodelDiagram>("metamodelDiagram", { name, text });
  /** The text edits that make a diagram edit in a metamodel document with the text `text`. */
  editMetamodel = (name: string, text: string, edit: MetamodelEdit) =>
    this.call<MetamodelEditResult>("editMetamodel", { name, text, edit });

  /** The rule diagrams of a Henshin document (text or `.henshin` file) with the text `text`. */
  transformationDiagram = (name: string, text: string) => this.call<TransformationDiagram>("transformationDiagram", { name, text });
  /** The text edits that make a diagram edit in a Henshin document with the text `text`. */
  editTransformation = (name: string, text: string, edit: TransformationEdit) =>
    this.call<MetamodelEditResult>("editTransformation", { name, text, edit });
  /** The edits that make `target` (the document of the other syntax, with `targetText`) mean
   * what the document `name` (with `text`) means. Fails while `name` has errors. */
  syncTransformation = (name: string, text: string, target: string, targetText: string) =>
    this.call<{ edits: { range: Range; text: string }[] }>("syncTransformation", { name, text, target, targetText });

  transformationUnits = (name: string) => this.call<TransformationUnits>("transformationUnits", { name });
  startTransformation = (model: string) => this.call<TransformationState>("startTransformation", { model });
  transformationState = (session: number, step?: number) =>
    this.call<TransformationState>("transformationState", { session, step });
  findMatches = (session: number, document: string, rule: string, parameters: ParameterValues, step?: number, limit?: number) =>
    this.call<MatchList>("findMatches", { session, document, rule, parameters, step, limit });
  applyTransformation = (
    session: number,
    document: string,
    unit: string,
    parameters: ParameterValues,
    options: { match?: number; step?: number; seed?: number } = {},
  ) => this.call<TransformationResult>("applyTransformation", { session, document, unit, parameters, ...options });
  exportTransformation = (session: number, format: "hutn" | "xmi", step?: number, target?: string) =>
    this.call<{ text: string }>("exportTransformation", { session, format, step, target });
  exploreStateSpace = (
    session: number,
    document: string,
    rules: string[],
    options: { parameters?: Record<string, ParameterValues>; step?: number; maxStates?: number; maxDepth?: number; validate?: boolean } = {},
  ) => this.call<StateSpaceInfo>("exploreStateSpace", { session, document, rules, ...options });
  stateSpaceState = (session: number, state: number) => this.call<StateSpaceStateDetails>("stateSpaceState", { session, state });
  adoptState = (session: number, state: number) => this.call<TransformationState>("adoptState", { session, state });
  closeTransformation = (session: number) => this.call<{ closed: boolean }>("closeTransformation", { session });

  /** The QVT transformations of the workspace; with `model`, which of their parameters it fits. */
  qvtTransformations = (model?: string) =>
    this.call<{ transformations: QvtTransformationInfo[] }>("qvtTransformations", model ? { model } : {}).then((r) => r.transformations);
  /** Runs a QVT transformation on copies of the workspace's models; the outputs come back as text. */
  runQvt = (request: QvtRunRequest) => this.call<QvtRunResult>("runQvt", request);
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

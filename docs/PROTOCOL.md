# Oxychloride JSON-RPC protocol

Version 1. One protocol, several transports:

| Transport | How |
|---|---|
| stdio | `oxy serve` — one JSON-RPC message per line on stdin, one response per line on stdout |
| C ABI | `oxy_session_request(session, json)` / `oxy_session_call(session, method, params)` — see `crates/oxy-ffi/include/oxychloride.h` |
| Rust | `oxychloride::rpc::Server::call(method, params)` (or use `oxychloride::Workspace` directly) |
| Language server | `oxy lsp` — LSP on stdio; the request `oxychloride/rpc` `{method, params}` calls any method below (see [The language server](#the-language-server-oxy-lsp)) |

Each server/session holds one **workspace**: loaded metamodels (`.ecore`), instance models (XMI) and open documents — Complete OCL, OCLinEcore metamodels, HUTN and XMI models, and Henshin transformations — plus the transformation sessions started on its models. Requests follow [JSON-RPC 2.0](https://www.jsonrpc.org/specification); requests without `id` are notifications and get no response; batches are supported.

Conventions: field names are camelCase. Positions are **0-based** `{line, column}`; columns count Unicode scalar values. A `Range` is `{start, end}`.

## Errors

| Code | Meaning |
|---|---|
| -32700 | Request is not valid JSON |
| -32600 | Invalid request (e.g. no `method`) |
| -32601 | Unknown method |
| -32602 | Invalid params |
| -32000 | A metamodel or model failed to load (message includes `file:line:col`) |
| -32001 | Not found (document, model resource, object) |

Problems *in OCL sources* are not errors: they are returned as `Diagnostic`s.

## Methods

### `initialize` → `{name, version, protocolVersion, methods}`

### `loadEcore` `{path}` or `{text, uri?}` → `LoadResult`
Loads a metamodel. Referenced `.ecore` files are loaded too (from disk, relative to `path`). A package whose nsURI is already loaded is an error.

### `loadModel` `{path}` or `{text, uri?}` → `LoadResult`
Loads an XMI instance model. Missing metamodels are loaded from `xsi:schemaLocation` when possible. Model files the model references (`href="other.xmi#..."`) are loaded too, like an EMF resource set; references into resources that cannot be found are reported as one warning per target resource and resolved automatically if that resource is loaded later.

### `mapUri` `{from, to}` → `null`
Maps a URI prefix to a directory, e.g. `{"from": "pathmap://PCM_MODELS/", "to": "/path/to/PCM_MODELS/"}`. Pending references into mapped resources are resolved immediately.

`LoadResult`: `{uri, packages?: [nsURI], objects?: number, warnings: [Diagnostic]}`.

### `reset` → `null` · `clearModels` → `null` · `reload` → `{problems: [Diagnostic]}`
`reset` empties the workspace; `clearModels` removes instance models only; `reload` re-reads all metamodels and models from their sources and re-resolves document imports.

### `openDocument` `{name, text, language?}` → `{diagnostics: [Diagnostic], language}`
Adds or replaces a document. `language` is `"ocl"` (Complete OCL), `"oclinecore"` (a metamodel), `"hutn"` or `"xmi"` (a model), `"henshin"` (graph transformation rules in Henshin's textual syntax) or `"henshinModel"` (the same as a `.henshin` model of Henshin's editors, see [Transformations](#transformations-henshin)); by default it follows the extension of `name` (`.oclinecore`, `.hutn`, `.xmi`, `.henshin_text`, `.henshin`, else Complete OCL). Use the file path as `name`: relative `import` statements are resolved against it and the imported `.ecore` files are loaded (an import of an `.oclinecore` file must name an open document).

An OCLinEcore document's packages become part of the workspace's metamodels (they show in `packages`); a HUTN document's objects become a model resource named `name` (they show in `models`, can be `context` objects and are validated). Both are rebuilt on every change, together with whatever depends on them. The diagnostics of a HUTN document include, once it has no errors, the invariants and pre-/postconditions its objects violate (errors) or that cannot be evaluated (warnings), located on the objects.

An XMI document is a model edited as text (unlike `loadModel`, which loads a file once): it is reloaded on every change, metamodels named in its `xsi:schemaLocation` are loaded if they are `.ecore` files (others are found by nsURI, e.g. packages of OCLinEcore documents), and its diagnostics are those of a HUTN document — load problems (malformed XML, unknown classes or features, bad values, unresolved references) as errors, unset required features as warnings, then constraint violations — located on the elements' tag names. A document that cannot be loaded at all is an empty model resource.

### `closeDocument` `{name}` → `{closed: boolean}`

### `diagnostics` `{name?}` → `{diagnostics: [Diagnostic]}`
Without `name`: diagnostics of all documents **and of the OCL embedded in loaded `.ecore` files** (invariants, operation bodies, derivations).

### `validate` → `ValidationReport`
Params (all optional):

| Field | Default | |
|---|---|---|
| `ecore` | `true` | include invariants embedded in the metamodels |
| `documents` | all open | names of documents whose invariants to check |
| `objects` | all | `[ObjectRef]` to restrict the check to |
| `includeSatisfied` | `false` | also report satisfied results |
| `contracts` | `true` | check pre- and postconditions (see below) |

`ecore` covers the constraints of OCLinEcore documents too: they are part of the metamodels.

Pre- and postconditions are checked at every operation call made while evaluating, and every operation with postconditions is called on every applicable object with each combination of arguments from finite domains (Booleans, enumeration literals, model objects); calls whose preconditions do not hold are skipped. `notes` lists operations that could not be checked that way.

`ValidationReport`:
```json
{
  "constraints": [{"name": "Adult", "kind": "invariant", "context": "library::Member", "source": "library.ocl",
                   "origin": {"kind": "document", "name": "library.ocl"}, "range": {...}, "text": "inv Adult(...): age >= 18"}],
  "results": [{"constraint": 0, "status": "violated", "message": "Member Bob is 16 years old",
               "object": {"resource": "/abs/library.xmi", "fragment": "//@members.1", "class": "library::Member", "label": "Member 'Bob'"}}],
  "summary": {"constraints": 8, "objects": 12, "evaluations": 23, "satisfied": 20, "violated": 3, "errors": 0},
  "diagnostics": [],
  "notes": []
}
```
`status` is `satisfied`, `violated`, or `error` (the constraint evaluated to `null`/`invalid`; `message` explains why). Constraint `kind` is `invariant`, `precondition` or `postcondition`; for the latter two, `operation` is the signature (`"hasSeat() : Boolean"`) and results carry the checked `call` (`"mayEnrol(Course #prog1)"`). Objects defined in HUTN documents have a `range` (their definition in the document named by `resource`).

### `evaluate` `{expression, context?: ObjectRef}` → `EvaluationResult`
Evaluates an expression with `self` bound to the context object. Definitions (`def:`) from all open documents and OCL operation bodies/derivations from the metamodels are available.

```json
{"display": "Sequence{'Alice', 'Bob'}", "type": "Sequence(String)", "invalid": false,
 "value": {"kind": "Sequence", "items": ["Alice", "Bob"]}, "diagnostics": []}
```

If the expression calls operations whose pre- or postconditions fail, `contracts` lists them: `[{constraint: ConstraintInfo, object: ObjectSummary, call, status, message?}]` (omitted when empty).

Structured `value` encoding: `null`; booleans, numbers, strings as JSON; `*` for unlimited; `{"invalid": reason}`; `{"object": ObjectSummary}`; `{"enum": "pkg::Enum", "literal": "A"}`; `{"kind": "Set"|"OrderedSet"|"Bag"|"Sequence", "items": [...]}`; `{"tuple": {"part": value}}`; `{"type": "Book"}`.

### `packages` → `[PackageInfo]`
The loaded metamodels (including those of OCLinEcore documents, whose `resource` is the document name): classifiers with features, operations, enum literals, and embedded OCL (invariants, derivations, bodies, and `preconditions`/`postconditions` of operations as `[{name, expression}]`).

### `models` → `[ModelResourceInfo]`
Object trees: `{uri, roots: [{fragment, class, label, range?, attributes: [{name, values}], references: [{name, values}], children: [...]}]}`. `range` locates objects of HUTN documents.

### `hover` `{name, position, text?}` → `{range, contents} | null`
The static type of the innermost OCL expression at `position`; in OCLinEcore documents also the element or type named there, in HUTN documents the class, feature or referenced object, in Henshin documents the package, class, node, feature, parameter, rule or unit named there. If `text` is given the document is updated first.

### `complete` `{name, position, text?}` → `{range, items: [{label, kind, detail?, insert?}]} | null`
Completion proposals. In OCL (Complete OCL documents and the expressions in OCLinEcore documents): features/operations after `.`, collection operations and iterators after `->`, enum literals after `Enum::`, and context-class features, variables (parameters, `result`), types and keywords elsewhere. In the structure of OCLinEcore documents: keywords for the enclosing element, type names, qualifiers. In HUTN documents: package blocks, concrete classes, the features an object does not set yet (`insert` is `"name: "`), and for values enumeration literals, `true`/`false`, classes for containment features, and identified objects of a fitting class (`kind` `"object"`, label `Class "id"`) for references. In Henshin documents: keywords of the enclosing block, packages after `ePackageImport`, the classes of the imported packages after `node name:`, data types and classes for parameters, attributes of the node's class in its braces and values after `=` (parameters, literals), the rule's nodes in edges and the references of the source's class after `src->tgt:` (those that fit the target first), condition graphs in formulas, rules and units to call in units. `range` is the text to replace; insert `insert` (or `label`).

### `classAt` `{name, position, text?}` → `string | null`
The qualified name (e.g. `university::Course`) of the class whose definition in the OCLinEcore document `name` contains `position`, anywhere from its first keyword to its closing brace; `null` outside classes and in other documents. Clients use it to act on "this class", e.g. to instantiate it. If `text` is given the document is updated first.

### `exportDocument` `{name, target?}` → `{text, format}`
An OCLinEcore document as an `.ecore` file (`format: "ecore"`, with its OCL as Pivot annotations), a HUTN document as XMI (`format: "xmi"`, with `xsi:schemaLocation`s of `.ecore` files relative to `target`), a Henshin text as a `.henshin` model (`format: "henshin"`, with references to `.ecore` files relative to `target`; the text must have no errors), or a `.henshin` model as a Henshin text (`format: "henshin_text"`). The text is returned, not written.

### `convertFile` `{path, to}` → `{text}`
An `.ecore` file as OCLinEcore text (`to: "oclinecore"`), an XMI model as HUTN text (`to: "hutn"`; metamodels come from the workspace or the model's schema locations), or a Henshin file in the other syntax (`to: "henshin"` for a `.henshin_text` file, `"henshinModel"` for a `.henshin` file). The workspace is not changed.

### `createObject` `{name, class, container?: {fragment, feature}, text?}` → `{edits: [{range, text}], cursor}`
The edits that add a new object of `class` (a qualified name such as `university::Course`) to the HUTN document `name`: as a value of the containment `feature` of the document's object at `fragment`, or else as a root object at the end of a package block for the class's package (a package block, and a `@Spec` entry, are added if the document has none). The new object is written as `Class {  }`, and `cursor` (a position in the edited text) is between its braces. Edits are in document order, their ranges refer to the text before the edit; the document is not changed. Abstract classes, interfaces, features that are not changeable containments, classes not conforming to the feature's type and single-valued features that are already set are refused (`INVALID_PARAMS`). If `text` is given the document is updated first.

### `containmentOptions` `{object: ObjectRef}` → `[{feature, type, many, full, classes}]`
The containment features of an object that a model document can set, with the qualified names of the concrete classes conforming to each feature's `type`; `full` marks single-valued features that already have a value.

### `schemas` → `[{nsUri, fileName, text}]`
XML Schemas (XSD) for XMI models of the loaded metamodels, in EMF's XMI style: one per package with an nsURI (Ecore's included) and one for the XMI namespace. They import each other by `fileName`, so write them into one directory; an XML catalog mapping each `nsUri` to its file then lets XML tools validate and complete XMI files, whatever their `xsi:schemaLocation` says. Limitations of XSD: an `xsi:type` naming a class that conforms only through a non-first supertype is rejected, and child elements of inherited features must precede those of a class's own features (as EMF writes them). See `crates/oxy-ecore/src/xsd_writer.rs`.

## Code generation

Native code for the classes of a metamodel, with its OCL translated (see [CODEGEN.md](CODEGEN.md)). The files are returned, not written: the client writes them into the folder of the user's choice.

### `codeTargets` → `[{id, name, description}]`
The languages code can be generated for (`python`, `rust`). Clients offer this list rather than a fixed one, so new targets need no client changes.

### `generateCode` `{target, resource?, packages?, name?, documents?}` → `GeneratedCode`
Generates code for the packages of a metamodel `resource` (an `.ecore` file's URI as `loadEcore` returns it, or an OCLinEcore document's name — `PackageInfo.resource`), or for the packages with the qualified names in `packages`, or (neither given) for all loaded metamodels. The packages they depend on (supertypes, types of features and operations) are generated too. `name` names the generated package or crate (default: the first package's name, in lower case). The OCL is that of the metamodels (`.ecore` annotations, OCLinEcore documents) and of the Complete OCL `documents` (default: all open ones): invariants, operation bodies, pre- and postconditions, derivations, initial values and `def:`s of the generated classes.

```json
{"target": "python", "packages": ["university"],
 "files": [{"path": "university/_runtime.py", "text": "..."}, {"path": "university/_model.py", "text": "..."}],
 "problems": [{"severity": "warning", "origin": "rules.ocl:12",
               "message": "Rust: 'matches' (regular expressions; the generated crate has no dependencies) is not supported; it evaluates to invalid"}]}
```

`files[].path` is relative to the output folder and uses `/`; all files are below one folder named after the package (`university/...`). `problems` lists OCL the target cannot translate (and OCL with errors): the code is generated anyway, and evaluating the affected expressions fails at run time (`OclInvalid` in Python, `Error::Invalid` in Rust) with the reason. Errors: an unknown `target` (`-32602`), an unknown `resource` or package (`-32001`).

## Metamodel diagrams

A metamodel document — an OCLinEcore document or an `.ecore` file — as a class diagram, and diagram edits as text edits. The text stays the only source of truth: clients apply the edits to the document and ask for the diagram again. Elements have ids made of their names: `pkg` and `pkg::sub` (packages), `pkg::Class` (classifiers), `pkg::Class.feature`, `pkg::Class.operation()` and `pkg::Enum.LITERAL`; a repeated name gets `#2`, `#3`, …. Types of other metamodels are named by their qualified names (`ecore::EString`).

### `metamodelDiagram` `{name, text?}` → `MetamodelDiagram`
For an OCLinEcore document `name` (updated with `text` first, if given), or an `.ecore` file `name` with the text `text` (else read from disk; the file is loaded into a metamodel of its own, with the files it references, and the workspace is not changed). Errors: `-32000` if an `.ecore` text cannot be loaded at all (e.g. malformed XML), `-32001` for an unknown document.

```json
{"syntax": "oclinecore",
 "packages": [{"id": "university", "name": "university", "nsUri": "http://…", "nsPrefix": "uni", "range": {…}}],
 "classifiers": [{"id": "university::Course", "name": "Course", "package": "university", "kind": "class",
                  "isAbstract": false, "isInterface": false, "superTypes": [],
                  "attributes": [{"id": "university::Course.code", "name": "code", "typeName": "String", "type": "ecore::EString",
                                  "lower": 1, "upper": 1, "containment": false, "ordered": true, "unique": true, "derived": false,
                                  "changeable": true, "transient": false, "volatile": false, "isId": true, "range": {…}}],
                  "references": [{"id": "university::Course.enrolments", "name": "enrolments", "typeName": "Enrolment",
                                  "type": "university::Enrolment", "lower": 0, "upper": -1, "opposite": "university::Enrolment.course", …}],
                  "operations": [{"id": "university::Course.hasSeat()", "name": "hasSeat", "parameters": [], "typeName": "Boolean",
                                  "type": "ecore::EBoolean", "lower": 1, "upper": 1, "hasBody": true, "range": {…}}],
                  "literals": [], "invariants": [{"name": "NotOverbooked", "range": {…}}], "range": {…}}],
 "external": [],
 "types": [{"id": "ecore::EString", "label": "String", "kind": "dataType"}, …],
 "problems": [{"severity": "error", "message": "…", "element": "university::Course.title", "range": {…}}]}
```

`kind` is `class`, `dataType` or `enum`; `upper` is `-1` for unbounded. `typeName` is the type as the document's syntax writes it (`String`, `ecore::EInt` in OCLinEcore; `EString` in Ecore), `type` its id if it is resolved. `range` is the element's definition in the document. `external` lists the classifiers of other metamodels that classifiers extend or that references point to (`{id, name, kind, isAbstract, isInterface, resource}`), `types` the types features can have (the document's classifiers, those of the metamodels it imports, Ecore's data types and `EObject`), labelled as the syntax writes them. `problems` are the document's diagnostics (`.ecore` files: load warnings), each on the innermost element whose definition contains it.

### `editMetamodel` `{name, text?, edit}` → `{edits: [{range, text}], element?}`
The text edits that make `edit` in the document (as for `metamodelDiagram`); the document is not changed. Edits are in document order, their ranges refer to the text before the edit; `element` is the id of the added or renamed element. `edit` is one of (`type`s are ids, multiplicities `lower`/`upper` with `-1` for unbounded):

| `kind` | Fields | |
|---|---|---|
| `addClassifier` | `package`, `classifier` (`class`, `abstractClass`, `interface`, `enum`, `dataType`), `name` | after the package's last classifier |
| `addAttribute` | `owner`, `name`, `type`, `lower?` (0), `upper?` (1) | after the class's last feature |
| `addReference` | `owner`, `name`, `type`, `lower?`, `upper?`, `containment?`, `opposite?` | with `opposite`, its opposite is added to the target class too |
| `addOperation` | `owner`, `name`, `type?`, `lower?`, `upper?`, `parameters?: [{name, type, lower?, upper?}]` | after the last operation |
| `addLiteral` | `owner`, `name` | |
| `addSuperType` / `removeSuperType` | `owner`, `superType` | cyclic inheritance is refused |
| `update` | `element`, `properties: {name?, isAbstract?, isInterface?, type?, lower?, upper?, containment?, ordered?, unique?, derived?, changeable?, transient?, volatile?, isId?, parameters?}` | absent properties stay; `type: ""` removes an operation's result type |
| `delete` | `elements: [id]` | also the references typed by deleted classes, and the supertypes and opposites that name deleted elements |

New elements are written as the document's syntax writes them (OCLinEcore like the printer, `.ecore` files like EMF, with the document's indentation); changed elements are changed in place (a name, type, multiplicity, attribute or qualifier list), so the rest of the text — its layout, comments and OCL — stays as it is. Renaming updates the references to the element in the document (types, supertypes, opposites, `#//…` fragments), not in OCL expressions or other documents. Deleting removes whole lines where the element has them to itself, in OCLinEcore with the comment lines right above it. Edits that make the metamodel invalid (a name that is taken, an attribute typed by a class, a reference typed by a data type, a multiplicity without a type) are refused with `-32602`; unknown elements are `-32001`.

## Transformations (Henshin)

Henshin documents define in-place graph transformation rules and units over the workspace's metamodels, in the textual syntax of [Henshin](https://www.eclipse.org/henshin/) (`.henshin_text`, language `"henshin"`) or as the models Henshin's graphical and tree editors write (`.henshin`, language `"henshinModel"`). The two mean the same, and every method that takes a Henshin document takes either (see [HENSHIN.md](HENSHIN.md) for how they map onto each other). Their problems (syntax, unknown classes, features, nodes and parameters, edges whose action does not fit their nodes, unsupported Java expressions) are diagnostics like those of other documents; in a `.henshin` model they are located at its XML (an unknown class at the `href` naming it), and so are hovers. Opening a `.henshin` model loads the `.ecore` files it refers to (relative to it) whose packages are not loaded yet.

Rules are applied in **transformation sessions**: a session starts from a model resource and keeps the states it went through as numbered steps (step 0 is the model). The model itself is never changed; a state can be exported. Rules and units are read from the document at every request, so a session survives editing them; when the metamodel changes, the session's states are mapped onto the new one by qualified names. Objects of a session are identified by node ids (numbers): created objects get new ids, ids are never reused, so the ids of two steps can be compared.

Semantics, as in Henshin: matching is injective (unless `injectiveMatching false`), attribute values given by a parameter that has no value bind it, `conditions [...]` must hold, `forbid` elements form one negative and `require` elements one positive application condition, a `matchingFormula` combines its condition graphs with `AND`/`OR`/`XOR`/`!`, and with `checkDangling` (the default) a node can only be deleted with all its links. A rule with multi-rules is applied at its kernel match together with all matches of each multi-rule (amalgamation). Units: a sequence fails (and with `rollback` undoes its steps) when a step fails (`strict`), `independent [...]` applies one of its steps at random (seeded), `priority [...]` the first applicable one, `if`/`for`/`while` as expected; calls pass the unit's parameters by position and get `OUT`/`INOUT` values back. Expressions: literals, parameters, `+ - * /`, comparisons, `== !=`, `AND OR !`, and enumeration literals as `Enum.LITERAL`; Java calls and attributes are not evaluated (warning).

Parameter values (`parameters`): `{name: value}`, where `value` is a string, number or Boolean for data types (strings are parsed for numbers and Booleans), an enumeration literal's name, or a node id for parameters typed by a class; `null` or a missing parameter is bound by matching.

### `transformationUnits` `{name, text?}` → `{units: [UnitInfo], diagnostics: [Diagnostic]}`
The rules and units of a Henshin document, in document order: `{name, kind: "rule"|"unit", parameters: [{name, kind: "in"|"out"|"inout"|"var"|"unknown", type, node, literals?}], range, multiRules?}`.

### `startTransformation` `{model}` → `TransformationState`
Starts a session on a model resource (URI or path of an XMI file, name of a HUTN/XMI document, or a file name): its objects and the links between them (references to objects of other resources are left out).

### `transformationState` `{session, step?}` → `TransformationState`
A step of a session (default: the last).

### `findMatches` `{session, document, rule, parameters?, step?, limit?}` → `{matches: [MatchInfo], more, messages}`
The matches of a rule on a step (default: the last; at most `limit`, default 100): `{index, label, parameters: [{name, value}], nodes: [{name, node}], matched: [node], instances?}` — the rule's nodes and the objects they matched (multi-rule nodes as `multiRule.node`), all matched objects, and the number of multi-rule matches. `messages` explain why matching failed, e.g. a parameter without a value.

### `applyTransformation` `{session, document, unit, parameters?, match?, step?, seed?}` → `{applied, state: TransformationState, messages}`
Applies a rule (at match number `match` of `findMatches`, else at its first match) or executes a unit, on a step (default: the last). If it applies, the steps after that step are discarded and the result is the new last step; else `state` is the step and `messages` say why.

`TransformationState`:
```json
{"session": 0, "model": "/…/university.hutn", "step": 1,
 "steps": [{"index": 0, "label": "initial model", "applications": []},
           {"index": 1, "label": "enrol(student=Student 'Brian', course=Course \"algo\")",
            "applications": [{"rule": "enrol", "label": "…", "parameters": [{"name": "student", "value": "Student 'Brian'"}],
                              "nodes": [{"name": "student", "node": 5}, {"name": "course", "node": 3}]}]}],
 "graph": {"nodes": [{"id": 5, "class": "university::Student", "label": "Student 'Brian'", "attributes": [{"name": "name", "values": ["'Brian'"]}], "container": 0}],
           "edges": [{"source": 5, "target": 16, "feature": "enrolments", "containment": true, "opposite": "student"}]},
 "changes": {"createdNodes": [16], "deletedNodes": [], "createdEdges": [...], "deletedEdges": [], "changedAttributes": [{"node": 3, "feature": "capacity", "old": ["3"], "new": ["5"]}]},
 "violations": [{"constraint": "CreditLimit", "context": "university::Student", "node": 5, "label": "Student 'Brian'", "status": "violated", "message": "…"}]}
```
`edges` lists one side of each bidirectional pair of references (the containment, else the one declared first) and leaves out containers (implied by `container`). `changes` compare the step with the one before; `deletedNodes` are the objects as they were. `violations` are the invariants (embedded in the metamodels and in Complete OCL documents) that the state violates or cannot evaluate.

### `exportTransformation` `{session, step?, format, target?}` → `{text}`
A step as a model document: `format` `"hutn"` or `"xmi"` (with `xsi:schemaLocation`s of `.ecore` files relative to `target`). The text is returned, not written.

### `exploreStateSpace` `{session, document, rules, parameters?, step?, maxStates?, maxDepth?, validate?}` → `StateSpaceInfo`
Explores the states that the given rules reach from a step of the session, breadth first, applying every match of every rule (`parameters`: fixed values by rule, `{rule: {param: value}}`). States isomorphic to a known state (same objects, attribute values and links up to object identity) are merged. Exploration stops at `maxStates` states (default 1000) and does not go beyond `maxDepth` steps. With `validate` (default `true`) the invariant violations of each state are counted.

```json
{"session": 0, "step": 0, "rules": ["takeLeft", "takeRight", "release"], "complete": true, "messages": [],
 "states": [{"id": 0, "depth": 0, "nodes": 7, "explored": true, "terminal": false, "violations": 0}],
 "transitions": [{"source": 0, "target": 1, "rule": "takeLeft", "label": "takeLeft(p: Philosopher 'Plato', fork: Fork \"f1\")", "count": 1}]}
```
`terminal`: no rule is applicable; `explored: false`: a limit stopped the exploration there. Transitions with the same rule and parameters between two states are one transition with a `count`. The session keeps the state space (until a step it was explored from is discarded).

### `stateSpaceState` `{session, state}` → `{id, graph, violations, path, changes}`
A state of the session's state space: its objects (`GraphInfo`), violations, the transitions (indices) of a shortest path from the initial state, and what the last of them changed.

### `adoptState` `{session, state}` → `TransformationState`
Replaces the steps after the step the state space was explored from by the shortest path to the state.

### `closeTransformation` `{session}` → `{closed: boolean}`

### `syncTransformation` `{name, text?, target, targetText}` → `{edits: [{range, text}]}`
The edits that make `target` — the Henshin document of the other syntax, whose current text is `targetText` (it need not be open) — mean what the document `name` means: towards a `.henshin` model it is written again from the text, keeping its ids and what the text cannot express (descriptions, annotations, …; renamed rules, units and nodes keep their ids), and compared line by line; towards a text, the rules and units that still mean the same keep their text (layout and comments), the others are written anew. No edits: the two are equivalent. Fails (`-32602`) while `name` has errors. Clients use it to keep a pair `x.henshin_text`/`x.henshin` in sync.

## Rule diagrams

A Henshin document of either syntax as diagrams in Henshin's notation, and diagram edits as text edits; as for metamodel diagrams, the text is the only source of truth. Ids are made of names: a rule `r`, a multi-rule `r/m` (nested `r/m/n`), a condition graph of a matching formula `r#g`; a node `<scope>::n`, an edge `<scope>::s->t:ref`, an attribute `<scope>::n.a` (the scope it is written in, e.g. a multi-rule's `reuse`); a unit `u` and the elements of its body `u@1`, `u@2.1.1` (element 2's first list's first element); a repeated id gets `#2`, ….

### `transformationDiagram` `{name, text?}` → `TransformationDiagram`
```ts
type TransformationDiagram = {
  syntax: "text" | "model"; imports: string[];
  rules: { id; name; parameters: {kind, name, type?}[]; checkDangling; injectiveMatching; conditions: string[]; javaImports: string[];
           multiRules: { id; name; kernel; checkDangling; injectiveMatching; conditions; range }[];
           formulas: { scope; formula; graphs: string[]; range }[];
           nodes: { id; name; class; classId?; action; multi?; condition?; parameter; range;
                    attributes: { id; name; value; action; set; scope?; range }[] }[];
           edges: { id; source; target; reference; action; multi?; condition?; range }[]; range }[];
  units: { id; name; parameters; text; range;
           body: { id; kind: "call"|"sequence"|"independent"|"priority"|"if"|"for"|"while"|"strict"|"rollback";
                   target?; arguments?; count?; value?; children?: Step[][]; range }[] }[];
  classes: { id; name; isAbstract; conformsTo: string[]; attributes: {name, type, literals?}[]; references: {name, target, containment, many}[] }[];
  types: string[];          // parameter types as written
  problems: DiagramProblem[];
};
```
`action` is `"preserve"`, `"create"`, `"delete"`, `"forbid"` or `"require"`. The elements of multi-rules and condition graphs are among their rule's nodes and edges, with `multi` (the multi-rule's id) or `condition` (the condition graph's id); a condition graph's elements have the action `forbid` if the graph is negated in a conjunction, else `require`. Values, conditions and classes are as written in the textual syntax; `text` is a unit's body in it. Ranges are in the document (for a model: its XML).

### `editTransformation` `{name, text?, edit}` → `{edits: [{range, text}], element?}`
The text edits that make a diagram edit (the document is not changed). `edit` is one of (`kind`):

- `addRule {name}`, `addUnit {name}`; `rename {element, name}` (rules, units, multi-rules, condition graphs, nodes, with the references in the document: calls, edges, `reuse`s, formulas, node parameters); `delete {elements}` (rules and units with their calls, multi-rules, condition graphs, nodes with their edges, edges, attributes, elements of units);
- `setParameters {element, parameters: [{kind, name, type?}]}`, `setRuleOptions {element, checkDangling?, injectiveMatching?}`, `setConditions {element, conditions: [string]}`, `addMultiRule {scope, name}`;
- `addNode {scope, name, class, action}`, `updateNode {element, class?, action?}` (a new action changes the node's edges and attributes so that they fit);
- `addEdge {scope?, source, target, reference, action}`, `updateEdge {element, action?, reference?}`;
- `addAttribute {node, scope?, name, value, action?, set?}` (in `scope`, a multi-rule or condition graph, it is written with `reuse`), `updateAttribute {element, name?, value?, action?, set?}`;
- `setUnitBody {element, body}` (the body in the textual syntax).

`element` is the id of the added or renamed element. In a text, new elements are written as the printer writes them and changes are made in place. A `.henshin` model is edited through its textual form, compiled and written back (keeping ids), and the changed lines are returned; an edit that would make it invalid is refused (`-32602`), as are names that are taken and values that are not expressions.

## The language server (`oxy lsp`)

`oxy lsp` speaks the Language Server Protocol on stdio (`--stdio` is accepted and ignored). It keeps one workspace:

- Documents open in the editor (`.ocl`, `.oclinecore`, `.hutn`, `.henshin_text` with language id `henshin`; `.xmi` and `.henshin` with language id `xml`) are workspace documents named by their path; unsaved documents are named by their URI, their language comes from the language id. The `.oclinecore`, `.ocl`, `.henshin_text` and `.henshin` files of the workspace folders are loaded too, from disk while they are not open (and reloaded on `workspace/didChangeWatchedFiles`), so models find their metamodels and constraints. A changed `.ecore` file that is loaded (imported, or named by a model) reloads all metamodels.
- Once changes settle (200 ms), the diagnostics of all documents are published (`source: "ocl"`); they depend on each other, e.g. editing a metamodel re-checks its models.
- `textDocument/hover` and `textDocument/completion` (LSP positions count UTF-16 code units; they are converted).
- `oxychloride/rpc` `{method, params}` → the result of any method above. Its positions are the engine's (Unicode scalar columns), and document names are paths (or URIs of unsaved documents).
- The notification `oxychloride/workspaceChanged` (no params) says that `packages` or `models` changed.

The VS Code extension (`vscode/`) is its client.

## Types

```ts
type ObjectRef = { resource?: string; fragment: string };  // resource: path/URI, file name or HUTN document name; default: first model
type Diagnostic = {
  severity: "error" | "warning" | "info";
  message: string;
  source: string;  // document name, or "library.ecore: library::Book::SufficientCopies"
  origin: { kind: "document"; name: string }
        | { kind: "ecore"; resource: string; element: string; key: string }
        | { kind: "expression" }
        | { kind: "file"; path: string };
  range: Range;    // for "ecore" origins: relative to the annotation's expression text
};
```

The authoritative definitions are `crates/oxychloride/src/protocol.rs` and `crates/oxychloride/src/rpc.rs`.

## Example session (`oxy serve`)

```
→ {"jsonrpc":"2.0","id":1,"method":"loadModel","params":{"path":"examples/library/library.xmi"}}
← {"jsonrpc":"2.0","id":1,"result":{"objects":12,"uri":"/…/library.xmi","warnings":[]}}
→ {"jsonrpc":"2.0","id":2,"method":"openDocument","params":{"name":"rules.ocl","text":"context library::Member inv: age >= 18"}}
← {"jsonrpc":"2.0","id":2,"result":{"diagnostics":[]}}
→ {"jsonrpc":"2.0","id":3,"method":"validate","params":{"ecore":false}}
← {"jsonrpc":"2.0","id":3,"result":{"summary":{"violated":1,…},…}}
```

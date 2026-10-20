# Oxychloride JSON-RPC protocol

Version 1. One protocol, several transports:

| Transport | How |
|---|---|
| stdio | `oxy serve` — one JSON-RPC message per line on stdin, one response per line on stdout |
| C ABI | `oxy_session_request(session, json)` / `oxy_session_call(session, method, params)` — see `crates/oxy-ffi/include/oxychloride.h` |
| Rust | `oxychloride::rpc::Server::call(method, params)` (or use `oxychloride::Workspace` directly) |
| Desktop app | Tauri command `rpc(method, params)` |
| Language server | `oxy lsp` — LSP on stdio; the request `oxychloride/rpc` `{method, params}` calls any method below (see [The language server](#the-language-server-oxy-lsp)) |

Each server/session holds one **workspace**: loaded metamodels (`.ecore`), instance models (XMI) and open documents — Complete OCL, OCLinEcore metamodels, and HUTN and XMI models. Requests follow [JSON-RPC 2.0](https://www.jsonrpc.org/specification); requests without `id` are notifications and get no response; batches are supported.

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
Adds or replaces a document. `language` is `"ocl"` (Complete OCL), `"oclinecore"` (a metamodel), `"hutn"` or `"xmi"` (a model); by default it follows the extension of `name` (`.oclinecore`, `.hutn`, `.xmi`, else Complete OCL). Use the file path as `name`: relative `import` statements are resolved against it and the imported `.ecore` files are loaded (an import of an `.oclinecore` file must name an open document).

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
The static type of the innermost OCL expression at `position`; in OCLinEcore documents also the element or type named there, in HUTN documents the class, feature or referenced object. If `text` is given the document is updated first.

### `complete` `{name, position, text?}` → `{range, items: [{label, kind, detail?, insert?}]} | null`
Completion proposals. In OCL (Complete OCL documents and the expressions in OCLinEcore documents): features/operations after `.`, collection operations and iterators after `->`, enum literals after `Enum::`, and context-class features, variables (parameters, `result`), types and keywords elsewhere. In the structure of OCLinEcore documents: keywords for the enclosing element, type names, qualifiers. In HUTN documents: package blocks, concrete classes, the features an object does not set yet (`insert` is `"name: "`), and for values enumeration literals, `true`/`false`, classes for containment features, and identified objects of a fitting class (`kind` `"object"`, label `Class "id"`) for references. `range` is the text to replace; insert `insert` (or `label`).

### `exportDocument` `{name, target?}` → `{text, format}`
An OCLinEcore document as an `.ecore` file (`format: "ecore"`, with its OCL as Pivot annotations), or a HUTN document as XMI (`format: "xmi"`, with `xsi:schemaLocation`s of `.ecore` files relative to `target`). The text is returned, not written.

### `convertFile` `{path, to}` → `{text}`
An `.ecore` file as OCLinEcore text (`to: "oclinecore"`), or an XMI model as HUTN text (`to: "hutn"`; metamodels come from the workspace or the model's schema locations). The workspace is not changed.

### `createObject` `{name, class, container?: {fragment, feature}, text?}` → `{edits: [{range, text}], cursor}`
The edits that add a new object of `class` (a qualified name such as `university::Course`) to the HUTN document `name`: as a value of the containment `feature` of the document's object at `fragment`, or else as a root object at the end of a package block for the class's package (a package block, and a `@Spec` entry, are added if the document has none). The new object is written as `Class {  }`, and `cursor` (a position in the edited text) is between its braces. Edits are in document order, their ranges refer to the text before the edit; the document is not changed. Abstract classes, interfaces, features that are not changeable containments, classes not conforming to the feature's type and single-valued features that are already set are refused (`INVALID_PARAMS`). If `text` is given the document is updated first.

### `containmentOptions` `{object: ObjectRef}` → `[{feature, type, many, full, classes}]`
The containment features of an object that a model document can set, with the qualified names of the concrete classes conforming to each feature's `type`; `full` marks single-valued features that already have a value.

### `schemas` → `[{nsUri, fileName, text}]`
XML Schemas (XSD) for XMI models of the loaded metamodels, in EMF's XMI style: one per package with an nsURI (Ecore's included) and one for the XMI namespace. They import each other by `fileName`, so write them into one directory; an XML catalog mapping each `nsUri` to its file then lets XML tools validate and complete XMI files, whatever their `xsi:schemaLocation` says. Limitations of XSD: an `xsi:type` naming a class that conforms only through a non-first supertype is rejected, and child elements of inherited features must precede those of a class's own features (as EMF writes them). See `crates/oxy-ecore/src/xsd_writer.rs`.

## The language server (`oxy lsp`)

`oxy lsp` speaks the Language Server Protocol on stdio (`--stdio` is accepted and ignored). It keeps one workspace:

- Documents open in the editor (`.ocl`, `.oclinecore`, `.hutn`; `.xmi` with language id `xml`) are workspace documents named by their path; unsaved documents are named by their URI, their language comes from the language id. The `.oclinecore` and `.ocl` files of the workspace folders are loaded too, from disk while they are not open (and reloaded on `workspace/didChangeWatchedFiles`), so models find their metamodels and constraints. A changed `.ecore` file that is loaded (imported, or named by a model) reloads all metamodels.
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

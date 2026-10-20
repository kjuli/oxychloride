# Architecture

```
                 ┌──────────┐ ┌───────────┐ ┌─────────────┐ ┌───────────┐ ┌───────────────┐
  front ends     │ oxy (CLI)│ │ oxy serve │ │ C ABI (ffi) │ │ Tauri app │ │ oxy lsp       │◄─ LSP: VS Code
                 │          │ │           │ │             │ │           │ │ (oxy-lsp)     │   extension
                 └─────┬────┘ └─────┬─────┘ └──────┬──────┘ └─────┬─────┘ └───────┬───────┘
                       │            └──────── JSON-RPC (rpc::Server) ─────────────┘
                 ┌─────▼──────────────────────────────────────────────────────────┐
  oxychloride    │ Workspace: metamodels + models + documents (OCL, OCLinEcore,   │
                 │ HUTN, XMI); protocol types; diagnostics, validation,           │
                 │ evaluation, hover/completion, export/convert, XML Schemas      │
                 └─────┬──────────────────────────────────────────────────────────┘
                 ┌─────▼──────────────────────────────────────────────────────────┐
  oxy-ocl        │ syntax (lexer, parser, AST; OCLinEcore) → oclinecore (build/   │
                 │ print metamodels) → env (EnvironmentBuilder) → check (types)   │
                 │ → eval (values, contracts) → validate                          │
                 └─────┬──────────────────────────────────────────────────────────┘
                 ┌─────▼──────────────────────────────────────────────────────────┐
  oxy-ecore      │ Metamodel (arena of packages/classifiers/features/operations)  │
                 │ Model (dynamic objects), .ecore/XMI loaders and writers, HUTN, │
                 │ XSD writer, mini XML DOM                                       │
                 └────────────────────────────────────────────────────────────────┘
```

## oxy-ecore

- `Metamodel` stores every loaded `EPackage` in arenas addressed by small `Copy` ids (`PackageId`, `ClassifierId`, `FeatureId`, `OperationId`). It only grows, so ids stay valid. Inheritance closures and feature lookup tables are recomputed after each load.
- `Ecore.ecore` is embedded (`builtin.rs`) and loaded by the same loader, so references like `ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString` resolve uniformly.
- `.ecore` loading is two-pass over a small DOM: build elements, then resolve `eType`/`eSuperTypes`/`eOpposite` hrefs (same file, by nsURI, or by relative path, loading referenced files on demand; cycles work because a resource is registered before its references are resolved). Unresolvable references are warnings, not errors.
- `Model` holds dynamic objects (`ObjectId`) with per-feature value lists. `Model::add` maintains containment and `eOpposite` consistency, which is what makes navigating the non-serialized side of a bidirectional reference work after loading XMI.
- XMI loading supports single-root and `xmi:XMI` multi-root documents, `xsi:type`, attribute and element-encoded values, `//@feature.index` fragments, name-based fragments and `xmi:id`s, and type-checks resolved references.
- Metamodels can also be built element by element (`add_resource`, `add_package`, … `finish_resource`), which is how OCLinEcore documents become packages. `ecore_writer` and `xmi_writer` serialize resources back to `.ecore` and XMI; derived, transient and container references are not written, and of a bidirectional pair of references only one side is.
- `hutn` parses HUTN (lexer, recovering parser producing an AST with byte ranges), loads it into a `Model` as a new resource (two passes: objects, then references by identifier; then multiplicity checks), reporting `Problem`s with ranges rather than failing, and prints a model resource as HUTN. The parser is metamodel-independent: whether a top-level block is a package block or an object is decided when loading.
- `xsd_writer` generates an XML Schema per package (EMF's XMI conventions: qualified root elements, unqualified children and attributes, one complex type per class extending its first supertype, an unbounded choice of child elements) plus one for the XMI namespace, so XML tools can check XMI files structurally.

## oxy-ocl

- **Syntax**: a hand-written lexer and recursive-descent/precedence-climbing parser. Document keywords (`context`, `inv`, `body`, …) are *not* reserved inside expressions (a model may well have a feature called `body`); the parser disambiguates by the following token. The parser never fails: errors become diagnostics plus `Error` nodes, and document-level recovery resynchronizes at the next `context`/`inv`/`def`/….
- **Environment** (`env.rs`): `EnvironmentBuilder` collects sources — Complete OCL documents and expressions from Ecore annotations — resolves contexts (with package scoping) and registers invariants, `def:` attributes/operations, operation bodies, derivations and initializers. `build()` type checks everything once all definitions are known, so documents can use each other's definitions.
- **Type checker** (`check.rs`): computes a `Type` for every expression (kept, with spans, for hovers and completion) and reports errors. `Type::Error` marks "already reported / unknown" and is accepted everywhere, which prevents cascades.
- **Evaluator** (`eval.rs`): a tree-walking interpreter over the same AST. Name resolution mirrors the checker (variables, then implicit-iterator/`self` properties, then types). Values (`value.rs`) are cheap to clone (`Rc`). Set semantics use a structural `Key` that agrees with OCL equality (e.g. `1 = 1.0`). Evaluation never panics on bad input: errors are `invalid` values that propagate per OCL's strictness rules, with the usual non-strict exceptions (`and`/`or`/`implies`, `oclIsUndefined`, `oclIsInvalid`). Recursion through definitions is bounded.
- **OCLinEcore** (`syntax/parser/oclinecore.rs`, `oclinecore.rs`): the parser is a child of the OCL parser and parses embedded expressions on the same token stream, so their spans are document offsets. `build` adds the document's packages to a metamodel in passes (declare classifiers, then members, then opposites once inheritance caches exist), stores the OCL as Pivot annotations (as Eclipse does, so export works), and returns the constraints with their owners. `EnvironmentBuilder::add_oclinecore` registers those constraints against the document source — not the annotations — so diagnostics, hovers and completion work on the document. `print` renders a resource back as OCLinEcore.
- **Contracts** (`eval.rs`): when an operation with an OCL body is called, the pre- and postconditions of it and of the operations it overrides are evaluated (postconditions with `result`); failures are recorded in the evaluator (`take_contract_checks`) and do not change the call's result. `@pre` evaluates to the current value: OCL queries have no side effects.
- **Validation** (`validate.rs`): every invariant is evaluated on every instance (including subclass instances) of its context class. With contracts, every operation that has postconditions is also called on every applicable object with all argument combinations from finite domains (Boolean, enumeration literals, model objects; capped per object); calls whose preconditions fail are skipped. Operations with other parameter types, or without a body, are reported as unchecked.

## oxychloride

`Workspace` is the façade the front ends share. It remembers where metamodels and models came from (so `reload` can rebuild everything), resolves document `import`s relative to the document path, and loads metamodels referenced by a model's `xsi:schemaLocation`. Compilation is cheap and currently redone per request; caching is an easy later optimization if documents get large.

Documents have a language (`Language`: Complete OCL, OCLinEcore, HUTN, XMI — by extension unless given). Because OCLinEcore documents define metamodel elements and model documents (HUTN, and XMI edited as text) define objects, and both change with every edit, the workspace keeps two layers of each:

- `base_mm` holds the `.ecore` files; `mm` is a clone of it plus the packages of the OCLinEcore documents (built in import order). Ids of `base_mm` elements are the same in `mm` (the metamodel only grows), so XMI models typed by `.ecore` metamodels survive a rebuild; XMI models typed by document classes are reloaded from their inputs.
- `xmi_model` holds the XMI files; `model` is a clone of it plus the objects of the model documents. XMI documents are loaded like HUTN ones (`load_xmi_document`): load warnings become errors located on the elements' tag names, multiplicities are checked with HUTN's `check_multiplicities`, and a document that does not load at all is an empty resource.

Operations mark what changed (`mm_dirty`, `xmi_stale`, `models_dirty`) and `finish` rebuilds the derived layers once at the end. `.ecore` files loaded meanwhile are added to both `base_mm` and `mm`, so a model being loaded can use them before `mm` is rebuilt in order. The diagnostics of a model document include the constraints its objects violate (when it has no errors), so the model editor doubles as a live constraint checker.

Editor support lives in `editor.rs` (Complete OCL and the shared OCL completion core) and `editor/oclinecore.rs`, `editor/hutn.rs`. Both scan the tokens before the cursor: for OCLinEcore to tell embedded OCL (completed like Complete OCL, with `self`, parameters and `result` bound from the enclosing element) from structure (keywords, types, qualifiers); for HUTN to find the enclosing object and slot, and propose features, literals, nested classes or identified objects whose class fits the feature's type.

`rpc::Server` maps JSON-RPC methods onto the workspace; `protocol.rs` holds the serialized types. Because the CLI's `serve`, the C ABI, the desktop app and the language server all go through this server, they cannot drift apart.

## oxy-lsp and the VS Code extension

`oxy-lsp` (run as `oxy lsp`) is a front end like the others: an LSP server (on `lsp-server`) that owns an `rpc::Server`. Editor documents become workspace documents; the `.oclinecore` and `.ocl` files of the workspace folders are loaded from disk while not open, so a model is checked against its metamodel and constraints without opening them. Text changes are buffered and handed to the engine before the next request or once edits settle, when the diagnostics of all documents are recomputed and the changed ones published; positions are converted between UTF-16 (LSP) and Unicode scalars (engine). Everything else — the views, the console, creating objects, validation — goes through one custom request, `oxychloride/rpc`, which forwards to the JSON-RPC methods, plus the notification `oxychloride/workspaceChanged`. A panic in the engine (a bug) is caught; the workspace is then rebuilt from the documents.

`vscode/` is the extension: TextMate grammars and language configurations, the language client, tree views (metamodels, models, validation results), the OCL console (a webview), commands that create instances and children through `createObject`, and the XML Schemas of `schemas`, written to the extension's storage with an XML catalog that is registered with the XML extension (`redhat.vscode-xml`) when it is installed.

## Design decisions

- **Dynamic models only.** There is no code generation; everything is reflective over the loaded metamodel, like EMF's dynamic EObjects. This is what lets other tools hand over arbitrary metamodels at runtime.
- **One AST for checking and evaluation.** No separate pivot model: the checker and evaluator each resolve names, which keeps the evaluator usable on expressions that failed type checking (useful in a console) at the cost of some duplicated resolution logic.
- **Lenient loading, strict reporting.** Loaders accept slightly broken inputs and report warnings; OCL problems are diagnostics with precise ranges rather than hard errors, so editors and tools can show all of them at once.

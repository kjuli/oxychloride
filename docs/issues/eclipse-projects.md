# Eclipse projects: what is still open

Eclipse metamodel projects can now be imported (`oxy import`, *Import Eclipse Projects…*) and work as in an Eclipse workspace without being one (see [PROJECTS.md](../PROJECTS.md#eclipse-projects)). PCM ([Palladio-Core-PCM](https://github.com/PalladioSimulator/Palladio-Core-PCM)) is the example project. These results were checked against real EMF, run on an EMF/Eclipse bundle pool:

- `.ecore` diagram edits: EMF re-saves edited `pcm.ecore` byte for byte.
- Model validation: results match EMF's `Diagnostician` with OCL delegates on PCM's test models.

What follows is still missing or open, most important first.

`oxy check` on a copy of PCM currently gives: **46 documents checked: 89 errors, 5 warnings**. Of these:

- 51 errors are invariant violations in PCM's test models. Those models break their constraints on purpose, and the reports agree with EMF.
- 38 errors are about the QVTo transformations (item 1).
- 4 warnings are required `id`s left unset in PCM's test models; EMF reports them too.
- 1 warning is a required bundle missing from the bundle pool (item 7).

## 1. QVTo: PCM's transformations do not compile

`bundles/org.palladiosimulator.pcm.resources/transformations/events/*.qvto` (`transformation-psm.qvto` and the libraries it imports) have 38 errors in `oxy-qvt` (12 of them are a library's errors reported again on the `import` that brings it in). Eclipse QVTo accepts these transformations. The gaps, by frequency:

- [ ] **Model types as types** (18 errors: `unknown type 'PCM_ALLOC'`, `'PCM_REP'`). Parameters are typed by a `modeltype`, e.g. `query Finder_findAllEventGroups(in allocationModel : PCM_ALLOC)` or `helper setupCentralMiddleware(repository : PCM_REP)`. In QVTo a model type is a type: the type of a model parameter (a model extent), with `objects()`, `rootObjects()`, `objectsOfType(T)`, ….
- [ ] **Contextual helpers called on collections with `->`** (7 errors, plus the units that import them). For example, `signatures->SEFFUtil_createForwardingSEFF(basicComponent)` calls `helper OperationSignature::SEFFUtil_createForwardingSEFF(…)` on a `Set(OperationSignature)`, and `Set(EventType)->InterfaceUtil_createOperationSignature(…)` does the same. QVTo applies the helper to each element, as `->map` does for mappings (an implicit `xcollect`). The checker reports `'…' is not defined for Set(OperationSignature)` instead.
- [ ] **`->oclAsType(T)` on a collection** (1 error). `Finder_findAllSourceRoles` returns `….requiredRoles_InterfaceRequiringEntity->select(sr | sr.oclIsTypeOf(SourceRole))->oclAsType(SourceRole)` as `Bag(SourceRole)`. We type the result as `SourceRole`, but Eclipse QVTo casts each element. Check the exact semantics against Eclipse QVTo (classic OCL) before changing the checker; codegen and the evaluator must agree (`crates/oxy-codegen/tests/conformance.rs`).
- [ ] **`object T { … }` without `@model` in libraries** (5 errors, in imported units). For example, `object ExternalCallAction { … }` in `library SEFFUtil` gives `no output model has ExternalCallAction's metamodel`. A library has no output models of its own: the object goes into the model of the transformation that runs it, whose metamodel fits, so this is decided when it runs, not when the library is compiled.
- [ ] Afterwards, *run* `transformation-psm.qvto` on PCM models. Compare the outputs with Eclipse QVTo's (the transformation needs FeatureConfig/FeatureModel metamodels, `http://sdq.ipd.uka.de/FeatureConfig/2.0`, from the target platform).

Tests for each gap go in `crates/oxy-qvt/tests/operational.rs` and `imports.rs`.

## 2. EMF conformance is not checked automatically

The ground truth (byte-identical re-saves, validation results) came from a Java harness. It ran real EMF on a bundle pool, from a temporary folder, and is not in the repository. The fixture tests in `crates/oxychloride/tests/eclipse.rs` keep the behaviour, but not the comparison with EMF.

- [ ] Add the harness (a small Java program plus a script: `validate <model>` → JSON diagnostics, `resave <ecore> --line-delimiter detect`) under e.g. `tools/emf-harness/`. Give it an opt-in test that runs when `EMF_POOL` (a bundle pool) and `java` are there, and skips itself otherwise, like the codegen tests do with `python3`/`cargo`.
- [ ] Add an opt-in PCM test, enabled when `PCM_REPO` is set. It copies the repository, runs `oxy import` and `oxy check`, and compares with recorded EMF results: the invariant violations per model, and the re-save of `pcm.ecore` after a set of diagram edits.

## 3. Known differences from EMF

- [ ] Four PCM constraints are implemented in Java only (generated `EValidator` code), not in OCL. They cannot be checked here. Make them visible, e.g. an info diagnostic or a note in `platform` info for `constraints` that have no OCL body.
- [ ] EMF in dynamic mode (no generated code) reports two things we don't: a required derived `expression` feature, and `EVERY_BIDIRECTIONAL_REFERENCE_IS_PAIRED` for some opposites. Both look like dynamic-EMF quirks, which generated PCM code does not have. Decide whether to match generated EMF (current behaviour) or dynamic EMF, and document the choice.

## 4. Target platform: OSGi resolution is simplified

`platform.rs` reads bundles the way PDE would for metamodels, but it is not an OSGi resolver:

- [ ] **Version ranges are ignored**: of several versions of a bundle the newest is used, even if `Require-Bundle: x;bundle-version="[1.0,2.0)"` excludes it. Pick the newest version within the range.
- [ ] **`Import-Package`** is not resolved, only `Require-Bundle`. Metamodel bundles rarely use it, but a package can come from a bundle that is only imported by package.
- [ ] **Fragments** (`Fragment-Host`) are not merged into their host, and `Bundle-ClassPath` is ignored. Models in fragments (e.g. localized ones) would not be found.
- [ ] **PDE target definitions** (`.target` files) cannot be the target platform; only folders, jars and installations can. Supporting them would mean reading their `<location>`s: directory and installation ones locally; p2 repositories would need downloading, which is out of scope unless they are cached in a bundle pool.

## 5. Editing Eclipse metamodels

- [ ] **The genmodel is not updated.** Adding, renaming or deleting classes and features in an `.ecore` file (text or diagram) leaves its `.genmodel` as it was. Eclipse reconciles the genmodel when it is reloaded, but Java code generation from it (in Eclipse) needs the new `genClasses`/`genFeatures`. Reconcile the genmodel on save, or at least warn that it is out of date.
- [ ] **OCL in annotations** can be edited in the text only. The metamodel diagram shows invariants of `.ecore` files but cannot add or change them (OCLinEcore documents can). `editMetamodel` needs edits for annotation details: the `Ecore` `constraints` list and the `OCL`/`Pivot`/`LPG` details.
- [ ] **New models of a bundle's file types**: the extension's *New Model* writes `.xmi`/`.hutn`. For Eclipse projects it should offer the bundle's extensions (`.repository`, `.system`, …) with a root object of the matching package. The content is the same XMI; only the file name changes.
- [ ] **Printing as OCLinEcore** (`oxy convert`, *Export*) writes `import 'stoex.ecore'` (a relative path) for metamodels of other bundles, instead of the `platform:/plugin/…` URI EMF uses. The text works here, but not in Eclipse OCL's OCLinEcore editor.

## 6. VS Code

- [ ] Opening a package from the *Eclipse Projects* node uses the location the engine reports, which is the canonical path. Where the workspace's path differs from it (a symbolic link; on macOS, `/var` vs `/private/var`), the file opens as a second document under the other name. Map locations back to the workspace's spelling before opening them.
- [ ] Every XML file of the workspace is now sent to the language server (`documentSelector` `{ language: "xml" }`), so that models of registered extensions are found. The server keeps those it does not know (`pom.xml`, `plugin.xml`, …) aside, unused. Filter on the client by the platform's `modelExtensions` instead, once `setPlatform` has answered.
- [ ] The *Package Registry* view does not list the packages that come from Eclipse projects and the target platform; they are shown under *Eclipse Projects* only. Show them in both, or explain the split in the views' tooltips.

## 7. Environment (not a code issue)

- `tools.mdsd.library.standalone.initialization` (required by `org.palladiosimulator.pcm.tests`) is not in the local bundle pool, so it is reported as missing. That is correct behaviour; adding the bundle to the target platform removes the warning.

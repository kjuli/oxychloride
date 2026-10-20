# Oxychloride for VS Code

OCL for Ecore models in VS Code: write metamodels in **OCLinEcore**, models in **HUTN** or
**XMI**, and see every OCL constraint they violate as you type. Built on the Oxychloride
engine (a Rust re-implementation of Eclipse OCL), the same engine as the `oxy` CLI and the
desktop app.

## Features

- **Languages:** Complete OCL (`.ocl`), OCLinEcore (`.oclinecore`) and HUTN (`.hutn`) with
  highlighting, bracket handling, comments, hover (types of OCL expressions, classes and
  features) and completion that proposes what the metamodel allows (features, operations,
  iterators, types, enumeration literals, nested classes, objects to refer to).
- **Live checking:** every invariant, precondition and postcondition — embedded in
  OCLinEcore metamodels and `.ecore` files, or written in Complete OCL documents — is
  checked on the open models while you edit them. Violations are shown on the objects in
  the Problems view. Editing a metamodel or a constraint re-checks its models right away.
  Metamodels (`.oclinecore`) and constraints (`.ocl`) of the workspace are found without
  opening them; `.ecore` metamodels are loaded through `import`s, `xsi:schemaLocation` or
  **Load .ecore Metamodel**.
- **XMI models:** `.xmi` files are checked like HUTN models (load problems, multiplicities,
  constraints). With the [XML extension](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-xml)
  installed, Oxychloride also generates an **XML Schema** for every loaded metamodel and
  registers it, so XMI files are validated and completed as XML (element and attribute
  names, enumeration literals, types of values), whatever their `xsi:schemaLocation` says.
  **Export XML Schema** writes the schemas to a folder.
- **OCL console** (panel): evaluate expressions against the open models, with any object as
  `self` (**Context…**, or **Use as Evaluation Context** on an object). Shows values, types,
  problems and the pre-/postconditions violated by the operation calls the expression makes.
  **Evaluate Selection in OCL Console** (⇧⌘↩ / Ctrl+Shift+Enter) sends the selected text.
- **Views** (Oxychloride activity bar):
  - **Metamodels** — packages, classes, features, operations, invariants. Right-click a
    class: **Create Instance** (in the current HUTN model, or a new one), **Create Dynamic
    Instance (XMI)…** (a new XMI file, like EMF's), **New Model** on a package.
  - **Models** — the objects of the open models. Click to show an object's definition;
    right-click: **Add Child…** (picks a containment feature and class, inserts the object
    and puts the cursor in it), **Use as Evaluation Context**.
  - **Validation** — **Validate Models** checks all models and lists every constraint with
    its results; click a result to show the object.
- **Generating `.ecore` files:** **Generate .ecore** (the editor title button of an
  OCLinEcore metamodel, or its Explorer context menu) writes the metamodel as an Ecore XML
  file next to it, `university.oclinecore` → `university.ecore`, ready for EMF tools. With
  `oxychloride.generateEcoreOnSave` it is regenerated whenever the metamodel is saved (not
  while it has errors).
- **Conversion:** **Export as .ecore…** (to a location of your choice), **Export as XMI**
  (HUTN), and **Convert to OCLinEcore / HUTN** for `.ecore` and `.xmi` files (Explorer
  context menu).
- **File icons** for `.ocl`, `.oclinecore` and `.hutn` files (in file icon themes that show
  language icons, such as the default Seti).

## Getting started

Open a folder with an `.oclinecore` metamodel (e.g. `examples/university` of the
repository), then its `.hutn` model: the Problems view lists the five violated constraints.
Right-click `Course` in the Metamodels view and choose **Create Instance**, or open the OCL
console and evaluate `Student.allInstances()->select(semester > 2)`.

## Settings

| Setting | |
|---|---|
| `oxychloride.server.path` | The `oxy` executable (default: bundled, else a build of an Oxychloride checkout, else `oxy` on the `PATH`) |
| `oxychloride.generateEcoreOnSave` | Write `<name>.ecore` next to an OCLinEcore metamodel whenever it is saved (default: off) |
| `oxychloride.xmlSchemas` | Generate and register XML Schemas for XMI files (default: on) |
| `oxychloride.validation.includeSatisfied` | List satisfied constraints in the Validation view |
| `oxychloride.trace.server` | Trace the language server protocol |

## Development

```sh
cargo build -p oxy-cli        # the language server: target/debug/oxy lsp
cd vscode
npm install
npm run build                 # bundles src/ into out/extension.js
```

Open `vscode/` in VS Code and press F5 (*Run Extension*), or run
`code --extensionDevelopmentPath=$PWD ../examples/university`. The extension finds
`target/debug/oxy` of the checkout.

- `npm test` runs the integration tests in a downloaded VS Code (with the XML extension) on
  a copy of `examples/` (`VSCODE_PATH` uses an installed VS Code instead).
- `npm run typecheck` type checks.
- `npm run package` builds a release `oxy`, bundles it into `server/` and packages a
  `.vsix` for this platform (package on each platform for the others).

The extension is a client of `oxy lsp` (see `docs/PROTOCOL.md`): standard LSP for
diagnostics, hover and completion, and the request `oxychloride/rpc` for everything else.

## Limitations

- Models are the open `.hutn`/`.xmi` documents (and XMI files loaded with **Load XMI
  Model**); unopened models of the workspace are not checked.
- Generated XML Schemas cannot express everything: an `xsi:type` naming a class that
  conforms only through a non-first supertype is rejected by the schema (not by the OCL
  engine), and child elements of inherited features must come before those of a class's own
  features, as EMF writes them.
- XMI files are not completed by the OCL engine itself; that is the XML extension's job.

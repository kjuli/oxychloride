# Oxychloride for VS Code

OCL for Ecore models in VS Code: write metamodels in **OCLinEcore** or **Ecore**, as text or
as a diagram, models in **HUTN** or **XMI**, and see every OCL constraint they violate as you
type; transform models with **Henshin** rules — as text or as rule diagrams in Henshin's notation, in
Henshin's textual syntax or as the `.henshin` files of Henshin's editors, kept equivalent — step by
step or by exploring their state space, or into other models with **QVT** (Operational Mappings and
Relations), run from a model's context menu.
Built on the Oxychloride engine (a Rust re-implementation of Eclipse OCL), the same engine as
the `oxy` CLI.

## Features

- **Languages:** Complete OCL (`.ocl`), OCLinEcore (`.oclinecore`) and HUTN (`.hutn`) (and QVT, below) with
  highlighting, bracket handling, comments, hover (types of OCL expressions, classes and
  features) and completion that proposes what the metamodel allows (features, operations,
  iterators, types, enumeration literals, nested classes, objects to refer to).
- **Metamodel diagrams:** **Open Metamodel Diagram** (the editor title button of an
  `.oclinecore` or `.ecore` file, or its Explorer context menu; also **Reopen Editor With…**)
  opens a class diagram beside the text: classes, interfaces, enumerations and data types
  with their attributes, operations, literals and invariants; references (containments with
  a diamond, opposites as one edge with both roles) and supertypes as edges; errors on the
  elements they concern. The diagram and the text editor edit **the same document**: every
  change in one shows in the other at once, and undo, redo and saving are shared. In the
  diagram, add classifiers from the toolbar (pick a tool, click into the diagram), drag from
  the dot on a class's right edge to another class to add a reference (or use the
  **Reference**, **Containment** and **Supertype** tools), double-click a name or a member to
  edit it in place (`name : String[0..*]`, `op(p : Course) : Boolean`), **+** adds members,
  `Delete` deletes, and the properties panel sets everything else (abstract, interface,
  supertypes, types, multiplicities, containment, ordered, unique, derived, read-only, …).
  Diagram edits change as little text as possible, written in the file's syntax, so its
  formatting, comments and OCL stay; renaming also renames the references to the element
  in the file. Selecting an element selects its text, and the cursor in the text selects
  the element. Where the classifiers are drawn is kept in a layout file next to the
  metamodel, `<file>.diagram.json` (e.g. `university.oclinecore.diagram.json`), written when
  you move something or use **Layout** (automatic layered layout); classifiers without a
  position are placed automatically.
- **Live checking:** every invariant, precondition and postcondition — embedded in
  OCLinEcore metamodels and `.ecore` files, or written in Complete OCL documents — is
  checked on the open models while you edit them. Violations are shown on the objects in
  the Problems view. Editing a metamodel or a constraint re-checks its models right away.
  Metamodels (`.oclinecore`) and constraints (`.ocl`) of the workspace are found without
  opening them; `.ecore` metamodels are loaded through the package registry, `import`s,
  `xsi:schemaLocation` or **Load .ecore Metamodel**.
- **Package registry:** like EMF's package registry and URI map, it says which file a URI
  stands for: a package's nsURI, or any URI that `.ecore` files, imports and models use for
  a metamodel — also one that cannot be reached, such as a placeholder
  (`http://example.org/shop`) or a `platform:/plugin/…` URI. Registered metamodels are loaded
  with the metamodels they import and are known everywhere: HUTN and XMI models of their
  nsURI, `import`s of OCL and OCLinEcore documents, Henshin transformations, and other
  `.ecore` files referring to them (`eType="http://example.org/shop#//Item"`). Register a
  file with **Register Metamodel…** (the **Package Registry** view, the Explorer context menu
  of `.ecore`/`.xml` files, or the quick fix on a "cannot find metamodel" problem), all
  `.ecore` files of the workspace at once with **Register Workspace Metamodels…**, or a
  folder for a URI prefix (`pathmap://MODELS/`) with **Map URI Prefix to Folder…**. The
  registry is kept in the setting `oxychloride.registry` (in `.vscode/settings.json`, with
  paths relative to the workspace folder), so it is shared with the project.
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
  - **Package Registry** — the registered URIs and their files, each with its packages,
    the metamodels it imports and its problems (a file that is missing or cannot be
    loaded, references that cannot be resolved). Click to open the file.
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
- **Generating code:** right-click an `.ecore` or `.oclinecore` file (Explorer or editor),
  or a package in the Metamodels view, **Generate Code…**: pick the language (Python or
  Rust) and the folder, and the metamodel becomes a Python package or a Rust crate —
  classes with consistent references, XMI loading and saving, and the OCL of the metamodel
  and of the workspace's Complete OCL documents (invariants, operation bodies, pre- and
  postconditions, derived features) as native code. See
  [docs/CODEGEN.md](../docs/CODEGEN.md).
- **Instantiating metamodels as XMI:** right-click an `.oclinecore` or `.ecore` file in the
  Explorer, **Instantiate as XML Model (XMI)…**, pick the root class (the classes no
  containment can hold come first), and choose where to save it (default: `<Class>.xmi` next
  to the metamodel). In an OCLinEcore editor, **Instantiate Class as XML Model (XMI)…** in
  the context menu takes the class at the cursor. The new model is an XMI file with an
  instance of the class as its root (with `xsi:schemaLocation` for `.ecore` metamodels),
  checked live and, with the XML extension, completed from the metamodel's schema.
- **Conversion:** **Export as .ecore…** (to a location of your choice), **Export as XMI**
  (HUTN), and **Convert to OCLinEcore / HUTN** for `.ecore` and `.xmi` files (Explorer
  context menu).
- **Henshin transformations** (`.henshin_text`, Henshin's textual syntax): highlighting
  (actions in colours: `create`, `delete`, `forbid`), problems as you type (unknown classes,
  features, nodes, edges whose action does not fit), hovers, and completion of keywords,
  classes of the imported packages, a node's attributes and their values, the rule's nodes
  and the references that fit an edge, rules and units to call.
  - **Open Transformation** (the editor title's ▷, the Explorer context menu, or
    **Transform with Henshin…** on a model in the Models view) opens the transformation view
    on a model: pick a rule or unit and its parameters (objects for node parameters),
    **Find Matches** (hover a match to see the objects it matches, tagged with the rule's
    node names), **Apply** or **Run Unit**. The diagram of each step shows what it created
    (green), deleted (red, dashed) and changed (amber), and which objects violate an
    invariant; the step list goes back and forth (← →), **Replay** plays the steps, and any
    step exports as HUTN or XMI. Applying a rule on an earlier step replaces the steps after
    it. The model itself is not changed.
  - **Explore State Space** draws all states the selected rules reach from the current step
    (isomorphic states merged): the initial state, states where no rule applies, states
    violating an invariant, and states a limit left unexplored. Click a state to see its
    objects and the shortest path to it; **Open Path as Steps** (or a double-click) makes
    that path the steps of the transformation.
- **Rule diagrams** for Henshin transformations, in Henshin's graphical notation: **Open Rule
  Diagram** in the editor title of a `.henshin_text` file, and the default editor of `.henshin`
  files (Henshin's models, as its graphical and tree editors write them). Each rule is a frame
  with its nodes and edges, their actions as stereotypes (`«preserve»`, `«create»`, `«delete»`,
  `«forbid»`, `«require»`) in Henshin's colors, multi-rule elements marked `«…*/m»` and drawn
  stacked, condition-graph elements `«forbid#g»`, changed attributes as `x=old->new`, attribute
  conditions as a yellow note; units show their control flow. Like the metamodel diagram it
  edits the document itself: add nodes (**Node** tool, with the toolbar's action and class),
  drag edges between nodes (the fitting reference, or a choice), edit names and attributes in
  place, change actions, classes, parameters, options, conditions, multi-rules and unit bodies
  in the properties; the text — or the `.henshin` file — changes with as few edits as possible.
  Positions are kept in `<file>.diagram.json`.
- **Pairs of a Henshin text and its `.henshin` model** (`rules.henshin_text` and `rules.henshin`
  in one folder) are kept equivalent: changing either changes the other (they are saved
  together; while the changed one has errors the other waits, and the status bar says so).
  **Create Equivalent .henshin / Henshin Text** writes the other one. `.henshin` files have
  problems, hovers, the transformation view and state space exploration like texts. See
  [docs/HENSHIN.md](../docs/HENSHIN.md).
- **QVT transformations:** QVT Operational Mappings (`.qvto`) and QVT Relations (`.qvtr`), as
  Eclipse QVTo and QVTd write them, with highlighting, problems as you type (unknown names,
  types, wrong arguments, model types whose metamodels cannot be found — with a quick fix to
  register a file for the URI), hovers (signatures, the types of expressions) and completion
  (after `.`, `->`, `::` and `map`; variables, features of `self`, mappings, keywords). Model
  types and imports find metamodels by nsURI, through the package registry, or as files next
  to the transformation. **Run QVT Transformation…** — on a model (`.xmi`, `.hutn`: Explorer,
  editor, the Models view) it offers the transformations of the workspace that take models of
  its metamodel; on a transformation (Explorer, editor title ▷) it asks for the models of its
  parameters. For QVT Relations it asks for the direction, and whether to create (enforce) the
  target model or only check an existing one. The results are written as XMI (or HUTN, for a
  `.hutn` file name) next to the model and opened; the log goes to the *Oxychloride* output, and
  problems of the run show on the transformation's lines. See [docs/QVT.md](../docs/QVT.md).
- **File icons** for `.ocl`, `.oclinecore`, `.hutn`, `.henshin_text`, `.qvto` and `.qvtr` files (in file icon themes that show
  language icons, such as the default Seti).

## Getting started

Open a folder with an `.oclinecore` metamodel (e.g. `examples/university` of the
repository), then its `.hutn` model: the Problems view lists the five violated constraints.
Right-click `Course` in the Metamodels view and choose **Create Instance**, or open the OCL
console and evaluate `Student.allInstances()->select(semester > 2)`. Open
`university.henshin_text` and press ▷ (**Open Transformation**) to enrol students step by
step, or open `university.henshin` to see the same rules as diagrams (editing either changes
both); in `examples/philosophers`, **Explore State Space** finds the deadlock of the dining
philosophers. In `examples/qvt`, right-click `company.hutn` and choose **Run QVT Transformation…**:
`uml2rdbms` (QVT Operational or QVT Relations) turns the class diagram into a relational schema.

## Settings

| Setting | |
|---|---|
| `oxychloride.server.path` | The `oxy` executable (default: bundled, else a build of an Oxychloride checkout, else `oxy` on the `PATH`) |
| `oxychloride.registry` | The package registry: `{"<uri>": "<file>"}`, e.g. `{"http://example.org/shop": "models/shop.ecore"}`; a URI ending in `/` maps a prefix to a folder |
| `oxychloride.generateEcoreOnSave` | Write `<name>.ecore` next to an OCLinEcore metamodel whenever it is saved (default: off) |
| `oxychloride.henshin.syncPairs` | Keep a Henshin text and the `.henshin` file of the same name next to it equivalent (default: on) |
| `oxychloride.xmlSchemas` | Generate and register XML Schemas for XMI files (default: on) |
| `oxychloride.codegen.target` | The language **Generate Code…** generates (`python`, `rust`; default: ask) |
| `oxychloride.codegen.outputFolder` | The folder **Generate Code…** writes into, absolute or relative to the workspace folder (default: ask) |
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
- `npm run typecheck` type checks (the extension, and the pages of the transformation view
  and the metamodel diagram in `webview/`, which `npm run build` bundles into
  `out/webview/`; the diagram is React with [React Flow](https://reactflow.dev) and
  [ELK](https://eclipse.dev/elk/)).
- `node ../.claude/skills/run-oxychloride/driver.mjs` serves the built diagram page to
  headless Chromium with a stand-in for VS Code and the real engine, to drive and screenshot
  it (see the skill's `SKILL.md`).
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
- Metamodel diagrams: renaming an element updates the references to it in the same file,
  not in its OCL expressions or in other files; generic types, annotations and EMF's
  `eKeys` are not shown or edited; edges are straight (or bent apart when several connect
  the same classes) and may cross classes.

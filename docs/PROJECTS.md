# Modeling projects

A modeling project is a folder that holds the artefacts of one modeling task: metamodels (OCLinEcore documents, `.ecore` files), constraints (Complete OCL), models (HUTN, XMI) and transformations (Henshin, QVT). The folder has a manifest, `oxychloride.json`, that names the project, the folders new documents go into, the project's package registry and how its code is generated. The engine, the `oxy` command line tool and the VS Code extension read the same manifest:

| | Creates a project | Works with a project |
|---|---|---|
| `oxy` | `oxy new shop`; `oxy import` for a folder of [Eclipse projects](#eclipse-projects) | `oxy check` (in the project or a folder of it): every document of the project, models included, with its registry |
| VS Code | **New Modeling Project…** (Command Palette, the *Modeling Projects* view, the Explorer's context menu of a folder, the empty Explorer); **Make Folder a Modeling Project** for an existing folder, **Import Eclipse Projects…** for Eclipse projects | The *Modeling Projects* view: documents by kind, new documents in the right folder, **Validate Project**, **Generate Code of Project**; the registry and code generation read the manifest |
| Other clients | `newProject`, `importEclipseProjects` (JSON-RPC, see [PROTOCOL.md](PROTOCOL.md)) | `Project::load`, `Workspace::open_project` (Rust) |

## A new project

```sh
oxy new shop                        # in ./shop: everything, metamodel in OCLinEcore
oxy new shop --ecore --with hutn,qvto --codegen python
oxy new shop --bare                 # the manifest and the metamodel only
cd shop && oxy check
```

The engine writes new projects from templates (`crates/oxychloride/src/project.rs`), so `oxy new`, the VS Code command and any client of `newProject` create the same files. The metamodel's package is named after the project (`My Shop` → `my_shop`, nsURI `http://example.org/my_shop` unless given), and every document is about it:

```
shop/
├── oxychloride.json                 the manifest
├── metamodels/shop.oclinecore       the metamodel: Model contains Elements, which depend on each other; invariants, an operation with a body
├── constraints/shop.ocl             a further invariant in Complete OCL (no element depends on itself)
├── models/example.hutn              a model …
├── models/example.xmi               … and the same model as EMF writes it
├── transformations/shop.henshin_text  Henshin rules (add an element, add a dependency) …
├── transformations/shop.henshin       … and the same rules as Henshin's model, kept equivalent by the extension
├── transformations/copy.qvto        copies a model, in QVT Operational Mappings …
├── transformations/copy.qvtr        … and in QVT Relations
├── README.md                        the layout, and how to work with it in VS Code and with oxy
└── .vscode/extensions.json          recommends the Oxychloride and XML extensions
```

[`examples/shop`](../examples/shop) is such a project (`oxy new examples/shop --name "My Shop" --codegen python`): open it in VS Code, or run `oxy check` in it. A test keeps it equal to what `oxy new` writes; after changing the templates, regenerate it with `--force`.

With `--ecore` (VS Code: *Ecore* as the metamodel's format) the metamodel is an `.ecore` file, registered under its nsURI in the manifest, and the XMI model names it in `xsi:schemaLocation` as EMF does. A new project is checked before it is written: if the name cannot be used by one of the languages (`rule` is a keyword of Henshin's), nothing is written and the problem is reported.

## The manifest

```json
{
  "name": "Shop",
  "description": "Orders and their items",
  "folders": {
    "metamodels": "metamodels",
    "constraints": "constraints",
    "models": "models",
    "transformations": "transformations",
    "generated": "src-gen"
  },
  "registry": {
    "http://example.org/shop": "metamodels/shop.ecore",
    "pathmap://LIBRARY/": "lib"
  },
  "codegen": { "target": "python", "outputFolder": "src-gen" }
}
```

Every field is optional; `{}` is a project named after its folder. The VS Code extension validates and completes the manifest with a JSON Schema (`vscode/schemas/oxychloride.schema.json`).

| Field | |
|---|---|
| `name` | The project's name (default: the folder's name) |
| `description` | A sentence about it |
| `folders` | Where new documents of each kind go, relative to the project (the defaults are shown above). They do not restrict where documents are: every document below the project folder belongs to it |
| `registry` | The project's [package registry](../README.md#metamodels-and-models-as-text): URIs (nsURIs, or URIs other files use for a metamodel, reachable or not) and the files they stand for, relative to the project; a URI ending in `/` maps a prefix to a folder |
| `codegen` | `target` (`python`, `rust`) and `outputFolder` (default: the `generated` folder): *Generate Code…* uses them for the project's metamodels without asking |
| `targetPlatform` | The target platform of the [Eclipse projects](#eclipse-projects) in the folder: bundles, folders of bundles, Eclipse installations; relative to the project, `~/` is the home folder |

## The documents of a project

The documents of a project are the files of the documents' languages (`.ocl`, `.oclinecore`, `.ecore`, `.hutn`, `.xmi`, `.henshin_text`, `.henshin`, `.qvto`, `.qvtr`, and the model extensions of its Eclipse projects) below its folder, except those in hidden folders, in `node_modules`, `target`, `out`, `dist` and `build`, in the folder of generated code, and in nested projects (a folder with its own manifest is a project of its own). An `.ecore` file next to an OCLinEcore document of the same name is that document's export, not a document of its own. `.ecore` documents are checked like OCLinEcore ones: their load problems and the OCL in their annotations.

`Workspace::open_project` gives the workspace the project's registry and opens all its documents, metamodels first and models last, so that `diagnostics` reports the problems of the whole project — including the constraints the models violate, which are diagnostics of the model documents. `oxy check` without documents does this for the project the current directory is in (`--project DIR` for another one; `-e`, `-m` and `--map` add to it), and exits with 1 if anything has errors.

## Eclipse projects

A modeling project can be made of Eclipse projects — plug-ins with metamodels, such as those of the [Palladio Component Model](https://github.com/PalladioSimulator/Palladio-Core-PCM) — and work with them as an Eclipse workspace would, without being one: no OSGi, no Java, no generated code is run. The folder's Eclipse projects are the folders below it with a `.project` (named by its `<name>`) or a `META-INF/MANIFEST.MF`; the manifest's `targetPlatform` names where the bundles they require are:

```json
{ "name": "PCM", "targetPlatform": ["~/.p2/pool/plugins"] }
```

`oxy import [DIR]` writes that manifest: it finds the Eclipse projects, picks the target platform on this machine that has most of the bundles they require (Oomph's bundle pools and installations, the PDE target platforms of their recent workspaces, `~/.p2/pool`, Eclipse applications; `-t PATH` gives one, `--no-target-platform` none) and reports the projects, their packages and the bundles that are missing. A target platform is bundles (jars, or folders, named `<name>_<version>`), folders of them (`plugins`, a bundle pool), Eclipse installations (`plugins` below them) or folders with those further down; of several versions of a bundle the newest is used.

As in EMF, then:

| | |
|---|---|
| `platform:/resource/<project>/<path>` | a file of the workspace project |
| `platform:/plugin/<bundle>/<path>` | a file of the project with that `Bundle-SymbolicName`, else of the target platform's bundle (read from its jar) |
| packages (nsURIs) | registered by the bundles' `plugin.xml` (`org.eclipse.emf.ecore.generated_package`, its `.ecore` found through its genmodel, and `dynamic_package`): the projects' are the workspace's metamodels, the target platform's are loaded when something refers to them |
| URI mappings | `org.eclipse.emf.ecore.uri_mapping`, e.g. `pathmap://PCM_MODELS/` → `platform:/plugin/org.palladiosimulator.pcm.resources/defaultModels/` |
| model files | the types of `org.eclipse.emf.ecore.extension_parser` (`.repository`, `.system`, `.allocation`, …) are XMI models, documents of the project |
| references | resolved against the URI a file has (`platform:/…`), and written so by edits: relative within a project or bundle, absolute between them |

The bundles the projects require (`Require-Bundle`, and what those require) that are neither projects nor in the target platform are reported on the manifest (optional ones, the OSGi framework's and EMF's own are not: Ecore is built in).

Metamodels are `.ecore` files, edited as text or in the metamodel diagram. OCL is in their annotations as EMF's OCL delegates read it (`http://www.eclipse.org/emf/2002/Ecore/OCL`, `…/OCL/Pivot`, `…/OCL/LPG`): `invariant` constraints, `derivation`s, operation `body`s, `pre`s and `post`s — checked, with problems, hover and completion in the attribute's value. Models are validated as EMF's `Diagnostician` does with those delegates: a required single-valued feature counts as set if it has a default value, and `allInstances()` are the instances in the object's own resource (as Eclipse OCL's), not in all models. Diagram edits write the file as EMF would save it (attribute order, lines wrapped at 80 columns, the file's line breaks), so a file EMF wrote, edited here and saved by EMF again, is unchanged by that save.

## In VS Code

The *Modeling Projects* view (first in the Oxychloride bar) lists the projects of the workspace — the folders with an `oxychloride.json` — and in each the documents by kind: metamodels, constraints, models, Henshin transformations, QVT transformations. The context menus of the documents have what applies to them (open the diagram or the rule diagram, run a QVT transformation, generate code, …), and each kind has a **+** that creates a new document of it in the folder the manifest names:

| Kind | Asks for | Writes |
|---|---|---|
| Metamodel | the package's name | `<metamodels>/<name>.oclinecore` (`newDocument`) |
| Constraints | the packages, the file's name | `<constraints>/<name>.ocl` importing them |
| Model | HUTN or XMI, the metamodel, the file's name | `<models>/<name>.hutn`, or an XMI model with a root object of a class to pick |
| Henshin transformation | diagram (`.henshin`) or text, the metamodel, the file's name | `<transformations>/<name>.henshin` or `.henshin_text` importing the metamodel's packages |
| QVT transformation | QVTo or QVTr, the source and target metamodels, the name | `<transformations>/<name>.qvto` or `.qvtr`, with a mapping (relation) between their root classes |

**Validate Project** opens every model of the project — also those not open in an editor — and shows the constraints they violate in the Validation view. **Generate Code of Project** generates the code of the project's metamodel with the manifest's `codegen` settings.

**Import Eclipse Projects…** (on a folder in the Explorer, or in the view's menu) makes a folder of Eclipse projects a modeling project: it asks for the target platform, offering the ones found on this machine with how many of the required bundles each has, and writes the manifest. The project then shows its *Eclipse Projects*: their versions, the packages they register (and whether they load) and the bundles that are missing. Files of the model extensions the bundles register open as XML and are models; the setting `oxychloride.targetPlatform` adds a target platform for the whole workspace.

The package registry of the workspace includes the registries of the projects (the *Package Registry* view names the project of an entry); registering a file of a project writes the entry into its manifest, with a path relative to the project, instead of the settings. Changes of a manifest — by hand or by the extension — take effect at once.

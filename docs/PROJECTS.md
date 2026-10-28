# Modeling projects

A modeling project is a folder that holds the artefacts of one modeling task: metamodels (OCLinEcore documents, `.ecore` files), constraints (Complete OCL), models (HUTN, XMI) and transformations (Henshin, QVT). The folder has a manifest, `oxychloride.json`, that names the project, the folders new documents go into, the project's package registry and how its code is generated. The engine, the `oxy` command line tool and the VS Code extension read the same manifest:

| | Creates a project | Works with a project |
|---|---|---|
| `oxy` | `oxy new shop` | `oxy check` (in the project or a folder of it): every document of the project, models included, with its registry |
| VS Code | **New Modeling Project…** (Command Palette, the *Modeling Projects* view, the Explorer's context menu of a folder, the empty Explorer); **Make Folder a Modeling Project** for an existing folder | The *Modeling Projects* view: documents by kind, new documents in the right folder, **Validate Project**, **Generate Code of Project**; the registry and code generation read the manifest |
| Other clients | `newProject` (JSON-RPC, see [PROTOCOL.md](PROTOCOL.md)) | `Project::load`, `Workspace::open_project` (Rust) |

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

## The documents of a project

The documents of a project are the files of the documents' languages (`.ocl`, `.oclinecore`, `.hutn`, `.xmi`, `.henshin_text`, `.henshin`, `.qvto`, `.qvtr`) below its folder, except those in hidden folders, in `node_modules`, `target`, `out`, `dist` and `build`, in the folder of generated code, and in nested projects (a folder with its own manifest is a project of its own). `.ecore` files are loaded when something refers to them (the registry, imports, `xsi:schemaLocation`s), not because they are in the project.

`Workspace::open_project` gives the workspace the project's registry and opens all its documents, metamodels first and models last, so that `diagnostics` reports the problems of the whole project — including the constraints the models violate, which are diagnostics of the model documents. `oxy check` without documents does this for the project the current directory is in (`--project DIR` for another one; `-e`, `-m` and `--map` add to it), and exits with 1 if anything has errors.

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

The package registry of the workspace includes the registries of the projects (the *Package Registry* view names the project of an entry); registering a file of a project writes the entry into its manifest, with a path relative to the project, instead of the settings. Changes of a manifest — by hand or by the extension — take effect at once.

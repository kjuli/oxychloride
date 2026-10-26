# File icons

16×16 icons for the document kinds, used by the VS Code extension (language icons, and the
tabs of metamodel diagrams; `vscode/esbuild.mjs` copies them into the extension).

| Icon | Kind | Glyph |
|---|---|---|
| `ocl` | Complete OCL (`.ocl`) | a check: constraints |
| `oclinecore` | OCLinEcore (`.oclinecore`) | a class box: a metamodel |
| `hutn` | HUTN (`.hutn`) | an object tree: a model |
| `ecore` | Ecore (`.ecore`) | a class box in angle brackets: a metamodel as XML |
| `xmi` | XMI (`.xmi`) | an object tree in angle brackets: a model as XML |
| `henshin` | Henshin (`.henshin_text`) | a node turning into a graph: a transformation |
| `qvto` | QVT Operational Mappings (`.qvto`) | a model mapped to a new one: a transformation |
| `qvtr` | QVT Relations (`.qvtr`) | two models related both ways: a transformation |

`<kind>.svg` is for light backgrounds, `<kind>-dark.svg` for dark ones; the colours are the
syntax colours (types blue, strings amber, keywords green; XML formats purple; transformations rose).

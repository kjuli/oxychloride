# File icons

16×16 icons for the document kinds, used by the desktop app (tabs, menus, lists) and by the
VS Code extension (language icons; `vscode/esbuild.mjs` copies them into the extension).

| Icon | Kind | Glyph |
|---|---|---|
| `ocl` | Complete OCL (`.ocl`) | a check: constraints |
| `oclinecore` | OCLinEcore (`.oclinecore`) | a class box: a metamodel |
| `hutn` | HUTN (`.hutn`) | an object tree: a model |
| `ecore` | Ecore (`.ecore`) | a class box in angle brackets: a metamodel as XML |
| `xmi` | XMI (`.xmi`) | an object tree in angle brackets: a model as XML |

`<kind>.svg` is for light backgrounds, `<kind>-dark.svg` for dark ones; the colours are the
app's syntax colours (types blue, strings amber, keywords green; XML formats purple).

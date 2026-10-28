# My Shop

Elements that depend on each other: the project oxy new creates

A modeling project of [Oxychloride](https://github.com/palladio/oxychloride): `oxychloride.json` describes it (its folders, its package registry, how code is generated).

| Folder | Contents | Files |
|---|---|---|
| `metamodels/` | The metamodel (Ecore, with its OCL) | `my_shop.oclinecore` |
| `constraints/` | Further constraints in Complete OCL | `my_shop.ocl` |
| `models/` | Models in HUTN and XMI, checked against all constraints | `example.hutn`, `example.xmi` |
| `transformations/` | Henshin graph transformations and QVT model transformations | `copy.qvto`, `copy.qvtr`, `my_shop.henshin`, `my_shop.henshin_text` |
| `src-gen/` | Generated code | |

## In VS Code

With the Oxychloride extension, the **Modeling Projects** view (in the Oxychloride bar) shows
the project's documents by kind, and creates new ones in the right folder. Documents are checked
as you type, models against every constraint of the metamodel and the Complete OCL documents;
**Validate Project** checks all models of the project at once. The metamodel and Henshin's rules
can be edited as diagrams too.

## On the command line

```sh
oxy check                                  # every document of the project, models included
oxy generate metamodels/my_shop.oclinecore -t python -o src-gen
oxy qvt transformations/copy.qvto -e metamodels/my_shop.oclinecore --in source=models/example.hutn --hutn
```

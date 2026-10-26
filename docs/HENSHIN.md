# Henshin transformations: text, models and rule diagrams

Oxychloride runs in-place graph transformations written for [Eclipse Henshin](https://www.eclipse.org/henshin/). A transformation can be written in either of Henshin's two notations, and both mean the same:

| | File | Written by | Edited in Oxychloride as |
|---|---|---|---|
| **Textual syntax** | `rules.henshin_text` | Henshin's Xtext editor (`org.eclipse.emf.henshin.text`), any text editor | text (highlighting, completion, hovers) and rule diagram |
| **Model** | `rules.henshin` | Henshin's graphical and tree editors (XMI of the metamodel `http://www.eclipse.org/emf/2011/Henshin`) | rule diagram (its default editor) and XML text |

Everything works on both: diagnostics, hovers, the transformation view (applying rules step by step, exploring state spaces), conversion, and the **rule diagram**, which draws rules in Henshin's graphical notation and edits whichever document it shows. When a text and a model of the same name are next to each other (`university.henshin_text` and `university.henshin`), they are a **pair**: changing one changes the other (see [Pairs](#pairs)).

`examples/university` and `examples/philosophers` have such pairs.

## One meaning, two syntaxes

The engine has one compiler for transformations (`oxy_henshin::Module`, see [ARCHITECTURE.md](ARCHITECTURE.md#oxy-henshin)). A text is parsed into a syntax tree; a `.henshin` file is *read into the same syntax tree* (`oxy_henshin::xmi::read`), with spans in the XML. So both compile — and are checked, run and drawn — by the same code, and problems of a `.henshin` file point at its XML (an unknown class at the `href` that names it).

The other direction, `oxy_henshin::xmi::write`, writes a `.henshin` file from the *compiled* module: the file means exactly what the engine runs. Writing what was read gives the same file again (texts and models round-trip to fixpoints; this is tested on the examples and was checked on all `.henshin` files of Henshin's repository whose metamodels are there).

### How the text maps onto Henshin's model

The mapping follows Henshin's own translation of the textual syntax (`Henshin_text2HenshinTransformation.qvto`), so files written here open in Henshin's editors and Henshin's files open here.

| Text | Henshin's model |
|---|---|
| `ePackageImport university` | `<imports href="…#/">` (the package's `.ecore` file relative to the `.henshin` file, or its nsURI for packages of OCLinEcore documents) |
| `rule r(IN a:EInt) { graph { … } }` | `Rule` with `Parameter`s and a left-hand side (`Lhs`) and right-hand side (`Rhs`) graph |
| `node n:C` (preserve) | a node in both sides, with a `Mapping` from the left to the right one |
| `delete node n:C` / `create node n:C` | a node of the left / the right side only |
| `(a->b:ref)`, `(delete …)`, `(create …)` | edges in both sides / the left / the right side |
| `x=v` on a preserved node | the attribute in both sides |
| `set x=w` (with `x=v`) | `x=v` on the left, `x=w` on the right side (Henshin's editor shows `x=v->w`) |
| `delete x=v` / `create x=v` | the attribute on the left / the right side only |
| `forbid …` elements | a negative application condition: `Not` of a `NestedCondition` whose conclusion graph is named `forbid` |
| `require …` elements | a positive application condition: a `NestedCondition` named `require` |
| `matchingFormula { formula !a AND b  conditionGraph a {…} … }` | the formula of the left-hand side (`And`, `Or`, `Xor`, `Not` over `NestedCondition`s named like the graphs); `reuse n {…}` in a condition graph is a copy of `n` (mapped from it) with further attributes |
| all application conditions of a rule | one formula, their conjunction |
| `conditions [x > 0]` | `AttributeCondition`s (`conditionText="x>0"`) |
| `checkDangling false`, `injectiveMatching false`, `javaImport p` | the rule's attributes and `javaImports` |
| `multiRule m { graph { … } }` | a multi-rule with copies of all nodes and edges of its kernel (related by `multiMappings`) and its own; `reuse n {…}` adds attributes to a copy |
| `unit u(…) { a(x) b(x) }` | a `SequentialUnit` with sub-units and `ParameterMapping`s (in both directions, by position) |
| `independent […]`, `priority […]`, `if (…) then {…} else {…}`, `for (n) {…}`, `while {…}` | `IndependentUnit`, `PriorityUnit`, `ConditionalUnit`, `IteratedUnit` (`iterations="n"`), `LoopUnit` |
| `strict false`, `rollback false` | attributes of a sequential unit |

Two things need more than one element in the model:

- **Nested blocks of units.** Henshin's units cannot nest; a block inside a unit (`{ … }`, a choice of several calls, a loop around a sequence) becomes a unit of its own, named after its place (`openAccounts_3`, `decide_else_2`), with the parameters of the enclosing unit mapped to it, and marked with the annotation `henshin_text:nested` (`block` or `inline`). Reading a model inlines these units again. A rule called twice with different arguments in one unit gets such a unit too, since Henshin maps parameters per unit, not per call.
- **Attribute values** are JavaScript in Henshin's model (`"SEMINAR"` for an enumeration literal, `x-amount`, `a && b`). Reading translates the JavaScript the textual syntax can express (literals, parameters, dotted names and calls, `! - + * / < <= > >= == != && ||`, also `===`/`!==`); enumeration literals of enumeration-typed attributes become `Kind.SEMINAR`. JavaScript it cannot express (`a ? b : c`, `null`, `%`, …) is kept as `javascript("a ? b : c")` in the text and written back unchanged; the engine reports that it cannot evaluate it.

### Files from Henshin's editors

Henshin's editors write things the textual syntax has no words for. Reading such a file:

- **Unnamed nodes** (the graphical editor does not ask for names) get names from their class: `account`, `account2`, …; names that are not names of the textual syntax (`a-b`, `2nd`) become `a_b`, `_2nd`. Writing the file again keeps the file's own names (and leaves unnamed nodes unnamed).
- **Unnamed** negative and positive application conditions become `forbid` and `require` elements, as Henshin's editor draws them; other conditions become a `matchingFormula`.
- **Parameters without a type** are allowed (`rule r(x)`), as in Henshin's model.
- **Calls without parameter mappings** (a sub-unit parameter the unit maps nothing to is unbound in Henshin) pass a new `VAR` parameter of the calling unit.
- **Ids and what the text cannot say** — `xmi:id`s, `description`s, annotations, the names of attribute conditions, `activated` flags, the names of the left- and right-hand side graphs — are kept when the file is written again: writing takes the previous file as a *template*. Elements are matched by keys made of names; a rule, unit or node that was renamed (a new name where the old one is gone, at the same position) keeps its ids. New elements get ids derived from their keys, so writing the same module twice gives the same file.

Not supported: sub-modules (ignored, with a warning), references to units or elements of other files, strict iterated units (`strict` is left out, with a warning), multi-rules with parameters of their own, and the old `henshin:TransformationSystem` format.

## The rule diagram

**Open Rule Diagram** (editor title, Explorer context menu) shows a Henshin text as diagrams; `.henshin` files open in it by default (**Show Text to the Side** shows their XML). Like the metamodel diagram it works on the document itself: the diagram is drawn from the document's text again after every change, edits in the diagram are text edits of the document, and undo, redo, saving and the dirty state are the document's.

### Henshin's notation

Each rule is a frame with its name, parameters and options (`checkDangling false`); its attribute conditions are a note in Henshin's yellow, its matching formulas and multi-rules notes too. Inside are its nodes — `«action»`, `name:Class` (parameter nodes underlined), the attributes — and its edges with `«action»` and the reference. As in Henshin's graphical editor:

- the action of an element is its stereotype: `«preserve»`, `«create»`, `«delete»`, `«forbid»`, `«require»`;
- elements of a multi-rule are marked with `*` and its path, `«delete*/dropEnrolments»`, and their nodes are drawn stacked;
- elements of a condition graph of a matching formula are marked with `#` and its name: `«forbid#heldByOther»` for a graph that is negated in a conjunction, else `«require#…»`;
- a changed attribute is `capacity=seats->seats + extra`.

The colors are those of Henshin's color modes (`org.eclipse.emf.henshin.edit/colorModes`): the *Default* mode in light themes, the *Dark* mode (same node colors, dark rules and units) in dark ones.

| Action | Line and stereotype | Fill |
|---|---|---|
| preserve | `rgb(128,128,128)` | `rgb(238,238,238)` |
| create | `rgb(0,200,0)` | `rgb(221,255,221)` |
| delete | `rgb(255,0,0)` | `rgb(255,221,221)` |
| forbid | `rgb(0,0,255)` | `rgb(221,221,255)` |
| require | `rgb(170,68,0)` | `rgb(255,238,221)` |

Units are frames too, with their control flow from the start to the end symbol: calls as invocation boxes, `independent`, `priority`, `if`/`then`/`else`, `for` and `while` as nested boxes.

### Editing

- **Node** tool: click into a rule to add a node of the toolbar's class with the toolbar's action. **Edge** tool (or the connector on a node's right edge): drag from a node to another; the reference is the one that fits (a menu asks when several do).
- The toolbar's **into** list chooses where new elements of a rule go: the rule, one of its multi-rules or condition graphs (also: **Add Elements Here** in their properties).
- Double-click names and attributes to edit them (`name=value`), **+** on a node adds an attribute, Delete removes the selection.
- The properties panel edits the rest: a rule's parameters (`IN name:Type, …`), options and attribute conditions, multi-rules; a node's class and action (changing it changes its edges and attributes so that they fit, e.g. the edges of a created node are created); an attribute's value, action and `set`; an edge's reference and action; a unit's parameters and body (in the textual syntax).
- Renaming renames the references in the document: calls of a rule, a node's edges, `reuse`s and node parameter, a condition graph in its formula. Deleting a node deletes its edges, deleting a rule its calls.

In a text, edits change as little text as possible: a new node is a line after the other nodes, a new edge goes into the last `edges [...]` list, a changed action is the keyword in place; layout and comments stay. A `.henshin` file is edited through its textual form: the edit is made on the text the file reads as, which is compiled and written back into the file (keeping its ids), and only the lines that changed are replaced. An edit that would make a `.henshin` file invalid (an unknown class, …) is refused, since such a file cannot be written.

Positions are kept in `<file>.diagram.json` next to the document (rule frames and units absolute, nodes relative to their rule), written when the user moves or lays out the diagram.

## Pairs

When `x.henshin_text` and `x.henshin` are in the same folder, the VS Code extension keeps them equivalent (setting `oxychloride.henshin.syncPairs`, on by default): when one changes — in a text editor, a rule diagram, or on disk — the engine computes the edits that make the other mean the same (`syncTransformation`), which are applied to it; saving one saves the other.

- Towards the `.henshin` file, it is written again from the text, with the file as template: ids, descriptions and the like stay, and only changed lines change.
- Towards the text, the rules and units that still mean the same keep their text — layout, comments and all; the changed ones are written anew (with the comments before them).
- While the changed document has errors, the other is not updated (the status bar and the diagram's toolbar say so).

**Create Equivalent .henshin / Henshin Text** writes the other document of a pair. From the command line: `oxy convert rules.henshin_text -e metamodel.oclinecore -o rules.henshin` and back.

Two documents of a pair mean the same, but are not the same text: a model lists the created nodes after the others and keeps all edges of a rule in one list, so a text converted from a model and back is laid out like the printer lays it out (`oxy_henshin::print`).

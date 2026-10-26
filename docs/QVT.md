# QVT model transformations: Operational Mappings and Relations

Oxychloride runs model-to-model transformations written in the two languages of the OMG's *MOF Query/View/Transformation* standard that are used in practice, as [Eclipse QVTo](https://projects.eclipse.org/projects/modeling.mmt.qvt-oml) and [Eclipse QVTd](https://projects.eclipse.org/projects/modeling.mmt.qvtd) define them:

| | File | Style | Runs |
|---|---|---|---|
| **QVT Operational Mappings** (QVTo) | `.qvto` | imperative: mappings from objects of input models to objects of output models, helpers, Imperative OCL | from its input models to its output models |
| **QVT Relations** (QVTr) | `.qvtr` | declarative: relations between object patterns of the models | in the direction of one of its models: creates (enforces) or only checks what the relations demand of it |

Both are documents of the workspace like the others: they are checked as they are edited (unknown names, types, wrong arguments, model types whose metamodels cannot be found), with hovers (signatures, the types of expressions) and completion (after `.`, `->`, `::` and `map`; variables, the features of `self`, declarations, keywords). The OCL inside them is Oxychloride's OCL — the expressions can call operations with OCL bodies and read derived features of the metamodels — extended by Imperative OCL for QVTo. A transformation runs on copies of the workspace's models (`runQvt`, `oxy qvt`, the VS Code command *Run QVT Transformation…*); the models it produces or changes come back as XMI or HUTN, with what it logged, the problems it met (located in the transformation) and the invariants the results violate.

`examples/qvt` has the same transformation in both languages — class diagrams (`uml.oclinecore`) to relational schemas (`rdbms.oclinecore`, whose invariants are checked on the results) — and a model (`company.hutn`). Both produce the same schema; this is tested.

```sh
cd examples/qvt
oxy qvt uml2rdbms.qvto -e uml.oclinecore -e rdbms.oclinecore --in source=company.hutn --out target=company.xmi
oxy qvt uml2rdbms.qvtr -e uml.oclinecore -e rdbms.oclinecore --in source=company.hutn --hutn
oxy qvt uml2rdbms.qvtr -e uml.oclinecore -e rdbms.oclinecore --in source=company.hutn --in target=company.xmi --check
oxy check uml2rdbms.qvto uml2rdbms.qvtr -e uml.oclinecore -e rdbms.oclinecore
```

## Metamodels: model types and imports

A QVTo `modeltype UML uses 'http://…';` and a QVTr `import uml : 'http://…';` name metamodels by URI: the nsURI of a loaded package (an `.ecore` file, an OCLinEcore document), a URI of the [package registry](../README.md) (also placeholders such as `platform:/plugin/…` URIs), or the path of a metamodel file relative to the transformation (loaded when the document is opened). QVTo also accepts packages by name (`uses simpleuml`) and both (`uses simpleuml('http://…')`); QVTr selects packages inside the resource with `'…'::pkg::sub`. A diagnostic names a URI that cannot be found, so the VS Code extension offers to register a file for it.

The name of a model type (QVTo) or import (QVTr) qualifies the classes of its packages: `UML::Class`, `UML::sub::Class`. Unqualified names are looked up in the transformation's metamodels first, then in all loaded ones (an ambiguous name must be qualified).

## QVT Operational Mappings

The grammar is Eclipse QVTo's (`QVTOParser.gi`, `ImperativeOCL.gi`); the expressions are OCL in the parser's QVT dialect (`//` comments, double-quoted strings, `:=`, `+=`, `!=`).

```
modeltype UML uses 'http://www.example.org/qvt/uml';
modeltype RDBMS uses 'http://www.example.org/qvt/rdbms';

transformation uml2rdbms(in source : UML, out target : RDBMS);

main() {
	source.rootObjects()[UML::Package]->map packageToSchema();
}

mapping UML::Package::packageToSchema() : RDBMS::Schema {
	name := self.name;
	tables := self.elements[UML::Class]->map classToTable();
}
```

Supported:

- **Transformations**: `transformation T(in a : A, out b : B, inout c : C)` (parameters may be unnamed: `out RDBMS`, then the model type names them), either followed by `;` and the elements, or with the elements in braces; `main()`.
- **Mappings**: `mapping [inout] Ctx::name(params) : Result` with `when { … }` (a guard: a mapping whose guard does not hold is not applied and gives `null`; with `xmap` it is an error), `where { … }` (checked afterwards), `init { … }`, the population (`population { … }` or just the statements) and `end { … }`; `inherits`, `merges`, `disjuncts` (the first disjunct whose context type and guard fit), `abstract`. A mapping is called with `map` (`self.map m()`, `coll->map m()` for each element, `map m()` without a source) or `xmap`, or like an operation (`self.m()`, old style).
- **Semantics as in Eclipse QVTo**: calling a mapping again with the same source and arguments returns the result of the first call (the trace); after `init`, a result that is still `null` is created (in the output model whose model type has its class, or the one named with `@`); in the population (without the `population` keyword) the result is the implicit object — `name := self.name` sets `result.name`, and names are looked up in it before `self`; contextual mappings and helpers are chosen by the runtime type of the source, the most specific first, overloads by the types of the arguments (the runtime types: Eclipse uses the static ones) and are not applied to a `null` source; in an `object`'s body (and a population) only the object's properties are implicit, not its operations. Objects of `in` models cannot be changed; `deepclone()` and `clone()` copy into the output models. Feature names that are keywords are escaped with `_` (`_abstract`).
- **Old-style transformations** (`transformation T;` with a `mapping main(in model : P) : R`): the input is bound to `model`, the output model holds the result of `main`.
- **Helpers and queries**: `query [Ctx::]name(params) : T = expr;` or `{ … return e; }` (a block without `return` gives its last value), called with or without a source like OCL operations.
- **Properties**: `property x : T = e;` (assignable, also as `this.x`), `configuration property x : T;` (set when the transformation is run), `intermediate property Ctx::x : T [= e];` (a property of the objects of a class, during the run).
- **Imperative OCL**: `var x : T := e` (also `=`, several at once), assignments `x := e` and `x += e` (to variables and properties; `+=` adds to collections and many-valued properties), blocks, `if (c) { … } elif (c) { … } else { … } [endif]` and `if c then e [elif …] [else e] endif` (the `else` is optional), `while (c) { … }` (also with a declaration), `coll->forEach(x | c) { … }` and `forOne`, `switch { case (c) e; … else e; }`, `compute (x : T := e) { … }`, `return`, `break`, `continue`, `log(message[, value]) [when c]`, `assert [warning|error|fatal] (c) [with log(…)]` (a fatal one ends the transformation), `List(T)` (a `Sequence`; `list->add(x)` adds to the variable), the iterators `xselect`, `xcollect`, `selectOne`, `collectOne`, `collectselect` (also `collectselect(i; r = e | c)`), the shorthands `coll[T]` and `coll[x | c]` (`xselect`), `coll![…]` (`selectOne`) and `coll->name` (`xcollect`).
- **Objects**: `object [x :] T[@model] { … }` (an existing object in `x` is updated, not created; `object x : { … }` creates one of `x`'s declared type if `x` is `null`; in its body the object is the implicit source), `new T()`.
- **Resolve**: `resolve`, `resolveone`, `resolveIn(Ctx::m, …)`, `resolveoneIn`, and the inverse `invresolve…`, with an optional type and condition (`x.resolveone(t : T | t.name = 'a')`), on single objects and (element-wise) on collections, without a source over all applications of the mapping; `late resolve…` in an assignment is done at the end of the transformation.
- **Library**: on models `rootObjects()`, `objects()`, `objectsOfType(T)`, `objectsOfKind(T)`, `removeElement(o)`; on objects `container()`, `subobjects()`, `allSubobjects()`, `subobjectsOfType(T)`, `subobjectsOfKind(T)`, `allSubobjectsOfType(T)`, `allSubobjectsOfKind(T)`, `deepclone()`, `clone()`, `metaClassName()`, `allInstances(T)` (the contents of that type, old style); on strings `firstToUpper()`, `lastToUpper()`, `toUpper()`, `toLower()`, `length()`, `substringBefore(s)`, `substringAfter(s)`, `replace(a, b)`, `match(regex)`, `asInteger()`, `asFloat()`, `asBoolean()`, `quotify(q)`, `unquotify(q)`, `isQuoted(q)`, `normalizeSpace()`, `format(values)`; `repr()` on everything — besides OCL's standard library.

Not supported (reported as such): libraries, `import`, `extends`/`access` of other transformations, intermediate classes, constructors, `Dict`, exceptions (`try`, `raise`), `blackbox`, tags, `rename`, `refines`, mappings with several results, `->object(i) T {…}` and `->switch(…)`, `modeltype … where { … }` conditions (ignored with a warning).

## QVT Relations

The grammar is Eclipse QVTd's (`QVTrelation.xtext`).

```
import uml : 'http://www.example.org/qvt/uml';
import rdbms : 'http://www.example.org/qvt/rdbms';

transformation uml2rdbms(source : uml, target : rdbms) {
	key rdbms::Table { schema, name };

	top relation ClassToTable {
		cn : String;
		checkonly domain source c : uml::Class { namespace = p : uml::Package {}, persistent = true, name = cn };
		enforce domain target t : rdbms::Table { schema = s : rdbms::Schema {}, name = cn };
		when { PackageToSchema(p, s); }
		where { AttributeToColumn(c, t); }
	}
	…
}
```

Supported: imports, parameters with one or more metamodels (`uml : {a, b}`), `key C { p, opposite(D.q) };`, `top`, `abstract` and `overrides` relations, variable declarations (`a, b : T [= e];`), `checkonly`, `enforce` (`replace` is treated like `enforce`) and plain domains with one or more patterns, `primitive domain x : T`, `default_values { x = e; }`; object templates `v : C { p = value, … } { condition }` whose values are expressions or nested templates, `opposite(D.q) = …`, collection templates `s : Set(C) { a, b ++ rest }` (`_` ignores); `when` and `where` clauses of relation calls (`R(a, b)`, `not R(a, b)`), assignments of unbound variables (`x = e`) and conditions; `query q(params) : T { e }`.

Running in the direction of a parameter (the **target**; with one enforced parameter it is the default):

1. The top relations are applied over and over until nothing changes. For each, every binding of its variables under which the patterns of its **source domains** (those of the other parameters) match and its `when` clause holds is enforced. Matching a template binds its variable to each candidate (the objects of its class in the parameter's model, or the values of the property it is the value of) and checks or binds its properties: a property whose value is a single unbound variable binds it (to each element of a many-valued property), other expressions are compared, nested templates match the property's values. A relation call in `when` holds where a binding of that relation (or of one overriding it) has been recorded, and binds the variables it is given.
2. **Check before enforce**: the pattern of the target domain is matched in the target model with these bindings. If it matches, the relation holds and nothing changes. Otherwise it is enforced: objects bound to variables are changed, objects a `key` identifies (by the values the template gives the key's properties — the property opposite to the one a template is the value of is its parent) are reused, the others are created; then the template's properties are set (single-valued ones replaced, values added to many-valued ones). `where` assignments that only need the source bindings (`sqltype = SqlType(pn)`) and `default_values` are evaluated first, since the target's templates may use them.
3. The binding is recorded (the trace), and the `where` clause applied: its relation calls apply those relations with the given roots (an abstract relation: the first relation overriding it that matches), its assignments bind, its conditions are checked. The `where` clause is applied again in the following rounds, so relations it calls can apply once the relations their `when` clauses need hold.

Enforcing again changes nothing (check before enforce, keys). In **check mode** nothing is created: every binding of the source domains under which the target pattern does not match is reported (`relation ClassToTable does not hold for c = Class 'Company'`).

Not supported: deleting objects of the target model that no relation demands (enforcing only creates and changes), `target` declarations (ignored), `extends`, properties and operations of transformations, `implementedby`, `dependsOn`/`iterates` (ignored with a warning).

## How it is built

QVT expressions are OCL, and both languages are built on `oxy-ocl` rather than beside it (see [ARCHITECTURE.md](ARCHITECTURE.md#oxy-qvt)):

- The OCL parser has a **QVT dialect** (`Dialect::Qvt`): the lexer reads `//` comments, double-quoted strings, `:=`, `+=`, `!=` and `++`, and expressions include Imperative OCL and QVTo's expressions (`ExprKind::Block`, `Assign`, `While`, `ForEach`, `Switch`, `Object`, `MapCall`, `Resolve`, …). The `Parser` is public: the QVTo and QVTr document parsers (`oxy-qvt`) run on the same token stream and call it for types and expressions.
- The **checker and the evaluator** handle Imperative OCL themselves and ask the language for the rest through hooks: a `CheckHost` (types of `this`, model parameters, properties, helpers and queries, mappings, extents) and a `Host` (their values, mapping calls, creating objects, the candidates of `resolve`, deferring `late` assignments, logging, which objects may be changed). Model types are `PackageAlias`es of the type lookup. Names are resolved in the same order by both: variables, the host's names, properties of the implicit sources (iterators, the object being populated, `self`), types.
- The evaluator can change the model it evaluates on (`Evaluator::new_mut`): assignments set features as EMF does (`Model::insert`/`set`: containment moves, opposites, single-valued replacement). Control flow (`return`, `break`, `continue`, fatal assertions) is a pending state that blocks and loops take, and problems of statements (failed assignments, invalid values) are collected with their spans (`take_errors`).
- A **QVTo** run is the evaluator's host: it keeps the trace, the extents (model parameters bound to resources of the model), the properties and the deferred `late` assignments; mapping calls run their sections in a call frame of the evaluator.
- A **QVTr** run matches templates and enforces them itself, evaluating the OCL inside with a host for queries (and relation calls used as conditions).

Recursion (operations, mappings, relation calls) is bounded by a depth, not by the thread's stack: the evaluator and the checker grow the stack when it runs low.

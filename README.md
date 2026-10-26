# Oxychloride

An OCL (Object Constraint Language) engine for Ecore models, written in Rust: a re-implementation of the core of [Eclipse OCL](https://eclipse.dev/ocl/) that runs without a JVM.

- **Library** — load `.ecore` metamodels and XMI models, parse and type check OCL, evaluate expressions, validate invariants and pre-/postconditions.
- **Textual modelling** — write metamodels in **OCLinEcore** (Ecore classes with embedded OCL) and models in **HUTN** (the OMG Human-Usable Textual Notation, a generic syntax for instances of any metamodel); convert both to and from `.ecore`/XMI.
- **Graph transformations** — in-place model transformations with **Henshin**, written in its textual syntax (`.henshin_text`) or as the `.henshin` models of Henshin's graphical editors — equivalent, converted into each other and kept in sync in pairs: rules with application conditions, attribute conditions and multi-rules, units, step-by-step application and state space exploration, with every state checked against the OCL constraints. See [docs/HENSHIN.md](docs/HENSHIN.md).
- **Code generation** — native **Python** packages and **Rust** crates from a metamodel, like EMF's generated Java: classes with consistent containment and opposites, XMI loading and saving, and the OCL (invariants, operation bodies, pre- and postconditions, derived features, definitions) translated into native code that does not need the engine. See [docs/CODEGEN.md](docs/CODEGEN.md).
- **CLI** (`oxy`) — check documents, run constraints against models, evaluate expressions, check the OCL embedded in metamodels, convert between XML and text formats, generate code.
- **Integration API** — a JSON-RPC protocol over stdio (`oxy serve`), the same protocol through a C ABI (`liboxychloride_ffi`), and a Rust API, so other modelling tools can validate their models with OCL.
- **VS Code extension** — the same in VS Code, through a language server (`oxy lsp`): OCLinEcore metamodels, HUTN and XMI models checked live against all their OCL constraints, a graphical **metamodel diagram** for `.ecore` and `.oclinecore` files that edits the same document as the text editor beside it, an OCL console, metamodel/model views that create instances, XML Schemas so the XML extension validates XMI files, code generation from the context menu, and Henshin transformations with highlighting, completion, a graphical **rule diagram** in Henshin's notation and colors (for texts and `.henshin` files, which are kept equivalent), and a view that applies rules step by step and draws state spaces.

## Quick start

```sh
cargo build --release
B=target/release/oxy

# Validate a model against a Complete OCL document and the OCL embedded in its metamodel.
$B run examples/library/library.ocl --model examples/library/library.xmi

# Type check documents (exit code 1 on errors).
$B check examples/library/library.ocl

# Evaluate an expression; `self` is the object at the given URI fragment.
$B eval "books->select(copies > 1).name" -m examples/library/library.xmi -c '//'

# Check the OCL embedded in an .ecore file (and validate a model against it).
$B ecore examples/library/library.ecore -m examples/library/library.xmi

# Print a metamodel outline, or serve JSON-RPC on stdio.
$B info examples/library/library.ecore
$B serve

# A metamodel in OCLinEcore and a model in HUTN: validate, then export as .ecore and XMI.
$B run examples/university/university.oclinecore examples/university/university.hutn
$B convert examples/university/university.oclinecore -o university.ecore
$B convert examples/university/university.hutn -e examples/university/university.oclinecore -o university.xmi

# And the other way: .ecore to OCLinEcore, XMI to HUTN.
$B convert examples/library/library.ecore
$B convert examples/library/library.xmi -e examples/library/library.ecore

# Generate a Python package (or a Rust crate) with the OCL as native code.
$B generate examples/library/library.ecore -d examples/library/library.ocl -t python -o generated
$B generate examples/university/university.oclinecore -t rust -o generated
```

Model files referenced by a model (e.g. a PCM allocation → system → repository) are loaded automatically; use `--map PREFIX=DIR` for `pathmap://` URIs, e.g. `--map pathmap://PCM_MODELS/=path/to/PCM_MODELS/`. `platform:/plugin/...` references between `.ecore` files resolve to a file of the same name next to the referencing file.

Wherever a metamodel or model file is expected (`-e`, `-m`, documents), `.oclinecore` and `.hutn` files work too. Every command accepts `--format json`. Exit codes: `0` success, `1` problems found (OCL errors, violated constraints), `2` the command itself failed (unreadable file, bad arguments).

```
✗ library::Book::SufficientCopies (library.ecore)
    violated on Book 'A Brief History of Time' [library.xmi#//@books.1]
✗ library::Book::HasIsbn (examples/library/library.ocl:16)
    violated on Book 'The Guns of August' [library.xmi#//@books.2]: Book 'The Guns of August' has no ISBN
✗ library::Member::Adult (examples/library/library.ocl:20)
    violated on Member 'Bob' [library.xmi#//@members.1]: Member Bob is 16 years old

✗ 3 violations, 0 evaluation errors (9 constraints on 12 objects, 26 evaluations, 23 satisfied)
```

## Using it from other tools

**Rust** — depend on the `oxychloride` crate:

```rust
use oxychloride::{ValidateOptions, Workspace};

let mut ws = Workspace::new();
ws.load_ecore_file("library.ecore")?;
ws.load_model_file("library.xmi")?;
ws.open_document("rules.ocl", "context library::Member inv Adult: age >= 18");
let report = ws.validate(&ValidateOptions::default())?;
let value = ws.evaluate("library::Book.allInstances()->size()", None)?;

// Metamodels and models as text: the language follows the extension.
ws.open_document("shop.oclinecore", "package shop : shop = 'http://shop' { class Item { attribute price : Integer; invariant Positive: price > 0; } }");
ws.open_document("items.hutn", "shop { Item \"pen\" { price: -1 } }");
let ecore = ws.export_document("shop.oclinecore", None)?.text;
```

**Any language, out of process** — spawn `oxy serve` and exchange newline-delimited JSON-RPC messages. **Any language, in process** — link `liboxychloride_ffi` (`cargo build -p oxy-ffi --release`) and call `oxy_session_call(session, "validate", "{}")`; see [`crates/oxy-ffi/include/oxychloride.h`](crates/oxy-ffi/include/oxychloride.h). Both speak the protocol described in [docs/PROTOCOL.md](docs/PROTOCOL.md).

## Metamodels and models as text

**OCLinEcore** (`.oclinecore`) is the textual Ecore syntax of Eclipse OCL: packages, classes, data types and enumerations with their features and operations, and OCL inside them — invariants (with messages), `derivation`s and `initial` values of properties, operation `body`s, `precondition`s and `postcondition`s.

```
package university : uni = 'http://www.example.org/university/1.0' {
  class Student {
    attribute name : String;
    property enrolments#student : Enrolment[*] { composes };
    property credits : Integer { derived, readonly, transient, volatile } {
      derivation: enrolments.course.credits->sum();
    }
    operation mayEnrol(course : Course) : Boolean {
      precondition NotEnrolled('already enrolled in ' + course.code): not enrolments.course->includes(course);
      body: course.prerequisites->forAll(p | passed(p)) and course.hasSeat();
      postcondition: result implies course.prerequisites->forAll(p | passed(p));
    }
    invariant CreditLimit('Student ' + name + ' takes ' + credits.toString() + ' credits'): credits <= 30;
  }
}
```

As in Eclipse OCL, a missing multiplicity means `[1]` (write `[?]` for optional), multi-valued features are unordered unless qualified `ordered`, and `Integer`/`Real` map to `EBigInteger`/`EBigDecimal` (other Ecore types are available as `ecore::EInt` etc.). The OCL is stored in the metamodel the way Eclipse stores it in `.ecore` files, so `oxy convert` (or *Export* in the app) produces `.ecore` files Eclipse tools can read. Not supported: generic types, `static` features, nested annotation contents.

**HUTN** (`.hutn`) is the OMG Human-Usable Textual Notation for instance models — generic, so it works with any metamodel:

```
@Spec { metamodel "university" { nsUri: "http://www.example.org/university/1.0" } }

university {
  University "tum" {
    name: "Technical University"
    courses: Course "prog1" { code: "IN0001" credits: 8 capacity: 3 }
    students: Student "ada" { name: "Ada" enrolments: Enrolment { course: Course "prog1" grade: 1 } }
  }
}
```

A package block (named by package name, nsPrefix or `@Spec` name) holds root objects `Class "identifier" { feature: value, ... }`. Containment features take nested objects; other references refer to objects by `Class "identifier"` (or just `"identifier"`). Values are strings, numbers, `true`/`false`, `null` and enumeration literals; multi-valued features take comma-separated lists. Boolean attributes can also be set as adjectives (`active Member "x" {}`, `~active` for `false`). Identifiers become `xmi:id`s. Errors (unknown classes or features, type mismatches, unresolved references, unset required features) are reported with exact ranges; once a model is free of errors, the constraints its objects violate are reported on the objects too.

## Graph transformations (Henshin)

**Henshin** documents describe in-place graph transformations of models, as in [Eclipse Henshin](https://www.eclipse.org/henshin/): in its textual syntax (`.henshin_text`, the `org.eclipse.emf.henshin.text` grammar), or as the models its graphical and tree editors write (`.henshin`). The two mean the same — a `.henshin` file is read into the syntax tree of the text and compiled by the same compiler, and written from the compiled rules — so either can be run, converted into the other, and edited as a rule diagram; a text and a `.henshin` file of the same name are kept equivalent (see [docs/HENSHIN.md](docs/HENSHIN.md)):

```
ePackageImport university

rule enrol(IN student:Student, IN course:Course) {
	graph {
		node student:Student
		node course:Course
		create node enrolment:Enrolment
		edges [(create student->enrolment:enrolments), (create enrolment->course:course)]
		forbid node existing:Enrolment
		edges [(forbid student->existing:enrolments), (forbid existing->course:course)]
	}
}

unit enrolEverywhere(IN student:Student, VAR course:Course) {
	while { enrol(student, course) }
}
```

Nodes and edges are preserved, created or deleted; `forbid` and `require` elements are negative and positive application conditions, and a `matchingFormula` combines named condition graphs with `AND`, `OR`, `XOR` and `!`. Attributes are matched (`name=param`; an unbound parameter is bound by the match), created, or changed (`set capacity=seats + extra`); `conditions [...]` constrain parameters. Multi-rules apply to all matches extending the kernel match, in the same step. Units combine rules: sequences (`strict`, `rollback`), `independent`, `priority`, `if`/`then`/`else`, `for` and `while`. Matching is injective unless `injectiveMatching false`, and with `checkDangling` (the default) a node is only deleted together with all its links.

Transformations run on a copy of a model in a session (see `startTransformation` and the other methods in [docs/PROTOCOL.md](docs/PROTOCOL.md)): rules are applied at a chosen match or units executed, step by step, each step reporting what it created, deleted and changed and which invariants the result violates; states export as HUTN or XMI. `exploreStateSpace` computes all states a set of rules reaches (merging isomorphic states), with the terminal ones and those violating invariants. `examples/philosophers` is the dining philosophers, whose state space has a deadlock that an OCL invariant points out.

In Henshin's models, attribute expressions are JavaScript; the engine evaluates the expression language of the textual syntax (literals, parameters, arithmetic, comparisons, `AND`, `OR`, `!`) with enumeration literals as `Enum.LITERAL`, which `.henshin` files are translated from and to. Java calls and attributes, and JavaScript the textual syntax cannot express (kept as `javascript("…")`), are not evaluated.

```sh
$B convert examples/university/university.henshin_text -e examples/university/university.oclinecore -o university.henshin
$B convert examples/university/university.henshin -e examples/university/university.oclinecore   # back to text
$B check examples/university/university.henshin -e examples/university/university.oclinecore
```

**Pre- and postconditions** are checked in two ways: whenever an operation with an OCL body is called during evaluation (in invariants, bodies, derivations or the console), its preconditions are checked before and its postconditions after the call (with `result` bound; `@pre` values are the current values, since OCL evaluation has no side effects); and validation calls every operation that has postconditions on every applicable object, with every combination of arguments from finite domains (Booleans, enumeration literals, model objects), skipping calls whose preconditions do not hold. Operations with other parameter types are only checked where they are called; the validation report says so.

## VS Code extension

See [vscode/README.md](vscode/README.md). In short: `cargo build -p oxy-cli`, then open `vscode/` in VS Code and press F5 (or `cd vscode && npm install && npm run package` for a `.vsix` with the language server inside).

## Repository layout

| Path | |
|---|---|
| `crates/oxy-ecore` | Ecore metamodels (`.ecore` loading, embedded `Ecore.ecore`), dynamic instance models, XMI loading, HUTN, XML Schema generation |
| `crates/oxy-ocl` | OCL lexer, parser (expressions and Complete OCL documents), type checker, evaluator, standard library, validation |
| `crates/oxy-henshin` | Henshin's textual syntax and `.henshin` models (reading, writing, JavaScript expressions), rules and units resolved against metamodels, graph matching and transformation, state space exploration |
| `crates/oxy-codegen` | Code generation: the code model of a metamodel with its OCL lowered, the Python and Rust targets and their runtimes |
| `crates/oxychloride` | `Workspace` API, JSON protocol types, JSON-RPC server, editor support (hover, completion), metamodel and rule diagrams and their edits, transformation sessions, converting and syncing the two syntaxes of Henshin |
| `crates/oxy-cli` | The `oxy` command line tool |
| `crates/oxy-ffi` | C ABI (`cdylib`/`staticlib`) |
| `crates/oxy-lsp` | Language server (`oxy lsp`) |
| `vscode/` | VS Code extension (TypeScript; grammars, views, OCL console, metamodel diagram, transformation view) |
| `assets/file-icons` | Icons of the document kinds (the extension ships a copy) |
| `examples/library` | Example metamodel with embedded OCL, model (XMI and HUTN), and Complete OCL document |
| `examples/university` | Example metamodel in OCLinEcore (with pre- and postconditions), model in HUTN, and Henshin transformations (as text and as the equivalent `.henshin` model) |
| `examples/philosophers` | The dining philosophers: metamodel, model and Henshin rules (text and `.henshin`) for state space exploration |
| `docs/` | [Protocol](docs/PROTOCOL.md), [architecture](docs/ARCHITECTURE.md), [code generation](docs/CODEGEN.md) and [Henshin](docs/HENSHIN.md) notes |

`cargo build` / `cargo test` cover the engine, the CLI, the C ABI and the language server; the extension is built with npm (see [vscode/README.md](vscode/README.md)).

## OCL support

Implemented:

- Complete OCL documents: `import`, `package … endpackage`, `context` for classifiers (with `self` alias), operations and properties; `inv` (with names and messages), `def` (attributes and operations), `pre`/`post` (checked at calls and by validation, see above), `body`, `init`, `derive`.
- OCLinEcore documents and the OCL embedded in `.ecore` files (Pivot and legacy annotation sources): invariants, operation bodies, pre- and postconditions, derivations, initial values, OCLinEcore invariant messages, EMF "invariant operations".
- OCL embedded in `.ecore` files (Pivot and legacy annotation sources): invariants, operation bodies, derivations, initial values, OCLinEcore invariant messages, EMF "invariant operations".
- Expressions: literals (including `*`, collection literals with ranges, tuples), `let`, `if … elseif … endif`, navigation with implicit `self` and implicit iterators, implicit collect, safe navigation (`?.`, `?->`), `oclAsType`/`oclIsKindOf`/`oclIsTypeOf`/`oclType`, `allInstances()`, enum literals, `oclContainer()`/`oclContents()`.
- Standard library: Boolean, Integer/Real (checked 64-bit integers), String (incl. regex `matches`/`replaceAll`), all collection kinds and their operations, iterators (`select`, `reject`, `collect`, `collectNested`, `forAll`, `exists` — with multiple iterators —, `any`, `one`, `isUnique`, `sortedBy`, `closure`, `iterate`).
- Both classic (LPG) and Pivot-style idioms, e.g. `collection.oclAsType(T)` is applied element-wise as in classic Eclipse OCL. Tested against the Palladio Component Model: all 82 invariants embedded in `pcm.ecore` type check, and 112 real PCM model sets validate with no evaluation errors.
- Three-valued logic with `null`/`invalid` propagation; evaluation never crashes — failures become `invalid` with an explanation.
- Static type checking with precise locations; error recovery so one mistake doesn't hide the rest.

Not yet implemented (roadmap):

- Operations with side effects (`@pre` is only meaningful for queries, where it equals the current value) and `oclIsNew()`.
- `Map` types and literals, `Lambda`, `OclMessage` (`^`, `^^`), association-class navigation and qualifiers, unnamed opposite navigation.
- Unbounded integers (Integer is 64-bit; overflow yields `invalid`), `UnlimitedNatural` arithmetic.
- Reflective access to Ecore elements as objects (e.g. `self.eClass().eAllAttributes`); `eClass()`/`oclType()` return types, of which only `name` is available.
- XMI: feature maps, `href`s to resources that are not loaded, `xsi:type`-less proxies.
- HUTN: association blocks, references across documents, configuration models (identifier rules, default values).
- Static operations; `def`s without a declared type are not type checked at their use sites.

---
name: run-oxychloride
description: Build, run, test and drive Oxychloride (the OCL engine, its `oxy` CLI and the VS Code extension with its metamodel and Henshin rule diagram editors). Use when asked to run the CLI or the extension, check a change in the real UI, take a screenshot of the metamodel or rule diagram, try OCLinEcore/HUTN/Complete OCL/.ecore documents, validate models, run QVT transformations, or run the tests.
---

Oxychloride is an OCL engine with a CLI (`oxy`) and a VS Code extension (`vscode/`, a client of `oxy lsp`). Engine changes are checked with the CLI and `cargo test`; the extension with its integration tests (a real VS Code); the metamodel diagram's page is driven with
`.Codex/skills/run-oxychloride/driver.mjs`, which serves the real page (the built webview bundle) to headless Chromium, stands in for its VS Code host and connects it to the real engine (`oxy serve`, the same `rpc::Server` that `oxy lsp` wraps).

All paths are relative to the repo root. Verified on macOS (darwin) with Rust 1.97 and Node 24; nothing here has been tried on Linux yet.

## Setup

```bash
cargo build -p oxy-cli                                   # target/debug/oxy (CLI, language server, `oxy serve`)
(cd vscode && npm install && npm run build)              # the extension and its webview bundles (out/)
npm install --prefix .Codex/skills/run-oxychloride      # the driver's Playwright
(cd .Codex/skills/run-oxychloride && npx playwright install chromium)
```

## Run: engine and CLI (most changes)

```bash
cargo run -q -p oxy-cli -- run examples/library/library.ocl -m examples/library/library.xmi --no-color
# ✗ 3 violations ... (9 constraints on 12 objects, 26 evaluations, 23 satisfied); exit code 1
cargo run -q -p oxy-cli -- run examples/university/university.oclinecore examples/university/university.hutn --no-color
# ✗ 5 violations ... (8 constraints on 16 objects, 30 evaluations, 25 satisfied); exit code 1
cargo run -q -p oxy-cli -- qvt examples/qvt/uml2rdbms.qvto -e examples/qvt/uml.oclinecore -e examples/qvt/rdbms.oclinecore --in source=examples/qvt/company.hutn --hutn
# prints the relational schema (HUTN); stderr: 10 mappings applied, 13 objects created (.qvtr: 8 relations applied)
```

Other commands (`check`, `eval`, `convert`, `info`, `serve`) are in README.md; QVT in docs/QVT.md. Tests:

```bash
cargo test                                  # engine, CLI, FFI, language server
cargo test -p oxychloride --test diagram    # metamodel diagrams and their text edits only
cargo test -p oxy-qvt                       # QVT Operational Mappings and Relations
cargo clippy --all-targets                  # must be warning-free
(cd vscode && npm run typecheck && npm test)   # extension + webviews; integration tests in a downloaded VS Code (~10 s)
```

## Run: metamodel diagram (agent path)

The driver opens a metamodel in the diagram, then executes commands from stdin, one per line, and shuts everything down on `quit`. The document is kept in memory (edits never touch the example files unless you `save`); its `<file>.diagram.json` is read if it exists, and the layout the page asks to write is kept in memory (`layout`).

```bash
cargo build -p oxy-cli && (cd vscode && npm run build)    # after any engine or webview change
node .Codex/skills/run-oxychloride/driver.mjs <<'EOF'
open examples/university/university.oclinecore
ss university
click button[title^="Add a class"]
clickat 860 140
wait .inline-edit input:focus
type Person\n
click button[title^="Add a supertype"]
connect @Student @Person
doc extends
connect @University @Person
doc person :
dblclick @Course .row:has-text("capacity")
press Meta+a
type capacity : Integer [0..1]\n
doc capacity :
edit-text class Enrolment => class Registration
cursor 21 4
ss edited
quit
EOF
```

This prints the engine's edits as they happen (`[edit] addClassifier: 1 text edit(s) → university::NewClass`, `[edit] update: … → university::Person`, `[edit] addSuperType`, `[edit] addReference: … → university::University.person`), and `doc` shows the text they made (`class Student extends Person {`, `property person : Person[?];`, `attribute capacity : Integer[?];`). Screenshots land in `.Codex/skills/run-oxychloride/screenshots/<name>.png` (git-ignored); **open them**. `--light` uses a light theme instead of a dark one; `.ecore` files work the same (`open examples/library/library.ecore`).

Commands (`help` prints them): `open <path>`, `ss <name>`, `click`/`dblclick <selector>`, `clickat <x> <y>`, `down`/`move <x> <y>`, `up`, `type <text>` (`\n` = Enter), `press <key>`, `drag <selector> <dx> <dy>`, `connect <from> <to>`, `wait`/`text`/`count <selector>`, `doc [regex]`, `edit-text <old> => <new>` (a change in the text editor), `cursor <line> <column>` (the text cursor moved), `layout`, `save <path>`, `js <expression>`, `quit`. In selectors, `@Name` is the node of classifier `Name`. A failing command prints `[failed] ...` and the session continues; page errors print as `[page error]`/`[console error]`; messages the page sends to VS Code print as `[reveal]`, `[error]`, ….

Useful selectors: `.react-flow__node[data-id="university::Course"]` (= `@Course`), `@Course .row:has-text("title")`, `@Course .compartment[data-title="attributes"] .add`, `@Course header` (double-click renames), `.react-flow__edge[data-id="university::Course.enrolments"]` (references by feature id; generalizations `university::Student extends university::Person`), `.properties h2`, `.notice` (error toasts), `button[title^="Add a class"]` and the other toolbar titles.

## Run: rule diagram (Henshin)

The same driver opens Henshin transformations — texts (`.henshin_text`) and Henshin's models (`.henshin`) — in the rule diagram; it loads the `.oclinecore`/`.ecore` metamodels next to the document into the engine first. Edits go through `editTransformation`:

```bash
node .Codex/skills/run-oxychloride/driver.mjs <<'EOF'
open examples/university/university.henshin_text
ss rules
click button[title^="Add a node"]
clickat 395 372
wait .inline-edit input:focus
type campus\n
doc campus
quit
EOF
```

This prints `[edit] addNode: 1 text edit(s) → drop::university`, `[edit] rename: … → drop::campus` and the line `node campus:University`. Rule frames are `.react-flow__node[data-id="rule:<rule>"]`, units `unit:<unit>`, nodes their ids (`drop::campus`; `@campus` matches any rule's node of that name). Where to `clickat` depends on the fitted zoom: ask the page (`js document.querySelector('.react-flow__node[data-id="rule:drop"]').getBoundingClientRect().toJSON()`). Edges: `down` on a node's right edge (its connector), `move` to another node, `up`. With `open examples/university/university.henshin` the same edits change the XML instead (`doc campus` shows `<nodes … name="campus">`).

## Run: in VS Code (human path)

Open `vscode/` in VS Code and press F5, or `code --extensionDevelopmentPath=$PWD/vscode examples/university`; open `university.oclinecore` and click **Open Metamodel Diagram** in the editor title. It was not run interactively while writing this skill — the integration tests (`npm test`) cover the diagram's host (opening it beside the text, edits through the shared document, undo from the diagram, `.ecore` files) and the driver covers its page.

## Gotchas

- Coordinates for `clickat`/`down`/`move` are CSS pixels of the 1440×900 page; screenshots are 2× (2880×1800), so halve what you measure on them. The properties panel takes the right 290 px; click into empty canvas (e.g. `clickat 860 140` on the university diagram) to place a classifier.
- A node added to the diagram is invisible until React Flow has measured it, so its inline name field takes the focus a few frames late: `wait .inline-edit input:focus` before `type`, or the first letters go nowhere (a renamed `rson` instead of `Person`).
- With a connection tool active the whole node starts a connection (the connector dot is hidden); `connect` handles both. Without a tool, `connect` drags from the dot on the node's right edge (a plain reference).
- `edit-text` and `cursor` simulate the text editor; the real host's behaviour around them (debouncing, applying `WorkspaceEdit`s, undo) is only exercised by `npm test`.
- `.ecore` diagrams are computed from the given text in a metamodel of their own, with referenced files loaded from disk relative to the opened path — keep the path real.
- `npm test` downloads VS Code into `vscode/.vscode-test` the first time (set `VSCODE_PATH` to use an installed one) and prints a lot of unrelated VS Code log noise; look for the `N passing` line.

## Troubleshooting

Errors hit while building this harness:

- `[failed] wait @Room: locator.waitFor: Timeout` after `type Room\n` → the rename went nowhere (see the focus gotcha); the class is still `NewClass`.
- `[failed] connect @Student @Person: locator.boundingBox: Timeout` → the node does not exist (an earlier step failed, often the rename above).
- A drag that pans the canvas instead of connecting → the pointer was not on the connector dot (positions settle after the initial `fitView`; `open` waits for that) or a connection tool covered the node; use `connect`.
- `[edit failed] removeSuperType: '…' is not a supertype of '…'` right after another removal → the same edge was both selected and focused; fixed in the page (deduplicated), so seeing it again means a regression.

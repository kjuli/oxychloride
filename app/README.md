# Oxychloride desktop editor

A Tauri v2 app around the Oxychloride engine. Edit three kinds of documents with live
diagnostics, completion and hover:

- **Metamodels in OCLinEcore** (`.oclinecore`): classes with invariants, derived
  properties, operation bodies, pre- and postconditions. The export button generates the
  `.ecore` file next to the document (optionally on every save) or exports it elsewhere.
- **Models in HUTN** (`.hutn`): completion proposes what the metamodel allows (features,
  literals, nested classes, objects to refer to), and the constraints the model violates are
  shown on its objects as you type. Export them as XMI.
- **Complete OCL documents** (`.ocl`) for existing `.ecore` metamodels.

Browse metamodels and models, validate (invariants and pre-/postconditions) and evaluate
expressions against model objects. *New* (toolbar or tab strip) creates any of the three
from a template, or converts an `.ecore` file or XMI model to text. Right-click a class in the
sidebar to create an instance of it in the current model (or a new one), or an object of a
model to add a child to one of its containment features.

- Frontend: Vite + React + TypeScript + CodeMirror 6 (`src/`). `src/api.ts` is the typed
  wrapper around the engine's JSON-RPC methods (`crates/oxychloride/src/rpc.rs`).
- Backend: `src-tauri/` holds an `oxychloride::rpc::Server` and exposes the commands
  `rpc`, `read_text_file` and `write_text_file`.

## Prerequisites

- Rust (see the workspace `rust-version`) and the
  [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS
  (Xcode command line tools on macOS; WebKitGTK etc. on Linux; WebView2 on Windows).
- Node.js 20.19+ or 22.12+ with npm.

## Develop

```sh
cd app
npm install
npm run tauri dev
```

`npm run tauri dev` starts Vite on http://localhost:1420 and the desktop shell with hot
reload. Opening the Vite URL in a normal browser shows the UI, but without the engine.

Try it with `examples/university/`: open `university.oclinecore` and `university.hutn`
(the model shows five constraint violations), then **Run**. Or with `examples/library/`:
open `library.ocl` (its `import` loads `library.ecore`), **Load model** `library.xmi`, **Run**.

## Build

```sh
cd app
npm run tauri build
```

The bundle ends up in `target/release/bundle/` at the workspace root.

## Checks

```sh
npm run build          # TypeScript typecheck + production bundle
npm test               # vitest: position/offset conversion
cargo test -p oxy-app  # (from the repo root) RPC shapes the UI relies on
```

The icons in `src-tauri/icons/` are generated from `app-icon.png` with `npx tauri icon app-icon.png`.

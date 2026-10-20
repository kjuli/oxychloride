# Oxychloride

Rust re-implementation of Eclipse OCL over Ecore models: library, `oxy` CLI, JSON-RPC/C ABI integration API, Tauri desktop app. See README.md and docs/ARCHITECTURE.md.

## Commands
- `cargo test` — all engine/CLI/FFI tests (the Tauri app is not a default member).
- `cargo clippy --all-targets` must be warning-free; `cargo fmt` (rustfmt.toml: max_width 120).
- `cargo build -p oxy-app` builds the desktop app; front end: `cd app && npm run build`.
- VS Code extension (`vscode/`, client of `oxy lsp`): `cargo build -p oxy-cli`, then `cd vscode && npm run build`; `npm test` runs its integration tests in a real VS Code.
- Manual check: `cargo run -p oxy-cli -- run examples/library/library.ocl -m examples/library/library.xmi` (expects 3 violations, exit code 1).
- Manual check (textual): `cargo run -p oxy-cli -- run examples/university/university.oclinecore examples/university/university.hutn` (expects 5 violations: 3 invariants, 2 postconditions; exit code 1).

## Conventions
- Layering: `oxy-ecore` ← `oxy-ocl` ← `oxychloride` ← (`oxy-lsp`, `oxy-cli`, `oxy-ffi`, `app/src-tauri`); `oxy-cli` runs `oxy-lsp` as `oxy lsp`. Front ends talk to the engine only through `oxychloride` (preferably `rpc::Server`, so all transports stay identical).
- Never panic on user input: loaders return `LoadError`/warnings, OCL problems are `Diagnostic`s, evaluation failures are `Value::Invalid` with a reason.
- Name resolution logic exists twice (checker in `check.rs`, evaluator in `eval.rs`) and must stay in sync: variables → implicit iterator/`self` properties → types.
- Protocol changes: update `protocol.rs`/`rpc.rs`, `docs/PROTOCOL.md`, `app/src/api.ts` and `vscode/src/engine.ts` together.
- Tests use `examples/library` and `examples/university`; when adding engine features, add cases to `crates/oxy-ocl/tests/library.rs` (OCLinEcore/HUTN workspace behaviour: `crates/oxychloride/tests/textual.rs`).
- Documents have a language (`.ocl`, `.oclinecore`, `.hutn`, `.xmi`). OCLinEcore documents rebuild `mm` from `base_mm`, model documents (HUTN, XMI) rebuild `model` from `xmi_model` (see docs/ARCHITECTURE.md); public operations mark what changed (`mm_dirty`, `xmi_stale`, `models_dirty`) and end with `Workspace::finish`, which rebuilds the derived state once.

---
name: run-oxychloride
description: Build, run, test and drive Oxychloride (the OCL engine, its `oxy` CLI and the Tauri desktop editor). Use when asked to run or start the app or CLI, check a change in the real UI, take a screenshot of the editor, try OCLinEcore/HUTN/Complete OCL documents, validate models, or run the tests.
---

Oxychloride is an OCL engine with a CLI (`oxy`) and a Tauri desktop editor (`app/`). Engine changes are checked with the CLI and `cargo test`; UI changes are driven with
`.Codex/skills/run-oxychloride/driver.mjs`, which serves the real UI (Vite) to headless Chromium and connects it to the real engine (`oxy serve`, the same `rpc::Server` the Tauri backend wraps).

All paths are relative to the repo root. Verified on macOS (darwin) with Rust 1.97 and Node 24; nothing here has been tried on Linux yet.

## Setup

```bash
cargo build -p oxy-cli                                   # the driver needs target/debug/oxy
(cd app && npm install)                                  # the UI's dependencies (Vite)
npm install --prefix .Codex/skills/run-oxychloride      # the driver's Playwright
(cd .Codex/skills/run-oxychloride && npx playwright install chromium)
```

## Run: engine and CLI (most changes)

```bash
cargo run -q -p oxy-cli -- run examples/library/library.ocl -m examples/library/library.xmi --no-color
# ✗ 3 violations ... (9 constraints on 12 objects, 26 evaluations, 23 satisfied); exit code 1
cargo run -q -p oxy-cli -- run examples/university/university.oclinecore examples/university/university.hutn --no-color
# ✗ 5 violations ... (8 constraints on 16 objects, 30 evaluations, 25 satisfied); exit code 1
```

Other commands (`check`, `eval`, `convert`, `info`, `serve`) are in README.md. Tests:

```bash
cargo test                                  # engine, CLI, FFI (not the Tauri crate)
cargo test -p oxychloride --test textual    # OCLinEcore/HUTN workspace behaviour only
cargo test -p oxy-app                       # JSON shapes the UI relies on (slow first build)
cargo clippy --all-targets                  # must be warning-free
(cd app && npm run build && npm test)       # typecheck + bundle, front-end unit tests
```

## Run: desktop UI (agent path)

The driver starts the engine bridge and Vite (port 1420), opens the UI in headless Chromium, then executes commands from stdin, one per line, and shuts everything down on `quit`:

```bash
cargo build -p oxy-cli    # after any engine change: the driver runs target/debug/oxy
node .Codex/skills/run-oxychloride/driver.mjs <<'EOF'
open examples/university/university.oclinecore
open examples/university/university.hutn
ss model
text .panel-tab:has-text("Problems")
run
text .validation-summary
ss validation
tab university.hutn
line 53
type \nse
ss completion
press Escape
eval Student.allInstances()->any(name = 'Ada').mayEnrol(Course.allInstances()->any(code = 'IN0001'))
quit
EOF
```

This prints `5` problems, `5 violated, 25 satisfied. 8 constraints on 16 objects.`, and the console output (including the failed pre/postconditions of the call). Screenshots land in `.Codex/skills/run-oxychloride/screenshots/<name>.png` (git-ignored); **open them** — `completion.png` must show the `semester  Integer` popup.

Commands (`help` prints them): `open <path>`, `dialog <path>`, `click <selector>`, `tab <name>`, `line <n>`, `type <text>` (`\n` = Enter), `press <key>`, `run`, `eval <ocl>`, `wait <selector>`, `text <selector>`, `ss <name>`, `js <expr>`, `quit`. A failing command prints `[failed] ...` and the session continues; page errors print as `[page error]`/`[console error]`.

Saving/exporting goes through a file dialog: queue its answer first (relative paths are relative to the repo root). This writes an `.ecore` file:

```bash
node .Codex/skills/run-oxychloride/driver.mjs <<'EOF'
open examples/university/university.oclinecore
dialog .Codex/skills/run-oxychloride/screenshots/u.ecore
click [aria-label="Export as .ecore…"]
wait text=Exported u.ecore
quit
EOF
```

## Run: desktop app (human path)

`cd app && npm run tauri dev` opens the native window (see app/README.md). It was not run while writing this skill and is useless headless — use the driver.

## Gotchas

- The UI only reaches the engine inside Tauri (`isTauri()`); in a plain browser every call fails with "The engine is only available in the desktop app". The driver's init script sets `window.isTauri` and a fake `window.__TAURI_INTERNALS__.invoke`; the Tauri commands it serves are `rpc`, `read_text_file`, `write_text_file`, and the dialog plugin's `open`/`save`/`confirm`. If the app gains a new Tauri command, add it to the driver.
- The engine keeps documents across page loads: each driver run starts its own `oxy serve`, so state never leaks between runs. A bridge reused across browser sessions shows the previous session's documents in Problems.
- CodeMirror renders only visible lines, so `.cm-line:has-text(...)` times out for lines below the fold. Use `line <n>`, which navigates with the keyboard (Meta+ArrowUp, ArrowDown ×n, Meta+ArrowRight — macOS key names).
- Typing right after a tab opens can land before the editor has focus (first characters lost, text at line 1). `open` waits 500 ms; after `click`ing a new-document menu item, `wait` for the tab before `type`.
- `getByRole("button", { name: "Open…" })` is ambiguous on the welcome screen (toolbar and welcome both have one); the driver uses the toolbar's `aria-label`.
- The Open dialog allows several files, so its answer is a list (`open` handles that); save dialogs take a single path (`dialog` queues a plain path).
- Model documents only show constraint violations while they have no errors: after `type`-ing half a slot, Problems drops to the syntax error alone. That is intended behaviour, not a driver failure.
- `cd app && npm install` warns about install scripts it did not run; Vite still starts.

## Troubleshooting

Errors hit while building this harness:

- `locator.click: Timeout 30000ms exceeded ... locator('.cm-line').filter({ hasText: ... })` → the line is below the fold and not rendered; use `line <n>`.
- An export or a new document contains the typed text at line 1 with its first characters missing (`niversity "u" ...`, "unknown class 'niversity'") → typing started before the editor had focus; `wait` for the tab (and pause) before `type`.
- Problems lists errors of documents the session never opened (e.g. `university.hutn` lines you never edited) → the engine process was reused from an earlier browser session; let the driver start a fresh one (it always does).
- `strict mode violation: getByRole('button', { name: 'Open…' }) resolved to 2 elements` → use the toolbar's `[aria-label="Open…"]` (what `open` does).

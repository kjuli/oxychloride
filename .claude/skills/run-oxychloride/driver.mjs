// Drives the VS Code extension's metamodel diagram (vscode/webview/diagram) in headless Chromium
// against the real engine.
//
// A webview cannot be driven outside VS Code, so this serves the same page (the built bundle
// vscode/out/webview/diagram.js and its CSS, with the CSS variables of a VS Code theme) to
// Chromium and stands in for its host (vscode/src/diagram.ts): `acquireVsCodeApi()` posts to
// this script, which keeps the document's text, asks `target/debug/oxy serve` (the engine's
// JSON-RPC server, the one behind `oxy lsp`) for diagrams and edits, applies the edits to the
// text and posts the results back. Nothing is written to disk unless asked (`save`).
// Commands are read from stdin, one per line (see HELP).
//
// Usage (from the repo root):  node .claude/skills/run-oxychloride/driver.mjs [--light] <<'EOF' ... EOF

import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const SHOTS = path.join(ROOT, ".claude/skills/run-oxychloride/screenshots");
const HELP = `commands (in selectors, @Name is the node of classifier Name, e.g. @Course):
  open <path>              open a metamodel (.oclinecore or .ecore) in the metamodel diagram, or a Henshin
                           transformation (.henshin_text or .henshin) in the rule diagram (and <path>.diagram.json)
  ss <name>                screenshot to .claude/skills/run-oxychloride/screenshots/<name>.png
  click <selector>         click, e.g. click @Course .row:has-text("title")   or   click button[title^="Add a class"]
  dblclick <selector>      double-click (edits a name or member in place)
  clickat <x> <y>          click a point of the page (e.g. into the diagram with a classifier tool)
  down <x> <y> / move <x> <y> / up    press, move and release the mouse
  type <text>              type text (\\n = Enter)
  press <key>              press a key, e.g. Enter, Escape, Delete, F2
  drag <selector> <dx> <dy>    drag an element by (dx, dy) pixels
  connect <from> <to>      drag from node <from> (its connector, or itself with a connection tool) to node <to>,
                           e.g. connect @Student @Course
  wait <selector>          wait for a selector
  text <selector>          print the text of the first match
  count <selector>         print the number of matches
  doc [regex]              print the document's text (or its lines matching regex)
  edit-text <old> => <new> change the document's text as a text editor would (first occurrence)
  cursor <line> <column>   move the text cursor (0-based), as in a text editor beside the diagram
  layout                   print the layout the diagram last asked to write
  save <path>              write the document's text to a file
  js <expression>          evaluate JavaScript in the page and print the result
  quit                     stop everything`;

const LIGHT = process.argv.includes("--light");

// The CSS variables VS Code gives webviews (a subset of Dark Modern / Light Modern).
const THEME = LIGHT
  ? {
      foreground: "#3b3b3b", descriptionForeground: "#717171", "editor-background": "#ffffff", "editor-foreground": "#3b3b3b",
      "editorWidget-background": "#f8f8f8", "editorWidget-border": "#c8c8c8", "panel-border": "#e5e5e5", focusBorder: "#005fb8",
      "list-hoverBackground": "#f2f2f2", "list-inactiveSelectionBackground": "#e4e6f1", "list-activeSelectionBackground": "#e8e8e8",
      "editorError-foreground": "#e51400", errorForeground: "#f85149", "editorCodeLens-foreground": "#919191",
      "input-background": "#ffffff", "input-foreground": "#3b3b3b", "input-border": "#cecece",
      "button-background": "#005fb8", "button-foreground": "#ffffff", "button-secondaryBackground": "#e5e5e5", "button-secondaryForeground": "#3b3b3b",
      "badge-background": "#cccccc", "badge-foreground": "#3b3b3b", "sideBar-background": "#f8f8f8", "editorGroupHeader-tabsBackground": "#f8f8f8",
      "toolbar-hoverBackground": "rgba(184, 184, 184, 0.31)", "inputOption-activeBackground": "rgba(190, 230, 253, 0.6)", "inputOption-activeBorder": "#005fb8",
      "dropdown-background": "#ffffff", "dropdown-foreground": "#3b3b3b", "dropdown-border": "#cecece", "textLink-foreground": "#005fb8",
      "editorIndentGuide-background1": "#d3d3d3", "notifications-background": "#ffffff", "editorWarning-foreground": "#bf8803",
      "symbolIcon-classForeground": "#d67e00", "symbolIcon-interfaceForeground": "#007acc", "symbolIcon-fieldForeground": "#007acc",
      "symbolIcon-methodForeground": "#652d90", "symbolIcon-enumeratorMemberForeground": "#007acc",
      "charts-green": "#388a34", "charts-purple": "#652d90", "charts-orange": "#d18616",
      "inputValidation-errorBackground": "#f2dede", "inputValidation-errorBorder": "#be1100",
      "inputValidation-warningBackground": "#f6f5d2", "inputValidation-warningBorder": "#b89500",
    }
  : {
      foreground: "#cccccc", descriptionForeground: "#9d9d9d", "editor-background": "#1f1f1f", "editor-foreground": "#cccccc",
      "editorWidget-background": "#202020", "editorWidget-border": "#454545", "panel-border": "#2b2b2b", focusBorder: "#0078d4",
      "list-hoverBackground": "#2a2d2e", "list-inactiveSelectionBackground": "#37373d", "list-activeSelectionBackground": "#04395e",
      "editorError-foreground": "#f85149", errorForeground: "#f85149", "editorCodeLens-foreground": "#999999",
      "input-background": "#313131", "input-foreground": "#cccccc", "input-border": "#3c3c3c",
      "button-background": "#0078d4", "button-foreground": "#ffffff", "button-secondaryBackground": "#313131", "button-secondaryForeground": "#cccccc",
      "badge-background": "#616161", "badge-foreground": "#f8f8f8", "sideBar-background": "#181818", "editorGroupHeader-tabsBackground": "#181818",
      "toolbar-hoverBackground": "rgba(90, 93, 94, 0.31)", "inputOption-activeBackground": "rgba(36, 137, 219, 0.51)", "inputOption-activeBorder": "#2488db",
      "dropdown-background": "#313131", "dropdown-foreground": "#cccccc", "dropdown-border": "#3c3c3c", "textLink-foreground": "#4daafc",
      "editorIndentGuide-background1": "#404040", "notifications-background": "#1f1f1f", "editorWarning-foreground": "#cca700",
      "symbolIcon-classForeground": "#ee9d28", "symbolIcon-interfaceForeground": "#75beff", "symbolIcon-fieldForeground": "#75beff",
      "symbolIcon-methodForeground": "#b180d7", "symbolIcon-enumeratorMemberForeground": "#75beff",
      "charts-green": "#89d185", "charts-purple": "#b180d7", "charts-orange": "#d18616",
      "inputValidation-errorBackground": "#5a1d1d", "inputValidation-errorBorder": "#be1100",
      "inputValidation-warningBackground": "#352a05", "inputValidation-warningBorder": "#b89500",
    };
const themeCss = [
  "--vscode-font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;",
  "--vscode-font-size: 13px;",
  "--vscode-editor-font-family: Menlo, Monaco, 'Courier New', monospace;",
  ...Object.entries(THEME).map(([k, v]) => `--vscode-${k}: ${v};`),
].join(" ");

// ---------------------------------------------------------------- engine

function startEngine() {
  const oxy = spawn(path.join(ROOT, "target/debug/oxy"), ["serve"], { stdio: ["pipe", "pipe", "inherit"] });
  const pending = new Map();
  let next = 1;
  readline.createInterface({ input: oxy.stdout }).on("line", (l) => {
    const m = JSON.parse(l);
    pending.get(m.id)?.(m);
    pending.delete(m.id);
  });
  const rpc = (method, params) =>
    new Promise((resolve, reject) => {
      const id = next++;
      pending.set(id, (m) => (m.error ? reject(new Error(m.error.message)) : resolve(m.result)));
      oxy.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  return { oxy, rpc };
}

/** Applies engine edits (0-based lines, columns in Unicode scalar values) to a text. */
function applyEdits(text, edits) {
  const lines = text.split("\n");
  const offset = (p) => {
    let o = 0;
    for (let i = 0; i < p.line && i < lines.length; i++) o += lines[i].length + 1;
    const line = lines[p.line] ?? "";
    return o + [...line].slice(0, p.column).join("").length;
  };
  let out = text;
  for (const e of [...edits].reverse()) out = out.slice(0, offset(e.range.start)) + e.text + out.slice(offset(e.range.end));
  return out;
}

// ---------------------------------------------------------------- page server

/** Whether a document is a Henshin transformation (shown as rule diagrams). */
const isRules = (p) => /\.henshin(_text)?$/i.test(p);

function startServer() {
  const files = {
    "/diagram.js": [path.join(ROOT, "vscode/out/webview/diagram.js"), "text/javascript"],
    "/bundle.css": [path.join(ROOT, "vscode/out/webview/diagram.css"), "text/css"],
    "/diagram.css": [path.join(ROOT, "vscode/media/diagram.css"), "text/css"],
    "/rules.js": [path.join(ROOT, "vscode/out/webview/rules.js"), "text/javascript"],
    "/rules-bundle.css": [path.join(ROOT, "vscode/out/webview/rules.css"), "text/css"],
    "/rules.css": [path.join(ROOT, "vscode/media/rules.css"), "text/css"],
  };
  // The metamodel diagram's page, or the rule diagram's (for Henshin documents).
  const html = () => {
    const rules = isRules(doc.path);
    const css = rules ? ["/rules-bundle.css", "/diagram.css", "/rules.css"] : ["/bundle.css", "/diagram.css"];
    return `<!DOCTYPE html><html lang="en" style="${themeCss}"><head><meta charset="UTF-8">
${css.map((c) => `<link rel="stylesheet" href="${c}">`).join("")}<title>${rules ? "Rule Diagram" : "Metamodel Diagram"}</title></head>
<body class="${LIGHT ? "vscode-light" : "vscode-dark"}"><div id="app"></div><script src="${rules ? "/rules.js" : "/diagram.js"}"></script></body></html>`;
  };
  const server = http.createServer((req, res) => {
    const file = files[req.url];
    if (file) {
      res.setHeader("Content-Type", file[1]);
      return res.end(fs.readFileSync(file[0]));
    }
    res.setHeader("Content-Type", "text/html");
    res.end(html());
  });
  return new Promise((resolve) => server.listen(0, () => resolve({ server, port: server.address().port })));
}

// ---------------------------------------------------------------- main

for (const f of ["target/debug/oxy", "vscode/out/webview/diagram.js"]) {
  if (!fs.existsSync(path.join(ROOT, f))) {
    console.error(`${f} is missing: run \`cargo build -p oxy-cli\` and \`(cd vscode && npm run build)\` first`);
    process.exit(2);
  }
}
const engine = startEngine();
const site = await startServer();
const browser = await chromium.launch();
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 })).newPage();
page.on("pageerror", (e) => console.log(`[page error] ${e}`));
page.on("console", (m) => m.type() === "error" && console.log(`[console error] ${m.text()}`));

/** The document the diagram shows, and the stand-in host's state. */
const doc = { path: "", text: "", layout: undefined };
const toPage = (m) => page.evaluate((x) => window.postMessage(x, "*"), m);

async function refresh() {
  try {
    const method = isRules(doc.path) ? "transformationDiagram" : "metamodelDiagram";
    await toPage({ type: "diagram", diagram: await engine.rpc(method, { name: doc.path, text: doc.text }) });
  } catch (e) {
    await toPage({ type: "unreadable", message: e.message });
  }
}

await page.exposeBinding("__toHost", async (_, m) => {
  switch (m.type) {
    case "ready": {
      const file = `${doc.path}.diagram.json`;
      const layout = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")).nodes : {};
      await toPage({ type: "init", name: path.basename(doc.path), layout, persistent: true });
      await refresh();
      break;
    }
    case "edit": {
      try {
        const method = isRules(doc.path) ? "editTransformation" : "editMetamodel";
        const r = await engine.rpc(method, { name: doc.path, text: doc.text, edit: m.edit });
        doc.text = applyEdits(doc.text, r.edits);
        console.log(`[edit] ${m.edit.kind}: ${r.edits.length} text edit(s)${r.element ? ` → ${r.element}` : ""}`);
        await toPage({ type: "edited", request: m.request, element: r.element });
        await refresh();
      } catch (e) {
        console.log(`[edit failed] ${m.edit.kind}: ${e.message}`);
        await toPage({ type: "edited", request: m.request, error: e.message });
      }
      break;
    }
    case "layout":
      if (m.persist === "always" || doc.layout) doc.layout = m.layout;
      break;
    case "reveal":
      console.log(`[reveal] ${m.range.start.line}:${m.range.start.column}${m.open ? " (open)" : ""}`);
      break;
    case "notice":
      console.log(`[${m.kind}] ${m.message}`);
      break;
    case "command":
      console.log(`[command] ${m.command}`);
      break;
  }
});
await page.addInitScript(() => {
  let state;
  window.acquireVsCodeApi = () => ({ postMessage: (m) => window.__toHost(m), getState: () => state, setState: (s) => (state = s) });
});
fs.mkdirSync(SHOTS, { recursive: true });
console.log("ready (type `help` for commands)");

const abs = (p) => path.resolve(ROOT, p);
/** `@Name` is the node of classifier `Name`. */
const sel = (s) => s.replace(/@([\w$]+)/g, (_, n) => `.react-flow__node[data-id$="::${n}"]`);
const commands = {
  help: async () => console.log(HELP),
  open: async (p) => {
    doc.path = abs(p);
    doc.text = fs.readFileSync(doc.path, "utf8");
    doc.layout = undefined;
    // A transformation needs its metamodels: the OCLinEcore documents and .ecore files beside it.
    if (isRules(doc.path)) {
      const dir = path.dirname(doc.path);
      for (const f of fs.readdirSync(dir).sort()) {
        const file = path.join(dir, f);
        if (f.endsWith(".oclinecore")) await engine.rpc("openDocument", { name: file, text: fs.readFileSync(file, "utf8") });
        else if (f.endsWith(".ecore")) await engine.rpc("loadEcore", { path: file }).catch(() => {});
      }
    }
    await page.goto(`http://localhost:${site.port}/`);
    await page.locator(".react-flow__node:not(.unplaced)").first().waitFor();
    await page.waitForTimeout(600);
  },
  click: async (s) => page.locator(sel(s)).first().click(),
  dblclick: async (s) => page.locator(sel(s)).first().dblclick(),
  clickat: async (xy) => {
    const [x, y] = xy.split(/\s+/).map(Number);
    await page.mouse.click(x, y);
  },
  type: async (text) => {
    const parts = text.split("\\n");
    for (let i = 0; i < parts.length; i++) {
      if (i > 0) await page.keyboard.press("Enter");
      await page.keyboard.type(parts[i], { delay: 20 });
    }
    await page.waitForTimeout(300);
  },
  press: async (key) => {
    await page.keyboard.press(key);
    await page.waitForTimeout(300);
  },
  drag: async (args) => {
    const m = /^(.*)\s+(-?\d+)\s+(-?\d+)$/.exec(args);
    const box = await page.locator(sel(m[1])).first().boundingBox();
    const [x, y] = [box.x + box.width / 2, box.y + 12];
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + Number(m[2]), y + Number(m[3]), { steps: 12 });
    await page.mouse.up();
    await page.waitForTimeout(300);
  },
  connect: async (args) => {
    const [from, to] = args.split(/\s+(?=@)/);
    // With a connection tool the whole node starts the connection, else its connector does.
    const tool = await page.locator(".diagram-app.tool-connect").count();
    await page.locator(sel(from)).first().hover({ force: true });
    const handle = await page.locator(tool ? sel(from) : `${sel(from)} .connector`).first().boundingBox();
    const target = await page.locator(sel(to)).first().boundingBox();
    await page.mouse.move(handle.x + handle.width / 2, handle.y + (tool ? 14 : handle.height / 2));
    await page.mouse.down();
    await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 15 });
    await page.mouse.up();
    await page.waitForTimeout(500);
  },
  down: async (xy) => {
    const [x, y] = xy.split(/\s+/).map(Number);
    await page.mouse.move(x, y);
    await page.mouse.down();
  },
  move: async (xy) => {
    const [x, y] = xy.split(/\s+/).map(Number);
    await page.mouse.move(x, y, { steps: 10 });
  },
  up: async () => page.mouse.up(),
  wait: async (s) => page.locator(sel(s)).first().waitFor(),
  text: async (s) => console.log((await page.locator(sel(s)).first().innerText()).trim()),
  count: async (s) => console.log(await page.locator(sel(s)).count()),
  doc: async (re) => {
    const lines = doc.text.split("\n");
    const shown = re ? lines.map((l, i) => [i, l]).filter(([, l]) => new RegExp(re).test(l)) : lines.map((l, i) => [i, l]);
    for (const [i, l] of shown) console.log(`${String(i).padStart(4)}  ${l}`);
  },
  "edit-text": async (args) => {
    const [from, to] = args.split(" => ").map((x) => x?.replaceAll("\\n", "\n"));
    if (!doc.text.includes(from)) throw new Error(`'${from}' is not in the document`);
    doc.text = doc.text.replace(from, to ?? "");
    await refresh();
    await page.waitForTimeout(400);
  },
  cursor: async (args) => {
    const [line, column] = args.split(/\s+/).map(Number);
    await toPage({ type: "cursor", position: { line, column } });
    await page.waitForTimeout(300);
  },
  layout: async () => console.log(JSON.stringify(doc.layout ?? null)),
  save: async (p) => {
    fs.writeFileSync(abs(p), doc.text);
    console.log(`wrote ${p}`);
  },
  ss: async (name) => {
    await page.waitForTimeout(500);
    const file = path.join(SHOTS, `${name || "screenshot"}.png`);
    await page.screenshot({ path: file });
    console.log(`screenshot: ${path.relative(ROOT, file)}`);
  },
  js: async (expr) => console.log(JSON.stringify(await page.evaluate(expr))),
};

for await (const raw of readline.createInterface({ input: process.stdin })) {
  const line = raw.trim();
  if (!line || line.startsWith("#")) continue;
  const [cmd, ...rest] = line.split(" ");
  if (cmd === "quit") break;
  const f = commands[cmd];
  if (!f) {
    console.log(`unknown command '${cmd}' (try help)`);
    continue;
  }
  try {
    console.log(`> ${line}`);
    await f(rest.join(" "));
  } catch (e) {
    console.log(`[failed] ${line}: ${e.message.split("\n")[0]}`);
  }
}
await browser.close().catch(() => {});
engine.oxy.kill();
site.server.close();
process.exit(0);

// Drives the Oxychloride desktop app's UI (app/) in headless Chromium against the real engine.
//
// The UI normally runs inside Tauri, which can't be driven headlessly. Here the Vite dev server
// serves the same UI to Chromium, and an init script stands in for Tauri's `invoke`: the `rpc`
// command goes to `target/debug/oxy serve` (the same rpc::Server the Tauri backend wraps),
// `read_text_file`/`write_text_file` go to the file system, and file dialogs are answered from
// a queue (`dialog <path>`). Commands are read from stdin, one per line (see HELP).
//
// Usage (from the repo root):  node .claude/skills/run-oxychloride/driver.mjs <<'EOF' ... EOF

import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const SHOTS = path.join(ROOT, ".claude/skills/run-oxychloride/screenshots");
const VITE_PORT = 1420; // fixed by app/vite.config.ts (strictPort)
const HELP = `commands:
  open <path>          open a document (.ocl/.oclinecore/.hutn) via the toolbar's Open dialog
  dialog <path>        queue the answer for the next file dialog (open or save)
  click <selector>     click a Playwright selector, e.g. text=Run validation, .doc-tab:has-text("x.hutn")
  tab <name>           activate the editor tab whose title contains <name>
  line <n>             put the cursor at the end of line n of the active editor
  type <text>          type text (use \\n for Enter)
  press <key>          press a key, e.g. Enter, Escape, Control+Space, Meta+z
  run                  run validation (Validation panel)
  eval <ocl>           evaluate an OCL expression in the Console panel
  wait <selector>      wait for a selector to be visible
  text <selector>      print the text of the first match
  ss <name>            screenshot to .claude/skills/run-oxychloride/screenshots/<name>.png
  js <expression>      evaluate JavaScript in the page and print the result
  quit                 stop everything`;

// ---------------------------------------------------------------- engine bridge

function startBridge() {
  const oxy = spawn(path.join(ROOT, "target/debug/oxy"), ["serve"], { stdio: ["pipe", "pipe", "inherit"] });
  const pending = new Map();
  let next = 1;
  readline.createInterface({ input: oxy.stdout }).on("line", (l) => {
    const m = JSON.parse(l);
    pending.get(m.id)?.(m);
    pending.delete(m.id);
  });
  const rpc = (method, params) =>
    new Promise((resolve) => {
      const id = next++;
      pending.set(id, resolve);
      oxy.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", async () => {
      res.setHeader("Access-Control-Allow-Origin", "*");
      res.setHeader("Access-Control-Allow-Headers", "*");
      if (req.method === "OPTIONS") return res.end();
      const { cmd, args } = JSON.parse(body);
      let out;
      try {
        if (cmd === "rpc") {
          const m = await rpc(args.method, args.params);
          out = m.error ? { error: m.error } : { ok: m.result };
        } else if (cmd === "read_text_file") out = { ok: fs.readFileSync(args.path, "utf8") };
        else if (cmd === "write_text_file") {
          fs.writeFileSync(args.path, args.contents);
          out = { ok: null };
        } else out = { error: { code: 0, message: `unsupported command ${cmd}` } };
      } catch (e) {
        out = { error: { code: 0, message: String(e) } };
      }
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify(out));
    });
  });
  return new Promise((resolve) => server.listen(0, () => resolve({ oxy, server, port: server.address().port })));
}

async function waitForUrl(url, seconds) {
  for (let i = 0; i < seconds * 4; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`${url} did not come up`);
}

// ---------------------------------------------------------------- main

if (!fs.existsSync(path.join(ROOT, "target/debug/oxy"))) {
  console.error("target/debug/oxy is missing: run `cargo build -p oxy-cli` first");
  process.exit(2);
}
const bridge = await startBridge();
const vite = spawn("npx", ["vite", "--port", String(VITE_PORT), "--strictPort"], {
  cwd: path.join(ROOT, "app"),
  stdio: ["ignore", "ignore", "inherit"],
  detached: true,
});
await waitForUrl(`http://localhost:${VITE_PORT}`, 30);

const browser = await chromium.launch();
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
page.on("pageerror", (e) => console.log(`[page error] ${e}`));
page.on("console", (m) => m.type() === "error" && console.log(`[console error] ${m.text()}`));
await page.addInitScript((bridgeUrl) => {
  window.isTauri = true;
  window.__dialogs = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd, args) => {
      if (cmd === "plugin:dialog|open" || cmd === "plugin:dialog|save") return window.__dialogs.shift() ?? null;
      if (cmd === "plugin:dialog|confirm") return true;
      if (cmd.startsWith("plugin:")) return null;
      const r = await (await fetch(bridgeUrl, { method: "POST", body: JSON.stringify({ cmd, args }) })).json();
      if (r.error) throw r.error;
      return r.ok;
    },
  };
}, `http://localhost:${bridge.port}`);
await page.goto(`http://localhost:${VITE_PORT}`);
await page.getByText("Write constraints").waitFor();
fs.mkdirSync(SHOTS, { recursive: true });
console.log("ready (type `help` for commands)");

const abs = (p) => path.resolve(ROOT, p);
const commands = {
  help: async () => console.log(HELP),
  dialog: async (p) => page.evaluate((x) => window.__dialogs.push(x), abs(p)),
  open: async (p) => {
    // The Open dialog allows several files, so its answer is a list.
    await page.evaluate((x) => window.__dialogs.push([x]), abs(p));
    await page.getByLabel("Open…").click();
    await page.locator(".doc-tab", { hasText: path.basename(p) }).waitFor();
    await page.waitForTimeout(500);
  },
  click: async (sel) => page.locator(sel).first().click(),
  tab: async (name) => page.locator(".doc-tab", { hasText: name }).first().click(),
  line: async (n) => {
    // CodeMirror only renders visible lines: navigate with the keyboard instead of clicking.
    await page.locator(".cm-content").click({ position: { x: 5, y: 5 } });
    await page.keyboard.press("Meta+ArrowUp");
    for (let i = 1; i < Number(n); i++) await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Meta+ArrowRight");
  },
  type: async (text) => {
    const parts = text.split("\\n");
    for (let i = 0; i < parts.length; i++) {
      if (i > 0) await page.keyboard.press("Enter");
      await page.keyboard.type(parts[i], { delay: 40 });
    }
    await page.waitForTimeout(400); // completion and diagnostics are debounced
  },
  press: async (key) => page.keyboard.press(key),
  run: async () => {
    await page.getByRole("button", { name: /^Run$/ }).first().click();
    await page.locator(".validation-summary").waitFor();
  },
  eval: async (expr) => {
    await page.getByRole("tab", { name: "Console" }).click();
    await page.locator(".console-input").fill(expr);
    await page.locator(".console-input").press("Enter");
    await page.waitForTimeout(500);
    console.log((await page.locator(".console-entry").last().innerText()).trim());
  },
  wait: async (sel) => page.locator(sel).first().waitFor(),
  text: async (sel) => console.log((await page.locator(sel).first().innerText()).trim()),
  ss: async (name) => {
    await page.waitForTimeout(500);
    const file = path.join(SHOTS, `${name || "screenshot"}.png`);
    await page.screenshot({ path: file });
    console.log(`screenshot: ${path.relative(ROOT, file)}`);
  },
  js: async (expr) => console.log(JSON.stringify(await page.evaluate(expr))),
};

async function shutdown(code) {
  await browser.close().catch(() => {});
  bridge.oxy.kill();
  bridge.server.close();
  try {
    process.kill(-vite.pid); // the whole npx/vite process group
  } catch {
    // already gone
  }
  process.exit(code);
}

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
await shutdown(0);

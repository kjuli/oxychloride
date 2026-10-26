// Bundles the extension (and the language client) into out/extension.js, and the scripts of the
// webviews: the transformation view into out/webview/transformation.js, the metamodel diagram
// (React Flow, ELK) into out/webview/diagram.js and diagram.css, the rule diagram into
// out/webview/rules.js and rules.css.
import * as fs from "node:fs";
import * as esbuild from "esbuild";

// The file icons live in assets/file-icons; the extension ships a copy.
fs.mkdirSync("media/file-icons", { recursive: true });
for (const f of fs.readdirSync("../assets/file-icons").filter((f) => f.endsWith(".svg"))) {
  fs.copyFileSync(`../assets/file-icons/${f}`, `media/file-icons/${f}`);
}

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

const contexts = await Promise.all([
  esbuild.context({
    entryPoints: ["src/extension.ts"],
    bundle: true,
    format: "cjs",
    platform: "node",
    target: "node20",
    outfile: "out/extension.js",
    external: ["vscode"],
    sourcemap: !production,
    minify: production,
    logLevel: "info",
  }),
  // The script of the transformation view (a webview).
  esbuild.context({
    entryPoints: ["webview/transformation.ts"],
    bundle: true,
    format: "iife",
    platform: "browser",
    target: "chrome120",
    outfile: "out/webview/transformation.js",
    sourcemap: production ? false : "inline",
    minify: production,
    logLevel: "info",
  }),
  // The rule diagram (a webview).
  esbuild.context({
    entryPoints: ["webview/rules.tsx"],
    bundle: true,
    format: "iife",
    platform: "browser",
    target: "chrome120",
    jsx: "automatic",
    outfile: "out/webview/rules.js",
    define: { "process.env.NODE_ENV": production ? '"production"' : '"development"' },
    sourcemap: production ? false : "inline",
    minify: production,
    logLevel: "info",
  }),
  // The metamodel diagram (a webview).
  esbuild.context({
    entryPoints: ["webview/diagram.tsx"],
    bundle: true,
    format: "iife",
    platform: "browser",
    target: "chrome120",
    jsx: "automatic",
    outfile: "out/webview/diagram.js",
    define: { "process.env.NODE_ENV": production ? '"production"' : '"development"' },
    sourcemap: production ? false : "inline",
    minify: production,
    logLevel: "info",
  }),
]);
if (watch) {
  await Promise.all(contexts.map((c) => c.watch()));
} else {
  await Promise.all(contexts.map((c) => c.rebuild()));
  await Promise.all(contexts.map((c) => c.dispose()));
}

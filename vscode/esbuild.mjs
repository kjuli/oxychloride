// Bundles the extension (and the language client) into out/extension.js.
import * as fs from "node:fs";
import * as esbuild from "esbuild";

// The file icons are shared with the desktop app (assets/file-icons); the extension ships a copy.
fs.mkdirSync("media/file-icons", { recursive: true });
for (const f of fs.readdirSync("../assets/file-icons").filter((f) => f.endsWith(".svg"))) {
  fs.copyFileSync(`../assets/file-icons/${f}`, `media/file-icons/${f}`);
}

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

const context = await esbuild.context({
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
});
if (watch) {
  await context.watch();
} else {
  await context.rebuild();
  await context.dispose();
}

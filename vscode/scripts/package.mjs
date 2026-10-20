// Packages the extension for this platform, with a release build of the language server:
// builds `oxy`, copies it to server/, bundles the extension and runs `vsce package --target`.
// The result works without Rust; package once per platform (e.g. in CI) for the others.

import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.resolve(here, "..");
const repo = path.resolve(extension, "..");
const exe = process.platform === "win32" ? "oxy.exe" : "oxy";
const run = (cmd, args, cwd) => execFileSync(cmd, args, { cwd, stdio: "inherit", shell: process.platform === "win32" });

run("cargo", ["build", "--release", "-p", "oxy-cli"], repo);
fs.mkdirSync(path.join(extension, "server"), { recursive: true });
fs.copyFileSync(path.join(repo, "target", "release", exe), path.join(extension, "server", exe));
fs.chmodSync(path.join(extension, "server", exe), 0o755);

run("node", ["esbuild.mjs", "--production"], extension);
const target = `${process.platform}-${process.arch}`;
run("npx", ["vsce", "package", "--no-dependencies", "--skip-license", "--target", target], extension);

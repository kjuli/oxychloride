// Finds the `oxy` executable that runs the language server.

import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";

const EXE = process.platform === "win32" ? "oxy.exe" : "oxy";

function isFile(p: string): boolean {
  try {
    return fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

/** The `oxy` to run: the configured one, else the newest of the bundled one and the builds of an
 * Oxychloride checkout (the extension's own, or a workspace folder), else `oxy` on the PATH. */
export function serverExecutable(extensionPath: string): string {
  const configured = vscode.workspace.getConfiguration("oxychloride").get<string>("server.path")?.trim();
  if (configured) {
    const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? "";
    return configured.replace("${workspaceFolder}", folder);
  }
  const bundled = path.join(extensionPath, "server", EXE);
  const roots = [path.join(extensionPath, ".."), ...(vscode.workspace.workspaceFolders ?? []).map((f) => f.uri.fsPath)];
  const builds = [bundled, ...roots.flatMap((root) => ["release", "debug"].map((p) => path.join(root, "target", p, EXE)))]
    .filter(isFile)
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  return builds[0] ?? EXE;
}

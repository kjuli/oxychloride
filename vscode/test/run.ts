// Runs the integration tests in a VS Code instance with the extension under development, on a
// copy of examples/university, examples/library and examples/qvt, and of the Eclipse projects of
// crates/oxychloride/tests/fixtures/eclipse (the tests edit and create files).

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { spawnSync } from "node:child_process";
import { downloadAndUnzipVSCode, resolveCliArgsFromVSCodeExecutablePath, runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const extensionDevelopmentPath = path.resolve(__dirname, "../..");
  const extensionTestsPath = path.resolve(__dirname, "suite/index.js");
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "oxychloride-test-"));
  for (const example of ["university", "library", "qvt"]) {
    fs.cpSync(path.resolve(extensionDevelopmentPath, "../examples", example), path.join(workspace, example), { recursive: true });
  }
  // Eclipse projects with a target platform of jars.
  fs.cpSync(path.resolve(extensionDevelopmentPath, "../crates/oxychloride/tests/fixtures/eclipse"), path.join(workspace, "eclipse"), { recursive: true });
  // OXYCHLORIDE_SERVER: run another language server executable (e.g. a logging wrapper).
  if (process.env.OXYCHLORIDE_SERVER) {
    fs.mkdirSync(path.join(workspace, ".vscode"));
    const settings = { "oxychloride.server.path": process.env.OXYCHLORIDE_SERVER };
    fs.writeFileSync(path.join(workspace, ".vscode/settings.json"), JSON.stringify(settings));
  }
  // VSCODE_PATH: test against an installed VS Code instead of downloading one.
  const vscodeExecutablePath = process.env.VSCODE_PATH || (await downloadAndUnzipVSCode());
  // The XML extension validates XMI models with the generated schemas.
  const [cli, ...args] = resolveCliArgsFromVSCodeExecutablePath(vscodeExecutablePath);
  spawnSync(cli, [...args, "--install-extension", "redhat.vscode-xml"], { stdio: "inherit", shell: process.platform === "win32" });
  try {
    await runTests({
      extensionDevelopmentPath,
      extensionTestsPath,
      vscodeExecutablePath,
      launchArgs: [workspace, "--disable-workspace-trust", "--skip-welcome", "--skip-release-notes"],
    });
  } finally {
    fs.rmSync(workspace, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});

// Generating code from a metamodel: Python packages, Rust crates (see crates/oxy-codegen and
// docs/CODEGEN.md). The languages come from the engine (`codeTargets`), so a target added
// there is offered here without changes to the extension.

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName } from "./documents";
import { errorMessage, type CodeTarget, type Diagnostic, type Engine, type GeneratedCode } from "./engine";

/** Answers to the command's questions, for calls from code (other extensions, tests). */
export interface GenerateOptions {
  /** A target's id (`python`, `rust`). */
  target?: string;
  /** The folder to write the code into. */
  outputFolder?: vscode.Uri;
  /** The name of the generated package or crate. */
  name?: string;
}

/** What to generate code for: a metamodel file, or a metamodel resource of the engine (a
 * package of the metamodel view). */
export type CodeSource = { uri: vscode.Uri } | { resource: string; label: string; folder?: vscode.Uri };

const LAST_TARGET = "oxychloride.codegen.lastTarget";
const folderKey = (resource: string) => `oxychloride.codegen.folder:${resource}`;

/** The defaults for a metamodel file (or folder): its modeling project's `codegen`. */
export type CodegenDefaults = (near: vscode.Uri | undefined) => { target?: string; outputFolder?: vscode.Uri };

export class CodeGenerator {
  constructor(
    private readonly engine: Engine,
    private readonly state: vscode.Memento,
    private readonly output: vscode.LogOutputChannel,
    private readonly defaults: CodegenDefaults = () => ({}),
  ) {}

  /** Generates code and writes it: asks for the language and the output folder unless given
   * (by `options`, the `codegen` of the metamodel's modeling project, or the settings
   * `oxychloride.codegen.target` and `oxychloride.codegen.outputFolder`). */
  async generate(source: CodeSource, options: GenerateOptions = {}): Promise<GeneratedCode | undefined> {
    let resource: string;
    let label: string;
    let near: vscode.Uri | undefined;
    try {
      if ("uri" in source) {
        label = path.basename(source.uri.path);
        near = source.uri.scheme === "file" ? vscode.Uri.joinPath(source.uri, "..") : undefined;
        if (/\.ecore$/i.test(source.uri.path)) {
          resource = (await this.engine.loadEcore(source.uri.fsPath)).uri;
        } else {
          // Opening the document hands it to the language server (if it is not in a workspace folder).
          await vscode.workspace.openTextDocument(source.uri);
          resource = documentName(source.uri);
          if (!(await this.confirmErrors(resource, label))) return undefined;
        }
      } else {
        ({ resource, label } = source);
        near = source.folder;
      }
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not load ${"uri" in source ? path.basename(source.uri.path) : source.label}: ${errorMessage(e)}`);
      return undefined;
    }

    const project = this.defaults("uri" in source ? source.uri : near);
    const target = options.target ?? project.target ?? (await this.pickTarget(label));
    if (!target) return undefined;
    const folder = options.outputFolder ?? project.outputFolder ?? (await this.pickFolder(resource, label, near));
    if (!folder) return undefined;

    let code: GeneratedCode;
    try {
      code = await this.engine.generateCode({ target, resource, name: options.name });
      for (const f of code.files) {
        const segments = f.path.split("/");
        if (segments.some((s) => s === ".." || s === "")) throw new Error(`refusing to write ${f.path}`);
        await vscode.workspace.fs.writeFile(vscode.Uri.joinPath(folder, ...segments), new TextEncoder().encode(f.text));
      }
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not generate code for ${label}: ${errorMessage(e)}`);
      return undefined;
    }
    await this.state.update(LAST_TARGET, target);
    await this.state.update(folderKey(resource), folder.toString());
    this.report(code, label, folder);
    return code;
  }

  /** Whether to go on although the metamodel document has errors. */
  private async confirmErrors(name: string, label: string): Promise<boolean> {
    const { diagnostics } = await this.engine.call<{ diagnostics: Diagnostic[] }>("diagnostics", { name });
    const errors = diagnostics.filter((d) => d.severity === "error").length;
    if (!errors) return true;
    const choice = await vscode.window.showWarningMessage(
      `${label} has ${errors} error${errors === 1 ? "" : "s"}. Generate code anyway?`,
      { modal: true, detail: "OCL with errors is generated as code that fails when it runs." },
      "Generate",
    );
    return choice === "Generate";
  }

  /** The target: the setting, else one to pick (the one used last first). */
  private async pickTarget(label: string): Promise<string | undefined> {
    const configured = vscode.workspace.getConfiguration("oxychloride").get<string>("codegen.target");
    if (configured) return configured;
    let targets: CodeTarget[];
    try {
      targets = await this.engine.codeTargets();
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return undefined;
    }
    const last = this.state.get<string>(LAST_TARGET);
    targets.sort((a, b) => Number(b.id === last) - Number(a.id === last));
    const item = await vscode.window.showQuickPick(
      targets.map((t) => ({ label: t.name, detail: t.description, target: t })),
      { title: `Generate code for ${label}`, placeHolder: "Language", matchOnDetail: true },
    );
    return item?.target.id;
  }

  /** The output folder: the setting (absolute, or relative to the workspace folder), else one
   * to pick (the one used last for this metamodel, else the metamodel's folder, suggested). */
  private async pickFolder(resource: string, label: string, near?: vscode.Uri): Promise<vscode.Uri | undefined> {
    const configured = vscode.workspace.getConfiguration("oxychloride").get<string>("codegen.outputFolder");
    const workspaceFolder = (near && vscode.workspace.getWorkspaceFolder(near)) || vscode.workspace.workspaceFolders?.[0];
    if (configured) {
      if (path.isAbsolute(configured)) return vscode.Uri.file(configured);
      if (workspaceFolder) return vscode.Uri.joinPath(workspaceFolder.uri, configured);
    }
    const last = this.state.get<string>(folderKey(resource));
    const folders = await vscode.window.showOpenDialog({
      canSelectFolders: true,
      canSelectFiles: false,
      canSelectMany: false,
      defaultUri: last ? vscode.Uri.parse(last) : (near ?? workspaceFolder?.uri),
      openLabel: "Generate Here",
      title: `Folder for the code of ${label}`,
    });
    return folders?.[0];
  }

  private report(code: GeneratedCode, label: string, folder: vscode.Uri): void {
    const where = vscode.workspace.asRelativePath(folder);
    this.output.info(`Generated ${code.files.length} files (${code.target}) for ${code.packages.join(", ")} into ${folder.fsPath}`);
    for (const p of code.problems) this.output.warn(`${p.origin ? `${p.origin}: ` : ""}${p.message}`);
    const top = code.files[0]?.path.split("/")[0];
    const actions = [top ? "Reveal" : undefined, code.problems.length ? "Show Problems" : undefined].filter((a): a is string => !!a);
    const problems = code.problems.length
      ? ` ${code.problems.length} OCL construct${code.problems.length === 1 ? " is" : "s are"} not supported (see the output).`
      : "";
    void vscode.window.showInformationMessage(`Generated ${code.target} code for ${label} in ${where}.${problems}`, ...actions).then((choice) => {
      if (choice === "Reveal" && top) void vscode.commands.executeCommand("revealInExplorer", vscode.Uri.joinPath(folder, top));
      if (choice === "Show Problems") this.output.show(true);
    });
  }
}

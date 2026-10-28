// Eclipse projects and the target platform (see "Eclipse projects" in docs/PROJECTS.md): the
// Eclipse projects below the modeling projects (folders with a `.project` or a
// `META-INF/MANIFEST.MF`, e.g. the plug-ins of PCM) and the target platform their manifests
// name (`targetPlatform`, and the setting `oxychloride.targetPlatform`) are given to the engine
// (`setPlatform`), so that `platform:/resource` and `platform:/plugin` URIs, the packages the
// bundles register in their `plugin.xml`, their URI mappings and the file extensions of their
// models mean what they mean in Eclipse. Files of those extensions (`.repository`, …) are XML:
// XMI models. "Import Eclipse Projects…" makes a folder of Eclipse projects a modeling project.

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import * as vscode from "vscode";
import { errorMessage, type Engine, type PlatformInfo, type PlatformSettings, type TargetCandidate } from "./engine";
import { MANIFEST, type Projects } from "./projects";

const SECTION = "oxychloride";
const SETTING = "targetPlatform";

/** A path of a manifest or setting: `~/…` and absolute ones as they are, others relative to `base`. */
function resolveTarget(location: string, base: string): string {
  const l = location.trim();
  return l.startsWith("~/") || path.isAbsolute(l) ? l : path.join(base, l);
}

/** A path with its symbolic links resolved (the engine reports locations so; on macOS `/var` is `/private/var`). */
function realPath(location: string): string {
  try {
    return fs.realpathSync.native(location);
  } catch {
    return location;
  }
}

/** A path below the home folder as `~/…`, as manifests write it (so that it works for others too). */
function contractHome(location: string): string {
  const home = os.homedir();
  const relative = path.relative(home, location);
  return relative && !relative.startsWith("..") && !path.isAbsolute(relative) ? `~/${relative.split(path.sep).join("/")}` : location;
}

/** Answers to the questions of "Import Eclipse Projects…", for calls from code (tests). */
export interface ImportOptions {
  /** The target platform for the manifest (`[]`: none). */
  targetPlatform?: string[];
}

export class EclipsePlatform implements vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<void>();
  /** The engine's state of the Eclipse projects and the target platform changed. */
  readonly onDidChange = this.changed.event;
  /** As the engine last reported it. */
  info: PlatformInfo | undefined;
  private readonly disposables: vscode.Disposable[] = [];

  constructor(
    private readonly engine: Engine,
    private readonly projects: Projects,
  ) {
    // Bundle metadata: what the engine reads of the projects.
    const metadata = vscode.workspace.createFileSystemWatcher("**/{.project,MANIFEST.MF,plugin.xml,*.genmodel}");
    const resync = () => void this.sync();
    this.disposables.push(
      metadata,
      metadata.onDidCreate(resync),
      metadata.onDidChange(resync),
      metadata.onDidDelete(resync),
      vscode.workspace.onDidOpenTextDocument((d) => void this.asXml(d)),
    );
  }

  dispose(): void {
    this.disposables.forEach((d) => d.dispose());
    this.changed.dispose();
  }

  /** The settings for the engine: the modeling projects' folders, and the target platforms of
   * their manifests and of the setting (one target platform, as in an Eclipse workspace). */
  settings(): PlatformSettings {
    const target: string[] = [];
    const add = (t: string) => {
      if (t.trim() && !target.includes(t)) target.push(t);
    };
    for (const project of this.projects.projects) {
      for (const t of project.manifest.targetPlatform ?? []) {
        if (typeof t === "string") add(resolveTarget(t, project.root.fsPath));
      }
    }
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      for (const t of vscode.workspace.getConfiguration(SECTION, folder.uri).get<string[]>(SETTING) ?? []) add(resolveTarget(t, folder.uri.fsPath));
    }
    return { workspace: this.projects.projects.map((p) => p.root.fsPath), target };
  }

  /** Gives the engine the settings and reads its state. */
  async sync(): Promise<void> {
    this.info = await this.engine.setPlatform(this.settings()).catch(() => this.info);
    this.changed.fire();
    for (const d of vscode.workspace.textDocuments) await this.asXml(d);
  }

  /** The file extensions of models the bundles register. */
  modelExtensions(): string[] {
    return this.info?.modelExtensions ?? [];
  }

  /** Whether a file is a model of an extension a bundle registers. */
  isModel(uri: vscode.Uri): boolean {
    const extension = path.extname(uri.path).slice(1).toLowerCase();
    return !!extension && this.modelExtensions().some((e) => e.toLowerCase() === extension);
  }

  /** Opens a model of a registered extension as XML (VS Code knows the extension as plain text). */
  private async asXml(document: vscode.TextDocument): Promise<void> {
    if (document.uri.scheme === "file" && document.languageId === "plaintext" && this.isModel(document.uri)) {
      await vscode.languages.setTextDocumentLanguage(document, "xml");
    }
  }

  /** The Eclipse projects below a folder (by their location). */
  projectsIn(folder: vscode.Uri): PlatformInfo["projects"] {
    const root = realPath(folder.fsPath);
    return (this.info?.projects ?? []).filter((p) => {
      const relative = path.relative(root, realPath(p.location));
      return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
    });
  }

  // ---------------------------------------------------------------- importing

  /** Makes a folder of Eclipse projects a modeling project: asks for its target platform (the
   * ones on this machine that have most of the bundles the projects require first) and writes
   * the manifest. Returns the manifest. */
  async importProjects(folder?: vscode.Uri, options: ImportOptions = {}): Promise<vscode.Uri | undefined> {
    folder ??= await this.chooseFolder();
    if (!folder) return undefined;
    const scan = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: "Looking for Eclipse projects and target platforms…" },
      () => this.engine.scanEclipseProjects(folder.fsPath),
    );
    if (!scan.projects.length) {
      void vscode.window.showErrorMessage(`There are no Eclipse projects in ${folder.fsPath} (folders with a .project or a META-INF/MANIFEST.MF).`);
      return undefined;
    }
    const target = options.targetPlatform ?? (await this.chooseTarget(scan.targets, scan.required.length, folder));
    if (target === undefined) return undefined;
    let imported;
    try {
      imported = await this.engine.importEclipseProjects(folder.fsPath, target);
    } catch (e) {
      void vscode.window.showErrorMessage(`The Eclipse projects cannot be imported: ${errorMessage(e)}`);
      return undefined;
    }
    const manifest = vscode.Uri.joinPath(folder, MANIFEST);
    await vscode.workspace.fs.writeFile(manifest, new TextEncoder().encode(imported.manifest.text));
    if (!vscode.workspace.getWorkspaceFolder(folder)) {
      vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders?.length ?? 0, 0, { uri: folder });
    }
    await this.projects.load();
    const { platform } = imported;
    const packages = platform.projects.reduce((n, p) => n + p.packages.length, 0);
    const missing = platform.missing.length
      ? ` ${platform.missing.length} required bundle${platform.missing.length === 1 ? " is" : "s are"} missing: ${platform.missing.map((m) => m.name).join(", ")}.`
      : "";
    void vscode.window.showInformationMessage(
      `Imported ${platform.projects.length} Eclipse project${platform.projects.length === 1 ? "" : "s"} with ${packages} package${packages === 1 ? "" : "s"}.${missing}`,
    );
    return manifest;
  }

  private async chooseFolder(): Promise<vscode.Uri | undefined> {
    const picked = await vscode.window.showOpenDialog({
      canSelectFolders: true,
      canSelectFiles: false,
      defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
      openLabel: "Import",
      title: "Import Eclipse Projects: the folder with the projects (e.g. a repository of plug-ins)",
    });
    return picked?.[0];
  }

  /** The target platform for the manifest (`[]`: none), or `undefined` if cancelled. */
  private async chooseTarget(candidates: TargetCandidate[], required: number, folder: vscode.Uri): Promise<string[] | undefined> {
    type Item = vscode.QuickPickItem & { target?: string[]; browse?: boolean };
    const items: Item[] = candidates.map((c) => ({
      label: contractHome(c.location),
      description: c.kind,
      detail: required ? `has ${c.provides} of the ${required} bundles the projects require (${c.bundles} bundles)` : `${c.bundles} bundles`,
      target: [contractHome(c.location)],
    }));
    items.push(
      { label: "$(folder-opened) Choose a Target Platform…", detail: "An Eclipse installation, a folder of bundles (plugins, a bundle pool), or bundles", browse: true },
      { label: "No Target Platform", detail: "Only the metamodels of the projects themselves", target: [] },
    );
    const picked = await vscode.window.showQuickPick(items, {
      title: "Import Eclipse Projects: the target platform",
      placeHolder: required ? `The projects require ${required} bundles that are not among them` : "The projects require no further bundles",
      matchOnDescription: true,
    });
    if (!picked) return undefined;
    if (!picked.browse) return picked.target;
    const chosen = await vscode.window.showOpenDialog({ canSelectFolders: true, canSelectFiles: true, filters: { Bundles: ["jar"] }, openLabel: "Use as Target Platform" });
    if (!chosen?.[0]) return undefined;
    const relative = path.relative(folder.fsPath, chosen[0].fsPath);
    return [relative && !relative.startsWith("..") && !path.isAbsolute(relative) ? relative.split(path.sep).join("/") : contractHome(chosen[0].fsPath)];
  }
}

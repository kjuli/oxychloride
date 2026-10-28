// Modeling projects: folders with a manifest, `oxychloride.json` (see docs/PROJECTS.md), that
// hold metamodels, constraints, models and transformations. The view "Modeling Projects" shows
// each project's documents by kind and creates new ones in the folders its manifest names;
// "New Modeling Project…" writes a project the engine makes from its templates (`newProject`,
// the same as `oxy new`). The package registry includes the projects' registries
// (`registry.ts`), code generation uses their `codegen` defaults (`codegen.ts`).

import * as path from "node:path";
import * as vscode from "vscode";
import type { Creator } from "./create";
import {
  errorMessage,
  type DocumentLanguage,
  type EclipseBundleInfo,
  type Engine,
  type NewProject,
  type PackageInfo,
  type ValidationReport,
} from "./engine";
import type { EclipsePlatform } from "./platform";

export const MANIFEST = "oxychloride.json";

/** Where new documents of each kind go, relative to the project. */
export interface ProjectFolders {
  metamodels: string;
  constraints: string;
  models: string;
  transformations: string;
  generated: string;
}

/** The manifest as written (see `ProjectManifest` in crates/oxychloride/src/project.rs). */
export interface ProjectManifest {
  name?: string;
  description?: string;
  folders?: Partial<ProjectFolders>;
  registry?: Record<string, string>;
  codegen?: { target?: string; outputFolder?: string };
  /** The target platform of the project's Eclipse projects (see `platform.ts`). */
  targetPlatform?: string[];
}

/** Mirrors `ProjectFolders::default` in crates/oxychloride/src/project.rs. */
export const DEFAULT_FOLDERS: ProjectFolders = {
  metamodels: "metamodels",
  constraints: "constraints",
  models: "models",
  transformations: "transformations",
  generated: "src-gen",
};

/** Folders that never hold a project's documents (besides hidden ones and generated code), as
 * in project.rs; also skipped when looking for projects. */
const SKIPPED_DIRS = ["node_modules", "target", "out", "dist", "build"];
const EXCLUDE = `{${SKIPPED_DIRS.map((d) => `**/${d}/**`).join(",")}}`;

/** Whether `uri` is `folder` or below it. */
function isUnder(uri: vscode.Uri, folder: vscode.Uri): boolean {
  const relative = path.relative(folder.fsPath, uri.fsPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

export class ModelingProject {
  constructor(
    readonly manifestUri: vscode.Uri,
    readonly manifest: ProjectManifest,
    /** Why the manifest cannot be read (the project then has the defaults). */
    readonly problem?: string,
  ) {}

  get root(): vscode.Uri {
    return vscode.Uri.joinPath(this.manifestUri, "..");
  }

  get name(): string {
    return this.manifest.name?.trim() || path.basename(this.root.fsPath);
  }

  get folders(): ProjectFolders {
    const configured = Object.entries(this.manifest.folders ?? {}).filter(([, v]) => typeof v === "string" && v.trim());
    return { ...DEFAULT_FOLDERS, ...Object.fromEntries(configured) };
  }

  /** A path relative to the project (or absolute) as a URI. */
  resolve(relative: string): vscode.Uri {
    return path.isAbsolute(relative) ? vscode.Uri.file(relative) : vscode.Uri.joinPath(this.root, ...relative.split(/[\\/]/).filter(Boolean));
  }

  folder(kind: keyof ProjectFolders): vscode.Uri {
    return this.resolve(this.folders[kind]);
  }

  /** The folder generated code goes into. */
  codeFolder(): vscode.Uri {
    return this.resolve(this.manifest.codegen?.outputFolder || this.folders.generated);
  }

  /** The project's registry entries with absolute locations (a prefix's folder ends in a separator). */
  registry(): { uri: string; location: string }[] {
    return Object.entries(this.manifest.registry ?? {})
      .filter(([uri, location]) => uri.trim() && typeof location === "string")
      .map(([uri, location]) => {
        let resolved = this.resolve(location).fsPath;
        if (uri.endsWith("/") && !/[\\/]$/.test(resolved)) resolved += path.sep;
        return { uri, location: resolved };
      });
  }

  /** A path as the manifest writes it: relative to the project if inside it, with `/`s. */
  relative(location: string): string {
    const relative = path.relative(this.root.fsPath, location);
    return relative.startsWith("..") || path.isAbsolute(relative) ? location : relative.split(path.sep).join("/");
  }

  contains(uri: vscode.Uri): boolean {
    return isUnder(uri, this.root);
  }

  /** Changes the manifest: `change` gets the manifest as written (not the defaults). */
  async update(change: (manifest: ProjectManifest) => void): Promise<void> {
    const text = new TextDecoder().decode(await vscode.workspace.fs.readFile(this.manifestUri));
    let manifest: ProjectManifest;
    try {
      manifest = JSON.parse(text) as ProjectManifest;
    } catch (e) {
      throw new Error(`${vscode.workspace.asRelativePath(this.manifestUri)} cannot be read: ${errorMessage(e)}`);
    }
    change(manifest);
    if (manifest.registry && !Object.keys(manifest.registry).length) delete manifest.registry;
    await vscode.workspace.fs.writeFile(this.manifestUri, new TextEncoder().encode(`${JSON.stringify(manifest, null, 2)}\n`));
  }
}

async function readProject(uri: vscode.Uri): Promise<ModelingProject> {
  try {
    const manifest = JSON.parse(new TextDecoder().decode(await vscode.workspace.fs.readFile(uri))) as unknown;
    if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) throw new Error("the manifest is not an object");
    return new ModelingProject(uri, manifest as ProjectManifest);
  } catch (e) {
    return new ModelingProject(uri, {}, errorMessage(e));
  }
}

/** A kind of document, and where the view shows it. */
interface Category {
  id: "metamodels" | "constraints" | "models" | "henshin" | "qvt";
  label: string;
  extensions: string[];
  folder: keyof ProjectFolders;
  icon: string;
}

const CATEGORIES: Category[] = [
  { id: "metamodels", label: "Metamodels", extensions: ["oclinecore", "ecore"], folder: "metamodels", icon: "symbol-class" },
  { id: "constraints", label: "Constraints", extensions: ["ocl"], folder: "constraints", icon: "checklist" },
  { id: "models", label: "Models", extensions: ["hutn", "xmi"], folder: "models", icon: "symbol-object" },
  { id: "henshin", label: "Henshin Transformations", extensions: ["henshin_text", "henshin"], folder: "transformations", icon: "type-hierarchy" },
  { id: "qvt", label: "QVT Transformations", extensions: ["qvto", "qvtr"], folder: "transformations", icon: "arrow-swap" },
];

const DOCUMENT_GLOB = `**/*.{${CATEGORIES.flatMap((c) => c.extensions).join(",")}}`;

/** The documents' files, with the models of extensions bundles register (`.repository`, …). */
const documentGlob = (models: string[]) =>
  models.length ? `**/*.{${[...CATEGORIES.flatMap((c) => c.extensions), ...models].join(",")}}` : DOCUMENT_GLOB;

/** The kind of a document; files of extensions that bundles register are models. */
const categoryOf = (uri: vscode.Uri, platform?: EclipsePlatform) => {
  const extension = path.extname(uri.path).slice(1).toLowerCase();
  if (platform?.isModel(uri)) return CATEGORIES.find((c) => c.id === "models");
  return CATEGORIES.find((c) => c.extensions.includes(extension));
};

export type ProjectNode =
  | { kind: "project"; project: ModelingProject }
  | { kind: "category"; project: ModelingProject; category: Category }
  | { kind: "file"; project: ModelingProject; uri: vscode.Uri }
  | { kind: "problem"; project: ModelingProject; message: string }
  /** The Eclipse projects in the project's folder, and one of them. */
  | { kind: "eclipseProjects"; project: ModelingProject }
  | { kind: "bundle"; project: ModelingProject; bundle: EclipseBundleInfo }
  | { kind: "bundlePackage"; project: ModelingProject; bundle: EclipseBundleInfo; package: EclipseBundleInfo["packages"][number] };

/** The file a command was invoked on: a URI (Explorer, code) or a file of the Projects view. */
export function fileOf(arg: unknown): vscode.Uri | undefined {
  if (arg instanceof vscode.Uri) return arg;
  const node = arg as ProjectNode | undefined;
  return node?.kind === "file" ? node.uri : undefined;
}

/** Answers to the questions of "New Modeling Project…", for calls from code (tests). Given a
 * `name`, the other questions are not asked but answered by default (except `parent`). */
export interface NewProjectOptions {
  name?: string;
  nsUri?: string;
  metamodel?: "oclinecore" | "ecore";
  include?: DocumentLanguage[];
  /** A code generation target, or `null` for none. */
  codegen?: string | null;
  /** The folder the project's folder is created in. */
  parent?: vscode.Uri;
  /** What then: open the project in this window, in a new one, add it to the workspace, or
   * nothing (default: asked, unless it is in the workspace already). */
  open?: "here" | "newWindow" | "addToWorkspace" | "none";
}

/** Answers to the questions of the commands creating documents in a project. */
export interface NewDocumentOptions {
  /** The file's name without extension. */
  name?: string;
  /** Qualified names of the packages the document is about (QVT: source, then target). */
  packages?: string[];
  /** Models: `hutn` or `xmi`; Henshin: `henshin` (text) or `henshinModel`; QVT: `qvto` or `qvtr`. */
  language?: DocumentLanguage;
}

const PARTS: { language: DocumentLanguage; label: string; description: string }[] = [
  { language: "ocl", label: "Constraints", description: "Complete OCL (.ocl)" },
  { language: "hutn", label: "Model as text", description: "HUTN (.hutn)" },
  { language: "xmi", label: "Model as XMI", description: "as EMF writes it (.xmi)" },
  { language: "henshin", label: "Henshin rules as text", description: "Henshin's textual syntax (.henshin_text)" },
  { language: "henshinModel", label: "Henshin rules as a diagram", description: "Henshin's model (.henshin), kept equivalent to the text" },
  { language: "qvto", label: "QVT Operational transformation", description: "QVTo (.qvto)" },
  { language: "qvtr", label: "QVT Relations transformation", description: "QVTr (.qvtr)" },
];

export class Projects implements vscode.TreeDataProvider<ProjectNode>, vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  private readonly projectsChanged = new vscode.EventEmitter<void>();
  /** The projects (their manifests) were read again. */
  readonly onDidChangeProjects = this.projectsChanged.event;
  private readonly disposables: vscode.Disposable[] = [this.changed, this.projectsChanged];
  /** The documents of each project (by manifest), as last listed. */
  private readonly documents = new Map<string, Promise<vscode.Uri[]>>();
  private reloading: NodeJS.Timeout | undefined;
  private relisting: NodeJS.Timeout | undefined;

  projects: ModelingProject[] = [];
  /** The Eclipse projects and the target platform (set once both exist). */
  platform: EclipsePlatform | undefined;
  /** Resolves once the projects have been read for the first time. */
  readonly ready: Promise<void>;

  constructor(
    private readonly engine: Engine,
    private readonly creator: Creator,
    /** Resolves once the language server runs (it needs the projects' registries to start). */
    private readonly engineReady: () => Promise<void>,
  ) {
    const manifests = vscode.workspace.createFileSystemWatcher(`**/${MANIFEST}`);
    const documents = vscode.workspace.createFileSystemWatcher(DOCUMENT_GLOB, false, true, false);
    const reload = () => {
      clearTimeout(this.reloading);
      this.reloading = setTimeout(() => void this.load(), 100);
    };
    const relist = () => {
      clearTimeout(this.relisting);
      this.relisting = setTimeout(() => {
        this.documents.clear();
        this.changed.fire();
      }, 100);
    };
    this.disposables.push(
      manifests,
      manifests.onDidCreate(reload),
      manifests.onDidChange(reload),
      manifests.onDidDelete(reload),
      documents,
      documents.onDidCreate(relist),
      documents.onDidDelete(relist),
      vscode.workspace.onDidChangeWorkspaceFolders(reload),
    );
    this.ready = this.load();
  }

  dispose(): void {
    clearTimeout(this.reloading);
    clearTimeout(this.relisting);
    for (const d of this.disposables) d.dispose();
  }

  /** Reads the manifests of the workspace. */
  async load(): Promise<void> {
    const manifests = await vscode.workspace.findFiles(`**/${MANIFEST}`, EXCLUDE);
    manifests.sort((a, b) => a.fsPath.localeCompare(b.fsPath));
    this.projects = await Promise.all(manifests.map(readProject));
    this.documents.clear();
    this.projectsChanged.fire();
    this.changed.fire();
  }

  /** The project a file or folder belongs to: the innermost one containing it. */
  projectOf(uri: vscode.Uri | undefined): ModelingProject | undefined {
    if (!uri || uri.scheme !== "file") return undefined;
    return this.projects.filter((p) => p.contains(uri)).sort((a, b) => b.root.fsPath.length - a.root.fsPath.length)[0];
  }

  /** The documents of a project (as `Project::documents` in project.rs lists them). */
  documentsOf(project: ModelingProject): Promise<vscode.Uri[]> {
    const key = project.manifestUri.toString();
    let listed = this.documents.get(key);
    if (!listed) {
      listed = this.listDocuments(project);
      this.documents.set(key, listed);
    }
    return listed;
  }

  private async listDocuments(project: ModelingProject): Promise<vscode.Uri[]> {
    const glob = documentGlob(this.platform?.modelExtensions() ?? []);
    const files = await vscode.workspace.findFiles(new vscode.RelativePattern(project.root, glob), EXCLUDE);
    const generated = [project.folder("generated"), project.codeFolder()];
    const nested = this.projects.filter((p) => p !== project && project.contains(p.root));
    return files
      .filter((f) => {
        const segments = path.relative(project.root.fsPath, f.fsPath).split(path.sep).slice(0, -1);
        return (
          !segments.some((s) => s.startsWith(".") || SKIPPED_DIRS.includes(s)) &&
          !generated.some((g) => isUnder(f, g)) &&
          !nested.some((p) => p.contains(f))
        );
      })
      .sort((a, b) => a.fsPath.localeCompare(b.fsPath));
  }

  refresh(): void {
    this.documents.clear();
    this.changed.fire();
  }

  // ---------------------------------------------------------------- the view

  async getChildren(node?: ProjectNode): Promise<ProjectNode[]> {
    if (!node) return this.projects.map((project): ProjectNode => ({ kind: "project", project }));
    if (node.kind === "project") {
      const problem: ProjectNode[] = node.project.problem ? [{ kind: "problem", project: node.project, message: node.project.problem }] : [];
      const eclipse: ProjectNode[] = this.bundlesOf(node.project).length ? [{ kind: "eclipseProjects", project: node.project }] : [];
      return [...problem, ...eclipse, ...CATEGORIES.map((category): ProjectNode => ({ kind: "category", project: node.project, category }))];
    }
    if (node.kind === "category") {
      const files = await this.documentsOf(node.project);
      return files
        .filter((f) => categoryOf(f, this.platform) === node.category)
        .map((uri): ProjectNode => ({ kind: "file", project: node.project, uri }));
    }
    if (node.kind === "eclipseProjects") {
      return this.bundlesOf(node.project).map((bundle): ProjectNode => ({ kind: "bundle", project: node.project, bundle }));
    }
    if (node.kind === "bundle") {
      const packages = node.bundle.packages.map(
        (p): ProjectNode => ({ kind: "bundlePackage", project: node.project, bundle: node.bundle, package: p }),
      );
      const missing = node.bundle.requires
        .filter((r) => !r.location && !r.optional)
        .map((r): ProjectNode => ({ kind: "problem", project: node.project, message: `${r.name} is required, but neither a project nor in the target platform` }));
      const problems = (node.bundle.problems ?? []).map((message): ProjectNode => ({ kind: "problem", project: node.project, message }));
      return [...packages, ...missing, ...problems];
    }
    return [];
  }

  /** The Eclipse projects in a project's folder. */
  bundlesOf(project: ModelingProject): EclipseBundleInfo[] {
    return this.platform?.projectsIn(project.root) ?? [];
  }

  getParent(node: ProjectNode): ProjectNode | undefined {
    if (node.kind === "category" || node.kind === "problem" || node.kind === "eclipseProjects") {
      return { kind: "project", project: node.project };
    }
    if (node.kind === "bundle") return { kind: "eclipseProjects", project: node.project };
    if (node.kind === "bundlePackage") return { kind: "bundle", project: node.project, bundle: node.bundle };
    if (node.kind === "file") {
      const category = categoryOf(node.uri, this.platform);
      return category && { kind: "category", project: node.project, category };
    }
    return undefined;
  }

  async getTreeItem(node: ProjectNode): Promise<vscode.TreeItem> {
    switch (node.kind) {
      case "project": {
        const p = node.project;
        const item = new vscode.TreeItem(p.name, vscode.TreeItemCollapsibleState.Expanded);
        const folder = vscode.workspace.getWorkspaceFolder(p.root);
        item.description = folder && folder.uri.fsPath === p.root.fsPath ? undefined : vscode.workspace.asRelativePath(p.root);
        item.tooltip = new vscode.MarkdownString(
          [`**${p.name}**`, p.manifest.description ?? "", `\`${p.root.fsPath}\``].filter(Boolean).join("\n\n"),
        );
        item.iconPath = new vscode.ThemeIcon(p.problem ? "warning" : "project");
        item.contextValue = "project";
        item.id = p.manifestUri.toString();
        return item;
      }
      case "category": {
        const files = (await this.documentsOf(node.project)).filter((f) => categoryOf(f, this.platform) === node.category);
        const item = new vscode.TreeItem(
          node.category.label,
          files.length ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.None,
        );
        item.description = files.length ? undefined : "none";
        item.tooltip = `New ${node.category.label.toLowerCase()} go into ${node.project.folders[node.category.folder]}/`;
        item.iconPath = new vscode.ThemeIcon(node.category.icon);
        item.contextValue = `projectCategory.${node.category.id}`;
        item.id = `${node.project.manifestUri.toString()}#${node.category.id}`;
        return item;
      }
      case "file": {
        const item = new vscode.TreeItem(node.uri, vscode.TreeItemCollapsibleState.None);
        const folder = path.relative(node.project.root.fsPath, path.dirname(node.uri.fsPath)).split(path.sep).join("/");
        const category = categoryOf(node.uri, this.platform);
        item.description = category && folder === node.project.folders[category.folder] ? undefined : folder || undefined;
        item.command = { command: "vscode.open", title: "Open", arguments: [node.uri] };
        item.contextValue = `projectFile.${path.extname(node.uri.path).slice(1).toLowerCase()}`;
        item.id = node.uri.toString();
        return item;
      }
      case "eclipseProjects": {
        const bundles = this.bundlesOf(node.project);
        const item = new vscode.TreeItem("Eclipse Projects", vscode.TreeItemCollapsibleState.Collapsed);
        const target = this.platform?.info?.settings.target ?? [];
        item.description = `${bundles.length}`;
        item.tooltip = new vscode.MarkdownString(
          [
            "Eclipse projects (plug-ins): their `platform:/resource` and `platform:/plugin` URIs, the packages their `plugin.xml` registers, their URI mappings and model file extensions work as in Eclipse.",
            target.length ? `Target platform: ${target.map((t) => `\`${t}\``).join(", ")}` : "No target platform (`targetPlatform` in the manifest).",
          ].join("\n\n"),
        );
        item.iconPath = new vscode.ThemeIcon("extensions");
        item.contextValue = "eclipseProjects";
        item.id = `${node.project.manifestUri.toString()}#eclipse`;
        return item;
      }
      case "bundle": {
        const b = node.bundle;
        const missing = b.requires.filter((r) => !r.location && !r.optional).length;
        const item = new vscode.TreeItem(b.name, b.packages.length || missing ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
        item.description = [b.version, missing ? `${missing} missing` : undefined].filter(Boolean).join(" · ");
        item.tooltip = new vscode.MarkdownString(
          [
            `**${b.name}**${b.version ? ` ${b.version}` : ""}`,
            `\`${b.location}\``,
            b.requires.length ? `Requires: ${b.requires.map((r) => (r.location ? r.name : `~~${r.name}~~`)).join(", ")}` : "",
            b.modelExtensions?.length ? `Models: ${b.modelExtensions.map((e) => `\`.${e}\``).join(", ")}` : "",
          ]
            .filter(Boolean)
            .join("\n\n"),
        );
        item.iconPath = new vscode.ThemeIcon(missing ? "warning" : "package");
        item.resourceUri = vscode.Uri.file(b.location);
        item.contextValue = "bundle";
        item.id = `${node.project.manifestUri.toString()}#bundle:${b.location}`;
        return item;
      }
      case "bundlePackage": {
        const p = node.package;
        const item = new vscode.TreeItem(p.nsUri, vscode.TreeItemCollapsibleState.None);
        item.description = p.status === "loaded" ? p.location && path.basename(p.location) : (p.message ?? p.status);
        item.tooltip = [p.nsUri, p.location, p.message].filter(Boolean).join("\n");
        const ok = p.status === "loaded";
        item.iconPath = new vscode.ThemeIcon(ok ? "symbol-namespace" : "error", ok ? undefined : new vscode.ThemeColor("problemsErrorIcon.foreground"));
        if (p.location && !p.location.startsWith("archive:")) {
          item.command = { command: "vscode.open", title: "Open", arguments: [vscode.Uri.file(p.location)] };
        }
        item.contextValue = "bundlePackage";
        return item;
      }
      case "problem": {
        const item = new vscode.TreeItem(node.message);
        item.iconPath = new vscode.ThemeIcon("error", new vscode.ThemeColor("problemsErrorIcon.foreground"));
        item.command = { command: "vscode.open", title: "Open", arguments: [node.project.manifestUri] };
        return item;
      }
    }
  }

  // ---------------------------------------------------------------- new projects

  /** Creates a modeling project: asks for its name, nsURI, metamodel format, documents, code
   * generation and folder (unless `options` answers), writes what the engine makes of that,
   * and opens it. Returns its manifest. */
  async newProject(options: NewProjectOptions = {}): Promise<vscode.Uri | undefined> {
    await this.engineReady();
    const steps = 5;
    const title = (step: number) => `New Modeling Project (${step}/${steps})`;
    let checked: NewProject | undefined;
    const check = async (name: string): Promise<string | undefined> => {
      if (!name.trim()) return "Enter a name";
      if (/[\\/:*?"<>|]/.test(name)) return "A project's name is its folder's name: no \\ / : * ? \" < > |";
      try {
        checked = await this.engine.newProject({ name });
        return undefined;
      } catch (e) {
        return errorMessage(e);
      }
    };
    const name =
      options.name ??
      (await vscode.window.showInputBox({
        title: title(1),
        prompt: "The project's name; its folder and its metamodel's package are named after it",
        placeHolder: "e.g. Shop",
        validateInput: check,
      }));
    if (!name?.trim()) return undefined;
    if (!checked && options.name) {
      const problem = await check(name);
      if (problem) {
        void vscode.window.showErrorMessage(problem);
        return undefined;
      }
    }
    const pkg = checked?.package ?? name;
    // From code: the defaults.
    const ask = options.name === undefined;
    const nsUri =
      options.nsUri ??
      (!ask ? checked?.nsUri : undefined) ??
      (await vscode.window.showInputBox({
        title: title(2),
        prompt: `The nsURI of the metamodel's package ${pkg}: the URI models and other documents know it by (it need not be reachable)`,
        value: checked?.nsUri,
        validateInput: (v) => (!v.trim() ? "Enter a URI" : /['"\\\s]/.test(v) ? "A URI has no quotes, backslashes or spaces" : undefined),
      }));
    if (!nsUri) return undefined;

    type FormatItem = vscode.QuickPickItem & { format: "oclinecore" | "ecore" };
    const metamodel =
      options.metamodel ??
      (!ask ? "oclinecore" : undefined) ??
      (
        await vscode.window.showQuickPick<FormatItem>(
          [
            { label: "OCLinEcore", description: `${pkg}.oclinecore`, detail: "Ecore with its OCL as text; edited as text or as a diagram", format: "oclinecore" },
            { label: "Ecore", description: `${pkg}.ecore`, detail: "Ecore's XML, as EMF writes it; registered in the project's package registry", format: "ecore" },
          ],
          { title: title(3), placeHolder: "Write the metamodel as" },
        )
      )?.format;
    if (!metamodel) return undefined;

    type PartItem = vscode.QuickPickItem & { language: DocumentLanguage };
    const include =
      options.include ??
      (!ask ? PARTS.map((p) => p.language) : undefined) ??
      (
        await vscode.window.showQuickPick<PartItem>(
          PARTS.map((p) => ({ ...p, picked: true })),
          { title: title(4), placeHolder: "Start with these documents about the metamodel", canPickMany: true },
        )
      )?.map((i) => i.language);
    if (!include) return undefined;

    let codegen = options.codegen ?? (!ask ? null : undefined);
    if (codegen === undefined) {
      type TargetItem = vscode.QuickPickItem & { target: string | null };
      const targets = await this.engine.codeTargets().catch(() => []);
      const pick = await vscode.window.showQuickPick<TargetItem>(
        [
          { label: "None", detail: "Choose a language when generating code", target: null },
          ...targets.map((t) => ({ label: t.name, detail: t.description, target: t.id })),
        ],
        { title: title(5), placeHolder: "Generate code from the metamodel into" },
      );
      if (!pick) return undefined;
      codegen = pick.target;
    }

    let created: NewProject;
    try {
      created = await this.engine.newProject({ name, nsUri, metamodel, include, codegen: codegen ?? undefined });
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not create the project: ${errorMessage(e)}`);
      return undefined;
    }

    const parent =
      options.parent ??
      (
        await vscode.window.showOpenDialog({
          canSelectFiles: false,
          canSelectFolders: true,
          canSelectMany: false,
          title: `Create the folder of ${name} in`,
          openLabel: "Create Project Here",
          defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
        })
      )?.[0];
    if (!parent) return undefined;
    const folder = vscode.Uri.joinPath(parent, name.trim());
    const targets = created.files.map((f) => ({ uri: vscode.Uri.joinPath(folder, ...f.path.split("/")), text: f.text }));
    for (const t of targets) {
      if (await exists(t.uri)) {
        void vscode.window.showErrorMessage(`Could not create the project: ${vscode.workspace.asRelativePath(t.uri)} exists.`);
        return undefined;
      }
    }
    try {
      for (const t of targets) {
        await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(t.uri, ".."));
        await vscode.workspace.fs.writeFile(t.uri, new TextEncoder().encode(t.text));
      }
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not write the project: ${errorMessage(e)}`);
      return undefined;
    }
    const manifest = targets[0].uri;
    await this.open(folder, targets[1]?.uri, options.open);
    return manifest;
  }

  /** Opens a new project: inside the workspace its metamodel is opened and the project
   * revealed, else the folder is opened as asked. */
  private async open(folder: vscode.Uri, metamodel: vscode.Uri | undefined, how?: NewProjectOptions["open"]): Promise<void> {
    const inWorkspace = !!vscode.workspace.getWorkspaceFolder(folder);
    if (!how) {
      if (inWorkspace) {
        how = "none";
      } else if (!vscode.workspace.workspaceFolders?.length) {
        how = "here";
      } else {
        const choice = await vscode.window.showInformationMessage(
          `Created the modeling project ${path.basename(folder.fsPath)}. Open it?`,
          { modal: true, detail: folder.fsPath },
          "Open",
          "Open in New Window",
          "Add to Workspace",
        );
        how = choice === "Open" ? "here" : choice === "Open in New Window" ? "newWindow" : choice === "Add to Workspace" ? "addToWorkspace" : "none";
      }
    }
    if (how === "here" || how === "newWindow") {
      await vscode.commands.executeCommand("vscode.openFolder", folder, { forceNewWindow: how === "newWindow" });
      return;
    }
    if (how === "addToWorkspace") {
      vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders?.length ?? 0, 0, { uri: folder });
    }
    await this.load();
    if (metamodel && (inWorkspace || how === "addToWorkspace")) await vscode.commands.executeCommand("vscode.open", metamodel);
  }

  /** Makes a folder (default: one of the workspace to pick) a modeling project: writes a
   * manifest naming the folders new documents go into. */
  async initProject(folder?: vscode.Uri): Promise<vscode.Uri | undefined> {
    folder ??= (await vscode.window.showWorkspaceFolderPick({ placeHolder: "Make which folder a modeling project?" }))?.uri;
    if (!folder) return undefined;
    const manifest = vscode.Uri.joinPath(folder, MANIFEST);
    if (await exists(manifest)) {
      await vscode.commands.executeCommand("vscode.open", manifest);
      return manifest;
    }
    const content: ProjectManifest = { name: path.basename(folder.fsPath), folders: { ...DEFAULT_FOLDERS } };
    await vscode.workspace.fs.writeFile(manifest, new TextEncoder().encode(`${JSON.stringify(content, null, 2)}\n`));
    await this.load();
    await vscode.commands.executeCommand("vscode.open", manifest);
    return manifest;
  }

  // ---------------------------------------------------------------- new documents

  /** The project of a node, of the active editor, or one to pick. */
  private async chooseProject(node?: ProjectNode): Promise<ModelingProject | undefined> {
    if (node) return node.project;
    await this.ready;
    const active = this.projectOf(vscode.window.activeTextEditor?.document.uri);
    if (active) return active;
    if (this.projects.length === 1) return this.projects[0];
    if (!this.projects.length) {
      const choice = await vscode.window.showInformationMessage("There is no modeling project in the workspace.", "New Modeling Project…", "Make Folder a Modeling Project");
      if (choice === "New Modeling Project…") await this.newProject();
      if (choice === "Make Folder a Modeling Project") await this.initProject();
      return undefined;
    }
    const pick = await vscode.window.showQuickPick(
      this.projects.map((p) => ({ label: p.name, description: vscode.workspace.asRelativePath(p.root), project: p })),
      { title: "In which modeling project?" },
    );
    return pick?.project;
  }

  /** Creates a document of a kind (a category of the view) in the project's folder for it. */
  async newDocument(kind: Category["id"], node?: ProjectNode, options: NewDocumentOptions = {}): Promise<vscode.Uri | undefined> {
    const project = await this.chooseProject(node);
    if (!project) return undefined;
    await this.engineReady();
    try {
      switch (kind) {
        case "metamodels":
          return await this.newMetamodel(project, options);
        case "constraints":
          return await this.newConstraints(project, options);
        case "models":
          return await this.newModel(project, options);
        case "henshin":
          return await this.newHenshin(project, options);
        case "qvt":
          return await this.newQvt(project, options);
      }
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return undefined;
    }
  }

  private async newMetamodel(project: ModelingProject, options: NewDocumentOptions): Promise<vscode.Uri | undefined> {
    const name =
      options.name ??
      (await vscode.window.showInputBox({
        title: `New Metamodel in ${project.name}`,
        prompt: "The name of its package",
        placeHolder: "e.g. orders",
        validateInput: (v) => (v.trim() ? undefined : "Enter a name"),
      }));
    if (!name?.trim()) return undefined;
    const text = await this.engine.newDocument({ language: "oclinecore", name });
    const pkg = /^package (\w+)/m.exec(text)?.[1] ?? name;
    return this.write(vscode.Uri.joinPath(project.folder("metamodels"), `${pkg}.oclinecore`), text);
  }

  private async newConstraints(project: ModelingProject, options: NewDocumentOptions): Promise<vscode.Uri | undefined> {
    const packages = options.packages ?? (await this.pickPackages(project, "Constraints on which packages?", true));
    if (!packages?.length) return undefined;
    const stem = options.name ?? (await this.askName(project.folder("constraints"), shortName(packages[0]), "ocl", "New Constraints"));
    if (!stem) return undefined;
    const text = await this.engine.newDocument({ language: "ocl", name: stem, packages });
    return this.write(vscode.Uri.joinPath(project.folder("constraints"), `${stem}.ocl`), text);
  }

  private async newModel(project: ModelingProject, options: NewDocumentOptions): Promise<vscode.Uri | undefined> {
    const language = options.language ?? (await this.pickLanguage("New Model", [
      { language: "hutn", label: "HUTN", detail: "A model as text (.hutn), completed and checked as you type" },
      { language: "xmi", label: "XMI", detail: "A model as EMF writes it (.xmi), with a root object of a class you pick" },
    ]));
    if (!language) return undefined;
    const packages = options.packages ?? (await this.pickPackages(project, "A model of which metamodel?"));
    if (!packages?.length) return undefined;
    const extension = language === "xmi" ? "xmi" : "hutn";
    const stem = options.name ?? (await this.askName(project.folder("models"), "model", extension, "New Model"));
    if (!stem) return undefined;
    const target = vscode.Uri.joinPath(project.folder("models"), `${stem}.${extension}`);
    if (language === "hutn") {
      return this.write(target, await this.engine.newDocument({ language: "hutn", name: stem, packages }));
    }
    const metamodel = await this.metamodelFile(packages[0]);
    if (!metamodel) throw new Error(`The metamodel of ${packages[0]} is not a file.`);
    await vscode.workspace.fs.createDirectory(project.folder("models"));
    return this.creator.instantiate(metamodel, undefined, target);
  }

  private async newHenshin(project: ModelingProject, options: NewDocumentOptions): Promise<vscode.Uri | undefined> {
    const language = options.language ?? (await this.pickLanguage("New Henshin Transformation", [
      { language: "henshinModel", label: "Diagram", detail: "Henshin's model (.henshin), edited as rule diagrams" },
      { language: "henshin", label: "Text", detail: "Henshin's textual syntax (.henshin_text), with the rule diagram beside it" },
    ]));
    if (!language) return undefined;
    const packages = options.packages ?? (await this.pickPackages(project, "Transform models of which metamodel?"));
    if (!packages?.length) return undefined;
    const extension = language === "henshin" ? "henshin_text" : "henshin";
    const folder = project.folder("transformations");
    const stem = options.name ?? (await this.askName(folder, shortName(packages[0]), extension, "New Henshin Transformation"));
    if (!stem) return undefined;
    await vscode.workspace.fs.createDirectory(folder);
    const source = await this.metamodelFile(packages[0]);
    return vscode.commands.executeCommand<vscode.Uri | undefined>("oxychloride.newHenshinTransformation", source, vscode.Uri.joinPath(folder, `${stem}.${extension}`));
  }

  private async newQvt(project: ModelingProject, options: NewDocumentOptions): Promise<vscode.Uri | undefined> {
    const language = options.language ?? (await this.pickLanguage("New QVT Transformation", [
      { language: "qvto", label: "QVT Operational Mappings", detail: "Mappings and imperative OCL (.qvto)" },
      { language: "qvtr", label: "QVT Relations", detail: "Relations between the models, enforced in a direction or checked (.qvtr)" },
    ]));
    if (!language) return undefined;
    let packages = options.packages;
    if (!packages) {
      const source = await this.pickPackages(project, "Transform models of which metamodel?");
      if (!source?.length) return undefined;
      const target = await this.pickPackages(project, "Into models of which metamodel?", false, source[0]);
      if (!target?.length) return undefined;
      packages = [source[0], target[0]];
    }
    const [from, to] = [shortName(packages[0]), shortName(packages[packages.length - 1])];
    const folder = project.folder("transformations");
    const stem = options.name ?? (await this.askName(folder, from === to ? `${from}Transformation` : `${from}2${to}`, language, "New QVT Transformation"));
    if (!stem) return undefined;
    const text = await this.engine.newDocument({ language, name: stem, packages });
    return this.write(vscode.Uri.joinPath(folder, `${stem}.${language}`), text);
  }

  private async pickLanguage(
    title: string,
    items: (vscode.QuickPickItem & { language: DocumentLanguage })[],
  ): Promise<DocumentLanguage | undefined> {
    return (await vscode.window.showQuickPick(items, { title }))?.language;
  }

  /** Lets the user pick loaded packages (qualified names): the project's own first. */
  private async pickPackages(project: ModelingProject, title: string, many = false, first?: string): Promise<string[] | undefined> {
    const all: { qualified: string; info: PackageInfo }[] = [];
    const add = (p: PackageInfo, prefix: string) => {
      const qualified = prefix ? `${prefix}::${p.name}` : p.name;
      all.push({ qualified, info: p });
      for (const s of p.subpackages) add(s, qualified);
    };
    for (const p of await this.engine.packages()) add(p, "");
    if (!all.length) throw new Error("No metamodel is loaded: create one first (New Metamodel…).");
    const own = (p: PackageInfo) => /^([A-Za-z]:)?[\\/]/.test(p.resource) && project.contains(vscode.Uri.file(p.resource));
    all.sort((a, b) => Number(own(b.info)) - Number(own(a.info)) || Number(b.qualified === first) - Number(a.qualified === first));
    if (all.length === 1) return [all[0].qualified];
    type Item = vscode.QuickPickItem & { qualified: string };
    const items: Item[] = all.map((p) => ({
      label: p.qualified,
      description: p.info.nsUri,
      detail: own(p.info) ? `in ${project.name}: ${project.relative(p.info.resource)}` : vscode.workspace.asRelativePath(p.info.resource),
      qualified: p.qualified,
      picked: many && p === all[0],
    }));
    if (many) return (await vscode.window.showQuickPick(items, { title, canPickMany: true, matchOnDescription: true }))?.map((i) => i.qualified);
    const pick = await vscode.window.showQuickPick(items, { title, matchOnDescription: true });
    return pick && [pick.qualified];
  }

  /** The metamodel file a package was loaded from. */
  private async metamodelFile(qualified: string): Promise<vscode.Uri | undefined> {
    const root = qualified.split("::")[0];
    const pkg = (await this.engine.packages()).find((p) => p.name === root);
    return pkg && /^([A-Za-z]:)?[\\/]/.test(pkg.resource) ? vscode.Uri.file(pkg.resource) : undefined;
  }

  /** Asks for the name of a new file in `folder` (without extension), suggesting a free one. */
  private async askName(folder: vscode.Uri, stem: string, extension: string, title: string): Promise<string | undefined> {
    let suggestion = stem;
    for (let n = 2; await exists(vscode.Uri.joinPath(folder, `${suggestion}.${extension}`)); n++) suggestion = `${stem}${n}`;
    const name = await vscode.window.showInputBox({
      title,
      prompt: `The file's name, in ${vscode.workspace.asRelativePath(folder)}/`,
      value: suggestion,
      validateInput: async (v) => {
        if (!/^[\w.-]+$/.test(v)) return "Letters, digits, '_', '-' and '.' only";
        return (await exists(vscode.Uri.joinPath(folder, `${v}.${extension}`))) ? `${v}.${extension} exists` : undefined;
      },
    });
    return name?.trim() || undefined;
  }

  /** Writes a new file (not replacing one) and opens it. */
  private async write(target: vscode.Uri, text: string): Promise<vscode.Uri | undefined> {
    if (await exists(target)) throw new Error(`${vscode.workspace.asRelativePath(target)} exists.`);
    await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(target, ".."));
    await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
    await vscode.window.showTextDocument(target, { preview: false });
    return target;
  }

  // ---------------------------------------------------------------- the project as a whole

  /** Checks every model of a project (also those not open) against all constraints. Returns
   * the report, restricted to the project's models. */
  async validate(node?: ProjectNode, includeSatisfied = false): Promise<ValidationReport | undefined> {
    const project = await this.chooseProject(node);
    if (!project) return undefined;
    await this.engineReady();
    const models = (await this.documentsOf(project)).filter((f) => categoryOf(f, this.platform)?.id === "models");
    // Opening a document hands it to the language server.
    for (const m of models) await vscode.workspace.openTextDocument(m);
    const report = await this.engine.validate({ ecore: true, contracts: true, includeSatisfied: true });
    const inProject = (resource: string) => /^([A-Za-z]:)?[\\/]/.test(resource) && project.contains(vscode.Uri.file(resource));
    const results = report.results.filter((r) => inProject(r.object.resource));
    const count = (status: string) => results.filter((r) => r.status === status).length;
    return {
      ...report,
      results: includeSatisfied ? results : results.filter((r) => r.status !== "satisfied"),
      summary: {
        ...report.summary,
        objects: new Set(results.map((r) => `${r.object.resource}#${r.object.fragment}`)).size,
        evaluations: results.length,
        satisfied: count("satisfied"),
        violated: count("violated"),
        errors: count("error"),
      },
    };
  }

  /** Generates the code of a project's metamodel (one to pick if it has several) with the
   * manifest's `codegen` settings. */
  async generateCode(node?: ProjectNode): Promise<unknown> {
    const project = await this.chooseProject(node);
    if (!project) return undefined;
    const metamodels = (await this.documentsOf(project)).filter((f) => categoryOf(f, this.platform)?.id === "metamodels");
    let metamodel: vscode.Uri | undefined = metamodels[0];
    if (metamodels.length > 1) {
      // A generated `.ecore` next to its OCLinEcore document is the same metamodel.
      const documents = metamodels.filter((m) => !metamodels.some((o) => o !== m && /\.oclinecore$/i.test(o.path) && o.path.replace(/\.oclinecore$/i, ".ecore") === m.path));
      metamodel = documents.length === 1 ? documents[0] : (
        await vscode.window.showQuickPick(
          documents.map((m) => ({ label: path.basename(m.path), description: project.relative(m.fsPath), uri: m })),
          { title: `Generate code for which metamodel of ${project.name}?` },
        )
      )?.uri;
    }
    if (!metamodel) {
      if (!metamodels.length) void vscode.window.showInformationMessage(`${project.name} has no metamodel.`);
      return undefined;
    }
    return vscode.commands.executeCommand("oxychloride.generateCode", metamodel);
  }

  /** The code generation defaults for a file: those of its project's manifest. */
  codegenDefaults(near: vscode.Uri | undefined): { target?: string; outputFolder?: vscode.Uri } {
    const project = this.projectOf(near);
    const codegen = project?.manifest.codegen;
    if (!project || !codegen) return {};
    return { target: codegen.target || undefined, outputFolder: project.codeFolder() };
  }
}

/** `a::b` → `b`. */
function shortName(qualified: string): string {
  return qualified.split("::").pop() ?? qualified;
}

async function exists(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}

// The package registry: URIs standing for metamodel files, like EMF's package registry and URI
// map. It is kept in the setting `oxychloride.registry` (`{"<uri>": "<path>"}`, paths relative to
// the workspace folder), given to the engine — which loads the registered files right away and
// reads every reference to a registered URI (`.ecore` hrefs, imports, nsURIs of models,
// `xsi:schemaLocation`s) from them — and shown in the view "Package Registry".

import * as path from "node:path";
import * as vscode from "vscode";
import { errorMessage, type Engine, type RegisteredPackage, type RegisteredResource, type RegistryEntry, type RegistryInfo } from "./engine";

const SECTION = "oxychloride";
const SETTING = "registry";

/** Extensions of files offered for registration (Ecore XML may have any extension). */
const METAMODEL_EXTENSIONS = ["ecore", "xml", "xmi", "emof"];

/** A registry entry as configured: where it is written, and its location as written. */
interface ConfiguredEntry extends RegistryEntry {
  folder?: vscode.WorkspaceFolder;
  value: string;
}

export type RegistryNode =
  | { kind: "entry"; info: RegistryInfo }
  | { kind: "resource"; resource: RegisteredResource }
  | { kind: "package"; package: RegisteredPackage }
  | { kind: "problem"; message: string; severity: "error" | "warning" };

/** The root packages (name and nsURI) of an Ecore XML text. */
export function packagesOf(text: string): RegisteredPackage[] {
  const packages: RegisteredPackage[] = [];
  for (const m of text.matchAll(/<(?:[\w.-]+:)?EPackage\b([^>]*)>/g)) {
    const attr = (name: string) => new RegExp(`\\b${name}\\s*=\\s*"([^"]*)"`).exec(m[1])?.[1];
    const nsUri = attr("nsURI");
    if (nsUri) packages.push({ name: attr("name") ?? "", nsUri });
  }
  return packages;
}

const icon = (id: string, color?: string) => new vscode.ThemeIcon(id, color ? new vscode.ThemeColor(color) : undefined);

export class PackageRegistry implements vscode.TreeDataProvider<RegistryNode>, vscode.CodeActionProvider {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;
  /** The entries and their state, as the engine last reported them. */
  infos: RegistryInfo[] = [];

  constructor(private readonly engine: Engine) {}

  // ---------------------------------------------------------------- the setting

  /** The configured entries, with absolute locations (the first folder defining a URI wins). */
  configured(): ConfiguredEntry[] {
    const folders = vscode.workspace.workspaceFolders ?? [];
    const out: ConfiguredEntry[] = [];
    for (const folder of folders.length ? folders : [undefined]) {
      const map = vscode.workspace.getConfiguration(SECTION, folder?.uri).get<Record<string, string>>(SETTING) ?? {};
      for (const [uri, value] of Object.entries(map)) {
        if (typeof value !== "string" || !uri.trim() || out.some((e) => e.uri === uri)) continue;
        out.push({ uri, value, folder, location: resolveLocation(value, folder) });
      }
    }
    return out;
  }

  /** The entries the engine needs (also the language server's `initializationOptions`). */
  entries(): RegistryEntry[] {
    return this.configured().map(({ uri, location }) => ({ uri, location }));
  }

  /** Gives the engine the configured registry. */
  async sync(): Promise<void> {
    this.infos = await this.engine.setRegistry(this.entries()).catch(() => this.infos);
    this.changed.fire();
  }

  /** Reads the state of the entries again (files may have changed). */
  async refresh(): Promise<void> {
    this.infos = await this.engine.registry().catch(() => this.infos);
    this.changed.fire();
  }

  /** Writes (or, with `location` undefined, removes) the entries for URIs: where they are
   * configured, else into the settings of the workspace folder containing `location`. Then
   * gives the engine the new registry. */
  private async write(changes: [uri: string, location: string | undefined][]): Promise<void> {
    const { WorkspaceFolder, Workspace, Global } = vscode.ConfigurationTarget;
    for (const [uri, location] of changes) {
      const existing = this.configured().find((e) => e.uri === uri);
      const folder =
        existing?.folder ??
        (location ? vscode.workspace.getWorkspaceFolder(vscode.Uri.file(location)) : undefined) ??
        vscode.workspace.workspaceFolders?.[0];
      const config = vscode.workspace.getConfiguration(SECTION, folder?.uri);
      const inspected = config.inspect<Record<string, string>>(SETTING);
      const target =
        inspected?.workspaceFolderValue?.[uri] !== undefined
          ? WorkspaceFolder
          : inspected?.workspaceValue?.[uri] !== undefined
            ? Workspace
            : inspected?.globalValue?.[uri] !== undefined || !folder
              ? Global
              : WorkspaceFolder;
      const current =
        target === WorkspaceFolder ? inspected?.workspaceFolderValue : target === Workspace ? inspected?.workspaceValue : inspected?.globalValue;
      const map = { ...(current ?? {}) };
      if (location === undefined) {
        delete map[uri];
      } else {
        map[uri] = target === Global || !folder ? location : relativeLocation(location, folder);
      }
      await config.update(SETTING, Object.keys(map).length ? map : undefined, target);
    }
    await this.sync();
  }

  // ---------------------------------------------------------------- commands

  /** Registers a metamodel file under a URI; asks for what is not given. Returns the entry's
   * state. */
  async register(file?: vscode.Uri, uri?: string): Promise<RegistryInfo | undefined> {
    file ??= await this.chooseFile(uri);
    if (!file || file.scheme !== "file") return undefined;
    let text: string;
    try {
      text = new TextDecoder().decode(await vscode.workspace.fs.readFile(file));
    } catch (e) {
      void vscode.window.showErrorMessage(`Cannot read ${path.basename(file.fsPath)}: ${errorMessage(e)}`);
      return undefined;
    }
    uri ??= await this.chooseUri(file, packagesOf(text));
    if (!uri) return undefined;
    const existing = this.configured().find((e) => e.uri === uri);
    if (existing && path.resolve(existing.location) !== path.resolve(file.fsPath)) {
      const choice = await vscode.window.showWarningMessage(
        `'${uri}' is registered for ${vscode.workspace.asRelativePath(existing.location)}. Register ${path.basename(file.fsPath)} instead?`,
        { modal: true },
        "Replace",
      );
      if (choice !== "Replace") return undefined;
    }
    await this.write([[uri, file.fsPath]]);
    const info = this.infos.find((i) => i.uri === uri);
    if (info && (info.status === "error" || info.status === "missing")) {
      void vscode.window.showWarningMessage(`Registered '${uri}', but ${path.basename(file.fsPath)} cannot be loaded: ${info.message ?? info.status}`);
    }
    return info;
  }

  private async chooseFile(uri?: string): Promise<vscode.Uri | undefined> {
    const active = vscode.window.activeTextEditor?.document.uri;
    if (!uri && active && /\.ecore$/i.test(active.path)) return active;
    const files = await vscode.window.showOpenDialog({
      canSelectMany: false,
      filters: { "Ecore metamodels": METAMODEL_EXTENSIONS, "All files": ["*"] },
      title: uri ? `Register a metamodel file for '${uri}'` : "Register metamodel",
      openLabel: "Register",
      defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
    });
    return files?.[0];
  }

  /** Asks for the URI to register a file under: the nsURI of one of its packages, or another. */
  private async chooseUri(file: vscode.Uri, packages: RegisteredPackage[]): Promise<string | undefined> {
    type Item = vscode.QuickPickItem & { uri?: string };
    const items: Item[] = packages.map((p) => ({ label: p.nsUri, description: `nsURI of package ${p.name}`, uri: p.nsUri }));
    items.push({
      label: "$(edit) Another URI…",
      description: "a URI other files use for it, e.g. a placeholder or a platform: URI",
    });
    const pick =
      packages.length === 0 ? items[items.length - 1] : await vscode.window.showQuickPick(items, { title: `Register ${path.basename(file.fsPath)} as` });
    if (!pick) return undefined;
    if (pick.uri) return pick.uri;
    const value = await vscode.window.showInputBox({
      title: `Register ${path.basename(file.fsPath)} as`,
      prompt: "The URI that stands for the file: references to it are read from the file (it need not be reachable).",
      placeHolder: "e.g. http://example.org/shop or platform:/plugin/org.example.shop/model/shop.ecore",
      value: packages[0]?.nsUri,
      validateInput: (v) => (v.trim() ? undefined : "Enter a URI"),
    });
    return value?.trim();
  }

  /** Maps a URI prefix to a folder (files under it are loaded when referred to). */
  async mapFolder(folder?: vscode.Uri, prefix?: string): Promise<RegistryInfo | undefined> {
    folder ??= (
      await vscode.window.showOpenDialog({
        canSelectFiles: false,
        canSelectFolders: true,
        title: "Map a URI prefix to folder",
        openLabel: "Map",
        defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
      })
    )?.[0];
    if (!folder || folder.scheme !== "file") return undefined;
    prefix ??= (
      await vscode.window.showInputBox({
        title: `Map a URI prefix to ${path.basename(folder.fsPath)}`,
        prompt: "URIs starting with the prefix are read from the folder.",
        placeHolder: "e.g. pathmap://MODELS/ or platform:/plugin/org.example.shop/model/",
        validateInput: (v) => (v.trim() ? undefined : "Enter a URI prefix"),
      })
    )?.trim();
    if (!prefix) return undefined;
    if (!prefix.endsWith("/")) prefix += "/";
    await this.write([[prefix, folder.fsPath]]);
    return this.infos.find((i) => i.uri === prefix);
  }

  async unregister(node?: RegistryNode | string): Promise<void> {
    let uri = typeof node === "string" ? node : node?.kind === "entry" ? node.info.uri : undefined;
    if (!uri) {
      const pick = await vscode.window.showQuickPick(
        this.infos.map((i) => ({ label: i.uri, description: vscode.workspace.asRelativePath(i.location) })),
        { title: "Unregister" },
      );
      uri = pick?.label;
    }
    if (uri) await this.write([[uri, undefined]]);
  }

  /** Registers `.ecore` files of the workspace that are not registered yet, under their
   * packages' nsURIs; the user picks which. */
  async registerWorkspace(): Promise<void> {
    const registered = new Set(this.infos.map((i) => path.resolve(i.location)));
    const files = await vscode.workspace.findFiles("**/*.ecore", "{**/node_modules/**,**/target/**,**/out/**,**/bin/**}");
    // Metamodels written in OCLinEcore are there already: their generated `.ecore` would clash.
    const documentPackages = new Map(
      (await this.engine.packages().catch(() => []))
        .filter((p) => !/\.ecore$/i.test(p.resource))
        .map((p) => [p.nsUri, path.basename(p.resource)]),
    );
    type Item = vscode.QuickPickItem & { file: vscode.Uri; nsUri: string };
    const items: Item[] = [];
    for (const file of files.sort((a, b) => a.fsPath.localeCompare(b.fsPath))) {
      if (registered.has(path.resolve(file.fsPath))) continue;
      const text = new TextDecoder().decode(await vscode.workspace.fs.readFile(file));
      const nsUri = packagesOf(text)[0]?.nsUri;
      if (!nsUri || this.infos.some((i) => i.uri === nsUri)) continue;
      const shadowing = documentPackages.get(nsUri);
      items.push({
        label: vscode.workspace.asRelativePath(file),
        description: nsUri,
        detail: shadowing && `defined by ${shadowing} already`,
        picked: !shadowing,
        file,
        nsUri,
      });
    }
    if (!items.length) {
      void vscode.window.showInformationMessage("All .ecore files of the workspace are registered.");
      return;
    }
    const picks = await vscode.window.showQuickPick(items, { canPickMany: true, title: "Register metamodels under their nsURIs" });
    if (!picks?.length) return;
    await this.write(picks.map((p): [string, string] => [p.nsUri, p.file.fsPath]));
  }

  async openSettings(): Promise<void> {
    const folder = vscode.workspace.workspaceFolders?.[0];
    await vscode.commands.executeCommand(folder ? "workbench.action.openWorkspaceSettingsFile" : "workbench.action.openSettingsJson");
  }

  // ---------------------------------------------------------------- quick fixes

  static readonly fixable = [
    /cannot find imported metamodel '([^']+)'/,
    /unknown package nsURI '([^']+)'/,
    /no metamodel with nsURI '([^']+)'/,
    /cannot resolve reference '([a-z][\w+.-]*:[^'#]*)#/i,
  ];

  provideCodeActions(_doc: vscode.TextDocument, _range: vscode.Range, context: vscode.CodeActionContext): vscode.CodeAction[] {
    const uris = new Set<string>();
    for (const d of context.diagnostics) {
      for (const pattern of PackageRegistry.fixable) {
        const uri = pattern.exec(d.message)?.[1];
        if (uri) uris.add(uri);
      }
    }
    return [...uris].map((uri) => {
      const action = new vscode.CodeAction(`Register a metamodel file for '${uri}'…`, vscode.CodeActionKind.QuickFix);
      action.command = { command: "oxychloride.registerMetamodel", title: action.title, arguments: [undefined, uri] };
      action.diagnostics = context.diagnostics.filter((d) => d.message.includes(uri));
      return action;
    });
  }

  // ---------------------------------------------------------------- the view

  getChildren(node?: RegistryNode): RegistryNode[] {
    if (!node) return this.infos.map((info): RegistryNode => ({ kind: "entry", info }));
    if (node.kind === "entry") {
      const i = node.info;
      return [
        ...(i.message ? [{ kind: "problem", message: i.message, severity: "error" } as const] : []),
        ...(i.resource?.packages ?? []).map((p): RegistryNode => ({ kind: "package", package: p })),
        ...i.imports.map((r): RegistryNode => ({ kind: "resource", resource: r })),
        ...i.warnings.map((w): RegistryNode => ({ kind: "problem", message: w.message, severity: "warning" })),
      ];
    }
    if (node.kind === "resource") return node.resource.packages.map((p): RegistryNode => ({ kind: "package", package: p }));
    return [];
  }

  getTreeItem(node: RegistryNode): vscode.TreeItem {
    switch (node.kind) {
      case "entry": {
        const i = node.info;
        const children = this.getChildren(node).length > 0;
        const item = new vscode.TreeItem(i.uri, children ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
        item.description = vscode.workspace.asRelativePath(i.location);
        const problems = i.status === "missing" || i.status === "error";
        item.iconPath =
          i.status === "folder"
            ? icon("folder")
            : problems
              ? icon("error", "problemsErrorIcon.foreground")
              : i.warnings.length
                ? icon("warning", "problemsWarningIcon.foreground")
                : icon("package");
        const aliases = (i.resource?.aliases ?? []).filter((a) => a !== i.uri);
        item.tooltip = new vscode.MarkdownString(
          [
            `**${i.uri}**`,
            `→ \`${i.location}\``,
            i.status === "folder" ? "URIs starting with this prefix are read from the folder." : "",
            i.message ?? "",
            aliases.length ? `Also known as: ${aliases.map((a) => `\`${a}\``).join(", ")}` : "",
            i.imports.length ? `Uses: ${i.imports.map((r) => path.basename(r.uri)).join(", ")}` : "",
          ]
            .filter(Boolean)
            .join("\n\n"),
        );
        item.contextValue = "registryEntry";
        if (i.status !== "folder" && i.status !== "missing") {
          item.command = { command: "vscode.open", title: "Open", arguments: [vscode.Uri.file(i.location)] };
        }
        return item;
      }
      case "resource": {
        const r = node.resource;
        const item = new vscode.TreeItem(path.basename(r.uri), vscode.TreeItemCollapsibleState.Collapsed);
        item.description = `imported · ${vscode.workspace.asRelativePath(r.uri)}`;
        item.tooltip = [r.uri, ...(r.aliases ?? []).map((a) => `also known as ${a}`)].join("\n");
        item.iconPath = icon("file-symlink-file");
        item.contextValue = "registryImport";
        if (/^([A-Za-z]:)?[\\/]/.test(r.uri)) item.command = { command: "vscode.open", title: "Open", arguments: [vscode.Uri.file(r.uri)] };
        return item;
      }
      case "package": {
        const item = new vscode.TreeItem(node.package.name);
        item.description = node.package.nsUri;
        item.iconPath = icon("symbol-namespace");
        return item;
      }
      case "problem": {
        const item = new vscode.TreeItem(node.message);
        item.tooltip = node.message;
        item.iconPath =
          node.severity === "error" ? icon("error", "problemsErrorIcon.foreground") : icon("warning", "problemsWarningIcon.foreground");
        return item;
      }
    }
  }
}

/** A configured location as an absolute path: `${workspaceFolder}` and relative paths refer to
 * the workspace folder whose settings define it. */
function resolveLocation(value: string, folder?: vscode.WorkspaceFolder): string {
  const base = folder?.uri.fsPath ?? vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? process.cwd();
  const expanded = value.replace(/\$\{workspaceFolder\}/g, base);
  return path.resolve(base, expanded);
}

/** A location as written into a folder's settings: relative to the folder if inside it. */
function relativeLocation(location: string, folder: vscode.WorkspaceFolder): string {
  const relative = path.relative(folder.uri.fsPath, location);
  return relative.startsWith("..") || path.isAbsolute(relative) ? location : relative.split(path.sep).join("/");
}

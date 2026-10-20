// Oxychloride for VS Code: a client of the engine's language server (`oxy lsp`) plus views
// (metamodels, models, validation results), an OCL console, commands to create model content,
// and XML Schemas for XMI models.

import * as path from "node:path";
import * as vscode from "vscode";
import { LanguageClient, TransportKind, type LanguageClientOptions, type ServerOptions } from "vscode-languageclient/node";
import { OclConsole, type ConsoleContext } from "./console";
import { Creator } from "./create";
import { documentName, reveal } from "./documents";
import {
  Engine,
  errorMessage,
  type ClassifierInfo,
  type ConstraintInfo,
  type Diagnostic,
  type ObjectInfo,
  type PackageInfo,
} from "./engine";
import { SchemaRegistry } from "./schemas";
import { serverExecutable } from "./server";
import { MetamodelTree, ModelTree, ValidationTree, shortName, type MetamodelNode, type ModelNode, type ValidationNode } from "./trees";

const LANGUAGES = ["ocl", "oclinecore", "hutn"];

/** What the extension offers other extensions (and its tests). */
export interface OxychlorideApi {
  engine: Engine;
  console: OclConsole;
  creator: Creator;
  metamodels: MetamodelTree;
  models: ModelTree;
  validation: ValidationTree;
  /** Resolves once the language server is running. */
  ready: Promise<void>;
}

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<OxychlorideApi> {
  const output = vscode.window.createOutputChannel("Oxychloride", { log: true });
  const engine = new Engine(() => client);
  const metamodels = new MetamodelTree(engine);
  const models = new ModelTree(engine);
  const validation = new ValidationTree();
  const creator = new Creator(engine, () => metamodels.packages);
  const schemas = new SchemaRegistry(engine, context, output);
  const oclConsole = new OclConsole(engine, () => chooseContext(models.models));

  const refresh = async () => {
    await Promise.all([metamodels.refresh(), models.refresh()]);
    await schemas.update();
  };

  const startClient = async () => {
    const command = serverExecutable(context.extensionPath);
    output.info(`Starting the language server: ${command} lsp`);
    const serverOptions: ServerOptions = { command, args: ["lsp"], transport: TransportKind.stdio };
    const clientOptions: LanguageClientOptions = {
      // Only real and unsaved files: not the old versions shown in diffs.
      documentSelector: [
        ...LANGUAGES.flatMap((language) => [
          { scheme: "file", language },
          { scheme: "untitled", language },
        ]),
        { scheme: "file", language: "xml", pattern: "**/*.xmi" },
      ],
      synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/*.{ocl,oclinecore,ecore}") },
      outputChannel: output,
    };
    client = new LanguageClient("oxychloride", "Oxychloride", serverOptions, clientOptions);
    client.onNotification("oxychloride/workspaceChanged", () => void refresh());
    await client.start();
    await refresh();
  };

  const ready = startClient().catch((e) => {
    const message = `Oxychloride: the language server could not be started (${errorMessage(e)}). Build it with \`cargo build -p oxy-cli\` or set "oxychloride.server.path".`;
    output.error(message);
    void vscode.window.showErrorMessage(message, "Open Settings").then((choice) => {
      if (choice) void vscode.commands.executeCommand("workbench.action.openSettings", "oxychloride.server.path");
    });
  });

  const setContext = (c: ConsoleContext | undefined) => {
    oclConsole.setContext(c);
    models.setContext(c);
  };

  const register = (id: string, run: (...args: never[]) => unknown) =>
    context.subscriptions.push(vscode.commands.registerCommand(id, run));

  context.subscriptions.push(
    output,
    vscode.window.registerTreeDataProvider("oxychloride.metamodels", metamodels),
    vscode.window.registerTreeDataProvider("oxychloride.models", models),
    vscode.window.registerTreeDataProvider("oxychloride.validation", validation),
    vscode.window.registerWebviewViewProvider(OclConsole.viewId, oclConsole, { webviewOptions: { retainContextWhenHidden: true } }),
    vscode.window.onDidChangeActiveTextEditor((e) => creator.track(e)),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("oxychloride.xmlSchemas")) void schemas.update();
    }),
    { dispose: () => void schemas.dispose() },
  );
  creator.track(vscode.window.activeTextEditor);

  // ---------------------------------------------------------------- creating

  register("oxychloride.createInstance", (node?: MetamodelNode) => {
    if (node?.kind === "classifier") return creator.createInstance(node.classifier);
  });
  register("oxychloride.createInstanceInNewModel", (node?: MetamodelNode) => {
    if (node?.kind === "classifier") return creator.createInstance(node.classifier, true);
  });
  register("oxychloride.createXmiInstance", (node?: MetamodelNode) => {
    if (node?.kind === "classifier") return creator.createXmiInstance(node.classifier);
  });
  register("oxychloride.newModel", (node?: MetamodelNode) =>
    creator.newModel(node?.kind === "package" ? node.package : undefined),
  );
  register("oxychloride.addChild", (node?: ModelNode) => node && creator.addChild(node));

  // ---------------------------------------------------------------- navigating

  register("oxychloride.showDefinition", (node?: ModelNode | ValidationNode) => {
    if (node?.kind === "object") return reveal(node.resource, node.object.range);
    if (node?.kind === "result") return reveal(node.result.object.resource, node.result.object.range);
  });
  register("oxychloride.showConstraint", (c?: ConstraintInfo) => {
    if (c?.origin.kind === "document") return reveal(c.origin.name, c.range);
  });
  register("oxychloride.useAsContext", (node?: ModelNode | ValidationNode) => {
    const o =
      node?.kind === "object"
        ? { ...node.object, resource: node.resource }
        : node?.kind === "result"
          ? node.result.object
          : undefined;
    if (!o) return;
    setContext({ resource: o.resource, fragment: o.fragment, label: o.label, class: shortName(o.class) });
    return oclConsole.reveal();
  });
  register("oxychloride.refresh", () => refresh());

  // ---------------------------------------------------------------- console

  register("oxychloride.openConsole", () => oclConsole.reveal());
  register("oxychloride.evaluate", async () => {
    const expression = await vscode.window.showInputBox({
      title: "Evaluate OCL",
      prompt: oclConsole.currentContext ? `self = ${oclConsole.currentContext.label}` : "No context object",
      placeHolder: "e.g. Student.allInstances()->select(semester > 2)",
    });
    if (!expression) return;
    await oclConsole.reveal();
    return oclConsole.evaluate(expression);
  });
  register("oxychloride.evaluateSelection", async () => {
    const editor = vscode.window.activeTextEditor;
    const expression = editor?.document.getText(editor.selection).trim();
    if (!expression) return;
    await oclConsole.reveal();
    return oclConsole.evaluate(expression);
  });
  oclConsole.onDidChangeContext((c) => models.setContext(c));

  // ---------------------------------------------------------------- validation

  register("oxychloride.validate", async () => {
    try {
      const report = await engine.validate({
        ecore: true,
        contracts: true,
        includeSatisfied: vscode.workspace.getConfiguration("oxychloride").get<boolean>("validation.includeSatisfied"),
      });
      validation.show(report);
      await vscode.commands.executeCommand("oxychloride.validation.focus");
      const s = report.summary;
      const failing = s.violated + s.errors;
      const text = `${failing ? `${s.violated} violated, ${s.errors} errors` : "All constraints are satisfied"} (${s.constraints} constraints on ${s.objects} objects, ${s.evaluations} evaluations).`;
      void (failing ? vscode.window.showWarningMessage(text) : vscode.window.showInformationMessage(text));
      return report;
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
    }
  });

  // ---------------------------------------------------------------- loading and converting

  register("oxychloride.loadMetamodel", async (uri?: vscode.Uri) => {
    const files = uri
      ? [uri]
      : await vscode.window.showOpenDialog({ canSelectMany: true, filters: { "Ecore metamodels": ["ecore"] }, title: "Load metamodel" });
    for (const f of files ?? []) {
      try {
        const r = await engine.loadEcore(f.fsPath);
        void vscode.window.showInformationMessage(`Loaded ${path.basename(f.fsPath)}: ${r.packages?.length ?? 0} packages.`);
        for (const w of r.warnings) output.warn(`${w.source}: ${w.message}`);
      } catch (e) {
        void vscode.window.showErrorMessage(`Could not load ${path.basename(f.fsPath)}: ${errorMessage(e)}`);
      }
    }
  });
  register("oxychloride.loadModel", async (uri?: vscode.Uri) => {
    const files = uri
      ? [uri]
      : await vscode.window.showOpenDialog({ canSelectMany: true, filters: { "XMI models": ["xmi", "model"] }, title: "Load model" });
    for (const f of files ?? []) {
      try {
        const r = await engine.loadModel(f.fsPath);
        void vscode.window.showInformationMessage(`Loaded ${path.basename(f.fsPath)}: ${r.objects ?? 0} objects.`);
        for (const w of r.warnings) output.warn(`${w.source}: ${w.message}`);
      } catch (e) {
        void vscode.window.showErrorMessage(`Could not load ${path.basename(f.fsPath)}: ${errorMessage(e)}`);
      }
    }
  });

  const exportDocument = async (extension: "ecore" | "xmi") => {
    const doc = vscode.window.activeTextEditor?.document;
    if (!doc) return;
    const source = doc.uri.scheme === "file" ? doc.uri : vscode.workspace.workspaceFolders?.[0]?.uri;
    const stem = path.basename(doc.uri.path).replace(/\.[^.]*$/, "");
    const target = await vscode.window.showSaveDialog({
      defaultUri: source && vscode.Uri.joinPath(doc.uri.scheme === "file" ? vscode.Uri.joinPath(source, "..") : source, `${stem}.${extension}`),
      filters: { [extension === "ecore" ? "Ecore metamodels" : "XMI models"]: [extension] },
      title: extension === "ecore" ? "Export as .ecore" : "Export as XMI",
    });
    if (!target) return;
    try {
      const { text } = await engine.exportDocument(documentName(doc.uri), target.fsPath);
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
      const choice = await vscode.window.showInformationMessage(`Exported ${path.basename(target.fsPath)}.`, "Open");
      if (choice) await vscode.window.showTextDocument(target);
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
    }
  };
  register("oxychloride.exportEcore", () => exportDocument("ecore"));

  /** Writes the `.ecore` of an OCLinEcore document next to it (`university.oclinecore` →
   * `university.ecore`). On save it is quiet, and skipped while the document has errors. */
  const generateEcore = async (doc: vscode.TextDocument, onSave = false): Promise<vscode.Uri | undefined> => {
    if (doc.languageId !== "oclinecore") return undefined;
    if (doc.uri.scheme !== "file") {
      void vscode.window.showWarningMessage("Save the metamodel first: its .ecore is written next to it.");
      return undefined;
    }
    const name = documentName(doc.uri);
    const target = doc.uri.with({ path: doc.uri.path.replace(/(\.[^./]*)?$/, ".ecore") });
    const file = path.basename(target.path);
    try {
      const { diagnostics } = await engine.call<{ diagnostics: Diagnostic[] }>("diagnostics", { name });
      const errors = diagnostics.filter((d) => d.severity === "error").length;
      if (errors) {
        const problem = `${path.basename(doc.uri.path)} has ${errors} error${errors === 1 ? "" : "s"}`;
        if (onSave) {
          vscode.window.setStatusBarMessage(`$(warning) ${file} not generated: ${problem}`, 6000);
          return undefined;
        }
        const choice = await vscode.window.showWarningMessage(`${problem}. Generate ${file} anyway?`, { modal: true }, "Generate");
        if (choice !== "Generate") return undefined;
      }
      const { text } = await engine.exportDocument(name, target.fsPath);
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
      if (onSave) {
        vscode.window.setStatusBarMessage(`$(check) Generated ${file}`, 4000);
      } else {
        void vscode.window.showInformationMessage(`Generated ${file}.`, "Open").then((choice) => {
          if (choice) void vscode.window.showTextDocument(target);
        });
      }
      return target;
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not generate ${file}: ${errorMessage(e)}`);
      return undefined;
    }
  };
  register("oxychloride.generateEcore", async (uri?: vscode.Uri) => {
    const doc = uri ? await vscode.workspace.openTextDocument(uri) : vscode.window.activeTextEditor?.document;
    if (doc?.isDirty) await doc.save();
    return doc && generateEcore(doc);
  });
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (vscode.workspace.getConfiguration("oxychloride").get<boolean>("generateEcoreOnSave")) {
        void generateEcore(doc, true);
      }
    }),
  );
  register("oxychloride.exportXmi", () => exportDocument("xmi"));

  register("oxychloride.convertToText", async (uri?: vscode.Uri) => {
    uri ??= vscode.window.activeTextEditor?.document.uri;
    if (!uri || uri.scheme !== "file") return;
    const to = uri.path.toLowerCase().endsWith(".ecore") ? "oclinecore" : "hutn";
    const defaultUri = uri.with({ path: uri.path.replace(/\.[^./]*$/, `.${to}`) });
    const target = await vscode.window.showSaveDialog({
      defaultUri,
      filters: { [to === "hutn" ? "HUTN models" : "OCLinEcore metamodels"]: [to] },
      title: to === "hutn" ? "Convert XMI to HUTN" : "Convert .ecore to OCLinEcore",
    });
    if (!target) return;
    try {
      const { text } = await engine.convertFile(uri.fsPath, to);
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
      await vscode.window.showTextDocument(target);
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
    }
  });

  register("oxychloride.exportSchema", async (node?: MetamodelNode) => {
    const folders = await vscode.window.showOpenDialog({
      canSelectFolders: true,
      canSelectFiles: false,
      title: "Export XML Schemas into folder",
      defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
    });
    if (!folders?.[0]) return;
    try {
      const written = await schemas.exportTo(folders[0]);
      const pkg = node?.kind === "package" ? node.package : undefined;
      void vscode.window.showInformationMessage(
        `Wrote ${written.length} schema files${pkg ? ` (including ${pkg.name})` : ""} to ${vscode.workspace.asRelativePath(folders[0])}.`,
      );
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
    }
  });

  register("oxychloride.restartServer", async () => {
    await client?.stop().catch(() => undefined);
    client = undefined;
    await startClient().catch((e) => vscode.window.showErrorMessage(errorMessage(e)));
  });

  return { engine, console: oclConsole, creator, metamodels, models, validation, ready };
}

export async function deactivate(): Promise<void> {
  await client?.stop();
  client = undefined;
}

/** Lets the user pick a model object as the console's `self`. */
async function chooseContext(models: { uri: string; roots: ObjectInfo[] }[]): Promise<ConsoleContext | undefined> {
  type Item = vscode.QuickPickItem & { context: ConsoleContext };
  const items: Item[] = [];
  const add = (o: ObjectInfo, resource: string, depth: number) => {
    items.push({
      label: `${"  ".repeat(depth)}${o.label}`,
      description: shortName(o.class),
      detail: undefined,
      context: { resource, fragment: o.fragment, label: o.label, class: shortName(o.class) },
    });
    for (const c of o.children) add(c, resource, depth + 1);
  };
  for (const m of models) {
    items.push({ label: path.basename(m.uri), kind: vscode.QuickPickItemKind.Separator } as Item);
    for (const o of m.roots) add(o, m.uri, 0);
  }
  if (!items.some((i) => i.context)) {
    void vscode.window.showInformationMessage("No model is open: open a .hutn or .xmi model first.");
    return undefined;
  }
  const pick = await vscode.window.showQuickPick(items, { title: "Evaluate with which object as self?", matchOnDescription: true });
  return pick?.context;
}

// Kept for API users that create instances programmatically.
export type { ClassifierInfo, PackageInfo };

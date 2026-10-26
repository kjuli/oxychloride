// Oxychloride for VS Code: a client of the engine's language server (`oxy lsp`) plus views
// (metamodels, the package registry, models, validation results), an OCL console, commands to create model content,
// XML Schemas for XMI models, the metamodel diagram (a graphical editor for `.ecore` and
// `.oclinecore` files), the rule diagram (a graphical editor for Henshin texts and `.henshin`
// files, which are kept equivalent in pairs), the transformation view for Henshin rules, and
// code generation.

import * as path from "node:path";
import * as vscode from "vscode";
import { LanguageClient, TransportKind, type LanguageClientOptions, type ServerOptions } from "vscode-languageclient/node";
import { CodeGenerator, type CodeSource, type GenerateOptions } from "./codegen";
import { OclConsole, type ConsoleContext } from "./console";
import { Creator } from "./create";
import { MetamodelDiagramEditor, openDiagram, type DiagramView } from "./diagram";
import { documentName, reveal } from "./documents";
import { HenshinPairs, counterpart } from "./henshinPairs";
import { PackageRegistry, type RegistryNode } from "./registry";
import { RuleDiagramEditor, openRuleDiagram, type RuleDiagramView } from "./ruleDiagram";
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
import { TransformationPanel, chooseModel, chooseTransformation } from "./transformation";
import { MetamodelTree, ModelTree, ValidationTree, shortName, type MetamodelNode, type ModelNode, type ValidationNode } from "./trees";

const LANGUAGES = ["ocl", "oclinecore", "hutn", "henshin"];

/** What the extension offers other extensions (and its tests). */
export interface OxychlorideApi {
  engine: Engine;
  console: OclConsole;
  creator: Creator;
  codeGenerator: CodeGenerator;
  metamodels: MetamodelTree;
  /** The package registry (URIs standing for metamodel files). */
  registry: PackageRegistry;
  models: ModelTree;
  validation: ValidationTree;
  /** The open metamodel diagrams. */
  diagrams(): DiagramView[];
  /** The open rule diagrams. */
  ruleDiagrams(): RuleDiagramView[];
  /** Keeps Henshin texts and `.henshin` files equivalent. */
  pairs: HenshinPairs;
  /** Opens the transformation view of a Henshin document (engine name) on a model resource. */
  openTransformation(document: string, model: string, tab?: "steps" | "space"): Promise<TransformationPanel>;
  /** Resolves once the language server is running. */
  ready: Promise<void>;
}

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<OxychlorideApi> {
  const output = vscode.window.createOutputChannel("Oxychloride", { log: true });
  const engine = new Engine(() => client);
  const metamodels = new MetamodelTree(engine);
  const registry = new PackageRegistry(engine);
  const models = new ModelTree(engine);
  const validation = new ValidationTree();
  const creator = new Creator(engine, () => metamodels.packages);
  const codeGenerator = new CodeGenerator(engine, context.workspaceState, output);
  const schemas = new SchemaRegistry(engine, context, output);
  const oclConsole = new OclConsole(engine, () => chooseContext(models.models));

  const refresh = async () => {
    await Promise.all([
      metamodels.refresh(),
      registry.refresh(),
      models.refresh(),
      ...TransformationPanel.all().map((p) => p.refresh()),
    ]);
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
        { scheme: "file", language: "xml", pattern: "**/*.henshin" },
      ],
      // Registered metamodels may be Ecore XML under other extensions.
      synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/*.{ocl,oclinecore,ecore,xml,henshin_text,henshin}") },
      // The package registry is there before the server loads the workspace folders.
      initializationOptions: () => ({ registry: registry.entries() }),
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

  const pairs = new HenshinPairs(engine, ready.then(() => undefined));

  const setContext = (c: ConsoleContext | undefined) => {
    oclConsole.setContext(c);
    models.setContext(c);
  };

  const register = (id: string, run: (...args: never[]) => unknown) =>
    context.subscriptions.push(vscode.commands.registerCommand(id, run));

  context.subscriptions.push(
    output,
    vscode.window.registerTreeDataProvider("oxychloride.metamodels", metamodels),
    vscode.window.registerTreeDataProvider("oxychloride.registry", registry),
    vscode.languages.registerCodeActionsProvider(
      [...LANGUAGES.map((language) => ({ language })), { language: "xml", pattern: "**/*.{xmi,henshin}" }],
      registry,
      { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] },
    ),
    vscode.window.registerTreeDataProvider("oxychloride.models", models),
    vscode.window.registerTreeDataProvider("oxychloride.validation", validation),
    vscode.window.registerWebviewViewProvider(OclConsole.viewId, oclConsole, { webviewOptions: { retainContextWhenHidden: true } }),
    vscode.window.onDidChangeActiveTextEditor((e) => creator.track(e)),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("oxychloride.xmlSchemas")) void schemas.update();
      if (e.affectsConfiguration("oxychloride.registry")) void registry.sync();
    }),
    { dispose: () => void schemas.dispose() },
    MetamodelDiagramEditor.register(context, engine, ready.then(() => undefined)),
    pairs,
    RuleDiagramEditor.register(context, engine, pairs, ready.then(() => undefined)),
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
  // A metamodel file (Explorer, or the active editor): the user picks the class.
  register("oxychloride.instantiateMetamodel", async (uri?: vscode.Uri) => {
    uri = uri instanceof vscode.Uri ? uri : vscode.window.activeTextEditor?.document.uri;
    if (!uri) return;
    await ready;
    return creator.instantiate(uri);
  });
  // The class at the cursor of an OCLinEcore editor.
  register("oxychloride.instantiateClass", async () => {
    const editor = vscode.window.activeTextEditor;
    if (editor?.document.languageId !== "oclinecore") return;
    await ready;
    return creator.instantiate(editor.document.uri, editor.selection.active);
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
  // The package registry. From the Explorer (a file, and the selection), the editor, a quick
  // fix (the URI to register a file for) or code (both).
  register("oxychloride.registerMetamodel", async (file?: vscode.Uri, uri?: unknown) => {
    await ready;
    return registry.register(file instanceof vscode.Uri ? file : undefined, typeof uri === "string" ? uri : undefined);
  });
  register("oxychloride.mapUriPrefix", async (folder?: vscode.Uri, prefix?: unknown) => {
    await ready;
    return registry.mapFolder(folder instanceof vscode.Uri ? folder : undefined, typeof prefix === "string" ? prefix : undefined);
  });
  register("oxychloride.unregisterMetamodel", (node?: RegistryNode | string) => registry.unregister(node));
  register("oxychloride.registerWorkspaceMetamodels", async () => {
    await ready;
    return registry.registerWorkspace();
  });
  register("oxychloride.editRegistry", () => registry.openSettings());

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

  // A metamodel file (Explorer, editor), or a package of the metamodel view. From code, the
  // second argument may answer the questions (`GenerateOptions`); the Explorer passes the
  // selected files there instead.
  register("oxychloride.generateCode", async (arg?: vscode.Uri | MetamodelNode, options?: GenerateOptions | vscode.Uri[]) => {
    await ready;
    let source: CodeSource | undefined;
    if (arg instanceof vscode.Uri) {
      source = { uri: arg };
    } else if (arg?.kind === "package") {
      const resource = arg.package.resource;
      const file = /^([A-Za-z]:)?[\\/]/.test(resource) ? vscode.Uri.file(resource) : undefined;
      source = { resource, label: arg.package.name, folder: file && vscode.Uri.joinPath(file, "..") };
    } else {
      const doc = vscode.window.activeTextEditor?.document;
      if (doc && (doc.languageId === "oclinecore" || /\.ecore$/i.test(doc.uri.path))) source = { uri: doc.uri };
    }
    if (!source) {
      const files = await vscode.window.showOpenDialog({
        canSelectMany: false,
        filters: { Metamodels: ["ecore", "oclinecore"] },
        title: "Generate code for metamodel",
      });
      if (!files?.[0]) return undefined;
      source = { uri: files[0] };
    }
    return codeGenerator.generate(source, Array.isArray(options) ? {} : options);
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

  // ---------------------------------------------------------------- diagrams

  // Beside the text editor; from code, `false` as the second argument opens it in place.
  register("oxychloride.openDiagram", (uri?: vscode.Uri, beside?: unknown) =>
    openDiagram(uri instanceof vscode.Uri ? uri : undefined, beside !== false),
  );
  // From a diagram: its text beside it.
  register("oxychloride.openDiagramText", async (uri?: vscode.Uri) => {
    const views = [...MetamodelDiagramEditor.all(), ...RuleDiagramEditor.all()];
    const view = views.find((v) => v.panel.active) ?? views.find((v) => v.document.uri.toString() === uri?.toString());
    const doc = view?.document ?? (uri instanceof vscode.Uri ? await vscode.workspace.openTextDocument(uri) : undefined);
    if (doc) await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside });
  });
  // Henshin documents: the rule diagram beside the text (from code, `false` opens it in place).
  register("oxychloride.openRuleDiagram", (uri?: vscode.Uri, beside?: unknown) =>
    openRuleDiagram(uri instanceof vscode.Uri ? uri : RuleDiagramEditor.all().find((v) => v.panel.active)?.document.uri, beside !== false),
  );
  // Writes a Henshin text as a `.henshin` file next to it (or the other way round); from then
  // on the two are kept equivalent.
  register("oxychloride.createHenshinCounterpart", async (uri?: vscode.Uri) => {
    uri = uri instanceof vscode.Uri ? uri : (vscode.window.activeTextEditor?.document.uri ?? RuleDiagramEditor.all().find((v) => v.panel.active)?.document.uri);
    const target = uri && counterpart(uri);
    if (!uri || !target || uri.scheme !== "file") return undefined;
    await ready;
    const doc = await vscode.workspace.openTextDocument(uri);
    const file = path.basename(target.path);
    try {
      await vscode.workspace.fs.stat(target);
      const choice = await vscode.window.showWarningMessage(`${file} exists. Replace it with ${path.basename(uri.path)}?`, { modal: true }, "Replace");
      if (choice !== "Replace") return undefined;
    } catch {
      // It does not exist yet.
    }
    try {
      const { text } = await engine.exportDocument(documentName(doc.uri), target.fsPath);
      await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(text));
      await pairs.sync(doc);
      void vscode.window.showInformationMessage(`Wrote ${file}; it is kept equivalent to ${path.basename(uri.path)}.`, "Open").then((choice) => {
        if (choice) void vscode.commands.executeCommand("vscode.open", target);
      });
      return target;
    } catch (e) {
      void vscode.window.showErrorMessage(`Could not write ${file}: ${errorMessage(e)}`);
      return undefined;
    }
  });

  // ---------------------------------------------------------------- transformations

  const openTransformation = (document: string, model: string, tab: "steps" | "space" = "steps") =>
    TransformationPanel.open(engine, context.extensionUri, document, model, tab);
  /** Opens the transformation view for a Henshin document (default: the active one, else
   * one to pick), on a model to pick. */
  const transform = async (tab: "steps" | "space", uri?: vscode.Uri, model?: string) => {
    await ready;
    uri ??= await chooseTransformation();
    if (!uri) return;
    // Opening the document hands it to the language server (if it is not in a workspace folder).
    const doc = await vscode.workspace.openTextDocument(uri);
    model ??= await chooseModel(await engine.models().catch(() => []));
    if (!model) return;
    try {
      return await openTransformation(documentName(doc.uri), model, tab);
    } catch (e) {
      void vscode.window.showErrorMessage(errorMessage(e));
      return undefined;
    }
  };
  register("oxychloride.openTransformation", (uri?: vscode.Uri) => transform("steps", uri instanceof vscode.Uri ? uri : undefined));
  register("oxychloride.exploreStateSpace", (uri?: vscode.Uri) => transform("space", uri instanceof vscode.Uri ? uri : undefined));
  register("oxychloride.transformModel", (node?: ModelNode) =>
    transform("steps", undefined, node?.kind === "resource" ? node.resource.uri : undefined),
  );

  register("oxychloride.restartServer", async () => {
    await client?.stop().catch(() => undefined);
    client = undefined;
    await startClient().catch((e) => vscode.window.showErrorMessage(errorMessage(e)));
  });

  return {
    engine,
    console: oclConsole,
    creator,
    codeGenerator,
    metamodels,
    registry,
    models,
    validation,
    diagrams: () => MetamodelDiagramEditor.all(),
    ruleDiagrams: () => RuleDiagramEditor.all(),
    pairs,
    openTransformation,
    ready,
  };
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

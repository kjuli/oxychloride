// QVT transformations — Operational Mappings (`.qvto`) and Relations (`.qvtr`) — run on the
// models of the workspace. From a model (the Explorer, its editor, the Models view) the
// transformations whose parameters it fits are offered; from a transformation, the models of
// its parameters are asked for; for QVT Relations, the direction too (and whether to change
// the target model or only check it). The engine runs the transformation on copies of the
// models (`runQvt`); the models it produces or changes are written here, its log goes to the
// output channel, and the problems it met are shown on the transformation.

import * as path from "node:path";
import * as vscode from "vscode";
import { documentName, toVsRange, uriOf } from "./documents";
import { errorMessage, type Engine, type QvtParameterInfo, type QvtRunResult, type QvtTransformationInfo } from "./engine";

/** Answers to the command's questions, for calls from code (other extensions, tests). */
export interface QvtRunOptions {
  /** The transformation document. */
  transformation?: vscode.Uri;
  /** The parameter the model is bound to. */
  parameter?: string;
  /** The models of further parameters: files, or models of the engine (by name). */
  models?: Record<string, vscode.Uri | string>;
  /** QVT Relations: the parameter to run in the direction of. */
  target?: string;
  /** QVT Relations: check the target model instead of changing it. */
  check?: boolean;
  /** The files the output models are written to (a `.hutn` file gets HUTN). */
  outputs?: Record<string, vscode.Uri>;
  /** QVT Operational: values of configuration properties, in OCL syntax (`'text'`, `3`). */
  configuration?: Record<string, string>;
  /** Open the written models (default: true). */
  open?: boolean;
}

/** Where a run starts: a model (a file, or a model of the engine by name), or a transformation. */
export interface QvtSource {
  model?: vscode.Uri | string;
  transformation?: vscode.Uri;
}

/** Model files the language server checks as documents. */
export const MODEL_FILES = /\.(xmi|hutn)$/i;
/** QVT documents. */
export const QVT_FILES = /\.qvt[or]$/i;

const LANGUAGE_NAMES = { qvto: "QVT Operational", qvtr: "QVT Relations" };

export class QvtRunner implements vscode.Disposable {
  /** The problems of the last run of each transformation. */
  private readonly problems = vscode.languages.createDiagnosticCollection("oxychloride-qvt");

  constructor(
    private readonly engine: Engine,
    private readonly output: vscode.LogOutputChannel,
  ) {}

  dispose(): void {
    this.problems.dispose();
  }

  /** Runs a QVT transformation; asks for what `options` does not answer. */
  async run(source: QvtSource, options: QvtRunOptions = {}): Promise<QvtRunResult | undefined> {
    try {
      return await this.runInner(source, options);
    } catch (e) {
      void vscode.window.showErrorMessage(`QVT: ${errorMessage(e)}`);
      return undefined;
    }
  }

  /** The engine's name of a model: model documents (HUTN, XMI) are opened, which hands them to
   * the language server; other files are loaded as XMI models. */
  private async modelName(model: vscode.Uri | string): Promise<string> {
    if (typeof model === "string") return model;
    if (MODEL_FILES.test(model.path)) {
      await vscode.workspace.openTextDocument(model);
      return documentName(model);
    }
    return (await this.engine.loadModel(model.fsPath)).uri;
  }

  private async runInner(source: QvtSource, options: QvtRunOptions): Promise<QvtRunResult | undefined> {
    const model = source.model !== undefined ? await this.modelName(source.model) : undefined;
    const transformationUri = options.transformation ?? source.transformation;
    // Opening a transformation hands it to the language server (if it is not in a workspace folder).
    if (transformationUri) await vscode.workspace.openTextDocument(transformationUri);
    const all = await this.engine.qvtTransformations(model);
    let t: QvtTransformationInfo | undefined;
    if (transformationUri) {
      const name = documentName(transformationUri);
      t = all.find((x) => x.document === name);
      if (!t) throw new Error(`${path.basename(transformationUri.path)} is not a QVT transformation`);
      if (model && !t.applicable?.length) {
        throw new Error(`${t.name} has no parameter for models like ${path.basename(model)}`);
      }
    } else {
      const candidates = model ? all.filter((x) => x.applicable?.length) : all;
      if (!candidates.length) {
        const what = model ? ` takes models like ${path.basename(model)}` : " is in the workspace";
        void vscode.window.showInformationMessage(`No QVT transformation (.qvto, .qvtr)${what}.`);
        return undefined;
      }
      t = await this.pickTransformation(candidates, model);
      if (!t) return undefined;
    }
    if (t.errors) {
      const choice = await vscode.window.showErrorMessage(
        `${t.name} has ${t.errors} error${t.errors === 1 ? "" : "s"}: fix ${t.errors === 1 ? "it" : "them"} first.`,
        "Show Transformation",
      );
      if (choice) await vscode.window.showTextDocument(uriOf(t.document));
      return undefined;
    }

    const models: Record<string, string> = {};
    for (const [p, m] of Object.entries(options.models ?? {})) models[p] = await this.modelName(m);
    if (model) {
      const applicable = t.applicable ?? [];
      const parameter = options.parameter ?? (applicable.length === 1 ? applicable[0] : await this.pickParameter(t, applicable));
      if (!parameter) return undefined;
      models[parameter] = model;
    }
    let target = options.target;
    let check = options.check ?? false;
    if (t.language === "qvtr" && !target) {
      const choice = await this.pickDirection(t, models, options.check);
      if (!choice) return undefined;
      ({ target, check } = choice);
    }
    for (const p of t.parameters) {
      if (models[p.name] || !isInput(t, p, target, check)) continue;
      const m = await this.pickModel(t, p);
      if (!m) return undefined;
      models[p.name] = m;
    }
    const outputs: Record<string, vscode.Uri> = {};
    if (!check) {
      for (const p of t.parameters.filter((p) => isOutput(t, p, target))) {
        const uri = options.outputs?.[p.name] ?? (await this.pickOutput(t, p, models));
        if (!uri) return undefined;
        outputs[p.name] = uri;
      }
    }

    const result = await this.engine.runQvt({
      document: t.document,
      models: Object.entries(models).map(([parameter, m]) => ({ parameter, model: m })),
      outputs: Object.entries(outputs).map(([parameter, uri]) => ({ parameter, path: uri.fsPath })),
      target,
      check,
      configuration: options.configuration,
    });
    const written: vscode.Uri[] = [];
    for (const o of result.outputs) {
      const uri = outputs[o.parameter];
      if (!uri) continue;
      await vscode.workspace.fs.writeFile(uri, new TextEncoder().encode(o.text));
      written.push(uri);
    }
    await this.report(t, result, check, target, written);
    if (options.open !== false) {
      for (const uri of written) await vscode.window.showTextDocument(uri, { viewColumn: vscode.ViewColumn.Beside, preview: false });
    }
    return result;
  }

  private async pickTransformation(candidates: QvtTransformationInfo[], model?: string): Promise<QvtTransformationInfo | undefined> {
    const items = candidates.map((t) => ({
      label: t.name || path.basename(t.document),
      description: `${LANGUAGE_NAMES[t.language]}${t.errors ? ` — ${t.errors} error${t.errors === 1 ? "" : "s"}` : ""}`,
      detail: `${vscode.workspace.asRelativePath(t.document)}: ${signature(t)}`,
      t,
    }));
    const pick = await vscode.window.showQuickPick(items, {
      title: model ? `Transform ${path.basename(model)} with` : "Run QVT transformation",
      matchOnDetail: true,
    });
    return pick?.t;
  }

  private async pickParameter(t: QvtTransformationInfo, names: string[]): Promise<string | undefined> {
    const items = t.parameters.filter((p) => names.includes(p.name)).map((p) => ({ label: p.name, description: p.modelType }));
    return (await vscode.window.showQuickPick(items, { title: `${t.name}: the model is` }))?.label;
  }

  /** QVT Relations: the parameter to run in the direction of, and whether to enforce or check. */
  private async pickDirection(
    t: QvtTransformationInfo,
    bound: Record<string, string>,
    check?: boolean,
  ): Promise<{ target: string; check: boolean } | undefined> {
    const free = t.targets.filter((p) => !bound[p]);
    const targets = free.length ? free : t.targets;
    if (!targets.length) throw new Error(`no relation of ${t.name} enforces a model`);
    if (targets.length === 1 && check !== undefined) return { target: targets[0], check };
    const items = targets.flatMap((target) => [
      ...(check === true ? [] : [{ label: `$(arrow-right) Create ${target}`, description: "enforce the relations: create or update the model", target, check: false }]),
      ...(check === false ? [] : [{ label: `$(check) Check ${target}`, description: "compare with an existing model, without changing it", target, check: true }]),
    ]);
    const pick = await vscode.window.showQuickPick(items, { title: `${t.name}: in the direction of` });
    return pick && { target: pick.target, check: pick.check };
  }

  private async pickModel(t: QvtTransformationInfo, p: QvtParameterInfo): Promise<string | undefined> {
    const models = await this.engine.models();
    const browse = "$(folder-opened) Browse…";
    const items = [
      ...models.map((m) => ({ label: path.basename(m.uri), description: vscode.workspace.asRelativePath(path.dirname(m.uri)), uri: m.uri })),
      { label: browse, description: "", uri: "" },
    ];
    const pick = await vscode.window.showQuickPick(items, { title: `${t.name}: the model of '${p.name}' (${p.modelType})` });
    if (!pick) return undefined;
    if (pick.label !== browse) return pick.uri;
    const files = await vscode.window.showOpenDialog({ canSelectMany: false, title: `The model of '${p.name}'` });
    return files?.[0] && this.modelName(files[0]);
  }

  /** Where an output model goes: the model of an `inout` parameter, else next to the first
   * model (`company.target.xmi`), or the transformation. */
  private async pickOutput(t: QvtTransformationInfo, p: QvtParameterInfo, models: Record<string, string>): Promise<vscode.Uri | undefined> {
    const own = models[p.name];
    let defaultUri: vscode.Uri;
    if (own) {
      defaultUri = uriOf(own);
    } else {
      const near = Object.values(models)[0] ?? t.document;
      const stem = path.basename(near).replace(/\.[^.]*$/, "");
      defaultUri = vscode.Uri.joinPath(uriOf(near), "..", `${stem}.${p.name}.xmi`);
    }
    return vscode.window.showSaveDialog({
      defaultUri,
      filters: { "XMI models": ["xmi"], "HUTN models": ["hutn"] },
      title: `${t.name}: write the model of '${p.name}' (${p.modelType}) to`,
    });
  }

  private async report(t: QvtTransformationInfo, result: QvtRunResult, check: boolean, target: string | undefined, written: vscode.Uri[]): Promise<void> {
    const doc = await vscode.workspace.openTextDocument(uriOf(t.document));
    this.problems.set(
      doc.uri,
      result.problems.map((d) => {
        const diagnostic = new vscode.Diagnostic(toVsRange(doc, d.range), d.message, vscode.DiagnosticSeverity.Warning);
        diagnostic.source = "QVT run";
        return diagnostic;
      }),
    );
    const what = t.language === "qvtr" ? "relations" : "mappings";
    this.output.info(
      `${t.name}: ${result.applications} ${what} ${check ? "checked" : "applied"}, ${result.created} objects created${check ? ` (checking ${target})` : ""}`,
    );
    for (const line of result.log) this.output.info(`  log: ${line}`);
    for (const p of result.problems) this.output.warn(`  ${path.basename(t.document)}:${p.range.start.line + 1}: ${p.message}`);
    for (const v of result.violations) this.output.warn(`  ${v}`);
    for (const v of result.invariants) this.output.warn(`  invariant: ${v}`);
    for (const uri of written) this.output.info(`  wrote ${uri.fsPath}`);

    const show = (choice: string | undefined) => {
      if (choice === "Show Output") this.output.show(true);
      if (choice === "Show Problems") void vscode.commands.executeCommand("workbench.actions.view.problems");
    };
    if (result.aborted) {
      void vscode.window.showErrorMessage(`${t.name} stopped: ${result.aborted}`, "Show Output").then(show);
    } else if (check) {
      const n = result.violations.length;
      if (n) {
        void vscode.window.showWarningMessage(`${n} relation binding${n === 1 ? " does" : "s do"} not hold for ${target}.`, "Show Output").then(show);
      } else {
        void vscode.window.showInformationMessage(`${target} is consistent with ${t.name}.`);
      }
    } else if (result.problems.length) {
      const n = result.problems.length;
      void vscode.window.showWarningMessage(`${t.name} ran with ${n} problem${n === 1 ? "" : "s"}.`, "Show Problems", "Show Output").then(show);
    } else if (result.invariants.length) {
      const n = result.invariants.length;
      void vscode.window.showWarningMessage(`The result violates ${n} invariant${n === 1 ? "" : "s"}.`, "Show Output").then(show);
    } else {
      const files = written.map((u) => path.basename(u.path)).join(", ");
      void vscode.window.showInformationMessage(`${t.name}: wrote ${files || "nothing"} (${result.created} objects created).`);
    }
  }
}

/** Whether a parameter needs a model before the run. */
function isInput(t: QvtTransformationInfo, p: QvtParameterInfo, target: string | undefined, check: boolean): boolean {
  if (t.language === "qvto") return p.direction !== "out";
  return p.name !== target || check;
}

/** Whether a run writes the model of a parameter. */
function isOutput(t: QvtTransformationInfo, p: QvtParameterInfo, target: string | undefined): boolean {
  if (t.language === "qvto") return p.direction === "out" || p.direction === "inout";
  return p.name === target;
}

/** `(in source : UML, out target : RDBMS)`. */
function signature(t: QvtTransformationInfo): string {
  const params = t.parameters.map((p) => `${p.direction ? `${p.direction} ` : ""}${p.name} : ${p.modelType}`);
  return `${t.name}(${params.join(", ")})`;
}

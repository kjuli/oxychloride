// Drives the extension in a real VS Code: live diagnostics for HUTN and XMI models, the views,
// creating objects, the OCL console, validation, metamodel diagrams, Henshin transformations and
// their rule diagrams, keeping a Henshin text and its `.henshin` file equivalent, the package
// registry, and QVT transformations.

import * as assert from "node:assert";
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";

/** The parts of the extension's API the tests use. */
interface Api {
  ready: Promise<void>;
  engine: { call<T>(method: string, params?: object | null): Promise<T> };
  console: {
    evaluate(expression: string): Promise<{ display: string; type: string; contracts?: unknown[] } | undefined>;
    setContext(c: { resource: string; fragment: string; label: string; class: string } | undefined): void;
  };
  creator: {
    createInstance(cls: unknown, inNew?: boolean): Promise<boolean>;
    addChild(node: unknown, choice?: { feature: string; cls: string }): Promise<boolean>;
    instantiate(uri: vscode.Uri, position?: vscode.Position, target?: vscode.Uri): Promise<vscode.Uri | undefined>;
  };
  metamodels: { packages: { name: string; classifiers: { name: string; qualifiedName: string }[] }[] };
  registry: RegistryApi;
  models: { models: { uri: string; roots: { fragment: string; label: string; children: unknown[] }[] }[] };
  validation: { report?: { summary: { violated: number } } };
  diagrams(): DiagramView[];
  ruleDiagrams(): RuleDiagramView[];
  pairs: { settled(): Promise<void> };
}

/** The parts of the package registry the tests use. */
interface RegistryApi {
  infos: { uri: string; status: string; imports: { uri: string }[]; warnings: unknown[] }[];
  register(file: vscode.Uri, uri: string): Promise<{ status: string } | undefined>;
  unregister(uri: string): Promise<void>;
  getChildren(): unknown[];
}

/** The parts of a rule diagram the tests use. */
interface RuleDiagramView {
  document: vscode.TextDocument;
  panel: vscode.WebviewPanel;
  current?: { syntax: string; rules: { id: string; nodes: { id: string; action: string }[] }[]; units: { id: string }[] };
  edit(edit: object): Promise<{ element?: string }>;
}

/** The parts of a metamodel diagram the tests use. */
interface DiagramView {
  document: vscode.TextDocument;
  panel: vscode.WebviewPanel;
  current?: { syntax: string; classifiers: { id: string; name: string; references: { name: string; upper: number }[] }[] };
  edit(edit: object): Promise<{ element?: string }>;
}

/** The parts of a transformation view the tests use. */
interface TransformationView {
  panel: vscode.WebviewPanel;
  current?: { session: number; step: number; graph: { nodes: { id: number; label: string }[] }; violations: unknown[] };
}

const folder = () => vscode.workspace.workspaceFolders![0].uri.fsPath;

async function waitFor<T>(what: string, check: () => T | undefined | false, timeout = 30000): Promise<T> {
  const start = Date.now();
  for (;;) {
    const value = check();
    if (value) return value;
    if (Date.now() - start > timeout) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const errors = (uri: vscode.Uri) =>
  vscode.languages.getDiagnostics(uri).filter((d) => d.severity === vscode.DiagnosticSeverity.Error);

describe("Oxychloride", () => {
  let api: Api;

  before(async () => {
    const ext = vscode.extensions.getExtension<Api>("palladio.oxychloride")!;
    api = await ext.activate();
    await api.ready;
  });

  it("checks a HUTN model against the constraints of a metamodel that is not open", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.hutn"));
    await vscode.window.showTextDocument(uri);
    const diagnostics = await waitFor("5 violations", () => errors(uri).length === 5 && errors(uri));
    assert.ok(diagnostics.some((d) => d.message.includes("CreditLimit")), diagnostics.map((d) => d.message).join("\n"));
    assert.ok(diagnostics.some((d) => d.message.includes("postcondition 'FreeSeat'")));
  });

  it("re-checks while typing", async () => {
    const editor = vscode.window.activeTextEditor!;
    const doc = editor.document;
    const thesis = doc.getText().indexOf('          Enrolment { course: Course "thesis" }');
    const line = doc.positionAt(thesis).line;
    // Carla drops the thesis: she is within the credit limit then.
    await editor.edit((e) => {
      e.delete(doc.lineAt(line).rangeIncludingLineBreak);
      const previous = doc.lineAt(line - 1);
      e.delete(new vscode.Range(previous.range.end.translate(0, -1), previous.range.end));
    });
    await waitFor("CreditLimit fixed", () => !errors(doc.uri).some((d) => d.message.includes("CreditLimit")));
    await vscode.commands.executeCommand("undo");
    await waitFor("CreditLimit back", () => errors(doc.uri).some((d) => d.message.includes("CreditLimit")));
  });

  it("shows metamodels and models", async () => {
    await waitFor("packages", () => api.metamodels.packages.some((p) => p.name === "university"));
    await waitFor("the model", () => api.models.models.some((m) => m.uri.endsWith("university.hutn")));
  });

  it("creates instances and children", async () => {
    const pkg = api.metamodels.packages.find((p) => p.name === "university")!;
    const course = pkg.classifiers.find((c) => c.name === "Course")!;
    const doc = vscode.window.activeTextEditor!.document;
    assert.ok(await api.creator.createInstance(course));
    assert.ok(doc.getText().includes("\n  Course {  }\n}"), doc.getText());
    const cursor = vscode.window.activeTextEditor!.selection.active;
    assert.strictEqual(doc.lineAt(cursor.line).text.slice(cursor.character - 2, cursor.character + 2), "{  }");

    const model = await waitFor("the model", () => api.models.models.find((m) => m.uri.endsWith("university.hutn")));
    const tum = model.roots[0];
    const node = { kind: "object", object: tum, resource: model.uri };
    assert.ok(await api.creator.addChild(node, { feature: "students", cls: "university::Student" }));
    assert.ok(doc.getText().includes("      },\n      Student {  }\n  }"), doc.getText());
    await vscode.commands.executeCommand("workbench.action.files.revert");

    // In a new, unsaved model.
    assert.ok(await api.creator.createInstance(course, true));
    const untitled = vscode.window.activeTextEditor!.document;
    assert.strictEqual(untitled.uri.scheme, "untitled");
    assert.ok(untitled.getText().includes("university {\n  Course {  }\n}"), untitled.getText());
    // Its objects are checked like those of saved models.
    await waitFor("required features", () =>
      vscode.languages.getDiagnostics(untitled.uri).some((d) => d.message.includes("required feature 'code'")),
    );
    await vscode.commands.executeCommand("workbench.action.revertAndCloseActiveEditor");
  });

  it("evaluates OCL in the console", async () => {
    const all = await api.console.evaluate("Student.allInstances()->collect(name)->sortedBy(n | n)");
    assert.strictEqual(all?.display, "Sequence{'Ada', 'Brian', 'Carla'}");
    const model = api.models.models.find((m) => m.uri.endsWith("university.hutn"))!;
    api.console.setContext({ resource: model.uri, fragment: "ada", label: "Ada", class: "Student" });
    const self = await api.console.evaluate("self.mayEnrol(Course.allInstances()->any(code = 'IN0001'))");
    assert.strictEqual(self?.type, "Boolean");
    assert.ok(self?.contracts?.length, "the violated precondition of the call");
    api.console.setContext(undefined);
  });

  it("validates", async () => {
    await vscode.commands.executeCommand("oxychloride.validate");
    assert.strictEqual(api.validation.report?.summary.violated, 5);
  });

  it("checks, highlights and completes Henshin transformations", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.henshin_text"));
    const editor = await vscode.window.showTextDocument(uri);
    const doc = editor.document;
    assert.strictEqual(doc.languageId, "henshin");
    // Errors appear as the rules are edited.
    const at = doc.getText().indexOf("create node enrolment:Enrolment");
    const typo = doc.positionAt(at + "create node enrolment:".length);
    await editor.edit((e) => e.insert(typo, "X"));
    await waitFor("unknown class", () => errors(uri).some((d) => d.message.includes("unknown class 'XEnrolment'")));
    await vscode.commands.executeCommand("undo");
    await waitFor("no errors", () => errors(uri).length === 0 || undefined);
    // Completion proposes the classes of the imported package, hovers describe names.
    const completions = await vscode.commands.executeCommand<vscode.CompletionList>("vscode.executeCompletionItemProvider", uri, typo);
    const labels = completions.items.map((i) => (typeof i.label === "string" ? i.label : i.label.label));
    assert.ok(labels.includes("Enrolment") && labels.includes("Course"), labels.join(", "));
    const hovers = await vscode.commands.executeCommand<vscode.Hover[]>("vscode.executeHoverProvider", uri, typo.translate(0, 2));
    const text = hovers.flatMap((h) => h.contents.map((c) => (typeof c === "string" ? c : c.value))).join("\n");
    assert.ok(text.includes("class university::Enrolment"), text);
  });

  it("transforms a model step by step and explores its state space", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.henshin_text"));
    const view = await vscode.commands.executeCommand<TransformationView | undefined>("oxychloride.openTransformation", uri);
    assert.ok(view, "the transformation view");
    const state = await waitFor("the initial state", () => view.current);
    assert.strictEqual(state.step, 0);
    assert.strictEqual(state.graph.nodes.length, 16);
    assert.strictEqual(state.violations.length, 3);
    const document = uri.fsPath;
    const result = await api.engine.call<{ applied: boolean; state: { step: number; changes: { createdNodes: number[] } } }>(
      "applyTransformation",
      { session: state.session, document, unit: "enrol" },
    );
    assert.ok(result.applied);
    assert.strictEqual(result.state.changes.createdNodes.length, 1);
    const space = await api.engine.call<{ states: unknown[]; complete: boolean }>("exploreStateSpace", {
      session: state.session,
      document,
      rules: ["enrol"],
      step: 0,
    });
    assert.strictEqual(space.states.length, 16);
    assert.ok(space.complete);
    view.panel.dispose();
  });

  it("instantiates metamodels as XMI models", async () => {
    // The class at the cursor of an OCLinEcore document.
    const metamodel = await vscode.workspace.openTextDocument(path.join(folder(), "university/university.oclinecore"));
    const inStudent = metamodel.positionAt(metamodel.getText().indexOf("attribute semester"));
    const student = vscode.Uri.file(path.join(folder(), "university/Student.xmi"));
    assert.strictEqual((await api.creator.instantiate(metamodel.uri, inStudent, student))?.fsPath, student.fsPath);
    const text = fs.readFileSync(student.fsPath, "utf8");
    assert.ok(text.includes('<uni:Student xmi:version="2.0"'), text);
    assert.ok(text.includes('xmlns:uni="http://www.example.org/university/1.0">'), text);
    assert.strictEqual(vscode.window.activeTextEditor?.document.uri.fsPath, student.fsPath);
    const models = () => api.models.models.find((m) => m.uri.endsWith("Student.xmi"));
    await waitFor("the new model", () => models()?.roots[0]?.label.startsWith("Student"));
    await vscode.commands.executeCommand("workbench.action.closeActiveEditor");

    // An .ecore file: the user picks a class, the model roots (no containment holds) first.
    const ecore = vscode.Uri.file(path.join(folder(), "library/library.ecore"));
    const target = vscode.Uri.file(path.join(folder(), "library/Library.xmi"));
    const result: { done: boolean; uri?: vscode.Uri } = { done: false };
    void api.creator.instantiate(ecore, undefined, target).then((uri) => Object.assign(result, { done: true, uri }));
    // Accepts the first class once the picker shows.
    for (const start = Date.now(); !result.done; ) {
      if (Date.now() - start > 30000) throw new Error("timed out waiting for the class picker");
      await vscode.commands.executeCommand("workbench.action.acceptSelectedQuickOpenItem");
      await new Promise((r) => setTimeout(r, 200));
    }
    assert.strictEqual(result.uri?.fsPath, target.fsPath);
    const library = fs.readFileSync(target.fsPath, "utf8");
    assert.ok(library.includes("<library:Library "), library);
    assert.ok(library.includes('xsi:schemaLocation="http://www.example.org/library/1.0 library.ecore"'), library);
    await vscode.commands.executeCommand("workbench.action.closeActiveEditor");
  });

  it("checks XMI models live", async () => {
    const file = path.join(folder(), "university/tum.xmi");
    fs.writeFileSync(
      file,
      `<?xml version="1.0" encoding="UTF-8"?>
<uni:University xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:uni="http://www.example.org/university/1.0" name="TUM">
  <courses code="IN1" title="Programming" kind="LECTURE" credits="8" capacity="0"/>
  <students name="Dora" semester="1">
    <enrolments course="//@courses.0"/>
  </students>
</uni:University>
`,
    );
    const uri = vscode.Uri.file(file);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    assert.strictEqual(doc.languageId, "xml");
    // Dora is enrolled in a course without capacity.
    const fromOcl = () => errors(uri).filter((d) => d.source === "ocl");
    const diagnostics = await waitFor("a violation", () => fromOcl().length > 0 && fromOcl());
    assert.ok(diagnostics.some((d) => d.message.includes("NotOverbooked")), diagnostics.map((d) => d.message).join("\n"));
    const typo = doc.getText().indexOf('semester="1"');
    const edit = new vscode.WorkspaceEdit();
    edit.replace(uri, new vscode.Range(doc.positionAt(typo), doc.positionAt(typo + 8)), "semestre");
    await vscode.workspace.applyEdit(edit);
    await waitFor("unknown feature", () => errors(uri).some((d) => d.message.includes("no feature 'semestre'")));
  });

  it("validates XMI models with the generated XML Schemas", async function () {
    const xml = vscode.extensions.getExtension("redhat.vscode-xml");
    if (!xml) this.skip();
    await xml!.activate();
    // The OCL engine and the XML extension both report the unknown attribute of the last test.
    const uri = vscode.Uri.file(path.join(folder(), "university/tum.xmi"));
    const fromXml = (d: vscode.Diagnostic) => d.source !== "ocl";
    const diagnostics = await waitFor(
      "a schema violation",
      () => vscode.languages.getDiagnostics(uri).some(fromXml) && vscode.languages.getDiagnostics(uri).filter(fromXml),
      90000,
    );
    const messages = diagnostics.map((d) => `${d.source}: ${d.message}`).join("\n");
    assert.ok(messages.includes("semestre"), messages);
    // Fixed, the document is valid XML for its schema: no complaints about the namespace either.
    const doc = await vscode.workspace.openTextDocument(uri);
    const edit = new vscode.WorkspaceEdit();
    const typo = doc.getText().indexOf("semestre");
    edit.replace(uri, new vscode.Range(doc.positionAt(typo), doc.positionAt(typo + 8)), "semester");
    await vscode.workspace.applyEdit(edit);
    await waitFor("no schema problems", () => !vscode.languages.getDiagnostics(uri).some(fromXml), 30000);
  });

  it("generates the .ecore of an OCLinEcore metamodel", async () => {
    const source = vscode.Uri.file(path.join(folder(), "university/university.oclinecore"));
    const ecore = path.join(folder(), "university/university.ecore");
    const target = await vscode.commands.executeCommand<vscode.Uri>("oxychloride.generateEcore", source);
    assert.strictEqual(target?.fsPath, vscode.Uri.file(ecore).fsPath);
    const xml = fs.readFileSync(ecore, "utf8");
    assert.ok(xml.includes('nsURI="http://www.example.org/university/1.0"') && xml.includes('name="Course"'), xml);
  });

  it("generates the .ecore on save, unless the metamodel has errors", async () => {
    const config = vscode.workspace.getConfiguration("oxychloride");
    await config.update("generateEcoreOnSave", true, vscode.ConfigurationTarget.Workspace);
    try {
      const ecore = path.join(folder(), "university/university.ecore");
      fs.rmSync(ecore, { force: true });
      const doc = await vscode.workspace.openTextDocument(path.join(folder(), "university/university.oclinecore"));
      const editor = await vscode.window.showTextDocument(doc);
      const end = doc.lineAt(doc.lineCount - 1).range.end;
      await editor.edit((e) => e.insert(end, "\n-- generated on save\n"));
      await doc.save();
      await waitFor("the generated .ecore", () => fs.existsSync(ecore));
      assert.ok(fs.readFileSync(ecore, "utf8").includes('name="Student"'));

      // With an error, the .ecore is left alone.
      fs.rmSync(ecore);
      const course = doc.getText().indexOf("class Course {");
      await editor.edit((e) => e.replace(new vscode.Range(doc.positionAt(course), doc.positionAt(course + 14)), "class Course extends Nowhere {"));
      await doc.save();
      await new Promise((r) => setTimeout(r, 1500));
      assert.ok(!fs.existsSync(ecore), "no .ecore from a metamodel with errors");
      await vscode.commands.executeCommand("undo");
      await vscode.commands.executeCommand("undo");
      await doc.save();
      await waitFor("the generated .ecore again", () => fs.existsSync(ecore));
    } finally {
      await config.update("generateEcoreOnSave", undefined, vscode.ConfigurationTarget.Workspace);
    }
  });

  it("generates Python and Rust code from metamodels", async () => {
    type Generated = { target: string; packages: string[]; files: { path: string }[]; problems: unknown[] };
    const out = vscode.Uri.file(path.join(folder(), "generated"));
    // An OCLinEcore metamodel, as Python.
    const university = vscode.Uri.file(path.join(folder(), "university/university.oclinecore"));
    const python = await vscode.commands.executeCommand<Generated>("oxychloride.generateCode", university, { target: "python", outputFolder: out });
    assert.deepStrictEqual(python?.packages, ["university"]);
    assert.deepStrictEqual(python.problems, []);
    const model = fs.readFileSync(path.join(out.fsPath, "university/_model.py"), "utf8");
    assert.ok(model.includes("class Course(_rt.EObject):") && model.includes("def _post_has_seat("), model.slice(0, 2000));
    // An .ecore file, with the Complete OCL document next to it, as a Rust crate.
    const library = vscode.Uri.file(path.join(folder(), "library/library.ecore"));
    const rust = await vscode.commands.executeCommand<Generated>("oxychloride.generateCode", library, { target: "rust", outputFolder: out, name: "lib_model" });
    assert.ok(rust?.files.some((f) => f.path === "lib_model/src/library.rs"), JSON.stringify(rust?.files.map((f) => f.path)));
    const code = fs.readFileSync(path.join(out.fsPath, "lib_model/src/library.rs"), "utf8");
    // The invariant of library.ocl (a document of the workspace) is part of it.
    assert.ok(code.includes("pub fn check_has_isbn(self, m: &crate::Model)"), code.slice(0, 2000));
  });

  it("draws an OCLinEcore metamodel as a diagram that edits the same document", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.oclinecore"));
    const editor = await vscode.window.showTextDocument(uri);
    await vscode.commands.executeCommand("oxychloride.openDiagram", uri);
    const view = await waitFor("the diagram", () => api.diagrams().find((v) => v.document.uri.fsPath === uri.fsPath && v.current));
    // The text editor and the diagram share the document.
    assert.strictEqual(view.document, editor.document);
    assert.deepStrictEqual(
      view.current!.classifiers.map((c) => c.name),
      ["University", "Kind", "Course", "Student", "Enrolment"],
    );
    // An edit in the diagram is an edit of the text…
    const result = await view.edit({ kind: "addClassifier", package: "university", classifier: "class", name: "Room" });
    assert.strictEqual(result.element, "university::Room");
    assert.ok(editor.document.getText().includes("\n  class Room;\n}"), editor.document.getText());
    assert.ok(editor.document.isDirty);
    await waitFor("Room in the diagram", () => view.current!.classifiers.some((c) => c.name === "Room"));
    // …and an edit of the text changes the diagram.
    const text = editor.document.getText();
    const at = editor.document.positionAt(text.indexOf("class Enrolment") + "class ".length);
    await editor.edit((e) => e.replace(new vscode.Range(at, at.translate(0, "Enrolment".length)), "Registration"));
    await waitFor("Registration in the diagram", () => view.current!.classifiers.some((c) => c.name === "Registration"));
    // Undo in the diagram undoes in the document.
    view.panel.reveal();
    await waitFor("the diagram to be active", () => view.panel.active);
    await vscode.commands.executeCommand("undo");
    await waitFor("the rename undone", () => !editor.document.getText().includes("Registration"));
    await vscode.commands.executeCommand("undo");
    await waitFor("Room undone", () => !editor.document.getText().includes("class Room"));
    await waitFor("Room gone from the diagram", () => !view.current!.classifiers.some((c) => c.name === "Room"));
    assert.ok(!editor.document.isDirty);
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("draws and edits .ecore files", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "library/library.ecore"));
    await vscode.commands.executeCommand("oxychloride.openDiagram", uri, false);
    const view = await waitFor("the diagram", () => api.diagrams().find((v) => v.document.uri.fsPath === uri.fsPath && v.current));
    assert.strictEqual(view.current!.syntax, "ecore");
    await view.edit({ kind: "update", element: "library::Loan.book", properties: { upper: -1 } });
    assert.ok(view.document.getText().includes('name="book" lowerBound="1" upperBound="-1" eType="#//Book"'), view.document.getText());
    await waitFor("the new multiplicity", () =>
      view.current!.classifiers.find((c) => c.name === "Loan")?.references.some((r) => r.name === "book" && r.upper === -1),
    );
    await vscode.commands.executeCommand("workbench.action.files.revert");
    assert.ok(!view.document.isDirty);
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("draws Henshin rules as diagrams that edit the text, and keeps the .henshin file equivalent", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.henshin_text"));
    const model = vscode.Uri.file(path.join(folder(), "university/university.henshin"));
    const editor = await vscode.window.showTextDocument(uri);
    await vscode.commands.executeCommand("oxychloride.openRuleDiagram", uri);
    const view = await waitFor("the rule diagram", () => api.ruleDiagrams().find((v) => v.document.uri.fsPath === uri.fsPath && v.current));
    assert.strictEqual(view.document, editor.document);
    assert.strictEqual(view.current!.syntax, "text");
    assert.deepStrictEqual(
      view.current!.rules.map((r) => r.id),
      ["enrol", "drop", "grade", "createSeminar", "addSeats", "closeCourse"],
    );
    const enrol = view.current!.rules[0];
    assert.strictEqual(enrol.nodes.find((n) => n.id === "enrol::existing")?.action, "forbid");
    // An edit in the diagram is an edit of the text …
    const result = await view.edit({ kind: "addNode", scope: "drop", name: "campus", class: "University", action: "create" });
    assert.strictEqual(result.element, "drop::campus");
    assert.ok(editor.document.getText().includes("\t\tcreate node campus:University\n"), editor.document.getText());
    await waitFor("campus in the diagram", () => view.current!.rules[1].nodes.some((n) => n.id === "drop::campus"));
    // … and of the .henshin file next to it.
    await api.pairs.settled();
    const henshin = () => vscode.workspace.textDocuments.find((d) => d.uri.fsPath === model.fsPath)?.getText() ?? fs.readFileSync(model.fsPath, "utf8");
    await waitFor("campus in university.henshin", () => henshin().includes('name="campus"'));
    // Saving the text saves the model too.
    await editor.document.save();
    await waitFor("university.henshin saved", () => fs.readFileSync(model.fsPath, "utf8").includes('name="campus"'));
    // A change of the model changes the text.
    const modelDoc = await vscode.workspace.openTextDocument(model);
    const modelEditor = await vscode.window.showTextDocument(modelDoc, vscode.ViewColumn.Beside);
    const at = modelDoc.getText().indexOf('name="campus"') + 'name="'.length;
    await modelEditor.edit((e) => e.replace(new vscode.Range(modelDoc.positionAt(at), modelDoc.positionAt(at + "campus".length)), "site"));
    await waitFor("site in the text", () => editor.document.getText().includes("create node site:University"));
    await api.pairs.settled();
    assert.ok(!editor.document.getText().includes("campus"), editor.document.getText());
    await vscode.commands.executeCommand("workbench.action.files.saveAll");
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("opens .henshin files as rule diagrams", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.henshin"));
    await vscode.commands.executeCommand("vscode.open", uri);
    const view = await waitFor("the rule diagram", () => api.ruleDiagrams().find((v) => v.document.uri.fsPath === uri.fsPath && v.current));
    assert.strictEqual(view.current!.syntax, "model");
    assert.ok(view.current!.rules.some((r) => r.id === "closeCourse"));
    await view.edit({ kind: "setRuleOptions", element: "enrol", injectiveMatching: false });
    assert.ok(view.document.getText().includes('name="enrol" injectiveMatching="false"'), view.document.getText().slice(0, 2000));
    await vscode.commands.executeCommand("workbench.action.files.revert");
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("registers metamodels under URIs that cannot be reached", async () => {
    // orders.xml refers to shop.ecore by nsURI and by a platform: URI (the engine's fixtures).
    const ext = vscode.extensions.getExtension("palladio.oxychloride")!;
    const fixtures = path.join(ext.extensionPath, "../crates/oxychloride/tests/fixtures/registry");
    const dir = path.join(folder(), "shop");
    fs.mkdirSync(path.join(dir, "lib"), { recursive: true });
    fs.copyFileSync(path.join(fixtures, "orders.xml"), path.join(dir, "orders.xml"));
    fs.copyFileSync(path.join(fixtures, "lib/shop.ecore"), path.join(dir, "lib/shop.ecore"));
    const hutn = vscode.Uri.file(path.join(dir, "orders.hutn"));
    fs.writeFileSync(
      hutn.fsPath,
      '@Spec { metamodel "orders" { nsUri: "http://example.org/orders" } }\norders { Order "o" { name: "first" items: Item "i" { price: 3 } } }\n',
    );
    await vscode.window.showTextDocument(hutn);
    await waitFor("the unknown metamodel", () => errors(hutn).some((d) => d.message.includes("no metamodel with nsURI 'http://example.org/orders'")));
    // A quick fix offers to register a file for the URI.
    const range = errors(hutn)[0].range;
    const actions = await vscode.commands.executeCommand<vscode.CodeAction[]>("vscode.executeCodeActionProvider", hutn, range);
    assert.ok(actions.some((a) => a.title === "Register a metamodel file for 'http://example.org/orders'…"), actions.map((a) => a.title).join("\n"));

    const registry = api.registry;
    await registry.register(vscode.Uri.file(path.join(dir, "orders.xml")), "http://example.org/orders");
    await registry.register(vscode.Uri.file(path.join(dir, "lib/shop.ecore")), "http://example.org/shop");
    const info = await registry.register(vscode.Uri.file(path.join(dir, "lib/shop.ecore")), "platform:/plugin/org.example.shop/model/shop.ecore");
    assert.strictEqual(info?.status, "loaded");
    // The setting, with paths relative to the workspace folder.
    const setting = vscode.workspace.getConfiguration("oxychloride", vscode.Uri.file(folder())).get<Record<string, string>>("registry");
    assert.deepStrictEqual(setting, {
      "http://example.org/orders": "shop/orders.xml",
      "http://example.org/shop": "shop/lib/shop.ecore",
      "platform:/plugin/org.example.shop/model/shop.ecore": "shop/lib/shop.ecore",
    });
    // The view: orders.xml imports shop.ecore; nothing is left unresolved.
    const orders = registry.infos.find((i) => i.uri === "http://example.org/orders")!;
    assert.deepStrictEqual(orders.imports.map((r) => path.basename(r.uri)), ["shop.ecore"]);
    assert.deepStrictEqual(orders.warnings, []);
    assert.strictEqual(registry.getChildren().length, 3);
    await waitFor("the model to load", () => errors(hutn).length === 0);
    await waitFor("the packages", () => api.metamodels.packages.some((p) => p.name === "orders"));

    // Unregistering takes the metamodel away again.
    await registry.unregister("http://example.org/orders");
    await waitFor("the unknown metamodel again", () => errors(hutn).some((d) => d.message.includes("no metamodel with nsURI")));
    for (const uri of ["http://example.org/shop", "platform:/plugin/org.example.shop/model/shop.ecore"]) await registry.unregister(uri);
    assert.strictEqual(registry.infos.length, 0);
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("checks, highlights and completes QVT transformations", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "qvt/uml2rdbms.qvto"));
    const editor = await vscode.window.showTextDocument(uri);
    const doc = editor.document;
    assert.strictEqual(doc.languageId, "qvto");
    const at = doc.positionAt(doc.getText().indexOf("self.persistent") + "self.".length);
    await editor.edit((e) => e.insert(at, "X"));
    await waitFor("unknown property", () => errors(uri).some((d) => d.message.includes("unknown property 'Xpersistent'")));
    await vscode.commands.executeCommand("undo");
    await waitFor("no errors", () => errors(uri).length === 0 || undefined);
    // Completion after `self.` proposes the features of the mapping's class.
    const completions = await vscode.commands.executeCommand<vscode.CompletionList>("vscode.executeCompletionItemProvider", uri, at);
    const labels = completions.items.map((i) => (typeof i.label === "string" ? i.label : i.label.label));
    assert.ok(labels.includes("persistent") && labels.includes("attributes"), labels.join(", "));
    const relations = await vscode.workspace.openTextDocument(path.join(folder(), "qvt/uml2rdbms.qvtr"));
    assert.strictEqual(relations.languageId, "qvtr");
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("runs QVT transformations on models and checks models with QVT relations", async () => {
    type Result = { outputs: { parameter: string; objects: number }[]; violations: string[]; problems: unknown[] };
    const qvt = (file: string) => vscode.Uri.file(path.join(folder(), "qvt", file));
    const model = qvt("company.hutn");
    // Operational mappings, from the model (as from its context menu).
    const operational = await vscode.commands.executeCommand<Result>("oxychloride.runQvt", model, {
      transformation: qvt("uml2rdbms.qvto"),
      outputs: { target: qvt("company.qvto.xmi") },
      open: false,
    });
    assert.ok(operational, "the run");
    assert.deepStrictEqual(operational.problems, []);
    assert.strictEqual(operational.outputs[0].objects, 13);
    const written = fs.readFileSync(qvt("company.qvto.xmi").fsPath, "utf8");
    assert.ok(written.includes("<rdbms:Schema"), written);
    // Relations, in the direction of the schema: the same schema.
    const relational = await vscode.commands.executeCommand<Result>("oxychloride.runQvt", model, {
      transformation: qvt("uml2rdbms.qvtr"),
      target: "target",
      check: false,
      outputs: { target: qvt("company.qvtr.xmi") },
      open: false,
    });
    assert.ok(relational, "the run");
    assert.strictEqual(fs.readFileSync(qvt("company.qvtr.xmi").fsPath, "utf8"), written);
    // Checking the written schema against the relations.
    const check = await vscode.commands.executeCommand<Result>("oxychloride.runQvt", model, {
      transformation: qvt("uml2rdbms.qvtr"),
      target: "target",
      check: true,
      models: { target: qvt("company.qvto.xmi") },
      open: false,
    });
    assert.ok(check, "the check");
    assert.deepStrictEqual(check.violations, []);
    await vscode.commands.executeCommand("workbench.action.closeAllEditors");
  });

  it("has icons for its languages", () => {
    const ext = vscode.extensions.getExtension("palladio.oxychloride")!;
    const languages: { id: string; icon?: { light: string; dark: string } }[] = ext.packageJSON.contributes.languages;
    for (const id of ["ocl", "oclinecore", "hutn", "henshin", "qvto", "qvtr"]) {
      const icon = languages.find((l) => l.id === id)?.icon;
      assert.ok(icon, `no icon for ${id}`);
      for (const file of [icon.light, icon.dark]) {
        const svg = fs.readFileSync(path.join(ext.extensionPath, file), "utf8");
        assert.ok(svg.startsWith("<svg"), file);
      }
    }
  });
});
